//! AI masks: cached model rasters and turning them into mask layers.
//!
//! A model raster is the model's answer for one photo and one target, at
//! [`mask_size`](tonality_core::mask::mask_size), in the *uncorrected* source (how the model saw
//! it). It lives in `cache/masks/` and can always be regenerated from the photo, so edits only
//! record what was asked for (`MaskShape::Ai`), never the pixels.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tonality_core::color::CameraColor;
use tonality_core::lens::{lens_table, LensCorrection, LensProfile, UvToSource};
use tonality_core::mask::{mask_size, AiTarget, BrushLayers, Mask, MaskShape};
use tonality_core::{EditState, ImageId};
use tonality_ml::registry::{self, Model};
use tonality_ml::session::Tensor32;
use tonality_ml::{download, input, refine, sam, scene, subject};
use ts_rs::TS;

use crate::history::CatalogState;
use crate::images::Images;
use crate::layers::LayerCache;
use crate::store::Store;
use crate::CmdResult;

/// Where the raster for `mask` of the photo with contents `hash` is cached.
pub fn raster_path(store: &Store, hash: &str, model: &str, key: &str) -> PathBuf {
    let model = model.replace(['@', '/', '\\'], "-");
    store.root().join("cache").join("masks").join(&hash[..2]).join(format!("{}-{model}-{key}.png", &hash[2..]))
}

fn raster_of(store: &Store, hash: &str, m: &Mask) -> Option<PathBuf> {
    match &m.shape {
        MaskShape::Ai { target, model, .. } => Some(raster_path(store, hash, model, &target.cache_key())),
        _ => None,
    }
}

/// A cached raster, if present and the expected size.
pub fn load_raster(path: &std::path::Path, w: u32, h: u32) -> Option<Vec<u8>> {
    let img = image::open(path).ok()?.into_luma8();
    (img.width() == w && img.height() == h).then(|| img.into_raw())
}

/// Save a raster (source space) atomically.
pub fn save_raster(path: &std::path::Path, data: Vec<u8>, w: u32, h: u32) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let img = image::GrayImage::from_raw(w, h, data).ok_or("raster has the wrong size")?;
    let tmp = path.with_extension("png.tmp");
    img.save_with_format(&tmp, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Every raster layer of `edit` for a photo (`hash`, `w × h`, lens `profile`): brush strokes,
/// and AI masks from the cache, warped into corrected uv when lens correction is on.
/// `missing` lists AI masks with no cached raster yet. Unchanged layers come from [`LayerCache`].
pub fn layers(app: &AppHandle, hash: &str, edit: &EditState, w: u32, h: u32, profile: Option<&LensProfile>) -> BrushLayers {
    let store = app.state::<Store>();
    let (lw, lh) = mask_size(w, h);
    let table = lens_table(profile, &edit.lens, w as f32, h as f32);
    app.state::<LayerCache>().layers(hash, &edit.masks, (w, h), &table, |m| raster_of(&store, hash, m), |p| load_raster(p, lw, lh))
}

// ---------------------------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------------------------

/// Models live outside the library (they're per machine, not per library).
/// `TONALITY_MODELS` overrides the default app-data folder.
pub fn models_dir(app: &AppHandle) -> PathBuf {
    if let Some(p) = std::env::var_os("TONALITY_MODELS").filter(|p| !p.is_empty()) {
        return p.into();
    }
    app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from(".")).join("models")
}

fn installed(app: &AppHandle, m: &Model) -> bool {
    let dir = models_dir(app);
    m.files.iter().all(|f| download::is_installed(&dir, f))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    /// "download" or "detect".
    stage: &'static str,
    model: &'static str,
    done: u64,
    total: u64,
}

/// Download `m`'s files if needed (emitting `ai-progress`) and return their paths.
fn ensure(app: &AppHandle, m: &'static Model) -> CmdResult<Vec<PathBuf>> {
    let dir = models_dir(app);
    let total = m.size();
    let mut before = 0;
    let mut paths = Vec::new();
    let mut last = 0u64;
    for f in m.files {
        let mut progress = |done: u64, _| {
            let d = before + done;
            if d - last > total / 100 || d == total {
                last = d;
                let _ = app.emit("ai-progress", Progress { stage: "download", model: m.id, done: d, total });
            }
        };
        paths.push(download::ensure(&dir, f, &mut progress).map_err(|e| e.to_string())?);
        before += f.size;
    }
    Ok(paths)
}

/// Serialises detection (models use every core) and keeps the last SAM embedding, so circling
/// another object in the same photo skips the encoder.
#[derive(Default)]
pub struct Detector {
    lock: Mutex<()>,
    embedding: Mutex<Option<(String, Tensor32)>>,
}

/// What to detect in, and for, one photo.
pub struct Photo<'a> {
    pub hash: &'a str,
    /// Camera-space linear pixels (half-float bits) and their size: any resolution ≥ the mask.
    pub pixels: &'a [u16],
    pub width: u32,
    pub height: u32,
    pub color: &'a CameraColor,
    pub profile: Option<&'a LensProfile>,
    /// Full-resolution size (sets the mask raster size).
    pub full_width: u32,
    pub full_height: u32,
}

