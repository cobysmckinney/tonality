//! Albums: user-made collections of library photos. A photo can be in any number of albums;
//! removing it from one (or deleting the album) never touches the photo itself.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::catalog::{Catalog, CatalogError, ImageId, Result};

pub type AlbumId = i64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AlbumInfo {
    #[ts(type = "number")]
    pub id: AlbumId,
    pub name: String,
    #[ts(type = "number")]
    pub created_at: i64,
    pub count: u32,
    /// The most recently added photo, shown as the album's cover.
    #[ts(type = "number | null")]
    pub cover: Option<ImageId>,
    /// The cover's HEAD commit (its thumbnail changes with it).
    #[ts(type = "number | null")]
    pub cover_head: Option<i64>,
}

fn clean_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CatalogError::Invalid("album name cannot be empty".into()));
    }
    Ok(name.chars().take(120).collect())
}

impl Catalog {
    /// All albums, by name.
    pub fn albums(&self) -> Result<Vec<AlbumInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.name, a.created_at,
                    (SELECT COUNT(*) FROM album_images ai WHERE ai.album_id = a.id),
                    (SELECT ai.image_id FROM album_images ai WHERE ai.album_id = a.id
                     ORDER BY ai.added_at DESC, ai.image_id DESC LIMIT 1)
             FROM albums a ORDER BY a.name COLLATE NOCASE, a.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(AlbumInfo {
                id: r.get(0)?,
                name: r.get(1)?,
                created_at: r.get(2)?,
                count: r.get(3)?,
                cover: r.get(4)?,
                cover_head: None,
            })
        })?;
        let mut out: Vec<AlbumInfo> = rows.collect::<rusqlite::Result<_>>()?;
        for a in &mut out {
            if let Some(c) = a.cover {
                a.cover_head = self
                    .conn
                    .query_row("SELECT commit_id FROM heads WHERE image_id = ?1", [c], |r| r.get(0))
                    .optional()?;
            }
        }
        Ok(out)
    }

    pub fn create_album(&mut self, name: &str) -> Result<AlbumId> {
        self.conn.execute(
            "INSERT INTO albums (name, created_at) VALUES (?1, ?2)",
            params![clean_name(name)?, self.now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn rename_album(&mut self, id: AlbumId, name: &str) -> Result<()> {
        let n = self.conn.execute("UPDATE albums SET name = ?1 WHERE id = ?2", params![clean_name(name)?, id])?;
        if n == 0 {
            return Err(CatalogError::NotFound(format!("album {id}")));
        }
        Ok(())
    }

    /// Delete an album. Its photos stay in the library.
    pub fn delete_album(&mut self, id: AlbumId) -> Result<()> {
        self.conn.execute("DELETE FROM albums WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Add photos to an album (ones already in it are left as they are). Returns how many were new.
    pub fn add_to_album(&mut self, id: AlbumId, images: &[ImageId]) -> Result<u32> {
        let now = self.now();
        let tx = self.conn.transaction()?;
        let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM albums WHERE id = ?1)", [id], |r| r.get(0))?;
        if !exists {
            return Err(CatalogError::NotFound(format!("album {id}")));
        }
        let mut added = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO album_images (album_id, image_id, added_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT DO NOTHING",
            )?;
            for img in images {
                added += stmt.execute(params![id, img, now])? as u32;
            }
        }
        tx.commit()?;
        Ok(added)
    }

    pub fn remove_from_album(&mut self, id: AlbumId, images: &[ImageId]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare("DELETE FROM album_images WHERE album_id = ?1 AND image_id = ?2")?;
            for img in images {
                stmt.execute(params![id, img])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::library::ImageQuery;
    use crate::Catalog;

    #[test]
    fn albums_hold_photos_without_owning_them() {
        let mut cat = Catalog::open_in_memory().unwrap();
        let a = cat.test_image("a");
        let b = cat.test_image("b");
        let trips = cat.create_album("  Trips ").unwrap();
        let fam = cat.create_album("Family").unwrap();
        assert!(cat.create_album("   ").is_err());

        assert_eq!(cat.add_to_album(trips, &[a, b]).unwrap(), 2);
        assert_eq!(cat.add_to_album(trips, &[a]).unwrap(), 0, "already there");
        cat.add_to_album(fam, &[b]).unwrap();

        let albums = cat.albums().unwrap();
        assert_eq!(albums.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Family", "Trips"]);
        assert_eq!(albums[1].count, 2);

        let in_album = |cat: &Catalog, id| {
            cat.images(&ImageQuery { album: Some(id), ..Default::default() })
                .unwrap()
                .into_iter()
                .map(|i| i.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(in_album(&cat, trips), [a, b]);
        assert_eq!(in_album(&cat, fam), [b]);

        cat.remove_from_album(trips, &[a]).unwrap();
        assert_eq!(in_album(&cat, trips), [b]);

        cat.rename_album(fam, "Kin").unwrap();
        cat.delete_album(trips).unwrap();
        assert_eq!(cat.albums().unwrap().len(), 1);
        assert_eq!(cat.image_count().unwrap(), 2, "photos survive their album");
        assert!(cat.add_to_album(trips, &[a]).is_err());
    }
}
