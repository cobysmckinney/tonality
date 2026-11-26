//! The SQLite catalog: images, edit states and the per-image commit graph.
//!
//! One file, WAL mode. Schema changes go through `MIGRATIONS` keyed on `PRAGMA user_version`,
//! so catalogs created by older builds upgrade in place.

use std::cell::RefCell;
use std::collections::HashMap;
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
    /// Whether each stored edit state (by hash) differs from as-shot. A stored state never
    /// changes, so an answer holds for good. See [`Catalog::images`].
    pub(crate) edited_states: RefCell<HashMap<String, bool>>,
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
    // v3: the shown version's shape (width / height), learnt from its thumbnail, so the
    // library grid can lay photos out before their thumbnails load.
    r#"
    ALTER TABLE images ADD COLUMN aspect REAL;
    "#,
    // v4: a photo's HEAD must name one of its branches. First repair any that doesn't (older
    // versions could leave one): point it at a branch with the same tip, else bring the branch
    // back. The check is deferred to commit, so a transaction can move HEAD off a branch it then
    // deletes; renaming a branch carries HEAD along.
    r#"
    UPDATE heads SET branch = (
        SELECT name FROM refs r WHERE r.image_id = heads.image_id AND r.commit_id = heads.commit_id ORDER BY r.id LIMIT 1
    )
    WHERE NOT EXISTS (SELECT 1 FROM refs r WHERE r.image_id = heads.image_id AND r.name = heads.branch)
      AND EXISTS (SELECT 1 FROM refs r WHERE r.image_id = heads.image_id AND r.commit_id = heads.commit_id);
    INSERT INTO refs (image_id, name, commit_id)
        SELECT image_id, branch, commit_id FROM heads h
        WHERE NOT EXISTS (SELECT 1 FROM refs r WHERE r.image_id = h.image_id AND r.name = h.branch);
    CREATE TABLE heads_v4 (
        image_id    INTEGER PRIMARY KEY REFERENCES images(id) ON DELETE CASCADE,
        branch      TEXT NOT NULL,
        commit_id   INTEGER NOT NULL REFERENCES commits(id),
        FOREIGN KEY (image_id, branch) REFERENCES refs(image_id, name)
            ON UPDATE CASCADE DEFERRABLE INITIALLY DEFERRED
    );
    INSERT INTO heads_v4 (image_id, branch, commit_id) SELECT image_id, branch, commit_id FROM heads;
    DROP TABLE heads;
    ALTER TABLE heads_v4 RENAME TO heads;
    "#,
    // v5: tombstones. Commits and branches deleted here (a slider gesture that ended where it
    // started, a deleted branch), keyed the way a sidecar names them, so merging a sidecar
    // written before the delete (the app died before rewriting it) doesn't bring them back.
    r#"
    CREATE TABLE dropped_commits (
        image_id    INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
        created_at  INTEGER NOT NULL,
        PRIMARY KEY (image_id, created_at)
    ) WITHOUT ROWID;
    CREATE TABLE deleted_branches (
        image_id        INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
        name            TEXT NOT NULL,
        tip_created_at  INTEGER NOT NULL,   -- the branch's tip when it was deleted
        PRIMARY KEY (image_id, name, tip_created_at)
    ) WITHOUT ROWID;
    "#,
    // v6: an index for each foreign key that had none. Deleting a row makes SQLite look for
    // rows that refer to it, and without one that is a scan of the whole table for every row
    // deleted: resetting 50 photos' history in a library of 60,000 steps took 12 seconds.
    r#"
    CREATE INDEX commits_state ON commits(state_hash);
    CREATE INDEX refs_commit ON refs(commit_id);
    CREATE INDEX heads_commit ON heads(commit_id);
    "#,
    // v7: how many changes a photo has had that its sidecar doesn't hold yet (0: the file is
    // up to date). A sidecar that is behind must not be read back over the catalog at the next
    // launch: it would bring back a cleared rating or flag, an album the photo left, or a
    // history that was reset. See `Catalog::sidecar_touch`.
    r#"
    ALTER TABLE images ADD COLUMN sidecar_pending INTEGER NOT NULL DEFAULT 0;
    "#,
    // v8: album names are unique, compared the way a sidecar's album names are matched
    // (ignoring ASCII case). `prepare` renames the duplicates older builds allowed first.
    r#"
    CREATE UNIQUE INDEX albums_name ON albums(name COLLATE NOCASE);
    "#,
];

