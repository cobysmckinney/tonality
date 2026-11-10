//! The on-disk library: one self-contained folder that tonality owns.
//!
//! ```text
//! tonality Library/
//!   catalog.sqlite            edits, history, ratings, metadata
//!   originals/2026/2026-11-02/DSC00084.ARW
//!   originals/2026/2026-11-02/DSC00084.ARW.tonality   its history, rating and flag (sidecar)
//!   cache/thumbs/…            regenerable
//! ```
//! Moving or backing up this folder moves everything. Originals stay ordinary files laid out by
//! capture date, so they are never locked inside tonality; with their sidecars, the library can
//! be rebuilt even without `catalog.sqlite`.

use std::path::{Path, PathBuf};

pub struct Store {
    root: PathBuf,
}

impl Store {
    /// `TONALITY_LIBRARY` overrides the default `~/Pictures/tonality Library`.
    pub fn resolve(pictures: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
        if let Some(p) = std::env::var_os("TONALITY_LIBRARY").filter(|p| !p.is_empty()) {
            return p.into();
        }
        pictures
            .or_else(|| home.map(|h| h.join("Pictures")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("tonality Library")
    }

    pub fn open(root: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(root.join("originals"))?;
        std::fs::create_dir_all(root.join("cache"))?;
        Ok(Store { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn catalog_path(&self) -> PathBuf {
        self.root.join("catalog.sqlite")
    }

    /// Absolute path of a library-relative file.
    pub fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn thumb_path(&self, hash: &str, edge: u32) -> PathBuf {
        self.root.join("cache").join("thumbs").join(&hash[..2]).join(format!("{}-{edge}.jpg", &hash[2..]))
    }

    /// Thumbnail of an image's current edited version (rendered by Edit).
    pub fn edited_thumb_path(&self, hash: &str) -> PathBuf {
        self.root.join("cache").join("edited").join(&hash[..2]).join(format!("{}.jpg", &hash[2..]))
    }

    /// Library-relative folder for a photo captured at `captured_at` (ISO 8601).
    pub fn originals_dir(captured_at: Option<&str>) -> String {
        match captured_at.filter(|d| d.len() >= 10 && d.as_bytes()[4] == b'-') {
            Some(d) => format!("originals/{}/{}", &d[..4], &d[..10]),
            None => "originals/undated".into(),
        }
    }
}
