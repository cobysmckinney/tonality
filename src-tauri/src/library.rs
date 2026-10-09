//! The managed library: one folder on disk holding the originals, the
//! database that indexes them, and the generated thumbnails and previews.
//!
//! ```text
//! Tonality/
//!   Originals/2026/2026-10-04/IMG_0001.CR3
//!   Exports/IMG_0001.jpg                      (unless another folder is chosen)
//!   .tonality/library.db   .tonality/lock
//!   .tonality/thumbs/…   .tonality/previews/…   .tonality/mattes/…
//! ```

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::search::{self, Facet, Facets, Search, Sort};

/// How long a photo stays in Recently Deleted before its files are removed.
pub const TRASH_RETENTION_DAYS: i64 = 30;

const SCHEMA: &str = "
CREATE TABLE imports (
    id          INTEGER PRIMARY KEY,
    created_at  INTEGER NOT NULL,
    source      TEXT NOT NULL
);

CREATE TABLE photos (
    -- AUTOINCREMENT: an id is never reused, so cached thumbnails stay valid.
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    -- Paths are relative to the library root. `path` is the file that gets
    -- edited (the RAW of a RAW+JPEG pair); `jpeg_path` is its paired JPEG.
    path          TEXT NOT NULL UNIQUE,
    jpeg_path     TEXT,
    file_name     TEXT NOT NULL,
    kind          TEXT NOT NULL,
    fingerprint   TEXT NOT NULL,
    file_size     INTEGER NOT NULL,
    -- Camera-local capture time, 'YYYY-MM-DDTHH:MM:SS'.
    taken_at      TEXT NOT NULL,
    width         INTEGER,
    height        INTEGER,
    make          TEXT,
    model         TEXT,
    lens          TEXT,
    iso           INTEGER,
    aperture      REAL,
    shutter       REAL,
    focal_length  REAL,
    favorite      INTEGER NOT NULL DEFAULT 0,
    -- 1 = pick, -1 = reject, 0 = unflagged.
    flag          INTEGER NOT NULL DEFAULT 0,
    import_id     INTEGER NOT NULL REFERENCES imports(id),
    deleted_at    INTEGER
);
CREATE INDEX photos_taken_at ON photos(taken_at);
CREATE INDEX photos_fingerprint ON photos(fingerprint);
CREATE INDEX photos_import ON photos(import_id);

CREATE TABLE albums (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE album_photos (
    album_id  INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
    photo_id  INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    PRIMARY KEY (album_id, photo_id)
) WITHOUT ROWID;
CREATE INDEX album_photos_photo ON album_photos(photo_id);
";

/// Version 2: photos carry an edit recipe.
const ADD_EDITS: &str = "
-- The edit recipe as JSON; NULL for a photo that is as shot.
ALTER TABLE photos ADD COLUMN edits TEXT;
-- Bumped whenever the recipe changes, so stale thumbnails are not reused.
ALTER TABLE photos ADD COLUMN version INTEGER NOT NULL DEFAULT 0;
";

/// Version 4: exports are recorded, and the app remembers a few choices.
const ADD_SETTINGS: &str = "
-- Choices kept between runs, each stored as JSON under a name.
CREATE TABLE settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
) WITHOUT ROWID;
";

/// Version 7: photos of film carry the film's own details.
const ADD_FILM: &str = "
-- What a film photo was shot on, typed in by hand: a camera scan's own
-- details describe the scanning camera instead. NULL where not given.
ALTER TABLE photos ADD COLUMN film_stock TEXT;
ALTER TABLE photos ADD COLUMN film_iso INTEGER;
ALTER TABLE photos ADD COLUMN film_camera TEXT;
ALTER TABLE photos ADD COLUMN film_lens TEXT;
ALTER TABLE photos ADD COLUMN film_frame INTEGER;
";

/// Which slice of the library a grid shows.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum View {
    Library,
    Favorites,
    Imports,
    Deleted,
    Album { id: i64 },
}

/// What the grid needs to draw one photo.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoItem {
    pub id: i64,
    pub file_name: String,
    pub kind: String,
    pub has_jpeg: bool,
    pub taken_at: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub favorite: bool,
    pub flag: i8,
    pub import_id: i64,
    pub deleted_at: Option<i64>,
    pub edited: bool,
    /// Changes whenever the photo's rendering does; part of its image URLs.
    pub version: i64,
    /// How many branches of edits the photo has; the grid shows the current one.
    pub branches: u32,
    /// The original is no longer in the library folder: moved or deleted outside the app.
    pub missing: bool,
}

