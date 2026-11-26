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
    // A dead connection must fail rather than hang detection forever. The body gets as long
    // as a slow (50 kB/s) link needs.
    let body_secs = 120 + file.size / 50_000;
    remove_stale_parts(&dest, Duration::from_secs(body_secs + 300));
    // Per process: another instance fetching the same file must not write into this one.
    let part = part_path(&dest, std::process::id());
    let res = (|| -> Result<()> {
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

/// Where process `pid` downloads `dest` to before it is checked and renamed into place.
fn part_path(dest: &Path, pid: u32) -> PathBuf {
    dest.with_extension(format!("{pid}.part"))
}

/// Delete what earlier runs left of downloading `dest`: a download that was cut short by the
/// app quitting or crashing leaves its part file (up to the whole model), and the next run,
/// with another process id, writes a new one beside it. Only files untouched for `older_than`,
/// which is longer than a download is allowed to take, so another instance's download that is
/// still running is left alone.
fn remove_stale_parts(dest: &Path, older_than: Duration) {
    let (Some(dir), Some(stem)) = (dest.parent(), dest.file_stem()) else {
        return;
    };
    let prefix = format!("{}.", stem.to_string_lossy());
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // `<stem>.<pid>.part`
        let is_part = name
            .strip_prefix(&prefix)
            .and_then(|rest| rest.strip_suffix(".part"))
            .is_some_and(|pid| !pid.is_empty() && pid.chars().all(|c| c.is_ascii_digit()));
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| at.elapsed().ok())
            .is_some_and(|age| age > older_than);
        if is_part && stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
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

    // -----------------------------------------------------------------------------------------
    // The download itself, against a server on this machine.
    // -----------------------------------------------------------------------------------------

    fn sha(bytes: &[u8]) -> &'static str {
        let hex: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Box::leak(hex.into_boxed_str())
    }

    /// A folder for one test, and a server that answers every request with `body`. Returns the
    /// folder and the URL to fetch.
    fn serve(tag: &str, body: Vec<u8>) -> (PathBuf, &'static str) {
        use std::net::TcpListener;
        let dir = std::env::temp_dir().join(format!("tonality-ml-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model.onnx", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                // The request, up to its blank line.
                let mut seen = Vec::new();
                let mut byte = [0u8; 1];
                while !seen.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => seen.push(byte[0]),
                        _ => break,
                    }
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                // The client may hang up early (a body over its limit): not this test's failure.
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        (dir, Box::leak(url.into_boxed_str()))
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_download_that_checks_out_is_installed() {
        let weights = b"these are the right weights".to_vec();
        let (dir, url) = serve("ok", weights.clone());
        let file = ModelFile {
            name: "good.onnx",
            url,
            size: weights.len() as u64,
            sha256: sha(&weights),
        };
        let mut reported = (0, 0);
        let path = ensure(&dir, &file, &mut |done, total| reported = (done, total)).unwrap();
        assert_eq!(path, dir.join("good.onnx"));
        assert_eq!(std::fs::read(&path).unwrap(), weights);
        assert_eq!(reported, (weights.len() as u64, weights.len() as u64));
        assert_eq!(leftovers(&dir), ["good.onnx"], "no part file left");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_download_with_the_wrong_contents_is_not_installed() {
        // The right length, so only the checksum can tell.
        let (dir, url) = serve("bad", b"tampered weights...........".to_vec());
        let wanted = b"these are the right weights";
        let file = ModelFile {
            name: "bad.onnx",
            url,
            size: wanted.len() as u64,
            sha256: sha(wanted),
        };
        let e = ensure(&dir, &file, &mut |_, _| {}).unwrap_err();
        assert!(
            matches!(&e, MlError::Download(m) if m.contains("checksum")),
            "{e}"
        );
        assert!(
            leftovers(&dir).is_empty(),
            "neither the file nor its part: {:?}",
            leftovers(&dir)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_download_longer_than_the_model_is_cut_off() {
        // A megabyte where 27 bytes were promised: it must not be written out in full.
        let (dir, url) = serve("long", vec![7u8; 1 << 20]);
        let wanted = b"these are the right weights";
        let file = ModelFile {
            name: "long.onnx",
            url,
            size: wanted.len() as u64,
            sha256: sha(wanted),
        };
        assert!(ensure(&dir, &file, &mut |_, _| {}).is_err());
        assert!(leftovers(&dir).is_empty(), "{:?}", leftovers(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_installed_file_is_fetched_again() {
        let weights = b"these are the right weights".to_vec();
        let (dir, url) = serve("damaged", weights.clone());
        let file = ModelFile {
            name: "damaged.onnx",
            url,
            size: weights.len() as u64,
            sha256: sha(&weights),
        };
        // The right size, the wrong contents: a model runtime would fail on it every time.
        std::fs::write(dir.join("damaged.onnx"), vec![b'?'; weights.len()]).unwrap();
        let path = ensure(&dir, &file, &mut |_, _| {}).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), weights);
        assert_eq!(leftovers(&dir), ["damaged.onnx"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn part_files_from_runs_that_were_cut_short_are_removed() {
        let weights = b"these are the right weights".to_vec();
        let (dir, url) = serve("stale", weights.clone());
        let dest = dir.join("silueta.onnx");
        let old = std::time::SystemTime::now() - Duration::from_secs(3 * 60 * 60);
        let aged = |name: &str, when: std::time::SystemTime| {
            let p = dir.join(name);
            std::fs::write(&p, b"half").unwrap();
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(when)
                .unwrap();
        };
        // An earlier run's, hours old; and one from another instance that is downloading now.
        aged("silueta.41234.part", old);
        aged("silueta.51877.part", std::time::SystemTime::now());
        // Old, but not this model's part file.
        aged("other.41234.part", old);
        aged("silueta.notes.part", old);

        remove_stale_parts(&dest, Duration::from_secs(600));
        assert_eq!(
            leftovers(&dir),
            [
                "other.41234.part",
                "silueta.51877.part",
                "silueta.notes.part"
            ]
        );

        // And `ensure` does it on the way to a download.
        aged("silueta.41234.part", old);
        let file = ModelFile {
            name: "silueta.onnx",
            url,
            size: weights.len() as u64,
            sha256: sha(&weights),
        };
        ensure(&dir, &file, &mut |_, _| {}).unwrap();
        assert!(!dir.join("silueta.41234.part").exists());
        assert!(dest.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
