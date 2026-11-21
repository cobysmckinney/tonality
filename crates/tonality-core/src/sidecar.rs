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

use crate::album::clean_name;
use crate::catalog::{Catalog, CatalogError, ImageId, Result};
use crate::edit::EditState;
use crate::history::{
    auto_branch_name, branches, head, lineage, parents, set_branch, set_head, store_state,
    BranchInfo, CommitId,
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
        self.head.is_none()
            && self.image.rating == 0
            && self.image.flag == 0
            && self.image.albums.is_empty()
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
            return Err(CatalogError::Invalid(
                "sidecar was written by a newer version of tonality".into(),
            ));
        }
        Ok(serde_json::from_value(v)?)
    }

    /// Whether [`Catalog::merge_sidecar`] would accept this file's contents (before checking
    /// which photo it belongs to).
    pub fn check(&self) -> Result<()> {
        self.validate().map(|_| ())
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
            // Not `validate_name`: older builds could write auto-generated names over the
            // length limit, and refusing the file would strand that photo's history.
            if b.name.trim().is_empty() {
                return bad("empty branch name");
            }
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
            .query_row(
                "SELECT name, hash, rating, flag FROM images WHERE id = ?1",
                [image],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
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
            image: SidecarImage {
                name,
                hash,
                rating,
                flag,
                albums,
            },
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
            let mut stmt =
                c.prepare("SELECT id, state_hash, created_at FROM commits WHERE image_id = ?1")?;
            for r in stmt.query_map([image], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))? {
                let (id, row) = r?;
                rows.insert(id, row);
            }
        }
        // `view.commits` is in id order, and a parent is always inserted before its children.
        let index: HashMap<CommitId, usize> = view
            .commits
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id, i))
            .collect();
        for ci in &view.commits {
            let (state, created) = rows
                .get(&ci.id)
                .cloned()
                .ok_or_else(|| CatalogError::NotFound("commit".into()))?;
            if !sc.states.contains_key(&state) {
                let json: String = c.query_row(
                    "SELECT json FROM edit_states WHERE hash = ?1",
                    [&state],
                    |r| r.get(0),
                )?;
                sc.states
                    .insert(state.clone(), serde_json::from_str(&json)?);
            }
            sc.commits.push(SidecarCommit {
                parent: ci.parent.and_then(|p| index.get(&p).copied()),
                state,
                summary: ci.summary.clone(),
                created,
                time: ci.time,
            });
        }
        sc.branches = view
            .branches
            .iter()
            .map(|b| SidecarBranch {
                name: b.name.clone(),
                tip: index[&b.tip],
            })
            .collect();
        sc.head = Some(SidecarHead {
            branch: view.head.branch.clone(),
            commit: index[&view.head.commit],
        });
        Ok(sc)
    }

    /// Merge a sidecar's history, rating and flag into `image`. Idempotent; never discards or
    /// moves anything the catalog already has except fast-forwarding a branch whose sidecar
    /// version strictly extends it. HEAD only follows the sidecar for photos with no history yet.
    /// Commits and branches this catalog deleted (a gesture that ended where it started, a
    /// deleted branch) stay deleted, so a sidecar left stale by a crash can't bring them back.
    pub fn merge_sidecar(&mut self, image: ImageId, sc: &Sidecar) -> Result<MergeSummary> {
        let states = sc.validate()?;
        let (hash, rating, flag): (String, u8, i8) = self
            .conn
            .query_row(
                "SELECT hash, rating, flag FROM images WHERE id = ?1",
                [image],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| CatalogError::NotFound(format!("image {image}")))?;
        if !sc.image.hash.is_empty() && sc.image.hash != hash {
            return Err(CatalogError::Invalid(
                "the sidecar belongs to a different photo".into(),
            ));
        }

        let mut out = MergeSummary::default();
        let tx = self.conn.transaction()?;
        let fresh = head(&tx, image)?.is_none();

        // Commits: reuse one with the same parent and state, otherwise add it, unless it was
        // deleted here since the sidecar was written (then it, and anything below it, is
        // skipped and stands for its nearest kept ancestor).
        let mut map: Vec<CommitId> = Vec::with_capacity(sc.commits.len());
        let mut skipped: Vec<bool> = Vec::with_capacity(sc.commits.len());
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
                        if same_state(&tx, root, &state_hash)? {
                            map.push(root);
                            skipped.push(false);
                            continue;
                        }
                        Some(root)
                    }
                    None => None,
                },
            };
            if c.parent.is_some_and(|p| skipped[p]) {
                map.push(parent.expect("has a parent"));
                skipped.push(true);
                continue;
            }
            let found = same_child(&tx, image, parent, &state_hash, c)?;
            // Or the same commit, rewritten here since that sidecar was written (a slider gesture
            // coalesces into its commit in place): same parent and creation time, updated later.
            // Without this, a sidecar left stale by a crash comes back as a diverged branch.
            let found = match found {
                Some(id) => Some(id),
                None => tx
                    .query_row(
                        "SELECT id FROM commits WHERE image_id = ?1 AND parent IS ?2 AND created_at = ?3
                         AND key IS NOT NULL AND updated_at > ?4 ORDER BY id LIMIT 1",
                        params![image, parent, c.created, c.time],
                        |r| r.get(0),
                    )
                    .optional()?,
            };
            let id = match found {
                Some(id) => id,
                None if c.parent.is_some() && was_dropped(&tx, image, c.created)? => {
                    map.push(parent.expect("has a parent"));
                    skipped.push(true);
                    continue;
                }
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
            skipped.push(false);
        }

        // Branches.
        let par = parents(&tx, image)?;
        let mut have: Vec<BranchInfo> = branches(&tx, image)?;
        let mut renamed: HashMap<&str, String> = HashMap::new();
        for b in &sc.branches {
            let name = b.name.trim();
            // Deleted here after the sidecar was written, or its tip was: leave it gone.
            if skipped[b.tip] || was_deleted(&tx, image, name, sc.commits[b.tip].created)? {
                continue;
            }
            let tip = map[b.tip];
            let contains = |t: CommitId| lineage(&par, t).contains(&tip);
            // A local branch at the same tip that the sidecar doesn't know by name is most
            // likely this one renamed: don't add it twice. A branch the sidecar names itself
            // is a genuine second branch at that commit (e.g. just created), so this one stays.
            let renamed_here = |h: &BranchInfo| {
                h.tip == tip && !sc.branches.iter().any(|s| s.name.trim() == h.name)
            };
            match have.iter().position(|h| h.name == name) {
                None if !fresh && have.iter().any(renamed_here) => {}
                None => {
                    set_branch(&tx, image, name, tip)?;
                    have.push(BranchInfo {
                        name: name.into(),
                        tip,
                    });
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
                    have.push(BranchInfo {
                        name: new.clone(),
                        tip,
                    });
                    renamed.insert(name, new);
                    out.branches += 1;
                }
            }
        }

        if fresh {
            if let Some(first) = sc.branches.first() {
                let h = sc
                    .head
                    .as_ref()
                    .map_or((first.name.trim(), first.tip), |h| {
                        (h.branch.trim(), h.commit)
                    });
                let name = renamed.get(h.0).cloned().unwrap_or_else(|| h.0.to_string());
                // HEAD must name a branch that exists, or every later commit fails.
                let branch = have
                    .iter()
                    .find(|b| b.name == name)
                    .or_else(|| {
                        have.iter()
                            .find(|b| lineage(&par, b.tip).contains(&map[h.1]))
                    })
                    .or(have.first());
                if let Some(b) = branch {
                    let commit = if lineage(&par, b.tip).contains(&map[h.1]) {
                        map[h.1]
                    } else {
                        b.tip
                    };
                    set_head(&tx, image, &b.name, commit)?;
                }
            }
        }

        if rating == 0 && sc.image.rating > 0 {
            tx.execute(
                "UPDATE images SET rating = ?2 WHERE id = ?1",
                params![image, sc.image.rating.min(5)],
            )?;
            out.labels = true;
        }
        if flag == 0 && sc.image.flag != 0 {
            tx.execute(
                "UPDATE images SET flag = ?2 WHERE id = ?1",
                params![image, sc.image.flag.clamp(-1, 1)],
            )?;
            out.labels = true;
        }

        // Albums: join each named album, creating it if this library has none by that name.
        let now = (self.clock)();
        for name in &sc.image.albums {
            // The same rules as naming an album in the app, and the same case-blind match the
            // album list uses.
            let Ok(name) = clean_name(name) else {
                continue;
            };
            let album: i64 = match tx
                .query_row(
                    "SELECT id FROM albums WHERE name = ?1 COLLATE NOCASE ORDER BY id LIMIT 1",
                    [&name],
                    |r| r.get(0),
                )
                .optional()?
            {
                Some(id) => id,
                None => {
                    tx.execute(
                        "INSERT INTO albums (name, created_at) VALUES (?1, ?2)",
                        params![name, now],
                    )?;
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

/// Whether a commit created at `created` was deleted from `image` here (see `dropped_commits`).
fn was_dropped(tx: &rusqlite::Connection, image: ImageId, created: i64) -> Result<bool> {
    Ok(tx
        .query_row(
            "SELECT 1 FROM dropped_commits WHERE image_id = ?1 AND created_at = ?2",
            params![image, created],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Whether branch `name`, with its tip created at `tip_created`, was deleted from `image` here.
fn was_deleted(
    tx: &rusqlite::Connection,
    image: ImageId,
    name: &str,
    tip_created: i64,
) -> Result<bool> {
    Ok(tx
        .query_row(
            "SELECT 1 FROM deleted_branches WHERE image_id = ?1 AND name = ?2 AND tip_created_at = ?3",
            params![image, name, tip_created],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Whether commit `id` holds the state hashed `hash`, comparing in today's format: a state
/// stored by an older version (before a setting existed) hashes differently from the same
/// state read back now, but is the same edit.
fn same_state(tx: &rusqlite::Connection, id: CommitId, hash: &str) -> Result<bool> {
    let (stored, json): (String, String) = tx.query_row(
        "SELECT c.state_hash, s.json FROM commits c JOIN edit_states s ON s.hash = c.state_hash WHERE c.id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(stored == hash || serde_json::from_str::<EditState>(&json).is_ok_and(|s| s.hash() == hash))
}

/// The catalog's child of `parent` that is the sidecar's commit `c` (state hashed `hash`), if any:
/// one with the same state (in today's format), or the very same commit (same creation and
/// update times and summary) whatever its stored state looks like.
fn same_child(
    tx: &rusqlite::Connection,
    image: ImageId,
    parent: Option<CommitId>,
    hash: &str,
    c: &SidecarCommit,
) -> Result<Option<CommitId>> {
    // The state as the sidecar names it (the hash it was stored under, maybe in an older
    // format) first, then as re-hashed today: two siblings can be the same state in both formats
    // (a copy an older merge made), and the sidecar's own name says which one this commit is.
    for h in [c.state.as_str(), hash] {
        let exact = tx
            .query_row(
                "SELECT id FROM commits WHERE image_id = ?1 AND parent IS ?2 AND state_hash = ?3 ORDER BY id LIMIT 1",
                params![image, parent, h],
                |r| r.get(0),
            )
            .optional()?;
        if exact.is_some() {
            return Ok(exact);
        }
    }
    let mut stmt = tx.prepare(
        "SELECT id, summary, created_at, updated_at FROM commits WHERE image_id = ?1 AND parent IS ?2 ORDER BY id",
    )?;
    let kids: Vec<(CommitId, String, i64, i64)> = stmt
        .query_map(params![image, parent], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (id, summary, created, updated) in &kids {
        if *created == c.created && *updated == c.time && *summary == c.summary {
            return Ok(Some(*id));
        }
    }
    for (id, ..) in &kids {
        if same_state(tx, *id, hash)? {
            return Ok(Some(*id));
        }
    }
    Ok(None)
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
        let summary: HashMap<CommitId, String> = v
            .commits
            .iter()
            .map(|c| (c.id, c.summary.clone()))
            .collect();
        v.branches
            .iter()
            .map(|b| {
                (
                    b.name.clone(),
                    lineage(&par, b.tip)
                        .iter()
                        .rev()
                        .map(|c| summary[c].clone())
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn long_branch_names_round_trip() {
        let ((mut a, ia), (mut b, ib)) = pair();
        let root = a.history_open(ia).unwrap().view.head.commit;
        let long = "x".repeat(64);
        a.create_branch(ia, root, Some(&long)).unwrap();
        a.commit(ia, &exp(1.0), None).unwrap();
        a.undo(ia).unwrap();
        a.commit(ia, &exp(2.0), None).unwrap(); // forks under an auto-generated name
        let forked = a.history_open(ia).unwrap().view.head.branch;
        assert_eq!(forked.chars().count(), 64);

        let sc = Sidecar::from_json(&a.sidecar(ia).unwrap().to_json()).unwrap();
        b.merge_sidecar(ib, &sc).unwrap();
        assert_eq!(graph(&a, ia), graph(&b, ib));
    }

    #[test]
    fn over_long_branch_names_from_older_builds_are_still_read() {
        let ((mut a, ia), (mut b, ib)) = pair();
        a.history_open(ia).unwrap();
        a.commit(ia, &exp(1.0), None).unwrap();
        let mut sc = a.sidecar(ia).unwrap();
        let long = format!("{}-2", "x".repeat(64));
        sc.branches[0].name = long.clone();
        sc.head.as_mut().unwrap().branch = long.clone();
        let sc = Sidecar::from_json(&sc.to_json()).unwrap();
        b.merge_sidecar(ib, &sc).unwrap();
        let v = b.history_open(ib).unwrap().view;
        assert_eq!(v.head.branch, long);
        assert_eq!(v.commits.len(), 2);
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
        assert_eq!(
            b.sidecar(ib).unwrap(),
            a.sidecar(ia).unwrap(),
            "same graph, same sidecar"
        );
        // Reading it again changes nothing.
        assert!(!b.merge_sidecar(ib, &sc).unwrap().changed());
    }

    /// A state stored before a setting existed reads back with it (at its default), so it
    /// hashes differently now. Its own sidecar must still merge back as the same history, not
    /// as a copy on a new branch (as happened when masks gained new adjustments).
    #[test]
    fn states_stored_by_an_older_version_still_match() {
        let mut a = Catalog::open_in_memory().unwrap();
        let ia = a.test_image("x");
        a.history_open(ia).unwrap();
        let mut st = exp(1.0);
        st.masks.push(crate::mask::Mask::default());
        a.commit(ia, &st, None).unwrap();
        st.light.exposure = 2.0;
        a.commit(ia, &st, None).unwrap();

        // Rewrite every stored state as an older version would have written it: without
        // the masks' `vibrance`.
        let rows: Vec<(String, String)> = {
            let mut q = a
                .conn
                .prepare("SELECT hash, json FROM edit_states")
                .unwrap();
            q.query_map([], |r| Ok((r.get(0).unwrap(), r.get(1).unwrap())))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        for (hash, json) in rows {
            let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
            for m in v
                .get_mut("masks")
                .and_then(|m| m.as_array_mut())
                .into_iter()
                .flatten()
            {
                m["adjust"].as_object_mut().unwrap().remove("vibrance");
            }
            let old = serde_json::to_string(&v).unwrap();
            let old_hash = blake3::hash(old.as_bytes()).to_hex().to_string();
            if old_hash == hash {
                continue;
            }
            a.conn
                .execute(
                    "INSERT INTO edit_states (hash, json) VALUES (?1, ?2)",
                    params![old_hash, old],
                )
                .unwrap();
            a.conn
                .execute(
                    "UPDATE commits SET state_hash = ?1 WHERE state_hash = ?2",
                    params![old_hash, hash],
                )
                .unwrap();
        }

        let before = graph(&a, ia);
        let sc = Sidecar::from_json(&a.sidecar(ia).unwrap().to_json()).unwrap();
        assert!(!a.merge_sidecar(ia, &sc).unwrap().changed(), "nothing new");
        assert_eq!(graph(&a, ia), before);
        assert_eq!(a.history_view(ia).unwrap().branches.len(), 1, "no main-2");
    }

    #[test]
    fn a_copy_in_todays_format_does_not_capture_the_original() {
        let mut a = Catalog::open_in_memory().unwrap();
        let ia = a.test_image("x");
        a.history_open(ia).unwrap();
        let mut st = exp(1.0);
        st.masks.push(crate::mask::Mask::default());
        let first = a.commit(ia, &st, None).unwrap().view.head.commit;
        st.light.exposure = 2.0;
        a.commit(ia, &st, None).unwrap();

        // `first` stored the way an older version wrote it (no `vibrance`)...
        let (hash, json): (String, String) = a
            .conn
            .query_row("SELECT s.hash, s.json FROM commits c JOIN edit_states s ON s.hash = c.state_hash WHERE c.id = ?1", [first], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        for m in v["masks"].as_array_mut().unwrap() {
            m["adjust"].as_object_mut().unwrap().remove("vibrance");
        }
        let old = serde_json::to_string(&v).unwrap();
        let old_hash = blake3::hash(old.as_bytes()).to_hex().to_string();
        a.conn
            .execute(
                "INSERT INTO edit_states (hash, json) VALUES (?1, ?2)",
                params![old_hash, old],
            )
            .unwrap();
        a.conn
            .execute(
                "UPDATE commits SET state_hash = ?1 WHERE id = ?2",
                params![old_hash, first],
            )
            .unwrap();
        // ...and a sibling copy of it in today's format, on its own branch, made by an older merge.
        a.conn
            .execute(
                "INSERT INTO commits (image_id, parent, state_hash, summary, key, created_at, updated_at)
                 SELECT image_id, parent, ?2, summary, key, created_at, updated_at FROM commits WHERE id = ?1",
                params![first, hash],
            )
            .unwrap();
        let copy = a.conn.last_insert_rowid();
        crate::history::set_branch(&a.conn, ia, "main-2", copy).unwrap();

        let before = graph(&a, ia);
        let sc = Sidecar::from_json(&a.sidecar(ia).unwrap().to_json()).unwrap();
        assert!(!a.merge_sidecar(ia, &sc).unwrap().changed(), "nothing new");
        assert_eq!(graph(&a, ia), before);
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
        assert_eq!(
            names,
            ["main", "main-2", "HDR"],
            "diverged main arrives as main-2"
        );
        assert_eq!(
            g[0].1.last().unwrap(),
            "Exposure -1.00",
            "local main untouched"
        );
        assert_eq!(g[1].1.last().unwrap(), "Exposure +2.00");
        assert_eq!(
            b.history_view(ib).unwrap().head.branch,
            "main",
            "HEAD stays put"
        );
        assert!(
            !b.merge_sidecar(ib, &a.sidecar(ia).unwrap())
                .unwrap()
                .changed(),
            "idempotent"
        );

        // A branch the sidecar extends fast-forwards.
        a.switch_branch(ia, "HDR").unwrap();
        a.commit(ia, &exp(4.0), None).unwrap();
        let m = b.merge_sidecar(ib, &a.sidecar(ia).unwrap()).unwrap();
        assert_eq!((m.commits, m.branches, m.advanced), (1, 0, 1));
        assert_eq!(graph(&b, ib)[2].1.last().unwrap(), "Exposure +4.00");
    }

    #[test]
    fn two_branches_at_one_commit_both_arrive_and_head_is_valid() {
        let ((mut a, ia), (mut b, ib)) = pair();
        a.history_open(ia).unwrap();
        a.commit(ia, &exp(1.0), None).unwrap();
        let tip = a.history_view(ia).unwrap().head.commit;
        // "HDR" created at main's tip and checked out, nothing committed to it yet.
        a.create_branch(ia, tip, Some("HDR")).unwrap();
        assert_eq!(a.history_view(ia).unwrap().head.branch, "HDR");

        b.merge_sidecar(ib, &a.sidecar(ia).unwrap()).unwrap();
        assert!(
            !b.merge_sidecar(ib, &a.sidecar(ia).unwrap())
                .unwrap()
                .changed(),
            "idempotent"
        );
        let v = b.history_open(ib).unwrap().view;
        let names: Vec<&str> = v.branches.iter().map(|x| x.name.as_str()).collect();
        assert!(
            names.contains(&"main") && names.contains(&"HDR"),
            "{names:?}"
        );
        assert_eq!(v.head.branch, "HDR");
        b.commit(ib, &exp(2.0), None)
            .expect("HEAD names a real branch");
    }

    #[test]
    fn a_sidecar_from_before_a_coalesced_edit_is_not_a_new_branch() {
        use std::sync::atomic::{AtomicI64, Ordering};
        use std::sync::Arc;
        let t = Arc::new(AtomicI64::new(1_000));
        let tc = t.clone();
        let mut a = Catalog::open_in_memory()
            .unwrap()
            .with_clock(Box::new(move || tc.load(Ordering::SeqCst)));
        let ia = a.test_image("x");
        a.history_open(ia).unwrap();
        t.store(2_000, Ordering::SeqCst);
        a.commit(ia, &exp(1.0), Some("exposure")).unwrap();
        let stale = a.sidecar(ia).unwrap();
        // The same drag goes on: coalesced into that commit, and the app exits before the
        // sidecar is rewritten.
        t.store(2_100, Ordering::SeqCst);
        a.commit(ia, &exp(1.5), Some("exposure")).unwrap();
        let before = graph(&a, ia);
        let m = a.merge_sidecar(ia, &stale).unwrap();
        assert!(!m.changed(), "{m:?}");
        assert_eq!(graph(&a, ia), before);
    }

    /// A clock the test moves by hand, so gestures coalesce.
    fn clocked() -> (
        Catalog,
        ImageId,
        std::sync::Arc<std::sync::atomic::AtomicI64>,
    ) {
        use std::sync::atomic::{AtomicI64, Ordering};
        let t = std::sync::Arc::new(AtomicI64::new(1_000));
        let tc = t.clone();
        let cat = Catalog::open_in_memory()
            .unwrap()
            .with_clock(Box::new(move || tc.load(Ordering::SeqCst)));
        let img = cat.test_image("x");
        (cat, img, t)
    }

    #[test]
    fn a_stale_sidecar_does_not_bring_back_an_undone_gesture() {
        use std::sync::atomic::Ordering;
        let (mut a, ia, t) = clocked();
        a.history_open(ia).unwrap();
        t.store(2_000, Ordering::SeqCst);
        a.commit(ia, &exp(1.0), Some("exposure")).unwrap();
        // The sidecar is written mid-drag...
        let stale = a.sidecar(ia).unwrap();
        // ...the drag ends back where it started, which drops its commit, and the app dies
        // before the sidecar is rewritten.
        t.store(2_100, Ordering::SeqCst);
        a.commit(ia, &exp(0.0), Some("exposure")).unwrap();
        assert_eq!(a.history_view(ia).unwrap().commits.len(), 1);
        let before = graph(&a, ia);
        let m = a.merge_sidecar(ia, &stale).unwrap();
        assert!(!m.changed(), "{m:?}");
        assert_eq!(graph(&a, ia), before);
        assert_eq!(a.history_open(ia).unwrap().state, exp(0.0));

        // Edits made after the undone gesture still merge normally.
        t.store(10_000, Ordering::SeqCst);
        a.commit(ia, &exp(2.0), Some("exposure")).unwrap();
        let newer = a.sidecar(ia).unwrap();
        let mut b = Catalog::open_in_memory().unwrap();
        let ib = b.test_image("x");
        b.merge_sidecar(ib, &newer).unwrap();
        assert_eq!(graph(&b, ib), graph(&a, ia));
        assert!(!a.merge_sidecar(ia, &newer).unwrap().changed());
    }

    #[test]
    fn a_stale_sidecar_does_not_bring_back_a_deleted_branch() {
        use std::sync::atomic::Ordering;
        let (mut a, ia, t) = clocked();
        a.history_open(ia).unwrap();
        t.store(2_000, Ordering::SeqCst);
        let tip = a.commit(ia, &exp(1.0), None).unwrap().view.head.commit;
        // "HDR" has commits of its own; "Alt" only shares main's tip.
        a.create_branch(ia, tip, Some("Alt")).unwrap();
        a.create_branch(ia, tip, Some("HDR")).unwrap();
        t.store(3_000, Ordering::SeqCst);
        a.commit(ia, &exp(2.0), None).unwrap();
        t.store(4_000, Ordering::SeqCst);
        a.commit(ia, &exp(3.0), None).unwrap();
        let stale = a.sidecar(ia).unwrap();
        a.switch_branch(ia, "main").unwrap();
        a.delete_branch(ia, "HDR").unwrap();
        a.delete_branch(ia, "Alt").unwrap();
        let before = graph(&a, ia);
        let m = a.merge_sidecar(ia, &stale).unwrap();
        assert!(!m.changed(), "{m:?}");
        assert_eq!(graph(&a, ia), before);

        // A new branch by the same name somewhere else is a different branch.
        a.create_branch(ia, tip, Some("HDR")).unwrap();
        t.store(5_000, Ordering::SeqCst);
        a.commit(ia, &exp(-1.0), None).unwrap();
        let mut b = Catalog::open_in_memory().unwrap();
        let ib = b.test_image("x");
        b.merge_sidecar(ib, &a.sidecar(ia).unwrap()).unwrap();
        assert_eq!(graph(&b, ib), graph(&a, ia));
        assert!(!a
            .merge_sidecar(ia, &a.sidecar(ia).unwrap())
            .unwrap()
            .changed());
        assert!(!a.merge_sidecar(ia, &stale).unwrap().changed());
    }

    #[test]
    fn a_renamed_branch_is_not_duplicated() {
        let ((mut a, ia), (mut b, ib)) = pair();
        a.history_open(ia).unwrap();
        a.commit(ia, &exp(1.0), None).unwrap();
        b.merge_sidecar(ib, &a.sidecar(ia).unwrap()).unwrap();
        // b renames main; a's sidecar still says "main" at the same tip.
        b.history_open(ib).unwrap();
        b.rename_branch(ib, "main", "Final").unwrap();
        let m = b.merge_sidecar(ib, &a.sidecar(ia).unwrap()).unwrap();
        assert_eq!(m.branches, 0);
        let names: Vec<String> = b
            .history_view(ib)
            .unwrap()
            .branches
            .into_iter()
            .map(|x| x.name)
            .collect();
        assert_eq!(names, ["Final"]);
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
        assert!(
            b.history_view(ib).is_err(),
            "failed merges leave nothing behind"
        );
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
        assert_eq!(
            albums
                .iter()
                .map(|x| (x.name.as_str(), x.count))
                .collect::<Vec<_>>(),
            [("Best of", 1), ("Trips", 1)]
        );
        assert_eq!(albums[1].id, local);
        assert!(!b.merge_sidecar(ib, &sc).unwrap().changed(), "idempotent");

        // Old sidecars without the field still read; empty lists aren't written.
        let plain = Sidecar::from_json(
            &b.sidecar(ib)
                .unwrap()
                .to_json()
                .replace(r#","albums":["Best of","Trips"]"#, ""),
        )
        .unwrap();
        assert!(plain.image.albums.is_empty());

        // Names follow the album rules: matched ignoring case, and no longer than the app allows.
        let mut odd = sc.clone();
        let long = "x".repeat(200);
        odd.image.albums = vec!["  trips ".into(), long.clone(), "   ".into()];
        let m = b.merge_sidecar(ib, &odd).unwrap();
        assert_eq!(m.albums, 1, "only the long one is new");
        let albums = b.albums().unwrap();
        assert_eq!(albums.len(), 3, "no second Trips");
        assert!(albums.iter().any(|x| x.name == "x".repeat(120)));
        assert!(!b.merge_sidecar(ib, &odd).unwrap().changed(), "idempotent");
        let albums: Vec<_> = albums
            .into_iter()
            .filter(|x| x.name != "x".repeat(120))
            .collect();

        b.remove_from_album(local, &[ib]).unwrap();
        b.delete_album(albums[0].id).unwrap();
        for x in b.albums().unwrap() {
            if x.name.len() == 120 {
                b.delete_album(x.id).unwrap();
            }
        }
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