/// Everything the info panel shows about one photo.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoInfo {
    pub id: i64,
    pub path: String,
    pub jpeg_path: Option<String>,
    pub file_size: u64,
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<u32>,
    pub aperture: Option<f64>,
    pub shutter: Option<f64>,
    pub focal_length: Option<f64>,
    pub imported_at: i64,
    pub albums: Vec<String>,
    pub film: FilmDetails,
}

/// What a photo of film was shot on and with, as typed in. These are details
/// about the photo, like its flag, rather than edits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilmDetails {
    /// The film stock: "Kodak Portra 400".
    pub stock: Option<String>,
    /// The speed the film was shot at, which may not be the box speed.
    pub iso: Option<u32>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    /// The frame's number on the roll.
    pub frame: Option<u32>,
}

impl FilmDetails {
    /// Whether any detail is given, which makes the photo one of film.
    pub fn is_set(&self) -> bool {
        *self != Self::default()
    }

    /// Text trimmed, with blanks and an ISO of 0 left out.
    pub fn tidied(self) -> Self {
        let text = |value: Option<String>| value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        Self {
            stock: text(self.stock),
            iso: self.iso.filter(|&iso| iso > 0),
            camera: text(self.camera),
            lens: text(self.lens),
            frame: self.frame,
        }
    }

    /// Reads the columns of [`FILM_COLUMNS`], starting at column `at`.
    pub(crate) fn from_row(r: &Row, at: usize) -> rusqlite::Result<Self> {
        Ok(Self { stock: r.get(at)?, iso: r.get(at + 1)?, camera: r.get(at + 2)?, lens: r.get(at + 3)?, frame: r.get(at + 4)? })
    }
}

/// The film columns, in the order [`FilmDetails::from_row`] reads them.
pub(crate) const FILM_COLUMNS: &str = "p.film_stock, p.film_iso, p.film_camera, p.film_lens, p.film_frame";

/// One photo's film details, with what puts photos in the order they were taken.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilmPhoto {
    pub id: i64,
    #[serde(default)]
    pub taken_at: String,
    #[serde(default)]
    pub file_name: String,
    pub film: FilmDetails,
}

/// Film details already in use, to offer while typing; the most used first.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilmSuggestions {
    pub stocks: Vec<String>,
    pub cameras: Vec<String>,
    pub lenses: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: i64,
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportInfo {
    pub id: i64,
    pub created_at: i64,
    pub source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub library_path: String,
    pub photo_count: u32,
    pub favorite_count: u32,
    pub deleted_count: u32,
    pub albums: Vec<Album>,
    pub imports: Vec<ImportInfo>,
}

/// A photo whose files are already in place under `Originals/`.
#[derive(Debug)]
pub struct NewPhoto {
    pub path: String,
    pub jpeg_path: Option<String>,
    pub file_name: String,
    pub kind: &'static str,
    pub fingerprint: String,
    pub file_size: u64,
    pub taken_at: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<u32>,
    pub aperture: Option<f64>,
    pub shutter: Option<f64>,
    pub focal_length: Option<f64>,
    pub import_id: i64,
}

/// The files a renderer needs to draw a photo.
#[derive(Debug)]
pub struct PhotoFiles {
    pub path: PathBuf,
    pub jpeg_path: Option<PathBuf>,
    pub is_raw: bool,
}

pub struct Library {
    root: PathBuf,
    db: Mutex<Connection>,
    /// Held for as long as the library is open, so a second copy of the app
    /// can't open it too.
    _lock: fs::File,
}

/// The library is already open in another copy of the app.
#[derive(Debug)]
pub struct LibraryInUse;

impl std::fmt::Display for LibraryInUse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("this library is already open in another copy of Tonality")
    }
}

impl std::error::Error for LibraryInUse {}

