//! In-memory caches for images open in Edit: small previews for the last few photos, and the
//! full-resolution decode of the current one (for 1:1 zoom and export).

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use tonality_core::lens::{LensProfile, LensSource};
use tonality_core::ImageId;
use tonality_raw::{FullImage, Preview, RawDecoder, Rawler};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImageInfo {
    #[ts(type = "number")]
    pub id: ImageId,
    pub name: String,
    /// Preview texture size.
    pub width: u32,
    pub height: u32,
    /// Full-resolution size (what 100% zoom and export use).
    pub full_width: u32,
    pub full_height: u32,
    pub make: String,
    pub model: String,
    /// As-shot white balance, shown on the sliders while WB is untouched.
    pub as_shot_temperature: f32,
    pub as_shot_tint: f32,
    /// The lens profile found for this photo, if any.
    pub lens: Option<LensInfo>,
}

/// What the lens panel shows about a photo's profile.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LensInfo {
    pub name: String,
    pub source: LensSource,
    pub distortion: bool,
    pub vignetting: bool,
    pub chromatic: bool,
}

impl From<&LensProfile> for LensInfo {
    fn from(p: &LensProfile) -> Self {
        Self {
            name: p.name.clone(),
            source: p.source,
            distortion: p.distortion.is_some(),
            vignetting: p.vignetting.is_some(),
            chromatic: p.chromatic,
        }
    }
}

/// Keeps the last few previews so flipping between photos is instant.
const KEEP: usize = 4;

type Recent = Vec<(ImageId, Arc<Preview>)>;
type FullSlot = Arc<OnceLock<Result<Arc<FullImage>, String>>>;

#[derive(Clone, Default)]
pub struct Images {
    previews: Arc<Mutex<Recent>>,
    /// Full-resolution decode of one image (they are large: ~150 MB for 24 MP).
    full: Arc<Mutex<Option<(ImageId, FullSlot)>>>,
}

impl Images {
    pub fn open(&self, id: ImageId, path: &Path, name: String, max_edge: usize) -> Result<ImageInfo, String> {
        let preview = match self.get(id) {
            Some(p) => p,
            None => {
                let p = Arc::new(Rawler.decode_preview(path, max_edge).map_err(|e| e.to_string())?);
                let mut cache = self.previews.lock().unwrap();
                cache.retain(|(i, _)| *i != id);
                cache.push((id, p.clone()));
                if cache.len() > KEEP {
                    cache.remove(0);
                }
                p
            }
        };
        // Start the full-resolution decode now, so zooming in is instant by the time it's needed.
        let (me, path) = (self.clone(), path.to_path_buf());
        std::thread::spawn(move || {
            let _ = me.full(id, &path);
        });

        let (t, tint) = preview.color.as_shot_temperature_tint();
        Ok(ImageInfo {
            id,
            name,
            width: preview.width,
            height: preview.height,
            full_width: preview.full_width,
            full_height: preview.full_height,
            make: preview.metadata.make.clone(),
            model: preview.metadata.model.clone(),
            as_shot_temperature: t.round(),
            as_shot_tint: tint.round(),
            lens: preview.lens.as_ref().map(LensInfo::from),
        })
    }

    pub fn get(&self, id: ImageId) -> Option<Arc<Preview>> {
        self.previews.lock().unwrap().iter().find(|(i, _)| *i == id).map(|(_, p)| p.clone())
    }

    /// The full-resolution image, decoding it if needed. Concurrent callers for the same image
    /// share one decode; asking for a different image replaces the cached one.
    pub fn full(&self, id: ImageId, path: &Path) -> Result<Arc<FullImage>, String> {
        let slot = {
            let mut cur = self.full.lock().unwrap();
            match &*cur {
                Some((i, slot)) if *i == id => slot.clone(),
                _ => {
                    let slot = FullSlot::default();
                    *cur = Some((id, slot.clone()));
                    slot
                }
            }
        };
        slot.get_or_init(|| tonality_raw::decode_full(path).map(Arc::new).map_err(|e| e.to_string())).clone()
    }

    /// The cached full-resolution image, if `id` is the one currently held.
    pub fn cached_full(&self, id: ImageId) -> Option<Arc<FullImage>> {
        let cur = self.full.lock().unwrap();
        match &*cur {
            Some((i, slot)) if *i == id => slot.get().and_then(|r| r.as_ref().ok().cloned()),
            _ => None,
        }
    }
}
