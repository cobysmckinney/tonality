//! Everything the interface can ask the backend to do.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::edit::{Adjustments, Shape};
use crate::export;
use crate::gpu::{self, Region, Session};
use crate::history::History;
use crate::import::{self, ImportSummary, ScanSession, ScanView};
use crate::library::{Library, Overview, PhotoInfo, PhotoItem, View};
use crate::presets::{self, ImportedPresets, Preset, Settings};
use crate::segment::Found;
use crate::thumbs;
use crate::volumes::{self, Volume};

pub struct AppState {
    pub library: Arc<Library>,
    /// The scan currently up for review, if any.
    pub scan: Mutex<Option<Arc<ScanSession>>>,
    pub cancel_import: AtomicBool,
    pub cancel_export: AtomicBool,
    /// Stops a paste or revert across many photos after the photo in hand.
    pub cancel_edits: AtomicBool,
    /// Renders thumbnails and previews without flooding the machine.
    pub renderers: rayon::ThreadPool,
    /// The photo open in the editor, loaded on the GPU.
    pub editing: Mutex<Option<(i64, Session)>>,
}

impl AppState {
    pub fn current_scan(&self, session_id: u64) -> Option<Arc<ScanSession>> {
        self.scan.lock().unwrap().clone().filter(|session| session.id == session_id)
    }
}

type CommandResult<T> = Result<T, String>;

fn message(error: anyhow::Error) -> String {
    format!("{error:#}")
}

#[derive(Clone, Serialize)]
struct Progress {
    done: usize,
    total: usize,
}

// ---- browsing ----

#[tauri::command(async)]
pub fn get_overview(state: State<AppState>) -> CommandResult<Overview> {
    state.library.overview().map_err(message)
}

#[tauri::command(async)]
pub fn list_photos(state: State<AppState>, view: View) -> CommandResult<Vec<PhotoItem>> {
    state.library.list_photos(view).map_err(message)
}

#[tauri::command(async)]
pub fn get_photo_info(state: State<AppState>, id: i64) -> CommandResult<PhotoInfo> {
    state.library.photo_info(id).map_err(message)
}

// ---- marking ----

#[tauri::command(async)]
pub fn set_favorite(state: State<AppState>, ids: Vec<i64>, favorite: bool) -> CommandResult<()> {
    state.library.set_favorite(&ids, favorite).map_err(message)
}

#[tauri::command(async)]
pub fn set_flag(state: State<AppState>, ids: Vec<i64>, flag: i8) -> CommandResult<()> {
    state.library.set_flag(&ids, flag).map_err(message)
}

// ---- deleting ----

#[tauri::command(async)]
pub fn trash_photos(state: State<AppState>, ids: Vec<i64>) -> CommandResult<()> {
    state.library.trash(&ids).map_err(message)
}

#[tauri::command(async)]
pub fn restore_photos(state: State<AppState>, ids: Vec<i64>) -> CommandResult<()> {
    state.library.restore(&ids).map_err(message)
}

#[tauri::command(async)]
pub fn purge_photos(state: State<AppState>, ids: Vec<i64>) -> CommandResult<u32> {
    state.library.purge(&ids).map_err(message)
}

// ---- albums ----

#[tauri::command(async)]
pub fn create_album(state: State<AppState>, name: String, ids: Vec<i64>) -> CommandResult<i64> {
    let album = state.library.create_album(&name).map_err(message)?;
    state.library.add_to_album(album, &ids).map_err(message)?;
    Ok(album)
}

#[tauri::command(async)]
pub fn rename_album(state: State<AppState>, id: i64, name: String) -> CommandResult<()> {
    state.library.rename_album(id, &name).map_err(message)
}

#[tauri::command(async)]
pub fn delete_album(state: State<AppState>, id: i64) -> CommandResult<()> {
    state.library.delete_album(id).map_err(message)
}

#[tauri::command(async)]
pub fn add_to_album(state: State<AppState>, album_id: i64, ids: Vec<i64>) -> CommandResult<()> {
    state.library.add_to_album(album_id, &ids).map_err(message)
}

#[tauri::command(async)]
pub fn remove_from_album(state: State<AppState>, album_id: i64, ids: Vec<i64>) -> CommandResult<()> {
    state.library.remove_from_album(album_id, &ids).map_err(message)
}

