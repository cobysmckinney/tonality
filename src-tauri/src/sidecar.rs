//! Sidecar files (`DSC00084.ARW.tonality`) next to each original, kept in step with the catalog.
//!
//! Any change to an image's history, rating or flag marks it dirty; a background thread
//! rewrites its sidecar shortly after (batched, atomically, only if the contents changed) and
//! on exit. At startup every photo is reconciled once: a sidecar newer than the catalog (an
//! older catalog restored from backup, a sidecar copied in from elsewhere) is merged in first,
//! then the file is rewritten from the merged history. A photo changed before the startup pass
//! reaches it is reconciled right then, ahead of the change, so the pass can't merge the old
//! history back over (say) a reset.
//!
//! A sidecar that can't be read (damaged, or written by a newer version) is left alone at
//! startup, and moved aside to `….tonality.rejected` rather than overwritten or deleted when
//! the photo next changes.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tonality_core::sidecar::SIDECAR_EXT;
use tonality_core::{Catalog, ImageId, Sidecar};

use crate::history::CatalogState;
use crate::store::Store;
use crate::CmdResult;

/// How long a burst of changes may keep going before its sidecars are written.
const DEBOUNCE: Duration = Duration::from_millis(400);
/// How long [`Sidecars::flush`] waits for the writer.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

pub fn path_for(original: &Path) -> PathBuf {
    let mut p = original.as_os_str().to_owned();
    p.push(".");
    p.push(SIDECAR_EXT);
    p.into()
}

/// The sidecar next to `original`: `None` if there isn't one, `Some(Err)` if it can't be used.
pub fn read(original: &Path) -> Option<Result<Sidecar, String>> {
    let text = match std::fs::read_to_string(path_for(original)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => return Some(Err(e.to_string())),
    };
    Some(Sidecar::from_json(&text).map_err(|e| e.to_string()))
}

/// Merge the sidecar next to `original` (if any) into `id`. Returns whether it brought anything.
pub fn adopt(catalog: &CatalogState, id: ImageId, original: &Path) -> bool {
    match read(original) {
        None => false,
        Some(Err(e)) => {
            eprintln!("[sidecar] ignoring {}: {e}", path_for(original).display());
            false
        }
        Some(Ok(sc)) => match catalog.edit(&[id], |c| c.merge_sidecar(id, &sc)) {
            Ok(m) => m.changed(),
            Err(e) => {
                eprintln!("[sidecar] ignoring {}: {e}", path_for(original).display());
                false
            }
        },
    }
}

