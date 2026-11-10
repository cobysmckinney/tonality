//! The SQLite catalog: images, edit states and the per-image commit graph.
//!
//! One file, WAL mode. Schema changes go through `MIGRATIONS` keyed on `PRAGMA user_version`,
//! so catalogs created by older builds upgrade in place.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

pub type ImageId = i64;

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("corrupt edit state: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0} not found")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, CatalogError>;

/// Milliseconds since the Unix epoch. Injectable so tests can control commit coalescing.
pub type Clock = Box<dyn Fn() -> i64 + Send>;

pub struct Catalog {
    pub(crate) conn: Connection,
    pub(crate) clock: Clock,
}

const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    -- One row per import session (a card, a folder, a few dragged-in files).
    CREATE TABLE imports (
        id          INTEGER PRIMARY KEY,
        name        TEXT NOT NULL,        -- e.g. the source folder's name
        source      TEXT NOT NULL,        -- where it came from (informational only)
        imported_at INTEGER NOT NULL
    );
    -- Photos owned by the library. `file` is relative to the library root; the original
    -- location is kept only as a note and never read again.
    CREATE TABLE images (
        id          INTEGER PRIMARY KEY,
        hash        TEXT NOT NULL UNIQUE, -- blake3 of the original file (deduplication)
        file        TEXT NOT NULL UNIQUE,
        name        TEXT NOT NULL,        -- original file name
        file_size   INTEGER NOT NULL,
        import_id   INTEGER REFERENCES imports(id) ON DELETE SET NULL,
        source_path TEXT,
        imported_at INTEGER NOT NULL,
        captured_at TEXT,                 -- ISO 8601 local time, for sorting
        meta        TEXT,                 -- JSON ImageMeta
        rating      INTEGER NOT NULL DEFAULT 0,
        flag        INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX images_import ON images(import_id);
    CREATE INDEX images_captured ON images(captured_at);
    CREATE TABLE edit_states (
        hash        TEXT PRIMARY KEY,
        json        TEXT NOT NULL
    );
    CREATE TABLE commits (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        image_id    INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
        parent      INTEGER REFERENCES commits(id),
        state_hash  TEXT NOT NULL REFERENCES edit_states(hash),
        summary     TEXT NOT NULL,
        -- Which control produced the commit; consecutive commits with the same key
        -- inside a short window are merged (e.g. arrow-key nudges).
        key         TEXT,
        created_at  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL
    );
    CREATE INDEX commits_image ON commits(image_id);
    CREATE INDEX commits_parent ON commits(parent);
    CREATE TABLE refs (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,  -- creation order
        image_id    INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
        name        TEXT NOT NULL,
        commit_id   INTEGER NOT NULL REFERENCES commits(id),
        UNIQUE (image_id, name)
    );
    CREATE TABLE heads (
        image_id    INTEGER PRIMARY KEY REFERENCES images(id) ON DELETE CASCADE,
        branch      TEXT NOT NULL,
        commit_id   INTEGER NOT NULL REFERENCES commits(id)
    );
    "#,
    // v2: albums
    r#"
    CREATE TABLE albums (
        id          INTEGER PRIMARY KEY,
        name        TEXT NOT NULL,
        created_at  INTEGER NOT NULL
    );
    CREATE TABLE album_images (
        album_id    INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
        image_id    INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
        added_at    INTEGER NOT NULL,
        PRIMARY KEY (album_id, image_id)
    );
    CREATE INDEX album_images_image ON album_images(image_id);
    "#,
];

pub fn system_clock() -> Clock {
    Box::new(|| {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
    })
}

impl Catalog {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(dir) = path.as_ref().parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| CatalogError::Invalid(format!("cannot create {dir:?}: {e}")))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: usize = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > MIGRATIONS.len() {
            return Err(CatalogError::Invalid(format!(
                "catalog schema v{version} is newer than this build of tonality supports"
            )));
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i + 1)?;
            tx.commit()?;
        }
        Ok(Catalog { conn, clock: system_clock() })
    }

    pub(crate) fn now(&self) -> i64 {
        (self.clock)()
    }

    /// Test helper: register a bare image.
    #[cfg(test)]
    pub(crate) fn test_image(&self, name: &str) -> ImageId {
        self.conn
            .execute(
                "INSERT INTO images (hash, file, name, file_size, imported_at) VALUES (?1, ?1, ?1, 0, 0)",
                [name],
            )
            .unwrap();
        self.conn.last_insert_rowid()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_and_reopens() {
        let dir = std::env::temp_dir().join(format!("tonality-test-{}", std::process::id()));
        let path = dir.join("catalog.sqlite");
        let _ = std::fs::remove_file(&path);
        let id = Catalog::open(&path).unwrap().test_image("a");
        let cat = Catalog::open(&path).unwrap();
        let n: i64 = cat.conn.query_row("SELECT COUNT(*) FROM images WHERE id = ?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
