//! In-memory cache of decoded previews. Replaced by catalog-backed ids in Phase 3.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tonality_raw::{Preview, RawDecoder, Rawler};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImageInfo {
    pub id: String,
    pub path: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub make: String,
    pub model: String,
    /// As-shot white balance, shown on the sliders while WB is untouched.
    pub as_shot_temperature: f32,
    pub as_shot_tint: f32,
}

#[derive(Clone, Default)]
pub struct Images(Arc<Mutex<HashMap<String, Arc<Preview>>>>);

impl Images {
    pub fn open(&self, path: &str, max_edge: usize) -> Result<ImageInfo, String> {
        let preview = match self.get(path) {
            Some(p) => p,
            None => {
                let p = Arc::new(Rawler.decode_preview(Path::new(path), max_edge).map_err(|e| e.to_string())?);
                self.0.lock().unwrap().insert(path.to_string(), p.clone());
                p
            }
        };
        let (t, tint) = preview.color.as_shot_temperature_tint();
        Ok(ImageInfo {
            id: path.to_string(),
            path: path.to_string(),
            name: Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            width: preview.width,
            height: preview.height,
            make: preview.metadata.make.clone(),
            model: preview.metadata.model.clone(),
            as_shot_temperature: t.round(),
            as_shot_tint: tint.round(),
        })
    }

    pub fn get(&self, id: &str) -> Option<Arc<Preview>> {
        self.0.lock().unwrap().get(id).cloned()
    }
}
