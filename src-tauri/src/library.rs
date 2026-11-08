//! Library commands: import (copy into the library), queries, ratings and flags.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use rayon::prelude::*;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tonality_core::library::{ImageQuery, ImageRow, ImportInfo, ImportSummary, NewImage};
use tonality_core::ImageId;
use ts_rs::TS;

use crate::history::CatalogState;
use crate::store::Store;
use crate::CmdResult;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportProgress {
    done: u32,
    total: u32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryOverview {
    pub root: String,
    pub total: u32,
    pub imports: Vec<ImportInfo>,
}

#[tauri::command]
pub fn library_overview(store: State<'_, Store>, catalog: State<'_, CatalogState>) -> CmdResult<LibraryOverview> {
    let (total, imports) = catalog.with(|c| Ok((c.image_count()?, c.imports()?)))?;
    Ok(LibraryOverview { root: store.root().to_string_lossy().into_owned(), total, imports })
}

#[tauri::command]
pub fn library_images(query: ImageQuery, catalog: State<'_, CatalogState>) -> CmdResult<Vec<ImageRow>> {
    catalog.with(|c| c.images(&query))
}

#[tauri::command]
pub fn library_set_rating(ids: Vec<ImageId>, rating: u8, catalog: State<'_, CatalogState>) -> CmdResult<()> {
    catalog.with(|c| c.set_rating(&ids, rating))
}

#[tauri::command]
pub fn library_set_flag(ids: Vec<ImageId>, flag: i8, catalog: State<'_, CatalogState>) -> CmdResult<()> {
    catalog.with(|c| c.set_flag(&ids, flag))
}

// ---------------------------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------------------------

/// RAW files under each path (files are taken as-is if they look like RAWs; folders are walked
/// recursively, skipping hidden entries and symlinks).
fn collect(paths: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                walk(&e.path(), out);
            } else if ft.is_file() && tonality_raw::is_raw_path(&e.path()) {
                out.push(e.path());
            }
        }
    }
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            walk(p, &mut out);
        } else if p.is_file() && tonality_raw::is_raw_path(p) {
            out.push(p.clone());
        }
    }
    let mut seen = HashSet::new();
    out.retain(|p| seen.insert(p.canonicalize().unwrap_or_else(|_| p.clone())));
    out
}

fn import_name(paths: &[PathBuf]) -> (String, String) {
    let name_of = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match paths {
        [p] if p.is_dir() => (name_of(p), p.to_string_lossy().into_owned()),
        [p, ..] => {
            let parent = p.parent().unwrap_or(p);
            (name_of(parent), parent.to_string_lossy().into_owned())
        }
        [] => ("Import".into(), String::new()),
    }
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut h = blake3::Hasher::new();
    h.update_reader(std::fs::File::open(path)?)?;
    Ok(h.finalize().to_hex().to_string())
}

enum Outcome {
    Added(ImageId),
    Duplicate(String),
    Failed(String),
}

struct Importer<'a> {
    store: &'a Store,
    catalog: &'a CatalogState,
    import: i64,
    /// Library-relative names claimed by in-flight copies.
    reserved: Mutex<HashSet<String>>,
}

impl Importer<'_> {
    /// Pick a free library-relative name in `dir`, appending " (2)", " (3)"… on collision.
    fn reserve(&self, dir: &str, name: &str) -> CmdResult<String> {
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (name.to_string(), String::new()),
        };
        let mut reserved = self.reserved.lock().unwrap();
        for n in 1.. {
            let file = if n == 1 { format!("{dir}/{name}") } else { format!("{dir}/{stem} ({n}){ext}") };
            if !reserved.contains(&file)
                && !self.store.abs(&file).exists()
                && !self.catalog.with(|c| c.file_taken(&file))?
            {
                reserved.insert(file.clone());
                return Ok(file);
            }
        }
        unreachable!()
    }

    fn one(&self, src: &Path, hash: String) -> Outcome {
        match self.try_one(src, hash) {
            Ok(o) => o,
            Err(e) => Outcome::Failed(format!("{}: {e}", src.display())),
        }
    }

    fn try_one(&self, src: &Path, hash: String) -> CmdResult<Outcome> {
        let meta = tonality_raw::read_meta(src).ok();
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("no file name")?;
        let file = self.reserve(&Store::originals_dir(meta.as_ref().and_then(|m| m.captured_at.as_deref())), &name)?;
        let dest = self.store.abs(&file);
        let part = dest.with_extension("part");

        let copy = || -> io::Result<u64> {
            std::fs::create_dir_all(dest.parent().unwrap())?;
            let n = std::fs::copy(src, &part)?;
            if n != std::fs::metadata(src)?.len() {
                return Err(io::Error::other("short copy"));
            }
            std::fs::File::open(&part)?.sync_all()?;
            // Originals are never modified; make that explicit.
            let mut perm = std::fs::metadata(&part)?.permissions();
            perm.set_readonly(true);
            std::fs::set_permissions(&part, perm)?;
            std::fs::rename(&part, &dest)?;
            Ok(n)
        };
        let size = match copy() {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                return Err(format!("copy failed: {e}"));
            }
        };

        let new = NewImage {
            hash: hash.clone(),
            file,
            name,
            file_size: size as i64,
            source_path: src.to_string_lossy().into_owned(),
            meta,
        };
        match self.catalog.with(|c| c.add_image(self.import, &new)) {
            Ok(Some(id)) => Ok(Outcome::Added(id)),
            Ok(None) => {
                remove_readonly(&dest);
                Ok(Outcome::Duplicate(hash))
            }
            Err(e) => {
                remove_readonly(&dest);
                Err(e)
            }
        }
    }
}