/// Run `model` for `target` and cache the raster(s). Returns whether anything was found.
pub fn detect(
    app: &AppHandle,
    det: &Detector,
    store: &Store,
    photo: &Photo,
    lens: &LensCorrection,
    target: &AiTarget,
    model_id: &str,
) -> CmdResult<bool> {
    let model = registry::by_id(model_id).ok_or_else(|| format!("unknown model {model_id}"))?;
    let files = ensure(app, model)?;
    let _busy = det.lock.lock().map_err(|_| "detector lock poisoned")?;
    let _ = app.emit("ai-progress", Progress { stage: "detect", model: model.id, done: 0, total: 1 });

    let (w, h) = mask_size(photo.full_width, photo.full_height);
    // Downsample before converting: the models never see more than the mask raster.
    let img = input::neutral_srgb_sized(photo.pixels, photo.width, photo.height, photo.color, w, h);
    let guide = input::guide(&img, w, h);
    let save = |key: &str, r: Vec<u8>| -> CmdResult<bool> {
        let found = r.iter().filter(|&&v| v > 128).count() > (w * h / 500) as usize;
        let path = raster_path(store, photo.hash, model.id, key);
        save_raster(&path, r, w, h)?;
        app.state::<LayerCache>().forget(&path);
        Ok(found)
    };
    let err = |e: tonality_ml::MlError| e.to_string();

    let found = match (model.id, target) {
        ("subject@1", AiTarget::Subject) => {
            let (p, pw, ph) = subject::detect(&files[0], subject::SubjectModel::U2NET, &img).map_err(err)?;
            save("subject", refine::to_raster(&p, pw, ph, &guide, w, h))?
        }
        ("scene@1", AiTarget::Sky | AiTarget::People) => {
            // One pass gives both; cache both.
            let (maps, pw, ph) =
                scene::detect(&files[0], scene::SceneModel::ADE_512, &img, &[scene::ade::SKY, scene::ade::PERSON]).map_err(err)?;
            let sky = save("sky", refine::sky_raster(&maps[0], pw, ph, &img))?;
            let people = save("people", refine::to_raster(&maps[1], pw, ph, &guide, w, h))?;
            if matches!(target, AiTarget::Sky) { sky } else { people }
        }
        ("sam@1", AiTarget::Object { lasso }) => {
            // The loop was drawn on the corrected image; the model sees the uncorrected one.
            let to_src = UvToSource::new(&lens_table(photo.profile, lens, photo.full_width as f32, photo.full_height as f32), photo.full_width, photo.full_height);
            let lasso: Vec<[f32; 2]> = lasso.iter().map(|&p| to_src.map(p)).collect();
            let mut cached = det.embedding.lock().map_err(|_| "embedding lock poisoned")?;
            let emb = match &*cached {
                Some((hash, e)) if hash == photo.hash => e.clone(),
                _ => {
                    let e = sam::encode(&files[0], &img).map_err(err)?;
                    *cached = Some((photo.hash.to_string(), e.clone()));
                    e
                }
            };
            drop(cached);
            let p = sam::decode(&files[1], &emb, &lasso, w, h).map_err(err)?;
            save(&target.cache_key(), refine::to_raster(&p, w, h, &guide, w, h))?
        }
        _ => return Err(format!("{} can't detect {}", model.label, target.label())),
    };
    let _ = app.emit("ai-progress", Progress { stage: "detect", model: model.id, done: 1, total: 1 });
    Ok(found)
}

/// The model each target uses in this build (new masks record it).
pub fn model_for(target: &AiTarget) -> &'static Model {
    match target {
        AiTarget::Subject => &registry::SUBJECT,
        AiTarget::Sky | AiTarget::People => &registry::SCENE,
        AiTarget::Object { .. } => &registry::OBJECT,
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiModelInfo {
    pub id: String,
    pub label: String,
    #[ts(type = "number")]
    pub size: u64,
    pub installed: bool,
    pub license: String,
}

/// Every model, and whether it's downloaded.
#[tauri::command]
pub async fn ai_models(app: AppHandle) -> Vec<AiModelInfo> {
    registry::ALL
        .iter()
        .map(|m| AiModelInfo {
            id: m.id.into(),
            label: m.label.into(),
            size: m.size(),
            installed: installed(&app, m),
            license: m.license.into(),
        })
        .collect()
}

/// The model new masks of `target` should use.
#[tauri::command]
pub async fn ai_model_for(target: AiTarget) -> String {
    model_for(&target).id.into()
}

/// Detect `target` in an open photo with `model` (downloading it first if needed) and cache the
/// result. Emits `ai-progress`. Returns whether anything was found.
#[tauri::command]
pub async fn ai_detect(
    image_id: ImageId,
    target: AiTarget,
    model: String,
    lens: Option<LensCorrection>,
    app: AppHandle,
) -> CmdResult<bool> {
    tauri::async_runtime::spawn_blocking(move || {
        let images = app.state::<Images>();
        let p = images.get(image_id).ok_or("image not open")?;
        let (_, hash, _) = app.state::<CatalogState>().with(|c| c.image_file(image_id))?;
        let photo = Photo {
            hash: &hash,
            pixels: &p.pixels,
            width: p.width,
            height: p.height,
            color: &p.color,
            profile: p.lens.as_ref(),
            full_width: p.full_width,
            full_height: p.full_height,
        };
        detect(&app, &app.state::<Detector>(), &app.state::<Store>(), &photo, &lens.unwrap_or_default(), &target, &model)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_paths_are_per_photo_model_and_target() {
        let store = Store::open(std::env::temp_dir().join(format!("tonality-ai-{}", std::process::id()))).unwrap();
        let p = raster_path(&store, "abcdef", "subject@1", "subject");
        assert!(p.ends_with("cache/masks/ab/cdef-subject-1-subject.png"), "{p:?}");

        let (w, h) = (8, 4);
        save_raster(&p, (0..32).collect(), w, h).unwrap();
        assert_eq!(load_raster(&p, w, h).unwrap(), (0..32).collect::<Vec<u8>>());
        assert!(load_raster(&p, 4, 4).is_none(), "wrong size counts as missing");
        let _ = std::fs::remove_dir_all(store.root());
    }
}