/// What migration `to` needs done before its SQL that SQL alone can't do.
fn prepare(to: usize, conn: &Connection) -> rusqlite::Result<()> {
    match to {
        8 => rename_duplicate_albums(conn),
        _ => Ok(()),
    }
}

/// Run migration `to` (1-based: the version the catalog is at afterwards).
fn migrate(to: usize, conn: &Connection) -> rusqlite::Result<()> {
    prepare(to, conn)?;
    conn.execute_batch(MIGRATIONS[to - 1])?;
    conn.pragma_update(None, "user_version", to as i64)
}

/// Give every album whose name another, older album already has (ignoring case) a name of its
/// own: "Trips" becomes "Trips 2", or the next number that's free. Its photos' sidecars still
/// say "Trips", which is the older album's name, so they are marked as behind the catalog and
/// get rewritten instead of read.
fn rename_duplicate_albums(conn: &Connection) -> rusqlite::Result<()> {
    let duplicates: Vec<(i64, String)> = conn
        .prepare(
            "SELECT id, name FROM albums a WHERE EXISTS
                 (SELECT 1 FROM albums b WHERE b.id < a.id AND b.name = a.name COLLATE NOCASE)
             ORDER BY id",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, name) in duplicates {
        let free = (2..)
            .map(|n| format!("{name} {n}"))
            .find_map(|candidate| {
                match conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM albums WHERE name = ?1 COLLATE NOCASE)",
                    [&candidate],
                    |r| r.get::<_, bool>(0),
                ) {
                    Ok(true) => None,
                    Ok(false) => Some(Ok(candidate)),
                    Err(e) => Some(Err(e)),
                }
            })
            .expect("unbounded")?;
        conn.execute(
            "UPDATE albums SET name = ?1 WHERE id = ?2",
            rusqlite::params![free, id],
        )?;
        conn.execute(
            "UPDATE images SET sidecar_pending = sidecar_pending + 1
             WHERE id IN (SELECT image_id FROM album_images WHERE album_id = ?1)",
            [id],
        )?;
    }
    Ok(())
}

