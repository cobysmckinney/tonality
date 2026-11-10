mod export;
mod history;
mod images;
mod library;
mod sidecar;
mod store;
mod thumbs;

use images::{ImageInfo, Images};
use tauri::ipc::Response;
use tauri::{Manager, State};
use store::Store;
use tonality_core::{Catalog, EditState, ImageId};
use tonality_render::{DevelopParams, GlslProgram};

pub(crate) type CmdResult<T> = Result<T, String>;

/// Long edge of the interactive preview texture.
const PREVIEW_EDGE: usize = 2560;

#[tauri::command]
fn default_edit_state() -> EditState {
    EditState::default()
}

/// Dev convenience: `TONALITY_OPEN=/path/to/file.ARW npm run tauri dev` imports a file and opens it.
#[tauri::command]
fn startup_path() -> Option<String> {
    std::env::var("TONALITY_OPEN").ok().filter(|p| !p.is_empty())
}

/// Dev convenience: `TONALITY_IMPORT=/path/to/folder` imports a folder into the Library on launch.
#[tauri::command]
fn startup_import() -> Option<String> {
    std::env::var("TONALITY_IMPORT").ok().filter(|p| !p.is_empty())
}

/// Dev: frontend console messages, mirrored to the terminal.
#[tauri::command]
fn dev_log(level: String, message: String) {
    eprintln!("[ui:{level}] {message}");
}

/// Dev/test hook: where the frontend should write a PNG of the first rendered frame.
#[tauri::command]
fn dev_snapshot_path() -> Option<String> {
    std::env::var("TONALITY_SNAPSHOT").ok().filter(|p| !p.is_empty())
}

/// Dev/test hook: `TONALITY_ZOOM=1` opens images at that zoom (and snapshots once full-res pixels arrive).
#[tauri::command]
fn dev_zoom() -> Option<f32> {
    std::env::var("TONALITY_ZOOM").ok().and_then(|z| z.parse().ok())
}

/// Dev/test hook (debug builds only): contents of the JS file at `TONALITY_DEV_SCRIPT`, run by
/// the frontend after startup with the app's state objects in scope, to drive the UI in tests.
#[tauri::command]
fn dev_script() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let p = std::env::var("TONALITY_DEV_SCRIPT").ok().filter(|p| !p.is_empty())?;
    std::fs::read_to_string(p).ok()
}

