//! Fetching model files: to `<name>.part`, sha256-checked, then renamed into place.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::registry::ModelFile;
use crate::{MlError, Result};

/// Where `file` lives (or will live) under `dir`.
pub fn path_of(dir: &Path, file: &ModelFile) -> PathBuf {
    dir.join(file.name)
}

pub fn is_installed(dir: &Path, file: &ModelFile) -> bool {
    std::fs::metadata(path_of(dir, file)).is_ok_and(|m| m.len() == file.size)
}

/// The lock for one destination path. Downloads of different files run side by side (and an
/// installed model's check never waits behind another model's download); two callers wanting the
/// same file take turns, and the second finds it installed.
fn file_lock(dest: &Path) -> Arc<Mutex<()>> {
    static LOCKS: Mutex<Option<HashMap<PathBuf, Arc<Mutex<()>>>>> = Mutex::new(None);
    let mut locks = LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks
        .get_or_insert_with(HashMap::new)
        .entry(dest.to_path_buf())
        .or_default()
        .clone()
}

/// Files whose contents were checked this run.
static VERIFIED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn verified(dest: &Path) -> bool {
    VERIFIED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|p| p == dest)
}

fn mark_verified(dest: &Path) {
    let mut v = VERIFIED.lock().unwrap_or_else(|e| e.into_inner());
    if !v.iter().any(|p| p == dest) {
        v.push(dest.to_path_buf());
    }
}

/// Download `file` into `dir` unless it is already there. `progress(done, total)` is called as
/// bytes arrive.
pub fn ensure(dir: &Path, file: &ModelFile, progress: &mut dyn FnMut(u64, u64)) -> Result<PathBuf> {
    let dest = path_of(dir, file);
    if verified(&dest) && is_installed(dir, file) {
        return Ok(dest);
    }
    let lock = file_lock(&dest);
    let _same_file = lock.lock().unwrap_or_else(|e| e.into_inner());
    if is_installed(dir, file) {
        // The size matches; check the contents once per run (a damaged file of the right
        // size would otherwise fail in the model runtime every time, never re-fetched).
        if verified(&dest) || sha256_file(&dest).is_ok_and(|h| h == file.sha256) {
            mark_verified(&dest);
            return Ok(dest);
        }
        let _ = std::fs::remove_file(&dest);
    }
    std::fs::create_dir_all(dir)?;
    // Per process: another instance fetching the same file must not write into this one.
    let part = dest.with_extension(format!("{}.part", std::process::id()));
    let res = (|| -> Result<()> {
        // A dead connection must fail rather than hang detection forever. The body gets as long
        // as a slow (50 kB/s) link needs.
        let body_secs = 120 + file.size / 50_000;
        let resp = ureq::get(file.url)
            .config()
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_recv_body(Some(Duration::from_secs(body_secs)))
            .build()
            .call()
            .map_err(|e| MlError::Download(format!("{}: {e}", file.name)))?;
        let mut body = resp.into_body();
        let mut reader = body.with_config().limit(file.size + 1024).reader();
        let mut out = std::fs::File::create(&part)?;
        let mut hash = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut done = 0u64;
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            hash.update(&buf[..n]);
            done += n as u64;
            progress(done, file.size);
        }
        out.sync_all()?;
        let got = hash
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if got != file.sha256 {
            return Err(MlError::Download(format!(
                "{}: checksum mismatch",
                file.name
            )));
        }
        std::fs::rename(&part, &dest)?;
        mark_verified(&dest);
        Ok(())
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    res.map(|_| dest)
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_are_per_file() {
        let a = file_lock(Path::new("/models/a.onnx"));
        let b = file_lock(Path::new("/models/b.onnx"));
        assert!(Arc::ptr_eq(&a, &file_lock(Path::new("/models/a.onnx"))));
        assert!(!Arc::ptr_eq(&a, &b));
    }

    /// An installed model is ready while another model's download holds its lock.
    #[test]
    fn installed_model_does_not_wait_for_another_download() {
        let dir = std::env::temp_dir().join(format!("tonality-ml-dl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = b"weights";
        std::fs::write(dir.join("installed.onnx"), bytes).unwrap();
        let sha: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let installed = ModelFile {
            name: "installed.onnx",
            url: "http://127.0.0.1:9/never",
            size: bytes.len() as u64,
            sha256: Box::leak(sha.into_boxed_str()),
        };

        // Another file mid-"download".
        let other = file_lock(&dir.join("downloading.onnx"));
        let _busy = other.lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let d = dir.clone();
        std::thread::spawn(move || {
            let _ = tx.send(ensure(&d, &installed, &mut |_, _| {}).map_err(|e| e.to_string()));
        });
        let got = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("ensure waited on another file's lock");
        assert_eq!(got.unwrap(), dir.join("installed.onnx"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
