mod commands;
pub mod develop;
pub mod edit;
pub mod export;
pub mod geometry;
pub mod gpu;
pub mod history;
pub mod import;
pub mod library;
pub mod masks;
pub mod media;
pub mod presets;
pub mod segment;
pub mod thumbs;
pub mod volumes;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use tauri::http::{header, Response, StatusCode};
use tauri::{Emitter, Manager};

use commands::AppState;
use library::Library;

/// Resolves a `photo://` request to the image file that answers it:
/// `thumb/<id>`, `preview/<id>`, or `scan/<session>/<index>` for a photo
/// under review that is not in the library yet.
fn resolve_image(state: &AppState, path: &str) -> Result<PathBuf> {
    let mut parts = path.trim_matches('/').split('/');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("thumb"), Some(id), None, None) => thumbs::ensure_thumb(&state.library, id.parse()?),
        (Some("preview"), Some(id), None, None) => thumbs::ensure_preview(&state.library, id.parse()?),
        (Some("scan"), Some(session), Some(index), None) => {
            let session = state.current_scan(session.parse()?).context("this scan is no longer open")?;
            thumbs::ensure_scan_thumb(&state.library, &session, index.parse()?)
        }
        _ => bail!("unknown image request: {path}"),
    }
}

fn image_response(state: &AppState, path: &str) -> Response<Vec<u8>> {
    let image = resolve_image(state, path).and_then(|file| Ok(std::fs::read(file)?));
    let response = Response::builder().header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    match image {
        Ok(bytes) => response
            .header(header::CONTENT_TYPE, "image/jpeg")
            .header(header::CACHE_CONTROL, "private, max-age=3600")
            .body(bytes),
        Err(error) => response
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(format!("{error:#}").into_bytes()),
    }
    .expect("static headers are valid")
}

fn library_root(app: &tauri::App) -> Result<PathBuf> {
    // TONALITY_LIBRARY points the app at another library, e.g. a scratch one for development.
    match std::env::var_os("TONALITY_LIBRARY") {
        Some(path) => Ok(PathBuf::from(path)),
        None => Ok(app.path().picture_dir().context("finding the Pictures folder")?.join("Tonality")),
    }
}

/// Development aid: runs each `.js` file that appears in `folder` in the
/// window, then deletes it. Lets a script drive what keyboard shortcuts
/// can't reach, such as dragging on the photo.
#[cfg(debug_assertions)]
fn run_scripts_from(app: tauri::AppHandle, folder: PathBuf) {
    std::thread::spawn(move || loop {
        let mut scripts: Vec<PathBuf> = std::fs::read_dir(&folder)
            .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "js")).collect())
            .unwrap_or_default();
        scripts.sort();
        for script in scripts {
            if let (Ok(code), Some(window)) = (std::fs::read_to_string(&script), app.webview_windows().into_values().next()) {
                let _ = window.eval(code);
            }
            let _ = std::fs::remove_file(&script);
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    media::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .register_asynchronous_uri_scheme_protocol("photo", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_string();
            let worker = app.clone();
            app.state::<AppState>().renderers.spawn(move || {
                responder.respond(image_response(&worker.state::<AppState>(), &path));
            });
        })
        .setup(|app| {
            let library = Arc::new(Library::open(&library_root(app)?)?);
            library.purge_expired()?;
            app.manage(AppState {
                library,
                scan: Mutex::new(None),
                cancel_import: AtomicBool::new(false),
                cancel_export: AtomicBool::new(false),
                cancel_edits: AtomicBool::new(false),
                renderers: rayon::ThreadPoolBuilder::new().build()?,
                editing: Mutex::new(None),
            });
            let handle = app.handle().clone();
            volumes::watch(move |cards| {
                let _ = handle.emit("volumes-changed", cards);
            });
            #[cfg(debug_assertions)]
            if let Some(folder) = std::env::var_os("TONALITY_EVAL") {
                run_scripts_from(app.handle().clone(), PathBuf::from(folder));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_overview,
            commands::list_photos,
            commands::get_photo_info,
            commands::set_favorite,
            commands::set_flag,
            commands::trash_photos,
            commands::restore_photos,
            commands::purge_photos,
            commands::create_album,
            commands::rename_album,
            commands::delete_album,
            commands::add_to_album,
            commands::remove_from_album,
            commands::list_volumes,
            commands::scan_import,
            commands::discard_scan,
            commands::run_import,
            commands::cancel_import,
            commands::open_editor,
            commands::close_editor,
            commands::render_frame,
            commands::mask_mattes,
            commands::find_parts,
            commands::prepare_circles,
            commands::refresh_rendered,
            commands::apply_edits,
            commands::cancel_edits,
            commands::get_edits,
            commands::list_presets,
            commands::create_preset,
            commands::rename_preset,
            commands::update_preset,
            commands::delete_preset,
            commands::favorite_presets,
            commands::set_preset_favorite,
            commands::preset_previews,
            commands::export_preset,
            commands::import_presets,
            commands::apply_preset,
            commands::get_history,
            commands::history_commit,
            commands::history_goto,
            commands::history_branch,
            commands::history_switch,
            commands::history_rename_branch,
            commands::history_delete_branch,
            commands::plan_export,
            commands::run_export,
            commands::cancel_export,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
