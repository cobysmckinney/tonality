mod ai;
mod cancel;
mod export;
pub mod formats;
mod history;
mod images;
mod jpeg;
mod layers;
mod library;
mod logging;
mod priority;
mod sidecar;
mod store;
mod thumbs;
mod updates;

use images::{ImageInfo, Images};
use store::Store;
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, State};
use tonality_core::{Catalog, EditState, ImageId};
use tonality_render::{DevelopParams, GlslProgram};

pub(crate) type CmdResult<T> = Result<T, String>;

/// Long edge of the interactive preview texture.
const PREVIEW_EDGE: usize = 2560;

#[tauri::command]
fn default_edit_state() -> EditState {
    EditState::default()
}

/// The white balance presets, in display order, with the slider values each sets.
#[derive(serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WbPresetInfo {
    pub preset: tonality_core::WbPreset,
    pub label: String,
    pub temperature: f32,
    pub tint: f32,
}

#[tauri::command]
fn wb_presets() -> Vec<WbPresetInfo> {
    tonality_core::WbPreset::ALL
        .iter()
        .filter_map(|&p| {
            let (temperature, tint) = p.values()?;
            Some(WbPresetInfo {
                preset: p,
                label: p.label().into(),
                temperature,
                tint,
            })
        })
        .collect()
}

/// The open photo's preview sampled inside `edit`'s crop, in camera RGB and lens-corrected as
/// the editor shows it (for Auto).
fn auto_samples(p: &tonality_raw::Preview, edit: &EditState) -> tonality_core::auto::Samples {
    let lens = tonality_core::lens::lens_table(
        p.lens.as_ref(),
        &edit.lens,
        p.full_width as f32,
        p.full_height as f32,
    );
    let px = |x: u32, y: u32| {
        let i = ((y * p.width + x) * 3) as usize;
        [0, 1, 2].map(|k| half::f16::from_bits(p.pixels[i + k]).to_f32())
    };
    tonality_core::auto::sample(
        p.width,
        p.height,
        px,
        &edit.crop,
        &lens,
        p.full_width,
        p.full_height,
        192,
    )
}

/// Auto Light for the open photo, as `edit` has it (its white balance and crop).
#[tauri::command]
async fn auto_tone(
    id: ImageId,
    edit: EditState,
    images: State<'_, Images>,
) -> CmdResult<tonality_core::edit::Light> {
    let p = images.get(id).ok_or("image not open")?;
    blocking(move || {
        let cam = auto_samples(&p, &edit);
        let wb = p.color.wb_multipliers(&edit.white_balance);
        Ok(tonality_core::auto::tone(&cam, &p.color, wb))
    })
    .await
}

/// Auto white balance for the open photo: `[temperature, tint]`, or `None` when the photo has
/// too little usable (nearly all clipped or black).
#[tauri::command]
async fn auto_white_balance(
    id: ImageId,
    edit: EditState,
    images: State<'_, Images>,
) -> CmdResult<Option<[f32; 2]>> {
    let p = images.get(id).ok_or("image not open")?;
    blocking(move || {
        let cam = auto_samples(&p, &edit);
        Ok(tonality_core::auto::white_balance(&cam.cam, &p.color).map(|(t, tint)| [t, tint]))
    })
    .await
}

/// White balance that makes the spot at frame position `at` (0..1 across the photo as `edit`
/// frames it) neutral grey: `[temperature, tint]`, read from the camera data of a 5 × 5 pixel
/// patch of the preview. `None` when the spot is outside the photo, clipped or black.
#[tauri::command]
async fn pick_white_balance(
    id: ImageId,
    edit: EditState,
    at: [f32; 2],
    images: State<'_, Images>,
) -> CmdResult<Option<[f32; 2]>> {
    let p = images.get(id).ok_or("image not open")?;
    blocking(move || {
        let lens = tonality_core::lens::lens_table(
            p.lens.as_ref(),
            &edit.lens,
            p.full_width as f32,
            p.full_height as f32,
        );
        let px = |x: u32, y: u32| {
            let i = ((y * p.width + x) * 3) as usize;
            [0, 1, 2].map(|k| half::f16::from_bits(p.pixels[i + k]).to_f32())
        };
        let cam = tonality_core::auto::sample_around(
            p.width,
            p.height,
            px,
            &edit.crop,
            &lens,
            p.full_width,
            p.full_height,
            at,
            5,
        );
        Ok(tonality_core::auto::white_balance_at(&cam.cam, &p.color).map(|(t, tint)| [t, tint]))
    })
    .await
}

/// Dev convenience: `TONALITY_OPEN=/path/to/file.ARW npm run tauri dev` imports a file and opens it.
#[tauri::command]
fn startup_path() -> Option<String> {
    std::env::var("TONALITY_OPEN")
        .ok()
        .filter(|p| !p.is_empty())
}

/// Dev convenience: `TONALITY_IMPORT=/path/to/folder` imports a folder into the Library on launch.
#[tauri::command]
fn startup_import() -> Option<String> {
    std::env::var("TONALITY_IMPORT")
        .ok()
        .filter(|p| !p.is_empty())
}