/// The photos a view shows, as `photos p …` ending in a WHERE clause that
/// more conditions can be added to with AND. Its values go onto `values`.
fn in_view(view: View, values: &mut Vec<Value>) -> String {
    match view {
        View::Library | View::Imports => "photos p WHERE p.deleted_at IS NULL".to_string(),
        View::Favorites => "photos p WHERE p.deleted_at IS NULL AND p.favorite = 1".to_string(),
        View::Deleted => "photos p WHERE p.deleted_at IS NOT NULL".to_string(),
        View::Album { id } => {
            values.push(Value::Integer(id));
            format!(
                "photos p JOIN album_photos ap ON ap.photo_id = p.id AND ap.album_id = ?{} WHERE p.deleted_at IS NULL",
                values.len()
            )
        }
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Library {
    /// Opens the library at `root`, creating it if it does not exist yet.
    /// Fails with [`LibraryInUse`] while another process has it open.
    pub fn open(root: &Path) -> Result<Self> {
        let data = root.join(".tonality");
        for dir in [root.join("Originals"), data.join("thumbs"), data.join("previews")] {
            fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(data.join("lock"))
            .context("opening library lock")?;
        // The operating system lets go of the lock when the process ends, even if it crashes.
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Err(LibraryInUse.into()),
            Err(fs::TryLockError::Error(error)) => return Err(error).context("locking library"),
        }
        let db = Connection::open(data.join("library.db")).context("opening library database")?;
        // The first statement is where a locked or damaged database shows itself.
        db.pragma_update(None, "journal_mode", "WAL").context("opening library database")?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 0 {
            db.execute_batch(&format!("BEGIN; {SCHEMA} PRAGMA user_version = 1; COMMIT;"))
                .context("creating library schema")?;
        }
        if version < 2 {
            db.execute_batch(&format!("BEGIN; {ADD_EDITS} PRAGMA user_version = 2; COMMIT;"))
                .context("upgrading library to hold edits")?;
        }
        if version < 3 {
            db.execute_batch(&format!("BEGIN; {} PRAGMA user_version = 3; COMMIT;", crate::history::SCHEMA))
                .context("upgrading library to hold edit history")?;
        }
        if version < 4 {
            db.execute_batch(&format!("BEGIN; {ADD_SETTINGS} {} PRAGMA user_version = 4; COMMIT;", crate::export::SCHEMA))
                .context("upgrading library to record exports")?;
        }
        if version < 5 {
            db.execute_batch(&format!("BEGIN; {} PRAGMA user_version = 5; COMMIT;", crate::presets::SCHEMA))
                .context("upgrading library to hold presets")?;
        }
        if version < 6 {
            db.execute_batch(&format!("BEGIN; {} PRAGMA user_version = 6; COMMIT;", crate::presets::FAVORITES_SCHEMA))
                .context("upgrading library to hold favorite presets")?;
        }
        if version < 7 {
            db.execute_batch(&format!("BEGIN; {ADD_FILM} PRAGMA user_version = 7; COMMIT;"))
                .context("upgrading library to hold film details")?;
        }
        let library = Self { root: root.to_path_buf(), db: Mutex::new(db), _lock: lock };
        // Leftovers from an import or review that was interrupted.
        let _ = fs::remove_dir_all(library.incoming_dir());
        let _ = fs::remove_dir_all(library.scan_cache_dir());
        crate::thumbs::forget_old_looks(&library).context("checking thumbnails against this version")?;
        Ok(library)
    }

    pub(crate) fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn originals_dir(&self) -> PathBuf {
        self.root.join("Originals")
    }

    /// Where files land while an import works out which date folder they belong in.
    pub fn incoming_dir(&self) -> PathBuf {
        self.root.join(".tonality/incoming")
    }

    pub fn scan_cache_dir(&self) -> PathBuf {
        self.root.join(".tonality/scan")
    }

    pub fn thumb_path(&self, id: i64) -> PathBuf {
        self.root.join(format!(".tonality/thumbs/{:02x}/{id}.jpg", id & 0xff))
    }

    pub fn preview_path(&self, id: i64) -> PathBuf {
        self.root.join(format!(".tonality/previews/{:02x}/{id}.jpg", id & 0xff))
    }

    /// Where the photo's found mattes are kept (`segment`): the start of
    /// their file names, which go on with each matte's key.
    pub fn matte_files(&self, id: i64) -> PathBuf {
        self.root.join(format!(".tonality/mattes/{:02x}/{id}-", id & 0xff))
    }

    // ---- settings ----

    /// A remembered choice, as the JSON it was stored as.
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.db().query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.db().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    // ---- reading ----

    pub fn overview(&self) -> Result<Overview> {
        self.purge_expired_quietly();
        let db = self.db();
        let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, u32>(0));
        let albums = db
            .prepare(
                "SELECT a.id, a.name, COUNT(p.id) FROM albums a
                 LEFT JOIN album_photos ap ON ap.album_id = a.id
                 LEFT JOIN photos p ON p.id = ap.photo_id AND p.deleted_at IS NULL
                 GROUP BY a.id ORDER BY a.name COLLATE NOCASE",
            )?
            .query_map([], |r| Ok(Album { id: r.get(0)?, name: r.get(1)?, count: r.get(2)? }))?
            .collect::<rusqlite::Result<_>>()?;
        let imports = db
            .prepare("SELECT id, created_at, source FROM imports ORDER BY id DESC")?
            .query_map([], |r| Ok(ImportInfo { id: r.get(0)?, created_at: r.get(1)?, source: r.get(2)? }))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Overview {
            library_path: self.root.to_string_lossy().into_owned(),
            photo_count: count("SELECT COUNT(*) FROM photos WHERE deleted_at IS NULL")?,
            favorite_count: count("SELECT COUNT(*) FROM photos WHERE deleted_at IS NULL AND favorite = 1")?,
            deleted_count: count("SELECT COUNT(*) FROM photos WHERE deleted_at IS NOT NULL")?,
            albums,
            imports,
        })
    }

    pub fn list_photos(&self, view: View) -> Result<Vec<PhotoItem>> {
        self.find_photos(view, &Search::default(), Sort::default())
    }

    /// The photos in a view that match the search, in the order asked for.
    pub fn find_photos(&self, view: View, search: &Search, sort: Sort) -> Result<Vec<PhotoItem>> {
        const COLUMNS: &str = "p.id, p.file_name, p.kind, p.jpeg_path IS NOT NULL, p.taken_at, \
             p.width, p.height, p.favorite, p.flag, p.import_id, p.deleted_at, p.edits IS NOT NULL, p.version, \
             (SELECT COUNT(*) FROM edit_branches b WHERE b.photo_id = p.id), p.path";
        let order = match (view, sort) {
            (View::Deleted, _) => "p.deleted_at DESC, p.id DESC",
            (View::Imports, _) | (_, Sort::Imported) => "p.import_id DESC, p.taken_at DESC, p.id DESC",
            (_, Sort::Newest) => "p.taken_at DESC, p.id DESC",
            (_, Sort::Oldest) => "p.taken_at ASC, p.id ASC",
            (_, Sort::Name) => "p.file_name COLLATE NOCASE ASC, p.id ASC",
        };
        if matches!(view, View::Deleted) {
            self.purge_expired_quietly();
        }
        let mut values = Vec::new();
        let photos = in_view(view, &mut values);
        let matching = search::conditions(search, &mut values);
        let db = self.db();
        let mut stmt = db.prepare(&format!("SELECT {COLUMNS} FROM {photos} AND {matching} ORDER BY {order}"))?;
        let map = |r: &Row| {
            let item = PhotoItem {
                id: r.get(0)?,
                file_name: r.get(1)?,
                kind: r.get(2)?,
                has_jpeg: r.get(3)?,
                taken_at: r.get(4)?,
                width: r.get(5)?,
                height: r.get(6)?,
                favorite: r.get(7)?,
                flag: r.get(8)?,
                import_id: r.get(9)?,
                deleted_at: r.get(10)?,
                edited: r.get(11)?,
                version: r.get(12)?,
                branches: r.get(13)?,
                missing: false,
            };
            Ok((item, r.get::<_, String>(14)?))
        };
        let rows: Vec<(PhotoItem, String)> =
            stmt.query_map(params_from_iter(&values), map)?.collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        drop(db);
        // A quick look at each file, in parallel, so a large library still lists promptly.
        let mut items: Vec<PhotoItem> = rows
            .into_par_iter()
            .map(|(mut item, path)| {
                item.missing = !self.root.join(path).exists();
                item
            })
            .collect();
        let missing: Vec<i64> = items.iter().filter(|item| item.missing).map(|item| item.id).collect();
        if !missing.is_empty() {
            // Moved within the library folder, perhaps. Not finding them mustn't stop the library from showing.
            match self.find_moved(&missing) {
                Ok(found) => items.iter_mut().filter(|item| found.contains(&item.id)).for_each(|item| item.missing = false),
                Err(error) => eprintln!("looking for moved originals: {error:#}"),
            }
        }
        Ok(items)
    }

    /// The cameras, lenses, film stocks and months of the photos in a view,
    /// to offer as things to search for.
    pub fn search_facets(&self, view: View) -> Result<Facets> {
        let mut values = Vec::new();
        let photos = in_view(view, &mut values);
        let db = self.db();
        let shown = format!("WITH shown AS (SELECT p.* FROM {photos})");
        let counted = |values_sql: &str, order: &str| -> Result<Vec<Facet>> {
            let mut stmt = db.prepare(&format!(
                "{shown} SELECT value, COUNT(DISTINCT id) FROM ({values_sql})
                 WHERE value IS NOT NULL AND trim(value) != '' GROUP BY value ORDER BY {order}"
            ))?;
            let facets = stmt
                .query_map(params_from_iter(&values), |r| Ok(Facet { value: r.get(0)?, count: r.get(1)? }))?
                .collect::<rusqlite::Result<_>>()?;
            Ok(facets)
        };
        let most_used = "COUNT(DISTINCT id) DESC, value COLLATE NOCASE";
        Ok(Facets {
            cameras: counted(
                &format!(
                    "SELECT p.id, {} AS value FROM shown p UNION ALL SELECT p.id, p.film_camera FROM shown p",
                    search::CAMERA
                ),
                most_used,
            )?,
            lenses: counted("SELECT p.id, p.lens AS value FROM shown p UNION ALL SELECT p.id, p.film_lens FROM shown p", most_used)?,
            films: counted("SELECT p.id, p.film_stock AS value FROM shown p", most_used)?,
            months: counted("SELECT p.id, substr(p.taken_at, 1, 7) AS value FROM shown p", "value DESC")?,
        })
    }

    pub fn photo_info(&self, id: i64) -> Result<PhotoInfo> {
        let db = self.db();
        let albums = db
            .prepare(
                "SELECT a.name FROM albums a JOIN album_photos ap ON ap.album_id = a.id
                 WHERE ap.photo_id = ?1 ORDER BY a.name COLLATE NOCASE",
            )?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let absolute = |relative: String| self.root.join(relative).to_string_lossy().into_owned();
        db.query_row(
            &format!(
                "SELECT p.path, p.jpeg_path, p.file_size, p.make, p.model, p.lens, p.iso, p.aperture,
                        p.shutter, p.focal_length, i.created_at, {FILM_COLUMNS}
                 FROM photos p JOIN imports i ON i.id = p.import_id WHERE p.id = ?1"
            ),
            [id],
            |r| {
                Ok(PhotoInfo {
                    id,
                    path: absolute(r.get(0)?),
                    jpeg_path: r.get::<_, Option<String>>(1)?.map(absolute),
                    file_size: r.get::<_, i64>(2)? as u64,
                    make: r.get(3)?,
                    model: r.get(4)?,
                    lens: r.get(5)?,
                    iso: r.get(6)?,
                    aperture: r.get(7)?,
                    shutter: r.get(8)?,
                    focal_length: r.get(9)?,
                    imported_at: r.get(10)?,
                    albums,
                    film: FilmDetails::from_row(r, 11)?,
                })
            },
        )
        .with_context(|| format!("photo {id} is not in the library"))
    }

    /// Where the photo's files should be, whether or not they are there.
    pub fn expected_files(&self, id: i64) -> Result<PhotoFiles> {
        self.db()
            .query_row("SELECT path, jpeg_path, kind FROM photos WHERE id = ?1", [id], |r| {
                Ok(PhotoFiles {
                    path: self.root.join(r.get::<_, String>(0)?),
                    jpeg_path: r.get::<_, Option<String>>(1)?.map(|p| self.root.join(p)),
                    is_raw: r.get::<_, String>(2)? == "raw",
                })
            })
            .with_context(|| format!("photo {id} is not in the library"))
    }

    /// Fails with a plain message when the original has gone from the library
    /// folder and can't be found elsewhere in it.
    pub fn photo_files(&self, id: i64) -> Result<PhotoFiles> {
        let files = self.expected_files(id)?;
        if files.path.exists() {
            return Ok(files);
        }
        if self.find_moved(&[id])?.contains(&id) {
            return self.expected_files(id);
        }
        let relative = files.path.strip_prefix(&self.root).unwrap_or(&files.path);
        bail!(
            "The original file is missing from the library folder ({}). \
             Put it back there to edit or export this photo.",
            relative.display()
        );
    }

    /// Which of these photos' originals are missing from the library folder,
    /// after looking for any that were moved within it.
    pub fn missing_originals(&self, ids: &[i64]) -> Result<HashSet<i64>> {
        let mut missing = Vec::new();
        for &id in ids {
            if !self.expected_files(id)?.path.exists() {
                missing.push(id);
            }
        }
        if missing.is_empty() {
            return Ok(HashSet::new());
        }
        let found = self.find_moved(&missing)?;
        Ok(missing.into_iter().filter(|id| !found.contains(id)).collect())
    }

    /// Looks through `Originals` for these photos' originals, which aren't
    /// where the library expects them: moved to another folder by hand, say.
    /// A file with the photo's name and fingerprint is its original, and the
    /// library follows it there, along with a paired JPEG of the same name
    /// that sits next to it. Returns the photos found.
    pub fn find_moved(&self, ids: &[i64]) -> Result<HashSet<i64>> {
        struct Lost {
            id: i64,
            fingerprint: String,
            size: u64,
            jpeg: Option<PathBuf>,
        }
        // By file name, which is what a quick walk of the folders can match on.
        let mut wanted: HashMap<OsString, Vec<Lost>> = HashMap::new();
        {
            let db = self.db();
            let mut stmt = db.prepare("SELECT path, jpeg_path, fingerprint, file_size FROM photos WHERE id = ?1")?;
            for &id in ids {
                let row = stmt
                    .query_row([id], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get(2)?, r.get::<_, i64>(3)?))
                    })
                    .optional()?;
                let Some((path, jpeg, fingerprint, size)) = row else { continue };
                if let Some(name) = Path::new(&path).file_name() {
                    let jpeg = jpeg.map(|jpeg| self.root.join(jpeg));
                    wanted.entry(name.to_owned()).or_default().push(Lost { id, fingerprint, size: size as u64, jpeg });
                }
            }
        }
        let mut found = HashSet::new();
        if wanted.is_empty() {
            return Ok(found);
        }
        let hidden = |entry: &walkdir::DirEntry| entry.file_name().to_string_lossy().starts_with('.');
        let walker = WalkDir::new(self.originals_dir()).into_iter().filter_entry(|entry| entry.depth() == 0 || !hidden(entry));
        let relative = |path: &Path| path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().into_owned();
        for entry in walker.flatten().filter(|entry| entry.file_type().is_file()) {
            let Some(lost) = wanted.get_mut(entry.file_name()) else { continue };
            // The size first: it's free, and rules out most files that only share a name.
            let Ok(size) = entry.metadata().map(|meta| meta.len()) else { continue };
            if !lost.iter().any(|photo| photo.size == size) {
                continue;
            }
            let Ok((fingerprint, _)) = crate::media::fingerprint(entry.path()) else { continue };
            let Some(at) = lost.iter().position(|photo| photo.size == size && photo.fingerprint == fingerprint) else {
                continue;
            };
            let photo = lost.swap_remove(at);
            // A paired JPEG that is no longer where it was is looked for next to its RAW.
            let jpeg = match photo.jpeg {
                Some(jpeg) if !jpeg.exists() => {
                    let beside = jpeg.file_name().and_then(|name| Some(entry.path().parent()?.join(name)));
                    Some(beside.filter(|beside| beside.is_file()).unwrap_or(jpeg))
                }
                jpeg => jpeg,
            };
            // A file another photo already stands for is left to it.
            let updated = self.db().execute(
                "UPDATE photos SET path = ?2, jpeg_path = ?3 WHERE id = ?1
                 AND NOT EXISTS (SELECT 1 FROM photos WHERE path = ?2 AND id != ?1)",
                params![photo.id, relative(entry.path()), jpeg.as_deref().map(relative)],
            )?;
            if updated == 1 {
                found.insert(photo.id);
            }
        }
        Ok(found)
    }

    /// Every fingerprint in the library, mapped to its photo and whether that
    /// photo is in Recently Deleted.
    pub fn fingerprints(&self) -> Result<HashMap<String, (i64, bool)>> {
        let db = self.db();
        let mut stmt = db.prepare("SELECT fingerprint, id, deleted_at IS NOT NULL FROM photos")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ---- editing ----

    /// The photo's edit recipe as stored, or `None` if it is as shot.
    pub fn edits(&self, id: i64) -> Result<Option<String>> {
        self.db()
            .query_row("SELECT edits FROM photos WHERE id = ?1", [id], |r| r.get(0))
            .with_context(|| format!("photo {id} is not in the library"))
    }

    /// Marks the photo's generated images as redrawn and returns its new version.
    /// The recipe itself only ever changes through the history (see history.rs).
    pub fn bump_version(&self, id: i64) -> Result<i64> {
        self.db()
            .query_row("UPDATE photos SET version = version + 1 WHERE id = ?1 RETURNING version", [id], |r| r.get(0))
            .with_context(|| format!("photo {id} is not in the library"))
    }

    // ---- importing ----

    pub fn create_import(&self, source: &str) -> Result<i64> {
        let db = self.db();
        db.execute("INSERT INTO imports (created_at, source) VALUES (?1, ?2)", params![now(), source])?;
        Ok(db.last_insert_rowid())
    }

    /// Drops an import that ended up bringing nothing in.
    pub fn discard_import_if_empty(&self, import_id: i64) -> Result<()> {
        self.db().execute(
            "DELETE FROM imports WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM photos WHERE import_id = ?1)",
            [import_id],
        )?;
        Ok(())
    }

    pub fn insert_photo(&self, photo: &NewPhoto) -> Result<i64> {
        let db = self.db();
        db.execute(
            "INSERT INTO photos (path, jpeg_path, file_name, kind, fingerprint, file_size, taken_at,
                                 width, height, make, model, lens, iso, aperture, shutter,
                                 focal_length, import_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                photo.path,
                photo.jpeg_path,
                photo.file_name,
                photo.kind,
                photo.fingerprint,
                photo.file_size as i64,
                photo.taken_at,
                photo.width,
                photo.height,
                photo.make,
                photo.model,
                photo.lens,
                photo.iso,
                photo.aperture,
                photo.shutter,
                photo.focal_length,
                photo.import_id,
            ],
        )?;
        Ok(db.last_insert_rowid())
    }

    // ---- marking ----

    fn for_each_id(&self, sql: &str, ids: &[i64], value: Option<i64>) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        {
            let mut stmt = tx.prepare(sql)?;
            for id in ids {
                match value {
                    Some(value) => stmt.execute(params![id, value])?,
                    None => stmt.execute([id])?,
                };
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn set_favorite(&self, ids: &[i64], favorite: bool) -> Result<()> {
        self.for_each_id("UPDATE photos SET favorite = ?2 WHERE id = ?1", ids, Some(favorite as i64))
    }

    pub fn set_flag(&self, ids: &[i64], flag: i8) -> Result<()> {
        self.for_each_id("UPDATE photos SET flag = ?2 WHERE id = ?1", ids, Some(flag.signum() as i64))
    }

    // ---- film details ----

    /// These photos' film details. Photos not in the library are left out.
    pub fn film_details(&self, ids: &[i64]) -> Result<Vec<FilmPhoto>> {
        let db = self.db();
        let mut stmt = db.prepare(&format!("SELECT p.taken_at, p.file_name, {FILM_COLUMNS} FROM photos p WHERE p.id = ?1"))?;
        let mut photos = Vec::with_capacity(ids.len());
        for &id in ids {
            let photo = stmt
                .query_row([id], |r| {
                    Ok(FilmPhoto { id, taken_at: r.get(0)?, file_name: r.get(1)?, film: FilmDetails::from_row(r, 2)? })
                })
                .optional()?;
            photos.extend(photo);
        }
        Ok(photos)
    }

    /// Gives each photo the film details that come with it, in place of
    /// what it had: a detail left empty is cleared.
    pub fn set_film_details(&self, photos: &[FilmPhoto]) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        {
            let mut stmt = tx.prepare(
                "UPDATE photos SET film_stock = ?2, film_iso = ?3, film_camera = ?4, film_lens = ?5, film_frame = ?6
                 WHERE id = ?1",
            )?;
            for photo in photos {
                let film = photo.film.clone().tidied();
                stmt.execute(params![photo.id, film.stock, film.iso, film.camera, film.lens, film.frame])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The stocks, cameras and lenses already given to photos in the
    /// library, the most used first.
    pub fn film_suggestions(&self) -> Result<FilmSuggestions> {
        let db = self.db();
        let used = |column: &str| -> Result<Vec<String>> {
            let mut stmt = db.prepare(&format!(
                "SELECT {column} FROM photos WHERE {column} IS NOT NULL AND deleted_at IS NULL
                 GROUP BY {column} ORDER BY COUNT(*) DESC, {column} COLLATE NOCASE"
            ))?;
            let values = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            Ok(values)
        };
        Ok(FilmSuggestions { stocks: used("film_stock")?, cameras: used("film_camera")?, lenses: used("film_lens")? })
    }

    // ---- deleting ----

    /// Moves photos to Recently Deleted. Their files stay where they are.
    pub fn trash(&self, ids: &[i64]) -> Result<()> {
        self.for_each_id(
            "UPDATE photos SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            ids,
            Some(now()),
        )
    }

    pub fn restore(&self, ids: &[i64]) -> Result<()> {
        self.for_each_id("UPDATE photos SET deleted_at = NULL WHERE id = ?1", ids, None)
    }

    /// Removes photos for good: originals, generated images and database rows.
    /// Only photos already in Recently Deleted are touched.
    ///
    /// The files go first and the row only once they are gone, so a file that
    /// can't be deleted is never left behind untracked: its photo stays in
    /// Recently Deleted and the error names it.
    pub fn purge(&self, ids: &[i64]) -> Result<u32> {
        let mut purged = 0;
        let mut failed = Vec::new();
        for &id in ids {
            let files: Option<(String, String, Option<String>)> = self
                .db()
                .query_row(
                    "SELECT file_name, path, jpeg_path FROM photos WHERE id = ?1 AND deleted_at IS NOT NULL",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((file_name, path, jpeg_path)) = files else { continue };
            let removed = std::iter::once(&path).chain(&jpeg_path).try_for_each(|relative| {
                let file = self.root.join(relative);
                match fs::remove_file(&file) {
                    // Already gone, perhaps by an earlier purge that was interrupted.
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
                    _ => {}
                }
                // Tidy up date folders that are now empty; `remove_dir` refuses otherwise.
                if let Some(day) = file.parent() {
                    if fs::remove_dir(day).is_ok() {
                        if let Some(year) = day.parent() {
                            let _ = fs::remove_dir(year);
                        }
                    }
                }
                Ok(())
            });
            if let Err(error) = removed {
                failed.push(format!("{file_name} ({error})"));
                continue;
            }
            self.db().execute("DELETE FROM photos WHERE id = ?1", [id])?;
            purged += 1;
            let _ = fs::remove_file(self.thumb_path(id));
            let _ = fs::remove_file(self.preview_path(id));
            let start = self.matte_files(id);
            if let (Some(dir), Some(prefix)) = (start.parent(), start.file_name().and_then(|n| n.to_str())) {
                for file in fs::read_dir(dir).into_iter().flatten().flatten() {
                    if file.file_name().to_str().is_some_and(|name| name.starts_with(prefix)) {
                        let _ = fs::remove_file(file.path());
                    }
                }
            }
        }
        if !failed.is_empty() {
            bail!(
                "Couldn’t delete {}, so {} in Recently Deleted: {}",
                if failed.len() == 1 { "1 photo".to_string() } else { format!("{} photos", failed.len()) },
                if failed.len() == 1 { "it stays" } else { "they stay" },
                failed.join(", ")
            );
        }
        Ok(purged)
    }

    /// Purges photos that have sat in Recently Deleted past the retention period.
    pub fn purge_expired(&self) -> Result<u32> {
        let cutoff = now() - TRASH_RETENTION_DAYS * 24 * 60 * 60;
        let expired: Vec<i64> = {
            let db = self.db();
            let mut stmt = db.prepare("SELECT id FROM photos WHERE deleted_at IS NOT NULL AND deleted_at < ?1")?;
            let ids = stmt.query_map([cutoff], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            ids
        };
        self.purge(&expired)
    }

    /// Purges expired photos whenever the library is looked at, so a long
    /// session doesn't leave them at "0 days left". A file that can't be
    /// deleted stays in Recently Deleted for next time; it mustn't stop the
    /// library from showing.
    fn purge_expired_quietly(&self) {
        if let Err(error) = self.purge_expired() {
            eprintln!("{error:#}");
        }
    }

    // ---- albums ----

    pub fn create_album(&self, name: &str) -> Result<i64> {
        let db = self.db();
        db.execute("INSERT INTO albums (name, created_at) VALUES (?1, ?2)", params![name.trim(), now()])?;
        Ok(db.last_insert_rowid())
    }

    pub fn rename_album(&self, id: i64, name: &str) -> Result<()> {
        self.db().execute("UPDATE albums SET name = ?2 WHERE id = ?1", params![id, name.trim()])?;
        Ok(())
    }

    /// Deletes the album only; its photos stay in the library.
    pub fn delete_album(&self, id: i64) -> Result<()> {
        self.db().execute("DELETE FROM albums WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn add_to_album(&self, album_id: i64, ids: &[i64]) -> Result<()> {
        self.for_each_id(
            "INSERT OR IGNORE INTO album_photos (photo_id, album_id) VALUES (?1, ?2)",
            ids,
            Some(album_id),
        )
    }

    pub fn remove_from_album(&self, album_id: i64, ids: &[i64]) -> Result<()> {
        self.for_each_id("DELETE FROM album_photos WHERE photo_id = ?1 AND album_id = ?2", ids, Some(album_id))
    }
}