// ---- importing ----

#[tauri::command(async)]
pub fn list_volumes() -> Vec<Volume> {
    volumes::list()
}

fn forget_scan(state: &AppState) {
    if let Some(previous) = state.scan.lock().unwrap().take() {
        let _ = std::fs::remove_dir_all(state.library.scan_cache_dir().join(previous.id.to_string()));
    }
}

/// Looks through files and folders for photos and reports which are new.
/// Nothing is copied until `run_import`.
#[tauri::command]
pub async fn scan_import(app: AppHandle, paths: Vec<PathBuf>, source: Option<String>) -> CommandResult<ScanView> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        forget_scan(&state);
        // Time-based so a review sheet never shows thumbnails cached from an earlier run.
        let session_id = chrono::Utc::now().timestamp_millis() as u64;
        let progress = |done, total| {
            let _ = app.emit("scan-progress", Progress { done, total });
        };
        let session = import::scan(&state.library, session_id, &paths, source, &progress).map_err(message)?;
        let view = session.view();
        *state.scan.lock().unwrap() = Some(Arc::new(session));
        Ok(view)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command(async)]
pub fn discard_scan(state: State<AppState>) {
    forget_scan(&state);
}

/// Copies the chosen photos from the reviewed scan into the library.
#[tauri::command]
pub async fn run_import(app: AppHandle, session_id: u64, indices: Vec<usize>) -> CommandResult<ImportSummary> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let session = state.current_scan(session_id).ok_or("This import review has expired. Scan again.")?;
        state.cancel_import.store(false, Ordering::Relaxed);
        let progress = |done, total| {
            let _ = app.emit("import-progress", Progress { done, total });
        };
        let summary =
            import::run(&state.library, &session, &indices, &state.cancel_import, &progress).map_err(message)?;
        forget_scan(&state);
        Ok(summary)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cancel_import(state: State<AppState>) {
    state.cancel_import.store(true, Ordering::Relaxed);
}

// ---- editing ----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorPhoto {
    /// The photo's full size in pixels, before any rotation or crop.
    width: u32,
    height: u32,
    history: History,
}