/// Dev/test hook: where the frontend should write a PNG of the first rendered frame.
#[tauri::command]
fn dev_snapshot_path() -> Option<String> {
    // Debug builds only: in a release build the page can't write files through this.
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("TONALITY_SNAPSHOT")
        .ok()
        .filter(|p| !p.is_empty())
}

/// Dev/test hook: `TONALITY_ZOOM=1` opens images at that zoom (and snapshots once full-res pixels arrive).
#[tauri::command]
fn dev_zoom() -> Option<f32> {
    std::env::var("TONALITY_ZOOM")
        .ok()
        .and_then(|z| z.parse().ok())
}

/// Dev/test hook (debug builds only): contents of the JS file at `TONALITY_DEV_SCRIPT`, run by
/// the frontend after startup with the app's state objects in scope, to drive the UI in tests.
#[tauri::command]
fn dev_script() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let p = std::env::var("TONALITY_DEV_SCRIPT")
        .ok()
        .filter(|p| !p.is_empty())?;
    std::fs::read_to_string(p).ok()
}

/// Dev/test hook: save a `data:image/png;base64,…` URL to `TONALITY_SNAPSHOT`.
#[tauri::command]
fn dev_save_snapshot(data_url: String, name: Option<String>) -> CmdResult<()> {
    use base64::Engine;
    let mut path =
        std::path::PathBuf::from(dev_snapshot_path().ok_or("TONALITY_SNAPSHOT not set")?);
    // Named snapshots go next to it: `snap.png` + "crop" → `snap-crop.png`.
    if let Some(n) = name.filter(|n| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        path.set_file_name(format!("{stem}-{n}.png"));
    }
    let b64 = data_url
        .split_once(',')
        .map(|(_, b)| b)
        .ok_or("bad data url")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

#[tauri::command]
async fn preview_shaders() -> CmdResult<GlslProgram> {
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
    let (rel, _, name) = catalog.read(move |c| c.image_file(id)).await?;
    let path = store.abs(&rel);
    let images = images.inner().clone();
    tauri::async_runtime::spawn_blocking(move || images.open(id, &path, name, PREVIEW_EDGE))
        .await
        .map_err(|e| e.to_string())?
}

/// Decode a library photo to show next to the one being edited (see `Images::open_reference`).
#[tauri::command]
async fn open_reference(
    id: ImageId,
    images: State<'_, Images>,
    store: State<'_, Store>,
    catalog: State<'_, history::CatalogState>,
) -> CmdResult<ImageInfo> {
    let (rel, _, name) = catalog.read(move |c| c.image_file(id)).await?;
    let path = store.abs(&rel);
    let images = images.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        images.open_reference(id, &path, name, PREVIEW_EDGE)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn close_reference(id: ImageId, images: State<'_, Images>) {
    images.close_reference(id);
}

/// Raw preview pixels: RGB16F little-endian, `width * height * 3` halves.
#[tauri::command]
async fn preview_pixels(id: ImageId, images: State<'_, Images>) -> CmdResult<Response> {
    let p = images.get(id).ok_or("image not open")?;
    blocking(move || Ok(Response::new(p.pixel_bytes()))).await
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
    let (rel, _, _) = catalog.read(move |c| c.image_file(id)).await?;
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
async fn develop_curve(edit: EditState) -> Response {
    Response::new(tonality_render::curve_lut_rgba16f(&edit))
}

/// Uniform block for `develop.wgsl`.
#[tauri::command]
async fn develop_params(
    id: ImageId,
    edit: EditState,
    images: State<'_, Images>,
) -> CmdResult<Vec<f32>> {
    let p = images.get(id).ok_or("image not open")?;
    let mut params = DevelopParams::new(
        &edit,
        &p.color,
        p.lens.as_ref(),
        p.full_width,
        p.full_height,
    );
    if tonality_render::presence::uses_maps(&edit) {
        let images = images.inner().clone();
        let maps = blocking(move || images.presence(id)).await?;
        params = params.with_presence(&maps);
    }
    Ok(params.to_vec())
}

/// Clarity / Dehaze maps for `develop.wgsl`: two little-endian u32s (width, height), then
/// RGBA16F little-endian.
#[tauri::command]
async fn presence_maps(id: ImageId, images: State<'_, Images>) -> CmdResult<Response> {
    let images = images.inner().clone();
    blocking(move || {
        let m = images.presence(id)?;
        let mut out = Vec::with_capacity(8 + m.data.len() * 8);
        out.extend_from_slice(&m.width.to_le_bytes());
        out.extend_from_slice(&m.height.to_le_bytes());
        out.extend_from_slice(&m.rgba16f());
        Ok(Response::new(out))
    })
    .await
}

/// Raster mask layers (brush and AI) for `develop.wgsl`. Layout, little-endian u32s: width,
/// height, layers, then the number of AI masks whose raster isn't generated yet and their
/// indices; then one 8-bit `width × height` layer per raster mask, in mask order.
#[tauri::command]
async fn mask_layers(id: ImageId, edit: EditState, app: AppHandle) -> CmdResult<Response> {
    blocking(move || {
        let p = app.state::<Images>().get(id).ok_or("image not open")?;
        let (_, hash, _) = app
            .state::<history::CatalogState>()
            .with(|c| c.image_file(id))?;
        let l = ai::layers(
            &app,
            &hash,
            &edit,
            p.full_width,
            p.full_height,
            p.lens.as_ref(),
        );
        let mut out = Vec::with_capacity(16 + 4 * l.missing.len() + l.data.len());
        for v in [l.width, l.height, l.layers, l.missing.len() as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for &i in &l.missing {
            out.extend_from_slice(&(i as u32).to_le_bytes());
        }
        out.extend_from_slice(&l.data);
        Ok(Response::new(out))
    })
    .await
}

/// Run blocking work (pixels, files, the catalog) on the blocking pool, off the async workers.
pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> CmdResult<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

/// The library's lock file ([`Store::lock`]), kept open while the app runs.
struct LibraryLock(#[allow(dead_code)] std::fs::File);

/// Why the library couldn't be opened at launch, when it couldn't: it's open in another
/// tonality, its folder can't be made or read, or its catalog was written by a newer version.
/// The window then shows this and nothing else, since nothing of the library is there for the
/// rest of the app to use.
struct StartupError(Option<String>);

#[tauri::command]
fn startup_error(error: tauri::State<'_, StartupError>) -> Option<String> {
    error.0.clone()
}

/// Close the app, from the screen that shows a [`StartupError`].
#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

/// Open the library and everything that hangs off it.
fn open_library(app: &tauri::App) -> Result<(), String> {
    let root = Store::resolve(app.path().picture_dir().ok(), app.path().home_dir().ok());
    let at = root.display().to_string();
    let store =
        Store::open(root).map_err(|e| format!("Couldn’t open the library at {at}: {e}."))?;
    // Before anything reads or writes it; held for the app's lifetime.
    let lock = store.lock().map_err(|e| match e.kind() {
        std::io::ErrorKind::WouldBlock => {
            format!("The library at {at} is already open in another tonality window.")
        }
        _ => format!("Couldn’t open the library at {at}: {e}."),
    })?;
    let mut catalog = Catalog::open(store.catalog_path())
        .map_err(|e| format!("Couldn’t open the library at {at}: {e}"))?;
    app.manage(LibraryLock(lock));
    thumbs::backfill_aspects(&store, &mut catalog);
    log::info!("library: {}", store.root().display());
    app.manage(store);
    app.manage(history::CatalogState::new(catalog));
    app.manage(sidecar::Sidecars::start(app.handle().clone()));
    app.manage(updates::Updates::load(app.handle()));
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Images::default())
        .manage(ai::Detector::default())
        .manage(layers::LayerCache::default())
        .register_asynchronous_uri_scheme_protocol(thumbs::SCHEME, thumbs::handle)
        .setup(|app| {
            // First, so that anything going wrong below is on record.
            match app.path().app_log_dir() {
                Ok(dir) => {
                    let version = app.package_info().version.to_string();
                    if let Err(e) = logging::init(&dir, &version) {
                        eprintln!("no log file in {}: {e}", dir.display());
                    }
                }
                Err(e) => eprintln!("no log file: {e}"),
            }
            // A library that can't be opened is said on screen. Failing here instead would
            // end the app before it has a window, with the reason only in the log.
            let failed = open_library(app).err();
            if let Some(why) = &failed {
                log::error!("{why}");
            }
            app.manage(StartupError(failed));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            default_edit_state,
            wb_presets,
            auto_tone,
            auto_white_balance,
            pick_white_balance,
            startup_path,
            startup_import,
            startup_error,
            quit,
            logging::ui_log,
            logging::log_path,
            dev_snapshot_path,
            dev_save_snapshot,
            dev_zoom,
            dev_script,
            preview_shaders,
            open_image,
            open_reference,
            close_reference,
            preview_pixels,
            detail_region,
            export::export_images,
            export::export_cancel,
            develop_params,
            presence_maps,
            develop_curve,
            mask_layers,
            ai::ai_models,
            ai::ai_model_for,
            ai::ai_detect,
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
            history::history_merge_preview,
            history::history_state,
            history::history_head,
            history::history_changes,
            history::history_reset,
            thumbs::library_save_edited,
            library::library_overview,
            library::library_import,
            library::library_import_cancel,
            library::library_recover,
            library::library_remove,
            library::library_images,
            library::library_set_rating,
            library::library_set_flag,
            library::album_create,
            library::album_rename,
            library::album_delete,
            library::album_add,
            library::album_remove,
            updates::settings_get,
            updates::settings_set,
            updates::app_version,
            updates::update_check,
            updates::update_download,
            updates::update_install,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tonality")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // Not there when the library couldn't be opened.
                if let Some(sidecars) = app.try_state::<sidecar::Sidecars>() {
                    if let Err(e) = sidecars.flush() {
                        log::warn!("sidecars not saved on exit: {e}");
                    }
                }
            }
        });
}