/// Write `sc` to `path` (or remove the file if there is nothing to keep). An existing file
/// that this build can't use is moved aside first, never overwritten or deleted.
fn write_file(path: &Path, sc: &Sidecar) -> CmdResult<()> {
    let json = (!sc.is_empty()).then(|| sc.to_json());
    match std::fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if json.is_none() {
                return Ok(());
            }
        }
        // Can't tell what's there: leave it.
        Err(e) => return Err(format!("{}: {e}", path.display())),
        Ok(old) if json.as_deref().map(str::as_bytes) == Some(old.as_slice()) => return Ok(()),
        Ok(old) => {
            let usable = std::str::from_utf8(&old)
                .ok()
                .and_then(|t| Sidecar::from_json(t).ok())
                .is_some_and(|o| {
                    o.check().is_ok() && (o.image.hash.is_empty() || o.image.hash == sc.image.hash)
                });
            if !usable {
                let aside = set_aside(path)?;
                eprintln!(
                    "[sidecar] kept unreadable {} as {}",
                    path.display(),
                    aside.display()
                );
            }
        }
    }
    let Some(json) = json else {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("{}: {e}", path.display()))
            }
            _ => Ok(()),
        };
    };
    let tmp = path.with_extension(format!("{SIDECAR_EXT}.tmp"));
    std::fs::write(&tmp, json)
        .and_then(|_| std::fs::rename(&tmp, path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Rename `path` to the first free `….rejected`, `….rejected-2`, … next to it.
fn set_aside(path: &Path) -> CmdResult<PathBuf> {
    let name = |n: u32| {
        let mut p = path.as_os_str().to_owned();
        p.push(if n == 1 {
            ".rejected".to_string()
        } else {
            format!(".rejected-{n}")
        });
        PathBuf::from(p)
    };
    let aside = (1..).map(name).find(|p| !p.exists()).expect("unbounded");
    std::fs::rename(path, &aside).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(aside)
}

fn write(app: &AppHandle, id: ImageId) -> CmdResult<()> {
    let store = app.state::<Store>();
    let catalog = app.state::<CatalogState>();
    let (sc, rel) = match catalog.with(|c| Ok((c.sidecar(id)?, c.image_file(id)?.0))) {
        Ok(v) => v,
        Err(_) => return Ok(()), // removed from the library meanwhile
    };
    write_file(&path_for(&store.abs(&rel)), &sc)
}

/// Photos the startup pass hasn't reconciled yet.
struct Pending {
    root: PathBuf,
    ids: Mutex<HashSet<ImageId>>,
}

#[derive(Debug, PartialEq, Eq)]
enum Reconciled {
    /// Merged (or nothing to merge): the sidecar can be rewritten from the catalog.
    Ready,
    /// There is a sidecar but it couldn't be used: leave the file as it is.
    Unreadable,
    /// Already done (or not ours to do).
    Skipped,
}

impl Pending {
    /// Merge `id`'s sidecar into the catalog, once. Runs under the catalog lock.
    fn reconcile(&self, c: &mut Catalog, id: ImageId) -> Reconciled {
        if !self.ids.lock().is_ok_and(|mut ids| ids.remove(&id)) {
            return Reconciled::Skipped;
        }
        let Ok((rel, _, _)) = c.image_file(id) else {
            return Reconciled::Skipped; // removed from the library meanwhile
        };
        let original = self.root.join(rel);
        let err = match read(&original) {
            None => return Reconciled::Ready,
            Some(Ok(sc)) => match c.merge_sidecar(id, &sc) {
                Ok(_) => return Reconciled::Ready,
                Err(e) => e.to_string(),
            },
            Some(Err(e)) => e,
        };
        eprintln!(
            "[sidecar] leaving {} alone: {err}",
            path_for(&original).display()
        );
        Reconciled::Unreadable
    }
}

enum Msg {
    Dirty(Vec<ImageId>),
    Flush(Sender<()>),
}

/// The writer thread's queue.
#[derive(Default)]
struct Queue {
    dirty: BTreeSet<ImageId>,
    flushes: Vec<Sender<()>>,
}

impl Queue {
    fn take(&mut self, m: Msg) {
        match m {
            Msg::Dirty(ids) => self.dirty.extend(ids),
            Msg::Flush(done) => self.flushes.push(done),
        }
    }

    /// Write every dirty sidecar, then answer whoever was waiting for that.
    fn write_all(&mut self, app: &AppHandle) {
        for id in std::mem::take(&mut self.dirty) {
            if let Err(e) = write(app, id) {
                eprintln!("[sidecar] {e}");
            }
        }
        for done in self.flushes.drain(..) {
            let _ = done.send(());
        }
    }
}

/// Startup pass: pull in anything newer from each photo's sidecar, then write it out. Answers
/// flushes as it goes, so a command waiting on one isn't held up behind the whole library.
fn reconcile(app: &AppHandle, pending: &Pending, rx: &Receiver<Msg>, q: &mut Queue) {
    let catalog = app.state::<CatalogState>();
    let mut ids: Vec<ImageId> = pending
        .ids
        .lock()
        .map(|ids| ids.iter().copied().collect())
        .unwrap_or_default();
    ids.sort_unstable();
    for id in ids {
        while let Ok(m) = rx.try_recv() {
            q.take(m);
        }
        if !q.flushes.is_empty() {
            q.write_all(app);
        }
        if catalog.with(|c| Ok(pending.reconcile(c, id))) == Ok(Reconciled::Ready) {
            if let Err(e) = write(app, id) {
                eprintln!("[sidecar] {e}");
            }
        }
    }
}

pub struct Sidecars(Mutex<Sender<Msg>>);

impl Sidecars {
    /// Start the writer thread (after `Store` and `CatalogState` are managed). Hooks itself
    /// into the catalog so every change marks the image dirty.
    pub fn start(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel::<Msg>();
        let catalog = app.state::<CatalogState>();
        let pending = Arc::new(Pending {
            root: app.state::<Store>().root().to_path_buf(),
            ids: Mutex::new(
                catalog
                    .with(|c| c.image_ids())
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
            ),
        });
        let early = pending.clone();
        catalog.before_edit(Box::new(move |c: &mut Catalog, ids: &[ImageId]| {
            for &id in ids {
                early.reconcile(c, id);
            }
        }));
        let notify = tx.clone();
        catalog.on_change(Box::new(move |ids: &[ImageId]| {
            let _ = notify.send(Msg::Dirty(ids.to_vec()));
        }));
        std::thread::Builder::new()
            .name("sidecars".into())
            .spawn(move || {
                let mut q = Queue::default();
                reconcile(&app, &pending, &rx, &mut q);
                while let Ok(m) = rx.recv() {
                    q.take(m);
                    while q.flushes.is_empty() {
                        match rx.recv_timeout(DEBOUNCE) {
                            Ok(m) => q.take(m),
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    q.write_all(&app);
                }
            })
            .expect("spawn sidecar writer");
        Sidecars(Mutex::new(tx))
    }

    /// Write everything pending (on exit, or before a command returns). Gives up after a few
    /// seconds rather than hang, and says so.
    pub fn flush(&self) -> CmdResult<()> {
        let (done, wait) = mpsc::channel();
        self.0
            .lock()
            .map_err(|_| "sidecar writer lock poisoned".to_string())?
            .send(Msg::Flush(done))
            .map_err(|_| "the sidecar writer has stopped".to_string())?;
        wait.recv_timeout(FLUSH_TIMEOUT)
            .map_err(|_| "timed out writing sidecars".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonality_core::library::NewImage;
    use tonality_core::EditState;

    /// A fresh, empty folder for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tonality-sidecar-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A catalog holding one photo, `photo.ARW` under `root`.
    fn library(root: &Path) -> (Catalog, ImageId) {
        let mut c = Catalog::open_in_memory().unwrap();
        let import = c.begin_import("test", "test").unwrap();
        let id = c
            .add_image(
                import,
                &NewImage {
                    hash: "abc".into(),
                    file: "photo.ARW".into(),
                    name: "photo.ARW".into(),
                    file_size: 0,
                    source_path: root.join("photo.ARW").to_string_lossy().into_owned(),
                    meta: None,
                    aspect: None,
                },
            )
            .unwrap()
            .unwrap();
        (c, id)
    }

    fn pending(root: &Path, id: ImageId) -> Pending {
        Pending {
            root: root.to_path_buf(),
            ids: Mutex::new(HashSet::from([id])),
        }
    }

    const NEWER: &str = r#"{"tonality":999,"image":{"hash":"abc"},"future":true}"#;

    #[test]
    fn unreadable_sidecar_is_left_alone_at_startup() {
        let root = scratch("startup");
        let (mut c, id) = library(&root);
        let file = root.join("photo.ARW.tonality");
        std::fs::write(&file, NEWER).unwrap();

        let p = pending(&root, id);
        assert_eq!(p.reconcile(&mut c, id), Reconciled::Unreadable);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), NEWER);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unreadable_sidecar_is_moved_aside_not_deleted() {
        let root = scratch("aside");
        let (c, id) = library(&root);
        let file = root.join("photo.ARW.tonality");
        let empty = c.sidecar(id).unwrap();
        assert!(empty.is_empty());

        std::fs::write(&file, NEWER).unwrap();
        write_file(&file, &empty).unwrap();
        assert!(!file.exists());
        let aside = root.join("photo.ARW.tonality.rejected");
        assert_eq!(std::fs::read_to_string(&aside).unwrap(), NEWER);

        // Again, with something to write: the earlier one stays put too.
        std::fs::write(&file, "not json").unwrap();
        let mut rated = empty.clone();
        rated.image.rating = 3;
        write_file(&file, &rated).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), rated.to_json());
        assert_eq!(std::fs::read_to_string(&aside).unwrap(), NEWER);
        assert_eq!(
            std::fs::read_to_string(root.join("photo.ARW.tonality.rejected-2")).unwrap(),
            "not json"
        );

        // A readable one is simply replaced, or removed once there's nothing to keep.
        write_file(&file, &empty).unwrap();
        assert!(!file.exists());
        assert!(!root.join("photo.ARW.tonality.rejected-3").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_change_before_the_startup_pass_is_not_undone() {
        let root = scratch("reset");
        let (mut c, id) = library(&root);
        c.history_open(id).unwrap();
        let mut s = EditState::default();
        s.light.exposure = 1.0;
        c.commit(id, &s, None).unwrap();
        let file = root.join("photo.ARW.tonality");
        write_file(&file, &c.sidecar(id).unwrap()).unwrap();

        // The catalog knows the history, the sidecar has it too; reset it before the startup
        // pass gets to this photo.
        let catalog = CatalogState::new(c);
        let p = Arc::new(pending(&root, id));
        let early = p.clone();
        catalog.before_edit(Box::new(move |c: &mut Catalog, ids: &[ImageId]| {
            for &id in ids {
                early.reconcile(c, id);
            }
        }));
        catalog.edit(&[id], |c| c.reset_history(&[id])).unwrap();

        assert_eq!(
            catalog.with(|c| Ok(p.reconcile(c, id))).unwrap(),
            Reconciled::Skipped
        );
        let sc = catalog.with(|c| c.sidecar(id)).unwrap();
        assert!(sc.head.is_none(), "history stays reset");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sidecar_sits_next_to_the_original() {
        assert_eq!(
            path_for(Path::new("/lib/originals/2026/DSC00084.ARW")),
            PathBuf::from("/lib/originals/2026/DSC00084.ARW.tonality")
        );
    }
}
