//! In-memory cache of decoded previews for images open in Edit.

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tonality_core::ImageId;
use tonality_raw::{Preview, RawDecoder, Rawler};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImageInfo {
    #[ts(type = "number")]
    pub id: ImageId,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub make: String,
    pub model: String,
    /// As-shot white balance, shown on the sliders while WB is untouched.
    pub as_shot_temperature: f32,
    pub as_shot_tint: f32,
}

/// Keeps the last few previews so flipping between photos is instant.
const KEEP: usize = 4;

type Recent = Vec<(ImageId, Arc<Preview>)>;

#[derive(Clone, Default)]
pub struct Images(Arc<Mutex<Recent>>);

impl Images {
    pub fn open(&self, id: ImageId, path: &Path, name: String, max_edge: usize) -> Result<ImageInfo, String> {
        let preview = match self.get(id) {
            Some(p) => p,
            None => {
                let p = Arc::new(Rawler.decode_preview(path, max_edge).map_err(|e| e.to_string())?);
                let mut cache = self.0.lock().unwrap();
                cache.retain(|(i, _)| *i != id);
                cache.push((id, p.clone()));
                if cache.len() > KEEP {
                    cache.remove(0);
                }
                p
            }
        };
        let (t, tint) = preview.color.as_shot_temperature_tint();
        Ok(ImageInfo {
            id,
            name,
            width: preview.width,
            height: preview.height,
            make: preview.metadata.make.clone(),
            model: preview.metadata.model.clone(),
            as_shot_temperature: t.round(),
            as_shot_tint: tint.round(),
        })
    }

    pub fn get(&self, id: ImageId) -> Option<Arc<Preview>> {
        self.0.lock().unwrap().iter().find(|(i, _)| *i == id).map(|(_, p)| p.clone())
    }
}

