//! Library commands: import (copy into the library), queries, ratings and flags.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use rayon::prelude::*;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tonality_core::album::{AlbumId, AlbumInfo};
use tonality_core::library::{ImageQuery, ImageRow, ImportInfo, ImportSummary, NewImage};
use tonality_core::ImageId;
use ts_rs::TS;

use crate::history::CatalogState;
use crate::sidecar;
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
    pub albums: Vec<AlbumInfo>,
}

#[tauri::command]
pub async fn library_overview(
    store: State<'_, Store>,
    catalog: State<'_, CatalogState>,
) -> CmdResult<LibraryOverview> {
    let (total, imports, albums) = catalog
        .read(|c| Ok((c.image_count()?, c.imports()?, c.albums()?)))
        .await?;
    Ok(LibraryOverview {
        root: store.root().to_string_lossy().into_owned(),
        total,
        imports,
        albums,
    })
}

#[tauri::command]
pub async fn library_images(
    query: ImageQuery,
    catalog: State<'_, CatalogState>,
) -> CmdResult<Vec<ImageRow>> {
    catalog.read(move |c| c.images(&query)).await
}

#[tauri::command]
pub async fn library_set_rating(
    ids: Vec<ImageId>,
    rating: u8,
    catalog: State<'_, CatalogState>,
) -> CmdResult<()> {
    catalog
        .change(ids.clone(), move |c| c.set_rating(&ids, rating))
        .await
}

#[tauri::command]
pub async fn library_set_flag(
    ids: Vec<ImageId>,
    flag: i8,
    catalog: State<'_, CatalogState>,
) -> CmdResult<()> {
    catalog
        .change(ids.clone(), move |c| c.set_flag(&ids, flag))
        .await
}

// ---------------------------------------------------------------------------------------------
// Albums. Membership is recorded in each photo's sidecar (by album name), so every change
// marks the photos it touches dirty.
// ---------------------------------------------------------------------------------------------

#[tauri::command]
pub async fn album_create(
    name: String,
    images: Vec<ImageId>,
    catalog: State<'_, CatalogState>,
) -> CmdResult<AlbumId> {
    catalog
        .change(images.clone(), move |c| {
            let id = c.create_album(&name)?;
            c.add_to_album(id, &images)?;
            Ok(id)
        })
        .await
}

#[tauri::command]
pub async fn album_rename(
    id: AlbumId,
    name: String,
    catalog: State<'_, CatalogState>,
) -> CmdResult<()> {
    let members = catalog.read(move |c| c.album_members(id)).await?;
    catalog
        .change(members, move |c| c.rename_album(id, &name))
        .await
}

#[tauri::command]
pub async fn album_delete(id: AlbumId, catalog: State<'_, CatalogState>) -> CmdResult<()> {
    let members = catalog.read(move |c| c.album_members(id)).await?;
    catalog.change(members, move |c| c.delete_album(id)).await
}

#[tauri::command]
pub async fn album_add(
    id: AlbumId,
    images: Vec<ImageId>,
    catalog: State<'_, CatalogState>,
) -> CmdResult<u32> {
    catalog
        .change(images.clone(), move |c| c.add_to_album(id, &images))
        .await
}

#[tauri::command]
pub async fn album_remove(
    id: AlbumId,
    images: Vec<ImageId>,
    catalog: State<'_, CatalogState>,
) -> CmdResult<()> {
    catalog
        .change(images.clone(), move |c| c.remove_from_album(id, &images))
        .await
}

// ---------------------------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------------------------

/// RAW files under each path (files are taken as-is if they look like RAWs; folders are walked
/// recursively, skipping hidden entries and symlinks).
fn collect(paths: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
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
    let name_of = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
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
    /// Library-relative names claimed by in-flight copies, as [`name_key`]s.
    reserved: Mutex<HashSet<String>>,
}

