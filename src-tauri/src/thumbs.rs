//! Generated images, made on first request and kept on disk: grid
//! thumbnails, viewer previews, and thumbnails for the import review sheet.
//! Deleting any of them is safe; they are rendered again when needed.

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::edit::Adjustments;
use crate::gpu::{self, Session};
use crate::import::ScanSession;
use crate::library::Library;
use crate::media::{self, Kind, PREVIEW_EDGE, THUMB_EDGE};

pub fn ensure_thumb(library: &Library, id: i64) -> Result<PathBuf> {
    render_photo(library, id, library.thumb_path(id), THUMB_EDGE, 82)
}

pub fn ensure_preview(library: &Library, id: i64) -> Result<PathBuf> {
    render_photo(library, id, library.preview_path(id), PREVIEW_EDGE, 90)
}

fn render_photo(library: &Library, id: i64, dest: PathBuf, edge: u32, quality: u8) -> Result<PathBuf> {
    if dest.exists() {
        return Ok(dest);
    }
    match library.edits(id)? {
        // Edited photos are drawn by the editor's own pipeline; this is the
        // slow path, only hit when the generated images have gone missing.
        Some(recipe) => {
            // One at a time: each open photo holds a few hundred megabytes on the GPU.
            static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if dest.exists() {
                return Ok(dest);
            }
            let session = open_session(library, id)?;
            EditedImages::draw(&session, &Adjustments::from_json(Some(&recipe)))?.write(library, id)?;
        }
        // Unedited photos use the camera's embedded JPEG, which is far quicker.
        None => {
            let files = library.photo_files(id)?;
            let image = media::render(&files.path, files.is_raw, files.jpeg_path.as_deref(), edge)?;
            media::write_jpeg(&image, &dest, quality)?;
        }
    }
    Ok(dest)
}

/// Loads a photo onto the GPU, ready to edit or render.
pub fn open_session(library: &Library, id: i64) -> Result<Session> {
    let files = library.photo_files(id)?;
    let image = crate::develop::load(&files.path, files.is_raw)?;
    let mut session = gpu::shared()?.open(image)?;
    session.matte_files = Some(library.matte_files(id));
    Ok(session)
}

/// A photo's thumbnail and preview, drawn with its edits but not yet saved.
pub struct EditedImages {
    preview: image::RgbImage,
    thumb: image::RgbImage,
}

impl EditedImages {
    /// Drawing is quick and needs the session; encoding is slow and does not,
    /// which is why the two are separate steps.
    pub fn draw(session: &Session, adjustments: &Adjustments) -> Result<Self> {
        let gpu = gpu::shared()?;
        Ok(Self {
            preview: gpu.render_image(session, adjustments, PREVIEW_EDGE)?,
            thumb: gpu.render_image(session, adjustments, THUMB_EDGE)?,
        })
    }

    pub fn write(&self, library: &Library, id: i64) -> Result<()> {
        media::write_jpeg(&self.preview, &library.preview_path(id), 90)?;
        media::write_jpeg(&self.thumb, &library.thumb_path(id), 82)
    }
}

/// Forgets a photo's generated images, so they are made afresh on next use.
pub fn clear_rendered(library: &Library, id: i64) {
    let _ = std::fs::remove_file(library.thumb_path(id));
    let _ = std::fs::remove_file(library.preview_path(id));
}

pub fn scan_thumb_path(library: &Library, session_id: u64, index: usize) -> PathBuf {
    library.scan_cache_dir().join(format!("{session_id}/{index}.jpg"))
}

/// Thumbnail for a photo that is still on the card or in a source folder.
pub fn ensure_scan_thumb(library: &Library, session: &ScanSession, index: usize) -> Result<PathBuf> {
    let dest = scan_thumb_path(library, session.id, index);
    if !dest.exists() {
        let item = session.items.get(index).context("no such photo in this scan")?;
        let image = media::render(&item.path, item.kind == Kind::Raw, item.jpeg.as_deref(), THUMB_EDGE)?;
        media::write_jpeg(&image, &dest, 82)?;
    }
    Ok(dest)
}