pub fn system_clock() -> Clock {
    Box::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
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
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let version = match usize::try_from(version) {
            Ok(v) if v <= MIGRATIONS.len() => v,
            _ => {
                return Err(CatalogError::Invalid(format!(
                    "catalog schema v{version} is newer than this build of tonality supports"
                )));
            }
        };
        for to in version + 1..=MIGRATIONS.len() {
            let tx = conn.unchecked_transaction()?;
            migrate(to, &tx)?;
            tx.commit()?;
        }
        Ok(Catalog {
            conn,
            clock: system_clock(),
            edited_states: RefCell::default(),
        })
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
    fn v4_repairs_a_dangling_head_then_enforces_it() {
        // A catalog at v3 whose HEAD names a branch that doesn't exist (the old sidecar bug).
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for (i, sql) in MIGRATIONS[..3].iter().enumerate() {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", i as i64 + 1)
                .unwrap();
        }
        conn.execute_batch(
            "INSERT INTO images (id, hash, file, name, file_size, imported_at) VALUES (1, 'h', 'f', 'n', 1, 0), (2, 'h2', 'f2', 'n2', 1, 0);
             INSERT INTO edit_states (hash, json) VALUES ('s', '{}');
             INSERT INTO commits (id, image_id, parent, state_hash, summary, created_at, updated_at) VALUES
                 (1, 1, NULL, 's', 'Original', 0, 0), (2, 2, NULL, 's', 'Original', 0, 0);
             INSERT INTO refs (image_id, name, commit_id) VALUES (1, 'main', 1);
             INSERT INTO heads (image_id, branch, commit_id) VALUES (1, 'main-2', 1), (2, 'gone', 2);",
        )
        .unwrap();
        let cat = Catalog::init(conn).unwrap();
        let head = |img: i64| -> String {
            cat.conn
                .query_row("SELECT branch FROM heads WHERE image_id = ?1", [img], |r| {
                    r.get(0)
                })
                .unwrap()
        };
        assert_eq!(head(1), "main", "pointed at the branch with its tip");
        assert_eq!(head(2), "gone", "branch brought back");
        let refs: i64 = cat
            .conn
            .query_row(
                "SELECT COUNT(*) FROM refs WHERE image_id = 2 AND name = 'gone'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(refs, 1);
        // Enforced from now on, and renames carry HEAD along.
        assert!(cat
            .conn
            .execute("DELETE FROM refs WHERE image_id = 1 AND name = 'main'", [])
            .is_err());
        cat.conn
            .execute("UPDATE refs SET name = 'renamed' WHERE image_id = 1", [])
            .unwrap();
        assert_eq!(head(1), "renamed");
    }

    /// Deleting a row makes SQLite look for rows that refer to it. Without an index on the
    /// referring column that's a scan of the whole table for every row deleted, under the
    /// lock every other command waits on.
    #[test]
    fn every_foreign_key_has_an_index_to_be_checked_by() {
        let cat = Catalog::open_in_memory().unwrap();
        let strings = |sql: &str| -> Vec<String> {
            let mut stmt = cat.conn.prepare(sql).unwrap();
            let rows = stmt.query_map([], |r| r.get(0)).unwrap();
            rows.collect::<rusqlite::Result<_>>().unwrap()
        };
        let mut unindexed = Vec::new();
        for table in strings("SELECT name FROM sqlite_master WHERE type = 'table'") {
            // The first column of each index on the table (the primary key's included).
            let mut leading = strings(&format!(
                "SELECT (SELECT name FROM pragma_index_info(l.name) WHERE seqno = 0)
                 FROM pragma_index_list('{table}') l"
            ));
            // An INTEGER PRIMARY KEY is the rowid: no index is listed for it.
            leading.extend(strings(&format!(
                "SELECT name FROM pragma_table_info('{table}') WHERE pk = 1"
            )));
            for column in strings(&format!(
                "SELECT \"from\" FROM pragma_foreign_key_list('{table}') WHERE seq = 0"
            )) {
                if !leading.contains(&column) {
                    unindexed.push(format!("{table}({column})"));
                }
            }
        }
        assert!(unindexed.is_empty(), "no index on {unindexed:?}");
    }

    #[test]
    fn duplicate_album_names_are_told_apart_when_upgrading() {
        // A catalog from before names had to be unique, with three "Trips" (one in another
        // case) and a "Trips 2" already taken.
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for to in 1..=5 {
            migrate(to, &conn).unwrap();
        }
        conn.execute_batch(
            "INSERT INTO images (id, hash, file, name, file_size, imported_at) VALUES (1, 'h', 'f', 'n', 1, 0), (2, 'h2', 'f2', 'n2', 1, 0);
             INSERT INTO albums (id, name, created_at) VALUES
                 (1, 'Trips', 0), (2, 'Trips 2', 0), (3, 'trips', 0), (4, 'Family', 0), (5, 'Trips', 0);
             INSERT INTO album_images (album_id, image_id, added_at) VALUES (1, 1, 0), (3, 2, 0);",
        )
        .unwrap();
        let mut cat = Catalog::init(conn).unwrap();
        let names: Vec<String> = cat.albums().unwrap().into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["Family", "Trips", "Trips 2", "trips 3", "Trips 4"]);
        // Nobody lost a photo, and the renamed album's photo will have its sidecar rewritten
        // from the catalog rather than read back (it still says "trips").
        assert_eq!(cat.album_members(1).unwrap(), [1]);
        assert_eq!(cat.album_members(3).unwrap(), [2]);
        assert_eq!(cat.sidecar_pending(1).unwrap(), 0);
        assert!(cat.sidecar_pending(2).unwrap() > 0);
        assert!(cat.create_album("TRIPS 3").is_err());
    }

    #[test]
    fn migrates_and_reopens() {
        let dir = std::env::temp_dir().join(format!("tonality-test-{}", std::process::id()));
        let path = dir.join("catalog.sqlite");
        let _ = std::fs::remove_file(&path);
        let id = Catalog::open(&path).unwrap().test_image("a");
        let cat = Catalog::open(&path).unwrap();
        let n: i64 = cat
            .conn
            .query_row("SELECT COUNT(*) FROM images WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
