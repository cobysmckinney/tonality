//! Developed RAWs kept on disk, so a photo opened again skips the develop,
//! the slowest part of opening it. Every full-size load of a library photo
//! comes through here: the editor, pasting edits in the grid, exporting a
//! photo that isn't open, and redrawing a thumbnail.
//!
//! An entry is the developed image in half-float, the precision the editor
//! works in, and `develop::load` rounds every RAW to it, so a photo is the
//! same whether it came from here or from its file. Entries are found by the
//! original's path, size and change times, and by this build's version and
//! `gpu::LOOK_VERSION`, so a replaced original or a changed develop never
//! finds an old one. Deleting any of them is safe.

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use anyhow::{ensure, Context, Result};
use half::f16;
use rayon::prelude::*;

use crate::develop::{self, LinearImage};

/// How much disk a library's developed photos may take: about a dozen
/// photos from a 24-megapixel camera, or seven at 45.
pub const BUDGET: u64 = 2 << 30;

/// The start of every entry, which changes when its layout does.
const MAGIC: [u8; 8] = *b"TNDEV001";
const HEADER: u64 = MAGIC.len() as u64 + 8;

pub struct Developed {
    dir: PathBuf,
    budget: u64,
    /// The entry being written, if any. Writing happens beside the work that
    /// needed the photo, one entry at a time.
    writing: Mutex<Option<JoinHandle<()>>>,
}

impl Developed {
    pub fn new(dir: PathBuf, budget: u64) -> Self {
        Self { dir, budget, writing: Mutex::new(None) }
    }

    /// Opens a photo's file for editing, as `develop::load` does, from the
    /// developed copy kept here if there is one. A RAW that had to be
    /// developed is kept for next time.
    pub fn load(&self, path: &Path, is_raw: bool) -> Result<LinearImage> {
        // Other photos decode quickly, and would take more room here than in their own file.
        if !is_raw {
            return develop::load(path, false);
        }
        // Found before the file is read, so a file replaced while it is
        // developed is kept under its old name, never the new.
        let entry = self.entry(path);
        if let Some(image) = entry.as_deref().and_then(read) {
            return Ok(image);
        }
        let image = develop::load(path, true)?;
        if let Some(entry) = entry {
            self.keep(entry, &image);
        }
        Ok(image)
    }

    /// The developed copy of a photo's file kept here, if there is one.
    pub fn kept(&self, path: &Path) -> Option<LinearImage> {
        read(&self.entry(path)?)
    }

    /// Waits for the entry being written, if any, to be finished.
    pub fn finish_writing(&self) {
        let writing = self.writing.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        if let Some(writing) = writing {
            let _ = writing.join();
        }
    }

    /// Where the developed copy of the file at `path` is kept, as it is now.
    /// None if the file can't be looked at, so it isn't kept at all.
    fn entry(&self, path: &Path) -> Option<PathBuf> {
        let file = fs::metadata(path).ok()?;
        let mut key = blake3::Hasher::new();
        key.update(&MAGIC);
        key.update(env!("CARGO_PKG_VERSION").as_bytes());
        key.update(&crate::gpu::LOOK_VERSION.to_le_bytes());
        key.update(path.as_os_str().as_encoded_bytes());
        key.update(&file.len().to_le_bytes());
        let nanos = |time: std::io::Result<SystemTime>| {
            time.ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos())
        };
        key.update(&nanos(file.modified()).to_le_bytes());
        key.update(&nanos(file.created()).to_le_bytes());
        // A file copied or moved over the original is a new file, even with the same size and date.
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            for value in [file.dev(), file.ino(), file.ctime() as u64, file.ctime_nsec() as u64] {
                key.update(&value.to_le_bytes());
            }
        }
        Some(self.dir.join(format!("{}.half", &key.finalize().to_hex()[..32])))
    }

    /// Writes an entry beside the caller's work. If one is already being
    /// written (a slow disk), this one isn't kept, rather than piling up
    /// copies of whole photos in memory.
    fn keep(&self, entry: PathBuf, image: &LinearImage) {
        if HEADER + image.pixels.len() as u64 * 6 > self.budget {
            return;
        }
        let mut writing = self.writing.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if writing.as_ref().is_some_and(|thread| !thread.is_finished()) {
            return;
        }
        let (width, height) = (image.width, image.height);
        let mut halves = vec![f16::ZERO; image.pixels.len() * 3];
        halves.par_chunks_mut(3).zip(image.pixels.par_iter()).for_each(|(out, pixel)| {
            for (half, &v) in out.iter_mut().zip(pixel) {
                *half = f16::from_f32(v);
            }
        });
        let (dir, budget) = (self.dir.clone(), self.budget);
        *writing = Some(std::thread::spawn(move || {
            if let Err(error) = write(&entry, width, height, &halves) {
                eprintln!("couldn't keep a developed photo at {}: {error:#}", entry.display());
            }
            drop(halves);
            make_room(&dir, budget, &entry);
        }));
    }
}