/// Dev/test hook: save a `data:image/png;base64,…` URL to `TONALITY_SNAPSHOT`.
#[tauri::command]
fn dev_save_snapshot(data_url: String, name: Option<String>) -> CmdResult<()> {
    use base64::Engine;
    let mut path = std::path::PathBuf::from(dev_snapshot_path().ok_or("TONALITY_SNAPSHOT not set")?);
    // Named snapshots go next to it: `snap.png` + "crop" → `snap-crop.png`.
    if let Some(n) = name.filter(|n| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        path.set_file_name(format!("{stem}-{n}.png"));
    }
    let b64 = data_url.split_once(',').map(|(_, b)| b).ok_or("bad data url")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

#[tauri::command]
fn preview_shaders() -> CmdResult<GlslProgram> {
    tonality_render::preview_glsl()
}

/// Decode a library photo (off the main thread) and cache its preview.
#[tauri::command]
async fn open_image(
    id: ImageId,
    images: State<'_, Images>,
    store: State<'_, Store>,
    catalog: State<'_, history::CatalogState>,
) -> CmdResult<ImageInfo> {
    let (rel, _, name) = catalog.with(|c| c.image_file(id))?;
    let path = store.abs(&rel);
    let images = images.inner().clone();
    tauri::async_runtime::spawn_blocking(move || images.open(id, &path, name, PREVIEW_EDGE))
        .await
        .map_err(|e| e.to_string())?
}

/// Raw preview pixels: RGB16F little-endian, `width * height * 3` halves.
#[tauri::command]
fn preview_pixels(id: ImageId, images: State<'_, Images>) -> CmdResult<Response> {
    let p = images.get(id).ok_or("image not open")?;
    Ok(Response::new(p.pixel_bytes()))
}

/// A region of the full-resolution image for 1:1 zoom, box-downsampled by `2^level`.
/// Layout: six little-endian u32s (x, y, w, h in full-res pixels; texture width, height), then
/// RGB16F little-endian. Waits for the full-resolution decode if it is still running.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri commands take their arguments flat.
async fn detail_region(
    id: ImageId,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    level: u32,
    images: State<'_, Images>,
    store: State<'_, Store>,
    catalog: State<'_, history::CatalogState>,
) -> CmdResult<Response> {
    let (rel, _, _) = catalog.with(|c| c.image_file(id))?;
    let path = store.abs(&rel);
    let images = images.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let full = images.full(id, &path)?;
        let r = full.region(x, y, w, h, level);
        let mut out = Vec::with_capacity(24 + r.pixels.len() * 2);
        for v in [r.x, r.y, r.w, r.h, r.width, r.height] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for p in &r.pixels {
            out.extend_from_slice(&p.to_le_bytes());
        }
        Ok(Response::new(out))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Tone-curve lookup table for `develop.wgsl`: RGBA16F little-endian, `LUT_SIZE` × 1.
#[tauri::command]
fn develop_curve(edit: EditState) -> Response {
    Response::new(tonality_render::curve_lut_rgba16f(&edit))
}

/// Uniform block for `develop.wgsl`.
#[tauri::command]
fn develop_params(id: ImageId, edit: EditState, images: State<'_, Images>) -> CmdResult<Vec<f32>> {
    let p = images.get(id).ok_or("image not open")?;
    Ok(DevelopParams::new(&edit, &p.color, p.lens.as_ref(), p.full_width, p.full_height).to_vec())
}

/// Brush mask rasters for `develop.wgsl`: three little-endian u32s (width, height, layers),
/// then one 8-bit `width × height` layer per brush mask, in mask order.
#[tauri::command]
fn mask_layers(id: ImageId, edit: EditState, images: State<'_, Images>) -> CmdResult<Response> {
    let p = images.get(id).ok_or("image not open")?;
    let l = tonality_core::mask::brush_layers(&edit.masks, p.full_width, p.full_height);
    let mut out = Vec::with_capacity(12 + l.data.len());
    for v in [l.width, l.height, l.layers] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&l.data);
    Ok(Response::new(out))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Images::default())
        .register_asynchronous_uri_scheme_protocol(thumbs::SCHEME, thumbs::handle)
        .setup(|app| {
            let root = Store::resolve(app.path().picture_dir().ok(), app.path().home_dir().ok());
            let store = Store::open(root.clone()).map_err(|e| format!("cannot open library at {root:?}: {e}"))?;
            let catalog = Catalog::open(store.catalog_path())?;
            eprintln!("[tonality] library: {}", store.root().display());
            app.manage(store);
            app.manage(history::CatalogState::new(catalog));
            app.manage(sidecar::Sidecars::start(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            default_edit_state,
            startup_path,
            startup_import,
            dev_log,
            dev_snapshot_path,
            dev_save_snapshot,
            dev_zoom,
            dev_script,
            preview_shaders,
            open_image,
            preview_pixels,
            detail_region,
            export::export_images,
            develop_params,
            develop_curve,
            mask_layers,
            history::history_open,
            history::history_commit,
            history::history_undo,
            history::history_redo,
            history::history_checkout,
            history::history_switch,
            history::history_branch,
            history::history_rename,
            history::history_delete,
            history::history_apply_from,
            history::history_state,
            thumbs::library_save_edited,
            library::library_overview,
            library::library_import,
            library::library_recover,
            library::library_images,
            library::library_set_rating,
            library::library_set_flag,
            library::album_create,
            library::album_rename,
            library::album_delete,
            library::album_add,
            library::album_remove,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tonality")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                app.state::<sidecar::Sidecars>().flush();
            }
        });
}
