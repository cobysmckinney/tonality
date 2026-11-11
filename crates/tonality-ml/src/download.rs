//! Fetching model files: to `<name>.part`, sha256-checked, then renamed into place.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
    if is_installed(dir, file) {
        return Ok(dest);
    }
    // One download at a time: two callers wanting the same file would otherwise write the same
    // `.part` and rename it into place under each other. The second finds it installed.
    static DOWNLOADING: Mutex<()> = Mutex::new(());
    let _one = DOWNLOADING.lock().unwrap_or_else(|e| e.into_inner());
    if is_installed(dir, file) {
        return Ok(dest);
    }
    std::fs::create_dir_all(dir)?;
    let part = dest.with_extension("part");
    let res = (|| -> Result<()> {
        let resp = ureq::get(file.url).call().map_err(|e| MlError::Download(format!("{}: {e}", file.name)))?;
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
        Ok(())
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    res.map(|_| dest)
}
