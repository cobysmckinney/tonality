//! Importing happens in two steps so the user can review before anything is
//! copied: `scan` finds photos and marks the ones already in the library,
//! then `run` copies the chosen ones into `Originals/<year>/<date>/`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Local, NaiveDateTime};
use rayon::prelude::*;
use serde::Serialize;
use walkdir::WalkDir;

use crate::library::{Library, NewPhoto};
use crate::media::{self, Kind};
use crate::thumbs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    New,
    /// Already in the library (or found twice in the same scan).
    Duplicate,
    /// Already in the library, but sitting in Recently Deleted.
    Deleted(i64),
}

#[derive(Debug)]
pub struct ScanItem {
    pub path: PathBuf,
    /// The camera JPEG shot alongside a RAW, imported as part of the same photo.
    pub jpeg: Option<PathBuf>,
    pub kind: Kind,
    pub size: u64,
    pub fingerprint: String,
    /// When it was taken, as far as a quick look at the file can tell; the
    /// file's own date otherwise. The review groups photos by this day.
    pub taken_at: NaiveDateTime,
    pub status: Status,
}

#[derive(Debug)]
pub struct ScanSession {
    pub id: u64,
    pub source: String,
    pub items: Vec<ScanItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanItemView {
    pub index: usize,
    pub file_name: String,
    pub kind: &'static str,
    pub has_jpeg: bool,
    pub size: u64,
    pub taken_at: String,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanView {
    pub session_id: u64,
    pub source: String,
    pub items: Vec<ScanItemView>,
}

impl ScanSession {
    pub fn view(&self) -> ScanView {
        let items = self
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| ScanItemView {
                index,
                file_name: file_name(&item.path),
                kind: item.kind.as_str(),
                has_jpeg: item.jpeg.is_some(),
                size: item.size,
                taken_at: item.taken_at.format("%Y-%m-%dT%H:%M:%S").to_string(),
                status: match item.status {
                    Status::New => "new",
                    Status::Duplicate => "duplicate",
                    Status::Deleted(_) => "deleted",
                },
            })
            .collect();
        ScanView { session_id: self.id, source: self.source.clone(), items }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

fn is_hidden(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.'))
}

/// Every importable file under `paths`, which may mix files and folders.
fn collect_files(paths: &[PathBuf], library_root: &Path) -> Vec<(PathBuf, Kind)> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let walker = WalkDir::new(path).into_iter().filter_entry(|entry| {
                // Never re-import the library into itself.
                entry.depth() == 0 || (!is_hidden(entry.path()) && entry.path() != library_root)
            });
            for entry in walker.flatten().filter(|entry| entry.file_type().is_file()) {
                if let Some(kind) = media::kind_of(entry.path()) {
                    files.push((entry.into_path(), kind));
                }
            }
        } else if let Some(kind) = media::kind_of(path) {
            files.push((path.clone(), kind));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files.dedup_by(|a, b| a.0 == b.0);
    files
}

/// Folds each camera JPEG into the RAW of the same name next to it.
fn pair_raw_and_jpeg(files: Vec<(PathBuf, Kind)>) -> Vec<(PathBuf, Kind, Option<PathBuf>)> {
    let mut by_stem: HashMap<(PathBuf, String), Vec<usize>> = HashMap::new();
    for (index, (path, _)) in files.iter().enumerate() {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
        by_stem.entry((path.parent().unwrap_or(Path::new("")).to_path_buf(), stem)).or_default().push(index);
    }
    let mut jpeg_of: HashMap<usize, usize> = HashMap::new();
    for group in by_stem.values() {
        let raws: Vec<_> = group.iter().filter(|&&i| files[i].1 == Kind::Raw).collect();
        let jpegs: Vec<_> = group.iter().filter(|&&i| media::is_jpeg(&files[i].0)).collect();
        if let ([raw], [jpeg]) = (raws.as_slice(), jpegs.as_slice()) {
            jpeg_of.insert(**raw, **jpeg);
        }
    }
    let paired: HashSet<usize> = jpeg_of.values().copied().collect();
    files
        .iter()
        .enumerate()
        .filter(|(index, _)| !paired.contains(index))
        .map(|(index, (path, kind))| (path.clone(), *kind, jpeg_of.get(&index).map(|&j| files[j].0.clone())))
        .collect()
}

fn describe_source(paths: &[PathBuf]) -> String {
    let name = |path: &Path| path.file_name().map(|n| n.to_string_lossy().into_owned());
    match paths {
        [single] if single.is_dir() => name(single).unwrap_or_else(|| single.to_string_lossy().into_owned()),
        [first, rest @ ..] => {
            let parent = first.parent();
            if rest.iter().all(|path| path.parent() == parent) {
                parent.and_then(name).unwrap_or_else(|| "Files".into())
            } else {
                "Files".into()
            }
        }
        [] => "Files".into(),
    }
}

/// Finds the photos under `paths` and works out which are new.
/// `progress` is called with (photos checked, photos found).
pub fn scan(
    library: &Library,
    session_id: u64,
    paths: &[PathBuf],
    source: Option<String>,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<ScanSession> {
    let candidates = pair_raw_and_jpeg(collect_files(paths, library.root()));
    let total = candidates.len();
    progress(0, total);

    let checked = AtomicUsize::new(0);
    let mut items: Vec<ScanItem> = candidates
        .into_par_iter()
        .filter_map(|(path, kind, jpeg)| {
            let item = media::fingerprint(&path).ok().map(|(fingerprint, size)| {
                let modified = || {
                    fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .map(|time| DateTime::<Local>::from(time).naive_local())
                        .unwrap_or_else(|_| Local::now().naive_local())
                };
                let taken_at = media::quick_taken_at(&path)
                    .or_else(|| media::quick_taken_at(jpeg.as_ref()?))
                    .unwrap_or_else(modified);
                ScanItem { path, jpeg, kind, size, fingerprint, taken_at, status: Status::New }
            });
            let done = checked.fetch_add(1, Ordering::Relaxed) + 1;
            if done.is_multiple_of(20) || done == total {
                progress(done, total);
            }
            item
        })
        .collect();
    items.sort_by(|a, b| b.taken_at.cmp(&a.taken_at).then_with(|| a.path.cmp(&b.path)));

    let known = library.fingerprints()?;
    let mut seen = HashSet::new();
    for item in &mut items {
        item.status = match known.get(&item.fingerprint) {
            Some(&(id, true)) => Status::Deleted(id),
            Some(_) => Status::Duplicate,
            None if !seen.insert(item.fingerprint.clone()) => Status::Duplicate,
            None => Status::New,
        };
    }

    Ok(ScanSession { id: session_id, source: source.unwrap_or_else(|| describe_source(paths)), items })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFailure {
    pub file_name: String,
    pub reason: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub imported: u32,
    /// Photos that were in Recently Deleted and were put back instead of copied again.
    pub restored: u32,
    pub failed: Vec<ImportFailure>,
    pub cancelled: bool,
}

enum Outcome {
    Imported,
    Restored,
}

/// Copies the chosen scan items into the library.
/// `progress` is called with (photos finished, photos chosen).
pub fn run(
    library: &Library,
    session: &ScanSession,
    indices: &[usize],
    cancel: &AtomicBool,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<ImportSummary> {
    let import_id = library.create_import(&session.source)?;
    fs::create_dir_all(library.incoming_dir())?;
    let total = indices.len();
    let done = AtomicUsize::new(0);
    let placement = Mutex::new(());
    let summary = Mutex::new(ImportSummary::default());

    // A few at a time: copying is bound by the disk, thumbnails by the CPU.
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build()?;
    pool.install(|| {
        indices.par_iter().for_each(|&index| {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let Some(item) = session.items.get(index) else { return };
            let outcome = import_one(library, session.id, index, item, import_id, &placement);
            let mut summary = summary.lock().unwrap();
            match outcome {
                Ok(Outcome::Imported) => summary.imported += 1,
                Ok(Outcome::Restored) => summary.restored += 1,
                Err(error) => summary
                    .failed
                    .push(ImportFailure { file_name: file_name(&item.path), reason: format!("{error:#}") }),
            }
            drop(summary);
            progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
        });
    });

    let _ = fs::remove_dir_all(library.incoming_dir());
    library.discard_import_if_empty(import_id)?;
    let mut summary = summary.into_inner().unwrap();
    summary.cancelled = cancel.load(Ordering::Relaxed);
    Ok(summary)
}

/// Copies `from` to `to`, keeping the modification time, and checks the size.
fn copy_file(from: &Path, to: &Path) -> Result<()> {
    let copied = fs::copy(from, to).with_context(|| format!("copying {}", from.display()))?;
    let source = fs::metadata(from)?;
    if copied != source.len() {
        bail!("only {copied} of {} bytes were copied", source.len());
    }
    if let (Ok(modified), Ok(file)) = (source.modified(), fs::File::options().write(true).open(to)) {
        let _ = file.set_modified(modified);
    }
    Ok(())
}

/// Picks names in `dir` that are free for the photo and its paired JPEG,
/// adding `-1`, `-2`… when a different photo already has the name.
fn free_names(dir: &Path, primary: &Path, jpeg: Option<&Path>) -> (PathBuf, Option<PathBuf>) {
    let stem = primary.file_stem().unwrap_or_default().to_string_lossy();
    let with_ext = |stem: &str, like: &Path| match like.extension() {
        Some(ext) => dir.join(format!("{stem}.{}", ext.to_string_lossy())),
        None => dir.join(stem),
    };
    (0..)
        .map(|n| if n == 0 { stem.to_string() } else { format!("{stem}-{n}") })
        .map(|stem| (with_ext(&stem, primary), jpeg.map(|jpeg| with_ext(&stem, jpeg))))
        .find(|(primary, jpeg)| !primary.exists() && !jpeg.as_ref().is_some_and(|jpeg| jpeg.exists()))
        .expect("an unbounded search always finds a free name")
}

fn import_one(
    library: &Library,
    session_id: u64,
    index: usize,
    item: &ScanItem,
    import_id: i64,
    placement: &Mutex<()>,
) -> Result<Outcome> {
    match item.status {
        Status::Deleted(id) => {
            library.restore(&[id])?;
            return Ok(Outcome::Restored);
        }
        Status::Duplicate => bail!("already in the library"),
        Status::New => {}
    }

    // Copy first, into a holding folder: the capture date decides the final
    // folder, and reading it from the local copy avoids a second pass over
    // a slow card.
    let incoming = library.incoming_dir();
    let holding_name = |tag: &str, like: &Path| {
        let ext = like.extension().unwrap_or_default().to_string_lossy();
        incoming.join(format!("{import_id}-{index}{tag}.{ext}"))
    };
    let holding = holding_name("", &item.path);
    let holding_jpeg = item.jpeg.as_ref().map(|jpeg| holding_name("-pair", jpeg));
    let cleanup = |paths: &[Option<&Path>]| {
        for path in paths.iter().flatten() {
            let _ = fs::remove_file(path);
        }
    };

    let copied = copy_file(&item.path, &holding).and_then(|()| match (&item.jpeg, &holding_jpeg) {
        (Some(jpeg), Some(holding_jpeg)) => copy_file(jpeg, holding_jpeg),
        _ => Ok(()),
    });
    if let Err(error) = copied {
        cleanup(&[Some(&holding), holding_jpeg.as_deref()]);
        return Err(error);
    }

    let meta = media::read_meta(&holding, item.kind);
    let taken_at = meta
        .taken_at
        .or_else(|| media::read_meta(holding_jpeg.as_ref()?, Kind::Image).taken_at)
        .unwrap_or(item.taken_at);

    let day = library
        .originals_dir()
        .join(taken_at.format("%Y").to_string())
        .join(taken_at.format("%Y-%m-%d").to_string());
    fs::create_dir_all(&day)?;

    let (dest, dest_jpeg) = {
        let _guard = placement.lock().unwrap();
        let (dest, dest_jpeg) = free_names(&day, &item.path, item.jpeg.as_deref());
        let moved = fs::rename(&holding, &dest).and_then(|()| match (&holding_jpeg, &dest_jpeg) {
            (Some(from), Some(to)) => fs::rename(from, to),
            _ => Ok(()),
        });
        if let Err(error) = moved {
            cleanup(&[Some(&holding), holding_jpeg.as_deref(), Some(&dest), dest_jpeg.as_deref()]);
            return Err(error).context("moving into the library");
        }
        (dest, dest_jpeg)
    };

    let relative = |path: &Path| path.strip_prefix(library.root()).unwrap_or(path).to_string_lossy().into_owned();
    let inserted = library.insert_photo(&NewPhoto {
        path: relative(&dest),
        jpeg_path: dest_jpeg.as_deref().map(relative),
        file_name: file_name(&dest),
        kind: item.kind.as_str(),
        fingerprint: item.fingerprint.clone(),
        file_size: item.size,
        taken_at: taken_at.format("%Y-%m-%dT%H:%M:%S").to_string(),
        width: meta.width,
        height: meta.height,
        make: meta.make,
        model: meta.model,
        lens: meta.lens,
        iso: meta.iso,
        aperture: meta.aperture,
        shutter: meta.shutter,
        focal_length: meta.focal_length,
        import_id,
    });
    let id = match inserted {
        Ok(id) => id,
        Err(error) => {
            cleanup(&[Some(&dest), dest_jpeg.as_deref()]);
            return Err(error);
        }
    };

    // The review sheet may already have rendered this photo; otherwise render
    // now so the grid is ready. A photo that cannot be rendered is still imported.
    let scan_thumb = thumbs::scan_thumb_path(library, session_id, index);
    let thumb = library.thumb_path(id);
    let reused = scan_thumb.exists()
        && thumb.parent().is_some_and(|dir| fs::create_dir_all(dir).is_ok())
        && fs::rename(&scan_thumb, &thumb).is_ok();
    if !reused {
        let _ = thumbs::ensure_thumb(library, id);
    }
    Ok(Outcome::Imported)
}
