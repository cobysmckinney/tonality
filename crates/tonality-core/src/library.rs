//! Library: the photos tonality owns, how they arrived (imports), ratings and flags.
//!
//! The library is *managed*: importing copies each original into the library folder, and the
//! catalog only ever refers to that copy (`images.file`, relative to the library root). Where a
//! photo came from is kept as a note but never read again.

use rusqlite::{params, params_from_iter, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::catalog::{Catalog, CatalogError, ImageId, Result};

pub type ImportId = i64;

/// Capture metadata read from the RAW file at import.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ImageMeta {
    pub make: String,
    pub model: String,
    pub lens: Option<String>,
    pub iso: Option<u32>,
    /// Exposure time in seconds.
    pub exposure_time: Option<f32>,
    pub f_number: Option<f32>,
    /// Millimetres.
    pub focal_length: Option<f32>,
    /// Local capture time, ISO 8601 without zone (`2026-11-02T14:03:11`).
    pub captured_at: Option<String>,
    /// In-camera rating, if the camera wrote one.
    pub rating: Option<u8>,
}

/// A photo that has been copied into the library and is ready to be catalogued.
#[derive(Debug, Clone)]
pub struct NewImage {
    pub hash: String,
    /// Path of the copy, relative to the library root.
    pub file: String,
    pub name: String,
    pub file_size: i64,
    pub source_path: String,
    pub meta: Option<ImageMeta>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportInfo {
    #[ts(type = "number")]
    pub id: ImportId,
    pub name: String,
    pub source: String,
    #[ts(type = "number")]
    pub imported_at: i64,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImageRow {
    #[ts(type = "number")]
    pub id: ImageId,
    pub name: String,
    pub rating: u8,
    /// 1 = pick, -1 = reject, 0 = none.
    pub flag: i8,
    pub meta: Option<ImageMeta>,
    /// Number of branches (versions) in this image's history; 0 if never edited.
    pub branches: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SortOrder {
    #[default]
    CaptureTime,
    ImportTime,
    Name,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ImageQuery {
    /// `None` = every photo in the library.
    #[ts(type = "number | null")]
    pub import: Option<ImportId>,
    pub min_rating: u8,
    /// `Some(1)` picks only, `Some(-1)` rejects only, `Some(0)` unflagged only.
    pub flag: Option<i8>,
    pub sort: SortOrder,
    pub descending: bool,
}

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportSummary {
    /// `None` when nothing new was added (e.g. everything was already in the library).
    pub import: Option<ImportInfo>,
    pub added: u32,
    /// Already in the library (same file contents).
    pub duplicates: u32,
    pub failed: u32,
    /// Ids of every photo in this import, new or duplicate, in import order.
    #[ts(type = "number[]")]
    pub images: Vec<ImageId>,
    /// First error message, if any file failed.
    pub error: Option<String>,
}

impl Catalog {
    pub fn begin_import(&mut self, name: &str, source: &str) -> Result<ImportId> {
        self.conn.execute(
            "INSERT INTO imports (name, source, imported_at) VALUES (?1, ?2, ?3)",
            params![name, source, (self.clock)()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// The image with these exact file contents, if the library already has it.
    pub fn image_by_hash(&self, hash: &str) -> Result<Option<ImageId>> {
        Ok(self.conn.query_row("SELECT id FROM images WHERE hash = ?1", [hash], |r| r.get(0)).optional()?)
    }

    /// Catalogue a photo copied into the library. Returns `None` if one with the same contents
    /// already exists (the caller should then discard its copy).
    pub fn add_image(&mut self, import: ImportId, img: &NewImage) -> Result<Option<ImageId>> {
        let meta_json = img.meta.as_ref().map(serde_json::to_string).transpose()?;
        let captured = img.meta.as_ref().and_then(|m| m.captured_at.clone());
        let rating = img.meta.as_ref().and_then(|m| m.rating).unwrap_or(0).min(5);
        let n = self.conn.execute(
            "INSERT INTO images (hash, file, name, file_size, import_id, source_path, imported_at,
                                 captured_at, meta, rating)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(hash) DO NOTHING",
            params![
                img.hash,
                img.file,
                img.name,
                img.file_size,
                import,
                img.source_path,
                (self.clock)(),
                captured,
                meta_json,
                rating
            ],
        )?;
        Ok((n == 1).then(|| self.conn.last_insert_rowid()))
    }

    /// Close an import. Imports that added nothing are removed so they don't clutter the list.
    pub fn finish_import(&mut self, import: ImportId) -> Result<Option<ImportInfo>> {
        let info = self.import_info(import)?;
        if info.count == 0 {
            self.conn.execute("DELETE FROM imports WHERE id = ?1", [import])?;
            return Ok(None);
        }
        Ok(Some(info))
    }

    fn import_info(&self, id: ImportId) -> Result<ImportInfo> {
        self.imports()?
            .into_iter()
            .find(|i| i.id == id)
            .ok_or_else(|| CatalogError::NotFound(format!("import {id}")))
    }

    /// All imports, newest first.
    pub fn imports(&self) -> Result<Vec<ImportInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.name, m.source, m.imported_at,
                    (SELECT COUNT(*) FROM images i WHERE i.import_id = m.id)
             FROM imports m ORDER BY m.imported_at DESC, m.id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ImportInfo { id: r.get(0)?, name: r.get(1)?, source: r.get(2)?, imported_at: r.get(3)?, count: r.get(4)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn image_count(&self) -> Result<u32> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM images", [], |r| r.get(0))?)
    }

    pub fn images(&self, q: &ImageQuery) -> Result<Vec<ImageRow>> {
        let mut sql = String::from(
            "SELECT i.id, i.name, i.rating, i.flag, i.meta,
                    (SELECT COUNT(*) FROM refs r WHERE r.image_id = i.id)
             FROM images i WHERE i.rating >= ?1",
        );
        let mut args: Vec<rusqlite::types::Value> = vec![(q.min_rating as i64).into()];
        if let Some(m) = q.import {
            args.push(m.into());
            sql += &format!(" AND i.import_id = ?{}", args.len());
        }
        if let Some(flag) = q.flag {
            args.push((flag as i64).into());
            sql += &format!(" AND i.flag = ?{}", args.len());
        }
        let dir = if q.descending { "DESC" } else { "ASC" };
        sql += &match q.sort {
            // Photos without a capture time sort after dated ones.
            SortOrder::CaptureTime => {
                format!(" ORDER BY i.captured_at IS NULL, i.captured_at {dir}, i.name COLLATE NOCASE {dir}, i.id")
            }
            SortOrder::ImportTime => format!(" ORDER BY i.imported_at {dir}, i.id {dir}"),
            SortOrder::Name => format!(" ORDER BY i.name COLLATE NOCASE {dir}, i.id"),
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args), |r| {
            let meta: Option<String> = r.get(4)?;
            Ok(ImageRow {
                id: r.get(0)?,
                name: r.get(1)?,
                rating: r.get(2)?,
                flag: r.get(3)?,
                meta: meta.and_then(|m| serde_json::from_str(&m).ok()),
                branches: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The library-relative file, content hash and original name of an image.
    pub fn image_file(&self, id: ImageId) -> Result<(String, String, String)> {
        self.conn
            .query_row("SELECT file, hash, name FROM images WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?
            .ok_or_else(|| CatalogError::NotFound(format!("image {id}")))
    }

    /// Whether a library-relative path is already taken (used when choosing a name for a copy).
    pub fn file_taken(&self, file: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM images WHERE file = ?1)", [file], |r| r.get(0))?)
    }

    pub fn set_rating(&mut self, ids: &[ImageId], rating: u8) -> Result<()> {
        self.update_images("rating", ids, rating.min(5) as i64)
    }

    pub fn set_flag(&mut self, ids: &[ImageId], flag: i8) -> Result<()> {
        self.update_images("flag", ids, flag.clamp(-1, 1) as i64)
    }

    fn update_images(&mut self, column: &str, ids: &[ImageId], value: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(&format!("UPDATE images SET {column} = ?1 WHERE id = ?2"))?;
            for id in ids {
                stmt.execute(params![value, id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(name: &str, hash: &str, at: Option<&str>) -> NewImage {
        NewImage {
            hash: hash.into(),
            file: format!("originals/{name}"),
            name: name.into(),
            file_size: 100,
            source_path: format!("/card/{name}"),
            meta: at.map(|at| ImageMeta { make: "Sony".into(), captured_at: Some(at.into()), ..Default::default() }),
        }
    }

    #[test]
    fn import_dedupes_and_sorts() {
        let mut cat = Catalog::open_in_memory().unwrap();
        let m = cat.begin_import("card", "/card").unwrap();
        assert!(cat.add_image(m, &img("b.ARW", "h1", Some("2026-11-02T10:00:00"))).unwrap().is_some());
        assert!(cat.add_image(m, &img("a.ARW", "h2", Some("2026-11-02T11:00:00"))).unwrap().is_some());
        assert!(cat.add_image(m, &img("c.ARW", "h3", None)).unwrap().is_some());
        let info = cat.finish_import(m).unwrap().unwrap();
        assert_eq!(info.count, 3);

        let names = |q: &ImageQuery| cat.images(q).unwrap().into_iter().map(|i| i.name).collect::<Vec<_>>();
        assert_eq!(names(&ImageQuery::default()), ["b.ARW", "a.ARW", "c.ARW"]);
        assert_eq!(names(&ImageQuery { sort: SortOrder::Name, ..Default::default() }), ["a.ARW", "b.ARW", "c.ARW"]);

        // Re-importing the same contents (even under another name) adds nothing,
        // and the empty import disappears.
        let m2 = cat.begin_import("again", "/card").unwrap();
        assert!(cat.image_by_hash("h1").unwrap().is_some());
        assert_eq!(cat.add_image(m2, &img("renamed.ARW", "h1", None)).unwrap(), None);
        assert_eq!(cat.finish_import(m2).unwrap(), None);
        assert_eq!(cat.imports().unwrap().len(), 1);
        assert_eq!(cat.image_count().unwrap(), 3);
    }

    #[test]
    fn ratings_flags_and_filters() {
        let mut cat = Catalog::open_in_memory().unwrap();
        let m = cat.begin_import("x", "/x").unwrap();
        let a = cat.add_image(m, &img("1.ARW", "h1", Some("1"))).unwrap().unwrap();
        let b = cat.add_image(m, &img("2.ARW", "h2", Some("2"))).unwrap().unwrap();
        cat.set_rating(&[a], 9).unwrap();
        cat.set_flag(&[a, b], -1).unwrap();
        cat.set_flag(&[b], 1).unwrap();
        let rated = cat.images(&ImageQuery { min_rating: 3, ..Default::default() }).unwrap();
        assert_eq!(rated.len(), 1);
        assert_eq!(rated[0].rating, 5, "clamped");
        let picks = cat.images(&ImageQuery { flag: Some(1), ..Default::default() }).unwrap();
        assert_eq!(picks.iter().map(|i| i.id).collect::<Vec<_>>(), [b]);
        let by_import = cat.images(&ImageQuery { import: Some(m + 1), ..Default::default() }).unwrap();
        assert!(by_import.is_empty());
    }

    #[test]
    fn branch_counts() {
        let mut cat = Catalog::open_in_memory().unwrap();
        let m = cat.begin_import("x", "/x").unwrap();
        let id = cat.add_image(m, &img("1.ARW", "h1", None)).unwrap().unwrap();
        assert_eq!(cat.images(&ImageQuery::default()).unwrap()[0].branches, 0);
        let root = cat.history_open(id).unwrap().view.head.commit;
        cat.create_branch(id, root, Some("B&W")).unwrap();
        assert_eq!(cat.images(&ImageQuery::default()).unwrap()[0].branches, 2);
    }
}
