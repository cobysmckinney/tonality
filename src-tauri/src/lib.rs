mod history;
mod images;

use images::{ImageInfo, Images};
use tauri::ipc::Response;
use tauri::{Manager, State};
use tonality_core::{Catalog, EditState};
use tonality_render::{DevelopParams, GlslProgram};

pub(crate) type CmdResult<T> = Result<T, String>;

/// Long edge of the interactive preview texture.
const PREVIEW_EDGE: usize = 2560;

#[tauri::command]
fn default_edit_state() -> EditState {
    EditState::default()
}

/// Dev convenience: `TONALITY_OPEN=/path/to/file.ARW npm run tauri dev` opens a file on launch.
#[tauri::command]
fn startup_path() -> Option<String> {
    std::env::var("TONALITY_OPEN").ok().filter(|p| !p.is_empty())
}

/// Dev/test hook: where the frontend should write a PNG of the first rendered frame.
#[tauri::command]
fn dev_snapshot_path() -> Option<String> {
    std::env::var("TONALITY_SNAPSHOT").ok().filter(|p| !p.is_empty())
}

/// Dev/test hook: save a `data:image/png;base64,…` URL to `TONALITY_SNAPSHOT`.
#[tauri::command]
fn dev_save_snapshot(data_url: String) -> CmdResult<()> {
    use base64::Engine;
    let path = dev_snapshot_path().ok_or("TONALITY_SNAPSHOT not set")?;
    let b64 = data_url.split_once(',').map(|(_, b)| b).ok_or("bad data url")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

#[tauri::command]
fn preview_shaders() -> CmdResult<GlslProgram> {
    tonality_render::preview_glsl()
}

/// Decode a RAW file (off the main thread) and cache its preview.
#[tauri::command]
async fn open_image(path: String, images: State<'_, Images>) -> CmdResult<ImageInfo> {
    let images = images.inner().clone();
    tauri::async_runtime::spawn_blocking(move || images.open(&path, PREVIEW_EDGE))
        .await
        .map_err(|e| e.to_string())?
}

/// Raw preview pixels: RGB16F little-endian, `width * height * 3` halves.
#[tauri::command]
fn preview_pixels(id: String, images: State<'_, Images>) -> CmdResult<Response> {
    let p = images.get(&id).ok_or("image not open")?;
    Ok(Response::new(p.pixel_bytes()))
}

/// Uniform block for `develop.wgsl` (24 floats).
#[tauri::command]
fn develop_params(id: String, edit: EditState, images: State<'_, Images>) -> CmdResult<Vec<f32>> {
    let p = images.get(&id).ok_or("image not open")?;
    Ok(DevelopParams::new(&edit, &p.color).to_vec())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Images::default())
        .setup(|app| {
            let path = match std::env::var_os("TONALITY_CATALOG") {
                Some(p) => p.into(),
                None => app.path().app_data_dir()?.join("catalog.sqlite"),
            };
            let catalog = Catalog::open(&path)?;
            app.manage(history::CatalogState::new(catalog));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            default_edit_state,
            startup_path,
            dev_snapshot_path,
            dev_save_snapshot,
            preview_shaders,
            open_image,
            preview_pixels,
            develop_params,
            history::history_open,
            history::history_commit,
            history::history_undo,
            history::history_redo,
            history::history_checkout,
            history::history_switch,
            history::history_branch,
            history::history_rename,
            history::history_delete,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tonality");
}
