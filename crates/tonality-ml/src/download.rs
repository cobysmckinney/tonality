//! Fetching model files: to `<name>.part`, sha256-checked, then renamed into place.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
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

/// Download `file` into `dir` unless it is already there. `progress(done, total)` is called as
/// bytes arrive.
pub fn ensure(dir: &Path, file: &ModelFile, progress: &mut dyn FnMut(u64, u64)) -> Result<PathBuf> {
    let dest = path_of(dir, file);
    // Files checked this run. Held throughout, so one download at a time: two callers wanting
    // the same file would otherwise write the same `.part`
    // and rename it into place under each other. The second finds it installed.
    static VERIFIED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let mut verified = VERIFIED.lock().unwrap_or_else(|e| e.into_inner());
    if is_installed(dir, file) {
        // The size matches; check the contents once per run (a damaged file of the right
        // size would otherwise fail in the model runtime every time, never re-fetched).
        if verified.contains(&dest) || sha256_file(&dest).is_ok_and(|h| h == file.sha256) {
            if !verified.contains(&dest) {
                verified.push(dest.clone());
            }
            return Ok(dest);
        }
        let _ = std::fs::remove_file(&dest);
    }
    std::fs::create_dir_all(dir)?;
    let part = dest.with_extension("part");
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
        let got = hash.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>();
        if got != file.sha256 {
            return Err(MlError::Download(format!("{}: checksum mismatch", file.name)));
        }
        std::fs::rename(&part, &dest)?;
        verified.push(dest.clone());
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