fn remove_readonly(path: &Path) {
    if let Ok(md) = std::fs::metadata(path) {
        let mut perm = md.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        let _ = std::fs::set_permissions(path, perm);
    }
    let _ = std::fs::remove_file(path);
}

/// Copy photos (files and/or folders) into the library. Emits `import-progress` events.
#[tauri::command]
pub async fn library_import(paths: Vec<String>, app: AppHandle) -> CmdResult<ImportSummary> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = app.state::<Store>();
        let catalog = app.state::<CatalogState>();
        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        let files = collect(&paths);
        let total = files.len() as u32;
        let _ = app.emit("import-progress", ImportProgress { done: 0, total });
        if files.is_empty() {
            return Ok(ImportSummary { error: Some("No RAW files found".into()), ..Default::default() });
        }

        let (name, source) = import_name(&paths);
        let import = catalog.with(|c| c.begin_import(&name, &source))?;
        let importer = Importer {
            store: &store,
            catalog: &catalog,
            import,
            reserved: Mutex::default(),
        };
        // Copying is IO-bound; a few workers saturate a card reader or SSD without thrashing.
        let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().map_err(|e| e.to_string())?;
        // Pass 1: hash everything. Duplicates (already in the library, or the same file twice in
        // this import) are settled here, before anything is copied, and deterministically: the
        // first file in folder order wins.
        let hashes: Vec<Result<String, String>> =
            pool.install(|| files.par_iter().map(|f| hash_file(f).map_err(|e| e.to_string())).collect());
        let mut seen = HashSet::new();
        let mut work = Vec::new(); // (index, file, hash) to copy
        let mut outcomes: Vec<Option<Outcome>> = Vec::with_capacity(files.len());
        for (i, (f, h)) in files.iter().zip(hashes).enumerate() {
            outcomes.push(match h {
                Err(e) => Some(Outcome::Failed(format!("{}: {e}", f.display()))),
                Ok(h) if !seen.insert(h.clone()) || catalog.with(|c| c.image_by_hash(&h))?.is_some() => {
                    Some(Outcome::Duplicate(h))
                }
                Ok(h) => {
                    work.push((i, f, h));
                    None
                }
            });
        }
        let dupes = (files.len() - work.len()) as u32;
        let _ = app.emit("import-progress", ImportProgress { done: dupes, total });

        // Pass 2: copy and catalogue the new ones.
        let done = AtomicU32::new(dupes);
        let copied: Vec<(usize, Outcome)> = pool.install(|| {
            work.into_par_iter()
                .map(|(i, f, h)| {
                    let o = importer.one(f, h);
                    let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if n.is_multiple_of(4) || n == total {
                        let _ = app.emit("import-progress", ImportProgress { done: n, total });
                    }
                    (i, o)
                })
                .collect()
        });
        for (i, o) in copied {
            outcomes[i] = Some(o);
        }
        let outcomes = outcomes.into_iter().flatten();

        let mut s = ImportSummary::default();
        for o in outcomes {
            match o {
                Outcome::Added(id) => {
                    s.added += 1;
                    s.images.push(id);
                }
                Outcome::Duplicate(hash) => {
                    s.duplicates += 1;
                    if let Some(id) = catalog.with(|c| c.image_by_hash(&hash))? {
                        s.images.push(id);
                    }
                }
                Outcome::Failed(e) => {
                    eprintln!("[import] {e}");
                    s.failed += 1;
                    s.error.get_or_insert(e);
                }
            }
        }
        s.import = catalog.with(|c| c.finish_import(import))?;
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}
