mod commands;
pub mod develop;
pub mod edit;
pub mod export;
pub mod geometry;
pub mod gpu;
pub mod history;
pub mod import;
pub mod library;
pub mod media;
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
            commands::refresh_rendered,
            commands::apply_edits,
            commands::cancel_edits,
            commands::get_edits,
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
