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

use std::fs::{File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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

    /// Take the library for this process: an exclusive lock on `<root>/.tonality.lock`, held
    /// until the returned file is dropped (or the process exits, however it exits). Fails if
    /// another tonality has the library open, since two importers would each believe a name in
    /// `originals/` is theirs. A filesystem that can't lock (some network shares) is let through.
    pub fn lock(&self) -> io::Result<File> {
        let f = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(".tonality.lock"))?;
        match f.try_lock() {
            Ok(()) => Ok(f),
            Err(TryLockError::WouldBlock) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "the library is already open in another tonality window",
            )),
            Err(TryLockError::Error(e)) => {
                log::warn!("cannot lock the library ({e}); continuing unlocked");
                Ok(f)
            }
        }
    }

    pub fn catalog_path(&self) -> PathBuf {
        self.root.join("catalog.sqlite")
    }

    /// Absolute path of a library-relative file.
    pub fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn thumb_path(&self, hash: &str, edge: u32) -> PathBuf {
        self.root
            .join("cache")
            .join("thumbs")
            .join(&hash[..2])
            .join(format!("{}-{edge}.jpg", &hash[2..]))
    }

    /// Thumbnail of an image's current edited version (rendered by Edit).
    pub fn edited_thumb_path(&self, hash: &str) -> PathBuf {
        self.root
            .join("cache")
            .join("edited")
            .join(&hash[..2])
            .join(format!("{}.jpg", &hash[2..]))
    }

    /// Library-relative folder for a photo captured at `captured_at` (ISO 8601).
    pub fn originals_dir(captured_at: Option<&str>) -> String {
        match captured_at.filter(|d| d.len() >= 10 && d.as_bytes()[4] == b'-') {
            Some(d) => format!("originals/{}/{}", &d[..4], &d[..10]),
            None => "originals/undated".into(),
        }
    }
}

/// A fresh sibling path to write `dest` through (write, then rename into place). Unique per
/// call, so concurrent writers of the same or same-stem files never share one.
pub fn temp_path(dest: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    dest.with_file_name(format!(".{name}.{}-{n}.part", std::process::id()))
}

/// Move `from` to `to`, failing with `AlreadyExists` rather than replacing anything at `to`
/// (on a case-insensitive filesystem, anything differing only in case).
///
/// A hard link claims `to` atomically everywhere; where links aren't supported (FAT and exFAT
/// cards and drives), an empty file created with `create_new` claims it instead and the rename
/// then replaces only that placeholder. On failure nothing this call didn't create is removed.
/// Don't make `from` read-only first: Windows can't unlink a read-only name.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::hard_link(from, to) {
        Ok(()) => {
            if let Err(e) = std::fs::remove_file(from) {
                log::warn!("cannot remove {}: {e}", from.display());
            }
            return Ok(());
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(e),
        // `from` itself is missing: the fallback can't do better.
        Err(e) if e.kind() == io::ErrorKind::NotFound && !from.exists() => return Err(e),
        Err(_) => {}
    }
    File::options().write(true).create_new(true).open(to)?;
    std::fs::rename(from, to).inspect_err(|_| {
        let _ = std::fs::remove_file(to);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tonality-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rename_noreplace_never_replaces() {
        let d = scratch("noreplace");
        let (a, b) = (d.join("a.part"), d.join("B.ARW"));
        std::fs::write(&a, b"new").unwrap();
        std::fs::write(&b, b"original").unwrap();
        let e = rename_noreplace(&a, &b).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&b).unwrap(), b"original");
        assert_eq!(std::fs::read(&a).unwrap(), b"new");

        let c = d.join("C.ARW");
        rename_noreplace(&a, &c).unwrap();
        assert_eq!(std::fs::read(&c).unwrap(), b"new");
        assert!(!a.exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn library_lock_is_exclusive() {
        let d = scratch("lock");
        let store = Store::open(d.clone()).unwrap();
        let held = store.lock().unwrap();
        let e = Store::open(d.clone()).unwrap().lock().unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::WouldBlock);
        drop(held);
        store.lock().unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn temp_paths_never_collide() {
        let a = temp_path(Path::new("/l/originals/IMG_0001.CR2"));
        let b = temp_path(Path::new("/l/originals/IMG_0001.DNG"));
        let c = temp_path(Path::new("/l/originals/IMG_0001.CR2"));
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.parent(), Some(Path::new("/l/originals")));
    }
}
