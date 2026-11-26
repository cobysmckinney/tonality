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

/// Whether `name` is one of [`temp_path`]'s, and if so the process that made it.
fn temp_owner(name: &str) -> Option<u32> {
    let rest = name.strip_prefix('.')?.strip_suffix(".part")?;
    let (_, tag) = rest.rsplit_once('.')?;
    let (pid, n) = tag.split_once('-')?;
    n.parse::<u64>().ok()?;
    pid.parse().ok()
}

/// Delete the temporary files ([`temp_path`]) under `dir` that an earlier run left behind:
/// a quit, crash or power cut in the middle of an import leaves each copy that was under way,
/// up to a whole raw file apiece, and nothing else would ever remove them (they are hidden,
/// and recovery skips hidden files). Returns how many were removed.
///
/// This run's own are left alone, so it's safe beside an import that has already started.
/// Other runs' are dead: call it only while holding the library's lock ([`Store::lock`]).
pub fn sweep_temp_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for e in entries.flatten() {
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() {
            removed += sweep_temp_files(&e.path());
        } else if kind.is_file()
            && temp_owner(&e.file_name().to_string_lossy())
                .is_some_and(|pid| pid != std::process::id())
        {
            match std::fs::remove_file(e.path()) {
                Ok(()) => removed += 1,
                Err(err) => log::warn!("cannot remove {}: {err}", e.path().display()),
            }
        }
    }
    removed
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
    fn temp_files_left_by_an_earlier_run_are_swept() {
        let d = scratch("sweep");
        let day = d.join("originals/2026/2026-11-02");
        std::fs::create_dir_all(&day).unwrap();
        let other_run = std::process::id() + 1;
        // Half a copy from a run that was killed, and one from an import running now.
        let stale = day.join(format!(".DSC00084.ARW.{other_run}-17.part"));
        let live = temp_path(&day.join("DSC00085.ARW"));
        // Not ours to touch: an original, its sidecar, and files that only look similar.
        let kept = [
            day.join("DSC00084.ARW"),
            day.join("DSC00084.ARW.tonality"),
            day.join("notes.part"),
            day.join(".hidden"),
            day.join(".DSC00084.ARW.part"),
            day.join(".DSC00084.ARW.abc-1.part"),
        ];
        for p in kept.iter().chain([&stale, &live]) {
            std::fs::write(p, b"x").unwrap();
        }

        assert_eq!(sweep_temp_files(&d.join("originals")), 1);

        assert!(!stale.exists(), "the dead run's file is gone");
        assert!(live.exists(), "this run's file is left alone");
        for p in &kept {
            assert!(p.exists(), "{} was removed", p.display());
        }
        assert_eq!(sweep_temp_files(&d.join("nowhere")), 0);
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