/// Reads an entry, marking it as just used. None if it isn't there or isn't whole.
fn read(entry: &Path) -> Option<LinearImage> {
    let mut file = File::options().read(true).write(true).open(entry).ok()?;
    let mut header = [0u8; HEADER as usize];
    file.read_exact(&mut header).ok()?;
    if header[..8] != MAGIC {
        return None;
    }
    let width = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let height = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let values = width as usize * height as usize * 3;
    if file.metadata().ok()?.len() != HEADER + values as u64 * 2 {
        return None;
    }
    let mut halves = vec![f16::ZERO; values];
    file.read_exact(bytemuck::cast_slice_mut(&mut halves)).ok()?;
    let _ = file.set_modified(SystemTime::now());
    let pixels = halves.par_chunks_exact(3).map(|rgb| [0, 1, 2].map(|c| rgb[c].to_f32())).collect();
    Some(LinearImage { width, height, pixels, scene_referred: true })
}

/// Writes an entry under a name of its own, then puts it in place, so it is
/// never found half written.
fn write(entry: &Path, width: u32, height: u32, halves: &[f16]) -> Result<()> {
    ensure!(cfg!(target_endian = "little"), "entries are written little-endian");
    let dir = entry.parent().context("no folder")?;
    fs::create_dir_all(dir)?;
    let part = entry.with_extension(format!("part{}", std::process::id()));
    let written = (|| -> Result<()> {
        let mut file = BufWriter::new(File::create(&part)?);
        file.write_all(&MAGIC)?;
        file.write_all(&width.to_le_bytes())?;
        file.write_all(&height.to_le_bytes())?;
        file.write_all(bytemuck::cast_slice(halves))?;
        file.into_inner()?.sync_data()?;
        Ok(fs::rename(&part, entry)?)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&part);
    }
    written
}

/// Forgets the photos used longest ago until the folder fits `budget`,
/// keeping `newest` whatever its size, and any writing left over from a
/// run that stopped part-way.
fn make_room(dir: &Path, budget: u64, newest: &Path) {
    let Ok(listing) = fs::read_dir(dir) else { return };
    let mut entries = Vec::new();
    for item in listing.flatten() {
        let Ok(info) = item.metadata() else { continue };
        let used = info.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if item.path().extension().is_some_and(|e| e == "half") {
            entries.push((used, info.len(), item.path()));
        } else if used.elapsed().is_ok_and(|age| age > Duration::from_secs(3600)) {
            let _ = fs::remove_file(item.path());
        }
    }
    // Newest first: keep them while they fit.
    entries.sort_by_key(|&(used, _, _)| std::cmp::Reverse(used));
    let mut total = 0;
    for (_, size, path) in entries {
        total += size;
        if total > budget && path != newest {
            let _ = fs::remove_file(path);
        }
    }
}