#[derive(serde::Deserialize)]
pub struct RegionArg {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// Loads a photo at full resolution for editing and returns its edit history.
#[tauri::command]
pub async fn open_editor(app: AppHandle, id: i64) -> CommandResult<EditorPhoto> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let history = state.library.history(id).map_err(message)?;
        let mut editing = state.editing.lock().unwrap();
        if !matches!(&*editing, Some((open, _)) if *open == id) {
            // Let go of the previous photo first; two at once is a lot of memory.
            *editing = None;
            let session = thumbs::open_session(&state.library, id).map_err(message)?;
            // Have found parts ready before the first frame needs them; usually a quick read of the cache.
            let adjustments = Adjustments::from_json(state.library.edits(id).map_err(message)?.as_deref());
            let found = crate::masks::found(&adjustments.masks);
            gpu::shared().map_err(message)?.ensure_found(&session, &found).map_err(message)?;
            *editing = Some((id, session));
        }
        let (_, session) = editing.as_ref().expect("just opened");
        Ok(EditorPhoto { width: session.width, height: session.height, history })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Gets the open photo ready for circles to be drawn on it: the object
/// model's first look at a photo takes a few seconds, and is kept.
#[tauri::command]
pub async fn prepare_circles(app: AppHandle, id: i64) -> CommandResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let picture = {
            let editing = state.editing.lock().unwrap();
            let (_, session) = editing.as_ref().filter(|(open, _)| *open == id).ok_or("This photo is no longer open in the editor.")?;
            session.picture()
        };
        picture.prepare_circles().map_err(message)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Finds what these mask parts pick out of the open photo (its subject, its
/// sky, a circled object), so masks can use them. A second or two each the
/// first time; after that each is kept. Parts drawn by hand are passed over.
#[tauri::command]
pub async fn find_parts(app: AppHandle, id: i64, shapes: Vec<Shape>) -> CommandResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let closed = || "This photo is no longer open in the editor.".to_string();
        // The models run without holding the editor, so frames keep coming meanwhile.
        let (picture, wanted) = {
            let editing = state.editing.lock().unwrap();
            let (_, session) = editing.as_ref().filter(|(open, _)| *open == id).ok_or_else(closed)?;
            let wanted: Vec<_> = shapes
                .iter()
                .filter_map(Found::of)
                .filter(|found| !session.has_matte(found))
                .map(|found| {
                    let path = session.matte_path(&found);
                    (found, path)
                })
                .collect();
            if wanted.is_empty() {
                return Ok(());
            }
            (session.picture(), wanted)
        };
        let mattes = wanted
            .into_iter()
            .map(|(found, path)| Ok((found.find_cached(&picture, path.as_deref())?, found)))
            .collect::<anyhow::Result<Vec<_>>>()
            .map_err(message)?;
        let editing = state.editing.lock().unwrap();
        let (_, session) = editing.as_ref().filter(|(open, _)| *open == id).ok_or_else(closed)?;
        for (matte, found) in mattes {
            session.set_matte(&found, matte);
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command(async)]
pub fn close_editor(state: State<AppState>) {
    *state.editing.lock().unwrap() = None;
}

/// Draws the open photo with a recipe applied.
///
/// The reply is binary: width, height and clipping flags (three u32s and a
/// spare), then a 256-bin histogram each for red, green, blue and brightness
/// (u32s), then the frame as RGBA bytes.
#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    state: State<AppState>,
    id: i64,
    adjustments: Adjustments,
    region: RegionArg,
    width: u32,
    height: u32,
    show_clipping: bool,
    uncropped: bool,
    mask_overlay: Option<u32>,
) -> Result<tauri::ipc::Response, String> {
    let editing = state.editing.lock().unwrap();
    let Some((_, session)) = editing.as_ref().filter(|(open, _)| *open == id) else {
        return Err("This photo is no longer open in the editor.".into());
    };
    let gpu = gpu::shared().map_err(message)?;
    let region = Region { x: region.x, y: region.y, width: region.width, height: region.height };
    let (width, height) = (width.clamp(1, 4096), height.clamp(1, 4096));
    let guides = gpu::Guides { show_clipping, uncropped, mask_overlay, matte: false };
    let frame = gpu.render(session, &adjustments, region, (width, height), guides).map_err(message)?;

    // The histogram always describes the whole photo, whatever the zoom.
    let small = gpu.render_image(session, &adjustments, 256).map_err(message)?;
    let mut histogram = [[0u32; 256]; 4];
    let (mut blown, mut crushed) = (0usize, 0usize);
    for pixel in small.pixels() {
        let [r, g, b] = pixel.0;
        histogram[0][r as usize] += 1;
        histogram[1][g as usize] += 1;
        histogram[2][b as usize] += 1;
        let brightness = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32).round() as usize;
        histogram[3][brightness.min(255)] += 1;
        blown += (r.max(g).max(b) >= 254) as usize;
        crushed += (r.max(g).max(b) <= 1) as usize;
    }
    // A handful of specular highlights is not worth a warning.
    let noticeable = (small.width() * small.height()) as usize / 2000;
    let flags = (blown > noticeable) as u32 | ((crushed > noticeable) as u32) << 1;

    let mut reply = Vec::with_capacity(16 + 4 * 256 * 4 + frame.len());
    for value in [width, height, flags, 0] {
        reply.extend_from_slice(&value.to_le_bytes());
    }
    for bin in histogram.iter().flatten() {
        reply.extend_from_slice(&bin.to_le_bytes());
    }
    reply.extend_from_slice(&frame);
    Ok(tauri::ipc::Response::new(reply))
}