/// How the filesystem compares names: case-folded where it is case-insensitive by default
/// (APFS on macOS, NTFS on Windows), so `IMG_0001.CR2` and `img_0001.cr2` are one name there.
fn name_key(file: &str) -> String {
    if cfg!(any(target_os = "macos", windows)) {
        file.to_lowercase()
    } else {
        file.to_string()
    }
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
            let file = if n == 1 {
                format!("{dir}/{name}")
            } else {
                format!("{dir}/{stem} ({n}){ext}")
            };
            if !reserved.contains(&name_key(&file))
                && !self.store.abs(&file).exists()
                && !self.catalog.with(|c| c.file_taken(&file))?
            {
                reserved.insert(name_key(&file));
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
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or("no file name")?;
        let dir = Store::originals_dir(meta.as_ref().and_then(|m| m.captured_at.as_deref()));
        let part = crate::store::temp_path(&self.store.abs(&format!("{dir}/{name}")));

        let copy = || -> io::Result<u64> {
            std::fs::create_dir_all(part.parent().unwrap())?;
            let n = std::fs::copy(src, &part)?;
            if n != std::fs::metadata(src)?.len() {
                return Err(io::Error::other("short copy"));
            }
            // `copy` gives the copy the source's permissions, and a read-only source (a locked
            // card, a photo coming back from the Trash) would leave one that can't be opened to
            // be flushed.
            make_writable(&part)?;
            // FlushFileBuffers on Windows needs write access; a read-only handle fails with
            // ERROR_ACCESS_DENIED.
            std::fs::OpenOptions::new()
                .write(true)
                .open(&part)?
                .sync_all()?;
            Ok(n)
        };
        let size = match copy() {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                return Err(format!("copy failed: {e}"));
            }
        };
        // Into a free name, never over a file: if something appears at the reserved name
        // first (another program, or a name differing only in case), take the next one.
        let mut placed = None;
        for _ in 0..100 {
            let file = match self.reserve(&dir, &name) {
                Ok(f) => f,
                Err(e) => {
                    let _ = std::fs::remove_file(&part);
                    return Err(e);
                }
            };
            let dest = self.store.abs(&file);
            match crate::store::rename_noreplace(&part, &dest) {
                Ok(()) => {
                    placed = Some((file, dest));
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    let _ = std::fs::remove_file(&part);
                    return Err(format!("copy failed: {e}"));
                }
            }
        }
        let Some((file, dest)) = placed else {
            let _ = std::fs::remove_file(&part);
            return Err("copy failed: no free name".into());
        };
        // From here `dest` is a file this call created, so it may remove it.
        // Originals are never modified; make that explicit.
        if let Err(e) = std::fs::metadata(&dest).and_then(|m| {
            let mut perm = m.permissions();
            perm.set_readonly(true);
            std::fs::set_permissions(&dest, perm)
        }) {
            remove_readonly(&dest);
            return Err(format!("copy failed: {e}"));
        }

        let new = NewImage {
            hash: hash.clone(),
            file,
            name,
            file_size: size as i64,
            source_path: src.to_string_lossy().into_owned(),
            meta,
            // Its thumbnail now (it'll be wanted as soon as the import shows), and with it the
            // shape the grid lays it out by.
            aspect: crate::thumbs::warm(self.store, &dest, &hash),
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

/// Let the owner write to `path`, if nobody can.
fn make_writable(path: &Path) -> io::Result<()> {
    let mut perm = std::fs::metadata(path)?.permissions();
    if !perm.readonly() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perm.set_mode(perm.mode() | 0o200);
    }
    #[cfg(not(unix))]
    #[allow(clippy::permissions_set_readonly_false)]
    perm.set_readonly(false);
    std::fs::set_permissions(path, perm)
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

/// Held by import and recovery: recovery must never see an original that an import has copied
/// but not yet catalogued.
static LIBRARY_WRITE: Mutex<()> = Mutex::new(());

/// Copy photos (files and/or folders) into the library. Emits `import-progress` events.
#[tauri::command]
pub async fn library_import(paths: Vec<String>, app: AppHandle) -> CmdResult<ImportSummary> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = LIBRARY_WRITE.lock().unwrap_or_else(|e| e.into_inner());
        let store = app.state::<Store>();
        let catalog = app.state::<CatalogState>();
        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        let files = collect(&paths);
        let total = files.len() as u32;
        let _ = app.emit("import-progress", ImportProgress { done: 0, total });
        if files.is_empty() {
            return Ok(ImportSummary {
                error: Some("No RAW files found".into()),
                ..Default::default()
            });
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
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .start_handler(|_| crate::priority::lower_this_thread())
            .build()
            .map_err(|e| e.to_string())?;
        // Pass 1: hash everything. Duplicates (already in the library, or the same file twice in
        // this import) are settled here, before anything is copied, and deterministically: the
        // first file in folder order wins.
        let hashes: Vec<Result<String, String>> = pool.install(|| {
            files
                .par_iter()
                .map(|f| hash_file(f).map_err(|e| e.to_string()))
                .collect()
        });
        let mut seen = HashSet::new();
        let mut work = Vec::new(); // (index, file, hash) to copy
        let mut outcomes: Vec<Option<Outcome>> = Vec::with_capacity(files.len());
        for (i, (f, h)) in files.iter().zip(hashes).enumerate() {
            outcomes.push(match h {
                Err(e) => Some(Outcome::Failed(format!("{}: {e}", f.display()))),
                Ok(h)
                    if !seen.insert(h.clone())
                        || catalog.with(|c| c.image_by_hash(&h))?.is_some() =>
                {
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
        let mut s = ImportSummary::default();
        for (src, o) in files.iter().zip(outcomes) {
            // Edits travelling with the photo (written by another tonality library) come along,
            // merged into any history the library already has for it.
            let mut adopt = |id| {
                if sidecar::adopt(&catalog, id, src) {
                    s.restored += 1;
                }
            };
            match o.expect("every file has an outcome") {
                Outcome::Added(id) => {
                    adopt(id);
                    s.added += 1;
                    s.images.push(id);
                }
                Outcome::Duplicate(hash) => {
                    s.duplicates += 1;
                    if let Some(id) = catalog.with(|c| c.image_by_hash(&hash))? {
                        adopt(id);
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

// ---------------------------------------------------------------------------------------------
// Removing photos
// ---------------------------------------------------------------------------------------------

/// What [`library_remove`] did.
#[derive(Debug, Default, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoveSummary {
    /// Photos taken out of the library; their files are in the system's bin.
    #[ts(type = "number[]")]
    pub removed: Vec<ImageId>,
    /// Photos left where they were, because their files couldn't be moved to the bin.
    pub failed: u32,
    /// The first of those failures.
    pub error: Option<String>,
}

/// Move files to the system's Trash / Recycle Bin.
fn to_bin(paths: &[PathBuf]) -> Result<(), String> {
    #[allow(unused_mut)]
    let mut bin = trash::TrashContext::default();
    // The default on macOS drives Finder by AppleScript, which asks the user for permission to
    // control Finder first. This way asks nothing (Finder then has no "Put Back" for the files;
    // they can still be dragged out of the Trash).
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        bin.set_delete_method(DeleteMethod::NsFileManager);
    }
    bin.delete_all(paths).map_err(|e| e.to_string())
}

/// Take `ids` out of the library: each photo's original and sidecar go to `discard` (the
/// system's bin), then its catalog rows and cached pictures are deleted. A photo whose files
/// can't be discarded is left alone; one whose files are already missing is just forgotten.
///
/// Files first, catalog second: stopping in between leaves photos listed without a file (which
/// can be removed again), not files that the next launch's recovery would bring back.
fn remove(
    store: &Store,
    catalog: &CatalogState,
    ids: &[ImageId],
    discard: &dyn Fn(&[PathBuf]) -> Result<(), String>,
) -> CmdResult<RemoveSummary> {
    let mut s = RemoveSummary::default();
    for &id in ids {
        let Ok((rel, _, name)) = catalog.with(|c| c.image_file(id)) else {
            continue; // not in the library (any more)
        };
        let original = store.abs(&rel);
        let files: Vec<PathBuf> = [sidecar::path_for(&original), original]
            .into_iter()
            .filter(|p| p.exists())
            .collect();
        if !files.is_empty() {
            if let Err(e) = discard(&files) {
                s.failed += 1;
                s.error.get_or_insert(format!("{name}: {e}"));
                continue;
            }
        }
        s.removed.push(id);
    }
    // Not `edit`: there's no sidecar left to keep in step.
    for (_, hash, _) in catalog.with(|c| c.remove_images(&s.removed))? {
        forget_cached(store, &hash);
    }
    Ok(s)
}

/// Delete everything cached for the photo with contents `hash`: its thumbnails and AI masks.
/// All of it can be made again, so failures are ignored.
fn forget_cached(store: &Store, hash: &str) {
    let _ = std::fs::remove_file(store.thumb_path(hash, crate::thumbs::EDGE));
    let _ = std::fs::remove_file(store.edited_thumb_path(hash));
    // `cache/masks/ab/cdef…-<model>-<target>.png`
    let any_mask = crate::ai::raster_path(store, hash, "", "");
    let prefix = format!("{}-", &hash[2..]);
    let Some(entries) = any_mask.parent().and_then(|d| std::fs::read_dir(d).ok()) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Remove photos from the library, moving their originals and sidecars to the system's Trash
/// (Recycle Bin on Windows). Their history, ratings and album places are deleted.
#[tauri::command]
pub async fn library_remove(ids: Vec<ImageId>, app: AppHandle) -> CmdResult<RemoveSummary> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = LIBRARY_WRITE.lock().unwrap_or_else(|e| e.into_inner());
        // So the sidecars that go to the bin hold everything: importing a photo back from
        // there, with its sidecar, brings its edits back.
        app.state::<sidecar::Sidecars>().flush()?;
        let s = remove(
            &app.state::<Store>(),
            &app.state::<CatalogState>(),
            &ids,
            &to_bin,
        )?;
        // Ids can be given out again, and these are kept by id.
        app.state::<crate::images::Images>().forget(&s.removed);
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------------------------

/// Catalogue originals that are in the library folder but not in the catalog (a lost or
/// replaced catalog, or files put there by hand), restoring their edits from sidecars.
/// Nothing is copied or moved. Emits `import-progress` while hashing.
#[tauri::command]
pub async fn library_recover(app: AppHandle) -> CmdResult<ImportSummary> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = LIBRARY_WRITE.lock().unwrap_or_else(|e| e.into_inner());
        let store = app.state::<Store>();
        let catalog = app.state::<CatalogState>();
        let root = store.root().to_path_buf();
        let rel = |p: &Path| -> Option<String> {
            let r = p.strip_prefix(&root).ok()?;
            Some(
                r.components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
            )
        };
        let mut untracked = Vec::new();
        for f in collect(&[root.join("originals")]) {
            if let Some(r) = rel(&f) {
                if !catalog.with(|c| c.file_taken(&r))? {
                    untracked.push((f, r));
                }
            }
        }
        let mut s = ImportSummary::default();
        if untracked.is_empty() {
            return Ok(s);
        }
        let total = untracked.len() as u32;
        let _ = app.emit("import-progress", ImportProgress { done: 0, total });
        let import = catalog
            .with(|c| c.begin_import("Recovered", &root.join("originals").to_string_lossy()))?;
        for (i, (path, file)) in untracked.into_iter().enumerate() {
            let added = (|| -> CmdResult<Option<ImageId>> {
                let hash = hash_file(&path).map_err(|e| e.to_string())?;
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let aspect = crate::thumbs::warm(&store, &path, &hash);
                let new = NewImage {
                    hash,
                    file,
                    name,
                    file_size: std::fs::metadata(&path).map_err(|e| e.to_string())?.len() as i64,
                    source_path: path.to_string_lossy().into_owned(),
                    meta: tonality_raw::read_meta(&path).ok(),
                    aspect,
                };
                // Same contents as a photo already catalogued: leave the stray copy alone.
                catalog.with(|c| c.add_image(import, &new))
            })();
            match added {
                Ok(Some(id)) => {
                    if sidecar::adopt(&catalog, id, &path) {
                        s.restored += 1;
                    }
                    s.added += 1;
                    s.images.push(id);
                }
                Ok(None) => s.duplicates += 1,
                Err(e) => {
                    let e = format!("{}: {e}", path.display());
                    eprintln!("[recover] {e}");
                    s.failed += 1;
                    s.error.get_or_insert(e);
                }
            }
            let _ = app.emit(
                "import-progress",
                ImportProgress {
                    done: i as u32 + 1,
                    total,
                },
            );
        }
        s.import = catalog.with(|c| c.finish_import(import))?;
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonality_core::Catalog;

    /// A library with one catalogued photo (file, sidecar and cached pictures all on disk).
    fn library_with_a_photo(tag: &str) -> (PathBuf, Store, CatalogState, ImageId) {
        let root =
            std::env::temp_dir().join(format!("tonality-remove-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = Store::open(root.clone()).unwrap();
        let catalog = CatalogState::new(Catalog::open_in_memory().unwrap());
        let file = "originals/2026/2026-11-02/DSC1.ARW";
        let hash = "abcdef0123";
        let write = |p: PathBuf| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        };
        write(store.abs(file));
        write(sidecar::path_for(&store.abs(file)));
        write(store.thumb_path(hash, crate::thumbs::EDGE));
        write(store.edited_thumb_path(hash));
        write(crate::ai::raster_path(&store, hash, "subject@1", "subject"));
        // Another photo's mask in the same cache folder.
        write(crate::ai::raster_path(
            &store,
            "abffff",
            "subject@1",
            "subject",
        ));
        let id = catalog
            .with(|c| {
                let import = c.begin_import("card", "/card")?;
                c.add_image(
                    import,
                    &NewImage {
                        hash: hash.into(),
                        file: file.into(),
                        name: "DSC1.ARW".into(),
                        file_size: 1,
                        source_path: "/card/DSC1.ARW".into(),
                        meta: None,
                        aspect: None,
                    },
                )
            })
            .unwrap()
            .unwrap();
        (root, store, catalog, id)
    }

    #[test]
    fn removing_a_photo_discards_its_files_and_forgets_it() {
        let (root, store, catalog, id) = library_with_a_photo("ok");
        let original = store.abs("originals/2026/2026-11-02/DSC1.ARW");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let discard = |paths: &[PathBuf]| -> Result<(), String> {
            for p in paths {
                std::fs::rename(p, bin.join(p.file_name().unwrap())).map_err(|e| e.to_string())?;
            }
            Ok(())
        };

        let s = remove(&store, &catalog, &[id], &discard).unwrap();

        assert_eq!(
            (s.removed.as_slice(), s.failed, &s.error),
            ([id].as_slice(), 0, &None)
        );
        assert!(
            bin.join("DSC1.ARW").exists(),
            "the original went to the bin"
        );
        assert!(
            bin.join("DSC1.ARW.tonality").exists(),
            "and its sidecar with it"
        );
        assert!(!original.exists() && !sidecar::path_for(&original).exists());
        assert!(catalog.with(|c| c.image_file(id)).is_err());
        assert!(!store.thumb_path("abcdef0123", crate::thumbs::EDGE).exists());
        assert!(!store.edited_thumb_path("abcdef0123").exists());
        assert!(!crate::ai::raster_path(&store, "abcdef0123", "subject@1", "subject").exists());
        assert!(
            crate::ai::raster_path(&store, "abffff", "subject@1", "subject").exists(),
            "another photo's cached mask is left alone"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_photo_that_cannot_be_discarded_stays_in_the_library() {
        let (root, store, catalog, id) = library_with_a_photo("fail");
        let discard = |_: &[PathBuf]| -> Result<(), String> { Err("no bin on this drive".into()) };

        let s = remove(&store, &catalog, &[id], &discard).unwrap();

        assert!(s.removed.is_empty());
        assert_eq!(s.failed, 1);
        assert!(s.error.unwrap().contains("no bin on this drive"));
        assert!(store.abs("originals/2026/2026-11-02/DSC1.ARW").exists());
        assert!(catalog.with(|c| c.image_file(id)).is_ok());
        assert!(store.thumb_path("abcdef0123", crate::thumbs::EDGE).exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_photo_whose_file_is_already_gone_can_still_be_removed() {
        let (root, store, catalog, id) = library_with_a_photo("gone");
        let original = store.abs("originals/2026/2026-11-02/DSC1.ARW");
        std::fs::remove_file(&original).unwrap();
        std::fs::remove_file(sidecar::path_for(&original)).unwrap();
        let discard = |paths: &[PathBuf]| -> Result<(), String> {
            panic!("nothing to discard, but asked to discard {paths:?}")
        };

        let s = remove(&store, &catalog, &[id], &discard).unwrap();

        assert_eq!(s.removed, [id]);
        assert!(catalog.with(|c| c.image_file(id)).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_read_only_file_can_be_imported() {
        // Originals in the library are read-only, so this is every photo that comes back from
        // the Trash, as well as files on a locked card.
        let root = std::env::temp_dir().join(format!("tonality-import-ro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = Store::open(root.join("library")).unwrap();
        let catalog = CatalogState::new(Catalog::open_in_memory().unwrap());
        let import = catalog.with(|c| c.begin_import("card", "/card")).unwrap();
        let src = root.join("DSC2.ARW");
        std::fs::write(&src, b"not really a raw").unwrap();
        let mut perm = std::fs::metadata(&src).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&src, perm).unwrap();
        let importer = Importer {
            store: &store,
            catalog: &catalog,
            import,
            reserved: Mutex::default(),
        };

        let outcome = importer.one(&src, "feedbeef".into());

        match outcome {
            Outcome::Added(id) => {
                let (file, _, _) = catalog.with(|c| c.image_file(id)).unwrap();
                let copy = store.abs(&file);
                assert_eq!(std::fs::read(&copy).unwrap(), b"not really a raw");
                assert!(std::fs::metadata(&copy).unwrap().permissions().readonly());
                remove_readonly(&copy);
            }
            Outcome::Duplicate(_) => panic!("duplicate"),
            Outcome::Failed(e) => panic!("{e}"),
        }
        remove_readonly(&src);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reserved_names_compare_like_the_filesystem() {
        let root =
            std::env::temp_dir().join(format!("tonality-import-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = Store::open(root.clone()).unwrap();
        let catalog = CatalogState::new(Catalog::open_in_memory().unwrap());
        let importer = Importer {
            store: &store,
            catalog: &catalog,
            import: 0,
            reserved: Mutex::default(),
        };
        let dir = "originals/2026/2026-11-02";
        assert_eq!(
            importer.reserve(dir, "IMG_0001.CR2").unwrap(),
            format!("{dir}/IMG_0001.CR2")
        );
        let other = importer.reserve(dir, "img_0001.cr2").unwrap();
        if cfg!(any(target_os = "macos", windows)) {
            assert_eq!(other, format!("{dir}/img_0001 (2).cr2"));
        } else {
            assert_eq!(other, format!("{dir}/img_0001.cr2"));
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
