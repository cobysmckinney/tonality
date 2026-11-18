//! Sidecar files (`DSC00084.ARW.tonality`) next to each original, kept in step with the catalog.
//!
//! Any change to an image's history, rating or flag marks it dirty; a background thread
//! rewrites its sidecar shortly after (batched, atomically, only if the contents changed) and
//! on exit. At startup every photo is reconciled once: a sidecar newer than the catalog (an
//! older catalog restored from backup, a sidecar copied in from elsewhere) is merged in first,
//! then the file is rewritten from the merged history.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tonality_core::sidecar::SIDECAR_EXT;
use tonality_core::{ImageId, Sidecar};

use crate::history::CatalogState;
use crate::store::Store;
use crate::CmdResult;

/// How long a burst of changes may keep going before its sidecars are written.
const DEBOUNCE: Duration = Duration::from_millis(400);

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

fn write(app: &AppHandle, id: ImageId) -> CmdResult<()> {
    let store = app.state::<Store>();
    let catalog = app.state::<CatalogState>();
    let (sc, rel) = match catalog.with(|c| Ok((c.sidecar(id)?, c.image_file(id)?.0))) {
        Ok(v) => v,
        Err(_) => return Ok(()), // removed from the library meanwhile
    };
    let path = path_for(&store.abs(&rel));
    if sc.is_empty() {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => return Ok(()),
        }
    }
    let json = sc.to_json();
    if std::fs::read_to_string(&path).is_ok_and(|old| old == json) {
        return Ok(());
    }
    let tmp = path.with_extension(format!("{SIDECAR_EXT}.tmp"));
    std::fs::write(&tmp, json)
        .and_then(|_| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Startup pass: pull in anything newer from each photo's sidecar, then write it out.
fn reconcile(app: &AppHandle) {
    let store = app.state::<Store>();
    let catalog = app.state::<CatalogState>();
    let Ok(ids) = catalog.with(|c| c.image_ids()) else {
        return;
    };
    for id in ids {
        if let Ok((rel, _, _)) = catalog.with(|c| c.image_file(id)) {
            adopt(&catalog, id, &store.abs(&rel));
        }
        if let Err(e) = write(app, id) {
            eprintln!("[sidecar] {e}");
        }
    }
}

enum Msg {
    Dirty(Vec<ImageId>),
    Flush(Sender<()>),
}

pub struct Sidecars(Mutex<Sender<Msg>>);

impl Sidecars {
    /// Start the writer thread (after `Store` and `CatalogState` are managed). Hooks itself
    /// into the catalog so every change marks the image dirty.
    pub fn start(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel::<Msg>();
        let notify = tx.clone();
        app.state::<CatalogState>()
            .on_change(Box::new(move |ids: &[ImageId]| {
                let _ = notify.send(Msg::Dirty(ids.to_vec()));
            }));
        std::thread::Builder::new()
            .name("sidecars".into())
            .spawn(move || {
                reconcile(&app);
                let mut dirty = BTreeSet::new();
                let mut flushes = Vec::new();
                let take = |m: Msg,
                            dirty: &mut BTreeSet<ImageId>,
                            flushes: &mut Vec<Sender<()>>| match m {
                    Msg::Dirty(ids) => dirty.extend(ids),
                    Msg::Flush(done) => flushes.push(done),
                };
                while let Ok(m) = rx.recv() {
                    take(m, &mut dirty, &mut flushes);
                    while flushes.is_empty() {
                        match rx.recv_timeout(DEBOUNCE) {
                            Ok(m) => take(m, &mut dirty, &mut flushes),
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    for id in std::mem::take(&mut dirty) {
                        if let Err(e) = write(&app, id) {
                            eprintln!("[sidecar] {e}");
                        }
                    }
                    for done in flushes.drain(..) {
                        let _ = done.send(());
                    }
                }
            })
            .expect("spawn sidecar writer");
        Sidecars(Mutex::new(tx))
    }

    /// Write everything pending (on exit). Gives up after a few seconds rather than hang.
    pub fn flush(&self) {
        let (done, wait) = mpsc::channel();
        if self.0.lock().map(|tx| tx.send(Msg::Flush(done))).is_ok() {
            let _ = wait.recv_timeout(Duration::from_secs(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_sits_next_to_the_original() {
        assert_eq!(
            path_for(Path::new("/lib/originals/2026/DSC00084.ARW")),
            PathBuf::from("/lib/originals/2026/DSC00084.ARW.tonality")
        );
    }
}