/// Each mask's coverage of the open photo, as a small black-and-white picture
/// of the cropped frame: white where the mask applies.
///
/// The reply is binary: width, height and the number of masks (three u32s
/// and a spare), then one byte a pixel for each mask in turn.
#[tauri::command(async)]
pub fn mask_mattes(
    state: State<AppState>,
    id: i64,
    adjustments: Adjustments,
    long_edge: u32,
) -> Result<tauri::ipc::Response, String> {
    let editing = state.editing.lock().unwrap();
    let Some((_, session)) = editing.as_ref().filter(|(open, _)| *open == id) else {
        return Err("This photo is no longer open in the editor.".into());
    };
    let gpu = gpu::shared().map_err(message)?;
    let (width, height) = gpu::picture_size(session.width, session.height, &adjustments, long_edge.clamp(8, 512));
    let mut reply = Vec::new();
    for value in [width, height, adjustments.masks.len() as u32, 0] {
        reply.extend_from_slice(&value.to_le_bytes());
    }
    for mask in &adjustments.masks {
        let guides = gpu::Guides { mask_overlay: Some(mask.id), matte: true, ..Default::default() };
        let rgba = gpu.render(session, &adjustments, Region::FULL, (width, height), guides).map_err(message)?;
        reply.extend(rgba.as_chunks::<4>().0.iter().map(|pixel| pixel[0]));
    }
    Ok(tauri::ipc::Response::new(reply))
}

/// The open photo with each preset laid over its edits, small, for choosing between them.
///
/// The reply is binary: width, height and the number of presets (three u32s
/// and a spare), then for each preset its id (an i64) and its picture as RGBA bytes.
#[tauri::command(async)]
pub fn preset_previews(
    state: State<AppState>,
    id: i64,
    adjustments: Adjustments,
    long_edge: u32,
) -> Result<tauri::ipc::Response, String> {
    let presets = state.library.presets().map_err(message)?;
    let editing = state.editing.lock().unwrap();
    let Some((_, session)) = editing.as_ref().filter(|(open, _)| *open == id) else {
        return Err("This photo is no longer open in the editor.".into());
    };
    let gpu = gpu::shared().map_err(message)?;
    // Presets never touch the framing, so every preview is the same size.
    let (width, height) = gpu::picture_size(session.width, session.height, &adjustments, long_edge.clamp(16, 512));
    let mut reply = Vec::new();
    for value in [width, height, presets.len() as u32, 0] {
        reply.extend_from_slice(&value.to_le_bytes());
    }
    for preset in &presets {
        let recipe = presets::apply(&preset.settings, &adjustments);
        let rgba = gpu.render(session, &recipe, Region::FULL, (width, height), gpu::Guides::default()).map_err(message)?;
        reply.extend_from_slice(&preset.id.to_le_bytes());
        reply.extend_from_slice(&rgba);
    }
    Ok(tauri::ipc::Response::new(reply))
}

/// Redraws each photo's thumbnail and preview to match its current recipe.
/// Returns the photos' new versions, in order.
fn redraw(state: &AppState, ids: &[i64]) -> anyhow::Result<Vec<i64>> {
    let mut versions = Vec::with_capacity(ids.len());
    for &id in ids {
        let adjustments = Adjustments::from_json(state.library.edits(id)?.as_deref());
        if adjustments.is_default() {
            // Back to as shot: the camera's own JPEG serves again.
            thumbs::clear_rendered(&state.library, id);
        } else {
            // Draw under the lock, encode after it: a redraw must not stall the sliders.
            let on_screen = match state.editing.lock().unwrap().as_ref() {
                Some((open, session)) if *open == id => Some(thumbs::EditedImages::draw(session, &adjustments)?),
                _ => None,
            };
            let images = match on_screen {
                Some(images) => images,
                // Not the photo on screen (edits pasted in the grid): load it just for this.
                None => thumbs::EditedImages::draw(&thumbs::open_session(&state.library, id)?, &adjustments)?,
            };
            images.write(&state.library, id)?;
        }
        versions.push(state.library.bump_version(id)?);
    }
    Ok(versions)
}

