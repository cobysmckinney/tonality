//! In-memory caches for images open in Edit: small previews for the last few photos, and the
//! full-resolution decode of the current one (for 1:1 zoom and export).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use tonality_core::lens::{LensProfile, LensSource};
use tonality_core::ImageId;
use tonality_raw::{FullImage, Preview, RawDecoder, Rawler};
use tonality_render::presence::{presence_maps, PresenceMaps};
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
/// The full-resolution decode starts once a photo has stayed open this long (stepping through
/// photos shouldn't start a 1–3 s decode for each), or as soon as detail is asked for.
const FULL_DELAY: Duration = Duration::from_millis(900);

type Recent = Vec<(ImageId, Arc<Preview>)>;
type RecentMaps = Vec<(ImageId, Arc<PresenceMaps>)>;
type FullSlot = Arc<OnceLock<Result<Arc<FullImage>, String>>>;
type Prefetch = Arc<OnceLock<Mutex<Sender<(ImageId, PathBuf)>>>>;

#[derive(Clone, Default)]
pub struct Images {
    previews: Arc<Mutex<Recent>>,
    /// Full-resolution decode of one image (they are large: ~150 MB for 24 MP).
    full: Arc<Mutex<Option<(ImageId, FullSlot)>>>,
    /// The photo opened last.
    latest: Arc<AtomicI64>,
    /// Feeds the one thread that decodes full resolution ahead of need.
    prefetch: Prefetch,
    /// Clarity / Dehaze maps of recently used photos (made from their previews).
    presence: Arc<Mutex<RecentMaps>>,
}

impl Images {
    pub fn open(&self, id: ImageId, path: &Path, name: String, max_edge: usize) -> Result<ImageInfo, String> {
        // Before decoding: a slower decode of the photo opened before must not claim "latest"
        // after this one has.
        self.latest.store(id, Ordering::Relaxed);
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
        self.prefetch_full(id, path);

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

    /// Decode full resolution ahead, so zooming in is instant by the time it's needed: once
    /// the photo has stayed open for `FULL_DELAY`, unless another has been opened meanwhile.
    fn prefetch_full(&self, id: ImageId, path: &Path) {
        let tx = self.prefetch.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<(ImageId, PathBuf)>();
            let me = self.clone();
            std::thread::Builder::new()
                .name("full-prefetch".into())
                .spawn(move || {
                    let mut job: Option<(ImageId, PathBuf)> = None;
                    loop {
                        let next = if job.is_none() {
                            rx.recv().ok()
                        } else {
                            match rx.recv_timeout(FULL_DELAY) {
                                Ok(j) => Some(j), // a newer photo: restart the wait for it
                                Err(RecvTimeoutError::Disconnected) => None,
                                Err(RecvTimeoutError::Timeout) => {
                                    let (id, path) = job.take().unwrap();
                                    if me.latest.load(Ordering::Relaxed) == id {
                                        // At low priority (it's ahead of need).
                                        let _ = crate::priority::pool().install(|| me.full(id, &path));
                                    }
                                    continue;
                                }
                            }
                        };
                        match next {
                            Some(j) => job = Some(j),
                            None => return,
                        }
                    }
                })
                .expect("prefetch thread");
            Mutex::new(tx)
        });
        let _ = tx.lock().unwrap().send((id, path.to_path_buf()));
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
        let res = slot.get_or_init(|| tonality_raw::decode_full(path).map(Arc::new).map_err(|e| e.to_string())).clone();
        if res.is_err() {
            // Don't keep a failure (a busy disk, say): the next request tries again.
            let mut cur = self.full.lock().unwrap();
            if cur.as_ref().is_some_and(|(_, s)| Arc::ptr_eq(s, &slot)) {
                *cur = None;
            }
        }
        res
    }

    /// The Clarity / Dehaze maps of an open photo, made from its preview on first use.
    pub fn presence(&self, id: ImageId) -> Result<Arc<PresenceMaps>, String> {
        if let Some(m) = self.presence.lock().unwrap().iter().find(|(i, _)| *i == id) {
            return Ok(m.1.clone());
        }
        let p = self.get(id).ok_or("image not open")?;
        let maps = Arc::new(presence_maps(&p.pixels, p.width, p.height, &p.color));
        let mut cache = self.presence.lock().unwrap();
        cache.retain(|(i, _)| *i != id);
        cache.insert(0, (id, maps.clone()));
        cache.truncate(4);
        Ok(maps)
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
