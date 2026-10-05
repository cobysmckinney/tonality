//! The managed library: one folder on disk holding the originals, the
//! database that indexes them, and the generated thumbnails and previews.
//!
//! ```text
//! Tonality/
//!   Originals/2026/2026-10-04/IMG_0001.CR3
//!   Exports/IMG_0001.jpg                      (unless another folder is chosen)
//!   .tonality/library.db
//!   .tonality/thumbs/…   .tonality/previews/…
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

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
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Library {
    /// Opens the library at `root`, creating it if it does not exist yet.
    pub fn open(root: &Path) -> Result<Self> {
        let data = root.join(".tonality");
        for dir in [root.join("Originals"), data.join("thumbs"), data.join("previews")] {
            fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let db = Connection::open(data.join("library.db")).context("opening library database")?;
        db.pragma_update(None, "journal_mode", "WAL")?;
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
        let library = Self { root: root.to_path_buf(), db: Mutex::new(db) };
        // Leftovers from an import or review that was interrupted.
        let _ = fs::remove_dir_all(library.incoming_dir());
        let _ = fs::remove_dir_all(library.scan_cache_dir());
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
        const COLUMNS: &str = "p.id, p.file_name, p.kind, p.jpeg_path IS NOT NULL, p.taken_at, \
             p.width, p.height, p.favorite, p.flag, p.import_id, p.deleted_at, p.edits IS NOT NULL, p.version, \
             (SELECT COUNT(*) FROM edit_branches b WHERE b.photo_id = p.id)";
        let (rest, album) = match view {
            View::Library => ("WHERE p.deleted_at IS NULL ORDER BY p.taken_at DESC, p.id DESC", None),
            View::Favorites => (
                "WHERE p.deleted_at IS NULL AND p.favorite = 1 ORDER BY p.taken_at DESC, p.id DESC",
                None,
            ),
            View::Imports => (
                "WHERE p.deleted_at IS NULL ORDER BY p.import_id DESC, p.taken_at DESC, p.id DESC",
                None,
            ),
            View::Deleted => ("WHERE p.deleted_at IS NOT NULL ORDER BY p.deleted_at DESC, p.id DESC", None),
            View::Album { id } => (
                "JOIN album_photos ap ON ap.photo_id = p.id AND ap.album_id = ?1 \
                 WHERE p.deleted_at IS NULL ORDER BY p.taken_at DESC, p.id DESC",
                Some(id),
            ),
        };
        let db = self.db();
        let mut stmt = db.prepare(&format!("SELECT {COLUMNS} FROM photos p {rest}"))?;
        let map = |r: &Row| {
            Ok(PhotoItem {
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
            })
        };
        let rows = match album {
            Some(id) => stmt.query_map([id], map)?.collect::<rusqlite::Result<_>>()?,
            None => stmt.query_map([], map)?.collect::<rusqlite::Result<_>>()?,
        };
        Ok(rows)
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
            "SELECT p.path, p.jpeg_path, p.file_size, p.make, p.model, p.lens, p.iso, p.aperture,
                    p.shutter, p.focal_length, i.created_at
             FROM photos p JOIN imports i ON i.id = p.import_id WHERE p.id = ?1",
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
                })
            },
        )
        .with_context(|| format!("photo {id} is not in the library"))
    }

    pub fn photo_files(&self, id: i64) -> Result<PhotoFiles> {
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

    /// Removes photos for good: database rows, originals and generated images.
    /// Only photos already in Recently Deleted are touched.
    pub fn purge(&self, ids: &[i64]) -> Result<u32> {
        let mut doomed = Vec::new();
        {
            let mut db = self.db();
            let tx = db.transaction()?;
            for &id in ids {
                let files: Option<(String, Option<String>)> = tx
                    .query_row(
                        "DELETE FROM photos WHERE id = ?1 AND deleted_at IS NOT NULL RETURNING path, jpeg_path",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                if let Some(files) = files {
                    doomed.push((id, files));
                }
            }
            tx.commit()?;
        }
        for (id, (path, jpeg_path)) in &doomed {
            for relative in std::iter::once(path).chain(jpeg_path) {
                let file = self.root.join(relative);
                let _ = fs::remove_file(&file);
                // Tidy up date folders that are now empty; `remove_dir` refuses otherwise.
                if let Some(day) = file.parent() {
                    if fs::remove_dir(day).is_ok() {
                        if let Some(year) = day.parent() {
                            let _ = fs::remove_dir(year);
                        }
                    }
                }
            }
            let _ = fs::remove_file(self.thumb_path(*id));
            let _ = fs::remove_file(self.preview_path(*id));
        }
        Ok(doomed.len() as u32)
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
