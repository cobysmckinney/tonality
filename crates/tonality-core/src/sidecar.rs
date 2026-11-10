//! Sidecars: an image's whole edit history (every branch and commit), rating and flag, as a
//! small JSON file next to the original (`DSC00084.ARW.tonality`).
//!
//! The catalog stays the source of truth; sidecars make each photo self-describing, so a
//! library can be rebuilt without its catalog and a RAW + sidecar pair carries its edits to
//! another library. Reading one back *merges* it, git-style: commits are matched by
//! (parent, state), branches that moved ahead fast-forward, and branches that diverged arrive
//! under a new name. Album membership is recorded by album name and merged as a union (missing
//! albums are created). Nothing already in the catalog is ever lost or rewritten.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, CatalogError, ImageId, Result};
use crate::edit::EditState;
use crate::history::{
    auto_branch_name, branches, head, lineage, parents, set_branch, set_head, store_state, validate_name, BranchInfo,
    CommitId,
};

/// Format version written by this build; newer files are refused rather than half-read.
pub const SIDECAR_VERSION: u32 = 1;
/// `DSC00084.ARW` → `DSC00084.ARW.tonality`
pub const SIDECAR_EXT: &str = "tonality";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sidecar {
    /// Format version.
    pub tonality: u32,
    pub image: SidecarImage,
    /// `None` if the photo has never been opened in Edit.
    #[serde(default)]
    pub head: Option<SidecarHead>,
    #[serde(default)]
    pub branches: Vec<SidecarBranch>,
    /// Oldest first; commits refer to each other by index into this list, parents first.
    #[serde(default)]
    pub commits: Vec<SidecarCommit>,
    /// Edit states by content hash (each stored once however many commits share it).
    #[serde(default)]
    pub states: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarImage {
    /// Original file name (informational).
    #[serde(default)]
    pub name: String,
    /// blake3 of the original file; a sidecar is only applied to the photo it was written for.
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub rating: u8,
    #[serde(default)]
    pub flag: i8,
    /// Names of the albums the photo is in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub albums: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SidecarHead {
    pub branch: String,
    pub commit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SidecarBranch {
    pub name: String,
    pub tip: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SidecarCommit {
    pub parent: Option<usize>,
    pub state: String,
    pub summary: String,
    /// Unix milliseconds.
    pub created: i64,
    pub time: i64,
}

impl Sidecar {
    /// Nothing worth keeping: never edited, unrated, unflagged.
    pub fn is_empty(&self) -> bool {
        self.head.is_none() && self.image.rating == 0 && self.image.flag == 0 && self.image.albums.is_empty()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("sidecars are always serializable")
    }

    pub fn from_json(s: &str) -> Result<Self> {
        let v: serde_json::Value = serde_json::from_str(s)?;
        let version = v.get("tonality").and_then(|v| v.as_u64()).unwrap_or(0);
        if version == 0 {
            return Err(CatalogError::Invalid("not a tonality sidecar".into()));
        }
        if version > SIDECAR_VERSION as u64 {
            return Err(CatalogError::Invalid("sidecar was written by a newer version of tonality".into()));
        }
        Ok(serde_json::from_value(v)?)
    }

    /// Check internal consistency and parse the states. Everything read from disk goes
    /// through this before touching the catalog.
    fn validate(&self) -> Result<HashMap<&str, EditState>> {
        let bad = |m: &str| Err(CatalogError::Invalid(format!("damaged sidecar: {m}")));
        let mut states = HashMap::new();
        for (k, v) in &self.states {
            states.insert(k.as_str(), serde_json::from_value::<EditState>(v.clone())?);
        }
        for (i, c) in self.commits.iter().enumerate() {
            match c.parent {
                None if i != 0 => return bad("more than one root commit"),
                Some(_) if i == 0 => return bad("the first commit has a parent"),
                Some(p) if p >= i => return bad("a commit precedes its parent"),
                _ => {}
            }
            if !states.contains_key(c.state.as_str()) {
                return bad("missing edit state");
            }
        }
        let mut names = HashSet::new();
        for b in &self.branches {
            validate_name(&b.name)?;
            if b.tip >= self.commits.len() {
                return bad("branch points nowhere");
            }
            if !names.insert(b.name.trim()) {
                return bad("duplicate branch name");
            }
        }
        if self.commits.is_empty() != self.branches.is_empty() {
            return bad("commits without branches");
        }
        if let Some(h) = &self.head {
            if h.commit >= self.commits.len() || !names.contains(h.branch.trim()) {
                return bad("HEAD points nowhere");
            }
        }
        Ok(states)
    }
}

/// What reading a sidecar changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeSummary {
    pub commits: u32,
    /// Branches created (new names, or diverged ones under a new name).
    pub branches: u32,
    /// Existing branches moved forward.
    pub advanced: u32,
    /// Rating or flag adopted from the sidecar.
    pub labels: bool,
    /// Albums the photo was added to.
    pub albums: u32,
}

impl MergeSummary {
    pub fn changed(&self) -> bool {
        self.commits + self.branches + self.advanced + self.albums > 0 || self.labels
    }
}

impl Catalog {
    /// Everything about `image` that a sidecar records.
    pub fn sidecar(&self, image: ImageId) -> Result<Sidecar> {
        let c = &self.conn;
        let (name, hash, rating, flag): (String, String, u8, i8) = c
            .query_row("SELECT name, hash, rating, flag FROM images WHERE id = ?1", [image], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?
            .ok_or_else(|| CatalogError::NotFound(format!("image {image}")))?;
        let albums = {
            let mut stmt = c.prepare(
                "SELECT a.name FROM albums a JOIN album_images ai ON ai.album_id = a.id
                 WHERE ai.image_id = ?1 ORDER BY a.name COLLATE NOCASE, a.id",
            )?;
            let rows = stmt.query_map([image], |r| r.get::<_, String>(0))?;
            let mut names: Vec<String> = rows.collect::<rusqlite::Result<_>>()?;
            names.dedup();
            names
        };
        let mut sc = Sidecar {
            tonality: SIDECAR_VERSION,
            image: SidecarImage { name, hash, rating, flag, albums },
            head: None,
            branches: vec![],
            commits: vec![],
            states: BTreeMap::new(),
        };
        if head(c, image)?.is_none() {
            return Ok(sc);
        }
        let view = self.history_view(image)?;
        let mut rows: HashMap<CommitId, (String, i64)> = HashMap::new();
        {
            let mut stmt = c.prepare("SELECT id, state_hash, created_at FROM commits WHERE image_id = ?1")?;
            for r in stmt.query_map([image], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))? {
                let (id, row) = r?;
                rows.insert(id, row);
            }
        }
        // `view.commits` is in id order, and a parent is always inserted before its children.
        let index: HashMap<CommitId, usize> = view.commits.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        for ci in &view.commits {
            let (state, created) = rows.get(&ci.id).cloned().ok_or_else(|| CatalogError::NotFound("commit".into()))?;
            if !sc.states.contains_key(&state) {
                let json: String = c.query_row("SELECT json FROM edit_states WHERE hash = ?1", [&state], |r| r.get(0))?;
                sc.states.insert(state.clone(), serde_json::from_str(&json)?);
            }
            sc.commits.push(SidecarCommit {
                parent: ci.parent.and_then(|p| index.get(&p).copied()),
                state,
                summary: ci.summary.clone(),
                created,
                time: ci.time,
            });
        }
        sc.branches = view.branches.iter().map(|b| SidecarBranch { name: b.name.clone(), tip: index[&b.tip] }).collect();
        sc.head = Some(SidecarHead { branch: view.head.branch.clone(), commit: index[&view.head.commit] });
        Ok(sc)
    }

    /// Merge a sidecar's history, rating and flag into `image`. Idempotent; never discards or
    /// moves anything the catalog already has except fast-forwarding a branch whose sidecar
    /// version strictly extends it. HEAD only follows the sidecar for photos with no history yet.
    pub fn merge_sidecar(&mut self, image: ImageId, sc: &Sidecar) -> Result<MergeSummary> {
        let states = sc.validate()?;
        let (hash, rating, flag): (String, u8, i8) = self
            .conn
            .query_row("SELECT hash, rating, flag FROM images WHERE id = ?1", [image], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?
            .ok_or_else(|| CatalogError::NotFound(format!("image {image}")))?;
        if !sc.image.hash.is_empty() && sc.image.hash != hash {
            return Err(CatalogError::Invalid("the sidecar belongs to a different photo".into()));
        }

        let mut out = MergeSummary::default();
        let tx = self.conn.transaction()?;
        let fresh = head(&tx, image)?.is_none();

        // Commits: reuse one with the same parent and state, otherwise add it.
        let mut map: Vec<CommitId> = Vec::with_capacity(sc.commits.len());
        let existing_root: Option<CommitId> = tx
            .query_row(
                "SELECT id FROM commits WHERE image_id = ?1 AND parent IS NULL ORDER BY id LIMIT 1",
                [image],
                |r| r.get(0),
            )
            .optional()?;
        for c in &sc.commits {
            let state_hash = store_state(&tx, &states[c.state.as_str()])?;
            let parent = match c.parent {
                Some(p) => Some(map[p]),
                // The sidecar's root lands on the catalog's root if they agree; otherwise below it
                // (one root per image).
                None => match existing_root {
                    Some(root) if fresh => Some(root),
                    Some(root) => {
                        let root_hash: String =
                            tx.query_row("SELECT state_hash FROM commits WHERE id = ?1", [root], |r| r.get(0))?;
                        if root_hash == state_hash {
                            map.push(root);
                            continue;
                        }
                        Some(root)
                    }
                    None => None,
                },
            };
            let found: Option<CommitId> = tx
                .query_row(
                    "SELECT id FROM commits WHERE image_id = ?1 AND parent IS ?2 AND state_hash = ?3 ORDER BY id LIMIT 1",
                    params![image, parent, state_hash],
                    |r| r.get(0),
                )
                .optional()?;
            let id = match found {
                Some(id) => id,
                None => {
                    tx.execute(
                        "INSERT INTO commits (image_id, parent, state_hash, summary, key, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6)",
                        params![image, parent, state_hash, c.summary, c.created, c.time],
                    )?;
                    out.commits += 1;
                    tx.last_insert_rowid()
                }
            };
            map.push(id);
        }

        // Branches.
        let par = parents(&tx, image)?;
        let mut have: Vec<BranchInfo> = branches(&tx, image)?;
        let mut renamed: HashMap<&str, String> = HashMap::new();
        for b in &sc.branches {
            let name = b.name.trim();
            let tip = map[b.tip];
            let contains = |t: CommitId| lineage(&par, t).contains(&tip);
            match have.iter().position(|h| h.name == name) {
                None if have.iter().any(|h| h.tip == tip) => {}
                None => {
                    set_branch(&tx, image, name, tip)?;
                    have.push(BranchInfo { name: name.into(), tip });
                    out.branches += 1;
                }
                Some(i) if contains(have[i].tip) => {}
                Some(i) if lineage(&par, tip).contains(&have[i].tip) => {
                    set_branch(&tx, image, name, tip)?;
                    have[i].tip = tip;
                    out.advanced += 1;
                }
                // Diverged. Already there under another name (an earlier merge)? Otherwise keep both.
                Some(_) if have.iter().any(|h| contains(h.tip)) => {}
                Some(_) => {
                    let new = auto_branch_name(&have, name);
                    set_branch(&tx, image, &new, tip)?;
                    have.push(BranchInfo { name: new.clone(), tip });
                    renamed.insert(name, new);
                    out.branches += 1;
                }
            }
        }

        if fresh {
            if let Some(first) = sc.branches.first() {
                let h = sc.head.as_ref().map_or((first.name.trim(), first.tip), |h| (h.branch.trim(), h.commit));
                let name = renamed.get(h.0).cloned().unwrap_or_else(|| h.0.to_string());
                let tip = have.iter().find(|b| b.name == name).map(|b| b.tip).unwrap_or(map[first.tip]);
                let commit = if lineage(&par, tip).contains(&map[h.1]) { map[h.1] } else { tip };
                set_head(&tx, image, &name, commit)?;
            }
        }

        if rating == 0 && sc.image.rating > 0 {
            tx.execute("UPDATE images SET rating = ?2 WHERE id = ?1", params![image, sc.image.rating.min(5)])?;
            out.labels = true;
        }
        if flag == 0 && sc.image.flag != 0 {
            tx.execute("UPDATE images SET flag = ?2 WHERE id = ?1", params![image, sc.image.flag.clamp(-1, 1)])?;
            out.labels = true;
        }

        // Albums: join each named album, creating it if this library has none by that name.
        let now = (self.clock)();
        for name in &sc.image.albums {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let album: i64 = match tx
                .query_row("SELECT id FROM albums WHERE name = ?1 ORDER BY id LIMIT 1", [name], |r| r.get(0))
                .optional()?
            {
                Some(id) => id,
                None => {
                    tx.execute("INSERT INTO albums (name, created_at) VALUES (?1, ?2)", params![name, now])?;
                    tx.last_insert_rowid()
                }
            };
            out.albums += tx.execute(
                "INSERT INTO album_images (album_id, image_id, added_at) VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
                params![album, image, now],
            )? as u32;
        }
        tx.commit()?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exp(v: f32) -> EditState {
        let mut s = EditState::default();
        s.light.exposure = v;
        s
    }

    /// Two catalogs holding the same photo (same contents hash).
    fn pair() -> ((Catalog, ImageId), (Catalog, ImageId)) {
        let a = Catalog::open_in_memory().unwrap();
        let b = Catalog::open_in_memory().unwrap();
        let (ia, ib) = (a.test_image("x"), b.test_image("x"));
        ((a, ia), (b, ib))
    }

    fn graph(cat: &Catalog, img: ImageId) -> Vec<(String, Vec<String>)> {
        let v = cat.history_view(img).unwrap();
        let par = parents(&cat.conn, img).unwrap();
        let summary: HashMap<CommitId, String> = v.commits.iter().map(|c| (c.id, c.summary.clone())).collect();
        v.branches
            .iter()
            .map(|b| (b.name.clone(), lineage(&par, b.tip).iter().rev().map(|c| summary[c].clone()).collect()))
            .collect()
    }

    #[test]
    fn round_trips_into_an_empty_catalog() {
        let ((mut a, ia), (mut b, ib)) = pair();
        let root = a.history_open(ia).unwrap().view.head.commit;
        a.commit(ia, &exp(1.0), None).unwrap();
        a.commit(ia, &exp(2.0), None).unwrap();
        a.create_branch(ia, root, Some("B&W")).unwrap();
        let mut bw = EditState::default();
        bw.color.saturation = -100.0;
        a.commit(ia, &bw, None).unwrap();
        a.undo(ia).unwrap(); // HEAD behind its tip
        a.set_rating(&[ia], 4).unwrap();
        a.set_flag(&[ia], 1).unwrap();

        let sc = Sidecar::from_json(&a.sidecar(ia).unwrap().to_json()).unwrap();
        assert_eq!(sc.commits.len(), 4);
        assert_eq!(sc.states.len(), 4);
        let m = b.merge_sidecar(ib, &sc).unwrap();
        assert_eq!((m.commits, m.branches, m.labels), (4, 2, true));

        let (va, vb) = (a.history_open(ia).unwrap(), b.history_open(ib).unwrap());
        assert_eq!(graph(&a, ia), graph(&b, ib));
        assert_eq!(vb.view.head.branch, "B&W");
        assert_eq!(vb.state, va.state, "HEAD position restored");
        assert!(vb.view.can_redo);
        assert_eq!(b.sidecar(ib).unwrap(), a.sidecar(ia).unwrap(), "same graph, same sidecar");
        // Reading it again changes nothing.
        assert!(!b.merge_sidecar(ib, &sc).unwrap().changed());
    }

    #[test]
    fn merges_like_git() {
        let ((mut a, ia), (mut b, ib)) = pair();
        // Shared start: main = Original → +1.
        a.history_open(ia).unwrap();
        a.commit(ia, &exp(1.0), None).unwrap();
        b.merge_sidecar(ib, &a.sidecar(ia).unwrap()).unwrap();
        // a moves main ahead and adds "HDR"; b moves main differently.
        a.commit(ia, &exp(2.0), None).unwrap();
        let tip = a.history_view(ia).unwrap().head.commit;
        a.create_branch(ia, tip, Some("HDR")).unwrap();
        a.commit(ia, &exp(3.0), None).unwrap();
        b.history_open(ib).unwrap();
        b.commit(ib, &exp(-1.0), None).unwrap();

        let m = b.merge_sidecar(ib, &a.sidecar(ia).unwrap()).unwrap();
        assert_eq!((m.commits, m.branches, m.advanced), (2, 2, 0));
        let g = graph(&b, ib);
        let names: Vec<&str> = g.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["main", "main-2", "HDR"], "diverged main arrives as main-2");
        assert_eq!(g[0].1.last().unwrap(), "Exposure -1.00", "local main untouched");
        assert_eq!(g[1].1.last().unwrap(), "Exposure +2.00");
        assert_eq!(b.history_view(ib).unwrap().head.branch, "main", "HEAD stays put");
        assert!(!b.merge_sidecar(ib, &a.sidecar(ia).unwrap()).unwrap().changed(), "idempotent");

        // A branch the sidecar extends fast-forwards.
        a.switch_branch(ia, "HDR").unwrap();
        a.commit(ia, &exp(4.0), None).unwrap();
        let m = b.merge_sidecar(ib, &a.sidecar(ia).unwrap()).unwrap();
        assert_eq!((m.commits, m.branches, m.advanced), (1, 0, 1));
        assert_eq!(graph(&b, ib)[2].1.last().unwrap(), "Exposure +4.00");
    }

    #[test]
    fn keeps_local_labels_and_rejects_bad_files() {
        let ((mut a, ia), (mut b, ib)) = pair();
        a.set_rating(&[ia], 2).unwrap();
        b.set_rating(&[ib], 5).unwrap();
        let sc = a.sidecar(ia).unwrap();
        assert!(sc.head.is_none() && !sc.is_empty());
        assert!(!b.merge_sidecar(ib, &sc).unwrap().changed());
        assert_eq!(b.sidecar(ib).unwrap().image.rating, 5);
        assert!(b.history_view(ib).is_err(), "no history created");

        let mut other = sc.clone();
        other.image.hash = "something else".into();
        assert!(b.merge_sidecar(ib, &other).is_err());

        a.history_open(ia).unwrap();
        a.commit(ia, &exp(1.0), None).unwrap();
        let good = a.sidecar(ia).unwrap();
        let mut cyclic = good.clone();
        cyclic.commits[1].parent = Some(1);
        let mut dangling = good.clone();
        dangling.branches[0].tip = 9;
        let mut missing = good.clone();
        missing.states.clear();
        for bad in [cyclic, dangling, missing] {
            assert!(b.merge_sidecar(ib, &bad).is_err());
        }
        assert!(b.history_view(ib).is_err(), "failed merges leave nothing behind");
        assert!(Sidecar::from_json(r#"{"tonality": 99}"#).is_err());
        assert!(Sidecar::from_json(r#"{"hello": 1}"#).is_err());
    }

    #[test]
    fn albums_travel_by_name() {
        let ((mut a, ia), (mut b, ib)) = pair();
        let trips = a.create_album("Trips").unwrap();
        let best = a.create_album("Best of").unwrap();
        a.add_to_album(trips, &[ia]).unwrap();
        a.add_to_album(best, &[ia]).unwrap();
        let sc = a.sidecar(ia).unwrap();
        assert_eq!(sc.image.albums, ["Best of", "Trips"]);
        assert!(!sc.is_empty(), "album membership alone is worth a sidecar");

        // b already has a "Trips"; "Best of" is created.
        let local = b.create_album("Trips").unwrap();
        let m = b.merge_sidecar(ib, &sc).unwrap();
        assert_eq!(m.albums, 2);
        let albums = b.albums().unwrap();
        assert_eq!(albums.iter().map(|x| (x.name.as_str(), x.count)).collect::<Vec<_>>(), [("Best of", 1), ("Trips", 1)]);
        assert_eq!(albums[1].id, local);
        assert!(!b.merge_sidecar(ib, &sc).unwrap().changed(), "idempotent");

        // Old sidecars without the field still read; empty lists aren't written.
        let plain = Sidecar::from_json(&b.sidecar(ib).unwrap().to_json().replace(r#","albums":["Best of","Trips"]"#, "")).unwrap();
        assert!(plain.image.albums.is_empty());
        b.remove_from_album(local, &[ib]).unwrap();
        b.delete_album(albums[0].id).unwrap();
        assert!(!b.sidecar(ib).unwrap().to_json().contains("albums"));
    }

    #[test]
    fn root_with_different_state_hangs_below_the_local_root() {
        let ((mut a, ia), (mut b, ib)) = pair();
        b.history_open(ib).unwrap();
        // A hand-made sidecar whose root isn't the default state.
        a.history_open(ia).unwrap();
        let mut sc = a.sidecar(ia).unwrap();
        let s = exp(0.5);
        sc.states = BTreeMap::from([(s.hash(), serde_json::to_value(&s).unwrap())]);
        sc.commits[0].state = s.hash();
        sc.branches[0].name = "imported".into();
        sc.head = None;
        b.merge_sidecar(ib, &sc).unwrap();
        let v = b.history_view(ib).unwrap();
        assert_eq!(v.commits.iter().filter(|c| c.parent.is_none()).count(), 1);
        assert_eq!(v.branches.len(), 2);
    }
}