/// Brings thumbnails up to date after edits. The editor calls this once the
/// sliders have been still for a moment, not on every change.
#[tauri::command]
pub async fn refresh_rendered(app: AppHandle, ids: Vec<i64>) -> CommandResult<Vec<i64>> {
    tauri::async_runtime::spawn_blocking(move || redraw(&app.state::<AppState>(), &ids).map_err(message))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedEdits {
    /// The new versions of the photos that were done: the first so many of
    /// the ones asked for, in order.
    versions: Vec<i64>,
    /// True if it was stopped before reaching every photo.
    cancelled: bool,
}

/// Changes several photos' recipes, as a step in each one's history.
/// `change` is given a photo's own recipe and returns what it becomes.
///
/// Photos are finished one at a time, each recorded and redrawn before the
/// next is touched, with an `edits-progress` event after each. That way
/// stopping part-way leaves every photo either done or untouched.
fn edit_each(
    app: &AppHandle,
    ids: &[i64],
    label: &str,
    change: impl Fn(&Adjustments) -> Adjustments,
) -> CommandResult<AppliedEdits> {
    let state = app.state::<AppState>();
    state.cancel_edits.store(false, Ordering::Relaxed);
    let mut versions = Vec::with_capacity(ids.len());
    for &id in ids {
        if state.cancel_edits.load(Ordering::Relaxed) {
            return Ok(AppliedEdits { versions, cancelled: true });
        }
        let own = Adjustments::from_json(state.library.edits(id).map_err(message)?.as_deref());
        state.library.history_commit(id, &change(&own), label).map_err(message)?;
        versions.extend(redraw(&state, &[id]).map_err(message)?);
        let _ = app.emit("edits-progress", Progress { done: versions.len(), total: ids.len() });
    }
    Ok(AppliedEdits { versions, cancelled: false })
}

/// Applies one recipe to several photos (pasting edits, or reverting to the
/// original, from the grid). With `keep_crop` each photo keeps its own crop,
/// straightening, turns and flips.
#[tauri::command]
pub async fn apply_edits(
    app: AppHandle,
    ids: Vec<i64>,
    adjustments: Adjustments,
    label: String,
    keep_crop: bool,
) -> CommandResult<AppliedEdits> {
    tauri::async_runtime::spawn_blocking(move || {
        edit_each(&app, &ids, &label, |own| {
            let mut recipe = adjustments.clone();
            if keep_crop {
                // Pasted edits bring the look, not the framing.
                recipe.keep_framing_of(own);
            }
            recipe
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Lays a preset over the edits of several photos, from the grid. Each
/// photo keeps the settings the preset does not cover.
#[tauri::command]
pub async fn apply_preset(app: AppHandle, ids: Vec<i64>, preset_id: i64) -> CommandResult<AppliedEdits> {
    tauri::async_runtime::spawn_blocking(move || {
        let preset = app.state::<AppState>().library.preset(preset_id).map_err(message)?;
        edit_each(&app, &ids, &format!("Preset: {}", preset.name), |own| presets::apply(&preset.settings, own))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cancel_edits(state: State<AppState>) {
    state.cancel_edits.store(true, Ordering::Relaxed);
}

/// A photo's saved recipe, for copying its edits from the grid.
#[tauri::command(async)]
pub fn get_edits(state: State<AppState>, id: i64) -> CommandResult<Adjustments> {
    Ok(Adjustments::from_json(state.library.edits(id).map_err(message)?.as_deref()))
}

// ---- presets ----

#[tauri::command(async)]
pub fn list_presets(state: State<AppState>) -> CommandResult<Vec<Preset>> {
    state.library.presets().map_err(message)
}

#[tauri::command(async)]
pub fn create_preset(state: State<AppState>, name: String, settings: Settings) -> CommandResult<Preset> {
    state.library.create_preset(&name, settings).map_err(message)
}

#[tauri::command(async)]
pub fn rename_preset(state: State<AppState>, id: i64, name: String) -> CommandResult<Preset> {
    state.library.rename_preset(id, &name).map_err(message)
}

#[tauri::command(async)]
pub fn update_preset(state: State<AppState>, id: i64, settings: Settings) -> CommandResult<Preset> {
    state.library.update_preset(id, settings).map_err(message)
}

#[tauri::command(async)]
pub fn favorite_presets(state: State<AppState>) -> CommandResult<Vec<i64>> {
    state.library.favorite_presets().map_err(message)
}

#[tauri::command(async)]
pub fn set_preset_favorite(state: State<AppState>, id: i64, favorite: bool) -> CommandResult<Vec<i64>> {
    state.library.set_preset_favorite(id, favorite).map_err(message)
}

#[tauri::command(async)]
pub fn delete_preset(state: State<AppState>, id: i64) -> CommandResult<()> {
    state.library.delete_preset(id).map_err(message)
}

#[tauri::command(async)]
pub fn export_preset(state: State<AppState>, id: i64, path: PathBuf) -> CommandResult<()> {
    state.library.export_preset(id, &path).map_err(message)
}

#[tauri::command(async)]
pub fn import_presets(state: State<AppState>, paths: Vec<PathBuf>) -> CommandResult<ImportedPresets> {
    state.library.import_presets(&paths).map_err(message)
}

// ---- history ----

#[tauri::command(async)]
pub fn get_history(state: State<AppState>, id: i64) -> CommandResult<History> {
    state.library.history(id).map_err(message)
}

#[tauri::command(async)]
pub fn history_commit(state: State<AppState>, id: i64, adjustments: Adjustments, label: String) -> CommandResult<History> {
    state.library.history_commit(id, &adjustments, &label).map_err(message)
}

#[tauri::command(async)]
pub fn history_goto(state: State<AppState>, id: i64, step_id: i64) -> CommandResult<History> {
    state.library.history_goto(id, step_id).map_err(message)
}

#[tauri::command(async)]
pub fn history_branch(state: State<AppState>, id: i64, step_id: i64, name: String) -> CommandResult<History> {
    state.library.history_branch(id, step_id, &name).map_err(message)
}

#[tauri::command(async)]
pub fn history_switch(state: State<AppState>, id: i64, branch_id: i64) -> CommandResult<History> {
    state.library.history_switch(id, branch_id).map_err(message)
}

#[tauri::command(async)]
pub fn history_rename_branch(state: State<AppState>, id: i64, branch_id: i64, name: String) -> CommandResult<History> {
    state.library.history_rename_branch(id, branch_id, &name).map_err(message)
}

#[tauri::command(async)]
pub fn history_delete_branch(state: State<AppState>, id: i64, branch_id: i64) -> CommandResult<History> {
    state.library.history_delete_branch(id, branch_id).map_err(message)
}

// ---- exporting ----

/// What an export job would write, before anything is written: the folder,
/// the file names and sizes, and the branch of edits each comes from.
/// Without `settings`, the ones used last time.
#[tauri::command(async)]
pub fn plan_export(
    state: State<AppState>,
    job: export::Job,
    settings: Option<export::Settings>,
) -> CommandResult<export::Plan> {
    export::plan(&state.library, &job, settings.as_ref()).map_err(message)
}

/// Writes the job's pictures out as image files, as `plan_export` described.
#[tauri::command]
pub async fn run_export(app: AppHandle, job: export::Job, settings: export::Settings) -> CommandResult<export::Summary> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state.cancel_export.store(false, Ordering::Relaxed);
        // A photo that is not in the editor is loaded here, and kept while
        // the files that follow are of the same photo (its other branches).
        let loaded: RefCell<Option<(i64, Session)>> = RefCell::new(None);
        let draw = |id: i64, recipe: &Adjustments, long_edge: u32, deep: bool| {
            let gpu = gpu::shared()?;
            let render = |session: &Session| -> anyhow::Result<image::DynamicImage> {
                Ok(match deep {
                    true => gpu.render_deep_image(session, recipe, long_edge)?.into(),
                    false => gpu.render_image(session, recipe, long_edge)?.into(),
                })
            };
            // The photo in the editor is on the GPU already.
            let on_screen = match state.editing.lock().unwrap().as_ref() {
                Some((open, session)) if *open == id => Some(render(session)?),
                _ => None,
            };
            if let Some(image) = on_screen {
                return Ok(image);
            }
            let mut loaded = loaded.borrow_mut();
            if !matches!(&*loaded, Some((have, _)) if *have == id) {
                // Let go of the last photo first; two at once is a lot of memory.
                *loaded = None;
                *loaded = Some((id, thumbs::open_session(&state.library, id)?));
            }
            render(&loaded.as_ref().expect("just loaded").1)
        };
        let progress = |done, total| {
            let _ = app.emit("export-progress", Progress { done, total });
        };
        export::run(&state.library, &job, &settings, &draw, &state.cancel_export, &progress).map_err(message)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cancel_export(state: State<AppState>) {
    state.cancel_export.store(true, Ordering::Relaxed);
}
