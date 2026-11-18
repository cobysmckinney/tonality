//! Git-like edit history.
//!
//! Every image has a tree of commits (each one a full [`EditState`] snapshot), one or more named
//! branches pointing at commits, and a HEAD = (branch, commit). HEAD's commit is always on its
//! branch: either the tip, or an ancestor of it after undo/checkout.
//!
//! History is never rewritten. Committing while HEAD is behind its branch tip (after undo, or
//! after checking out an older commit) forks a new branch instead of discarding anything. The
//! only exception is *coalescing*: repeated commits from the same control within
//! [`COALESCE_MS`] update the tip commit in place, so a run of arrow-key nudges becomes one step.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::catalog::{Catalog, CatalogError, ImageId, Result};
use crate::edit::{describe_change, EditState, Panel};

pub type CommitId = i64;

pub const DEFAULT_BRANCH: &str = "main";
pub const COALESCE_MS: i64 = 1500;
const MAX_BRANCH_NAME: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommitInfo {
    #[ts(type = "number")]
    pub id: CommitId,
    #[ts(type = "number | null")]
    pub parent: Option<CommitId>,
    pub summary: String,
    /// Unix milliseconds of the last change to this commit.
    #[ts(type = "number")]
    pub time: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BranchInfo {
    pub name: String,
    #[ts(type = "number")]
    pub tip: CommitId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Head {
    pub branch: String,
    #[ts(type = "number")]
    pub commit: CommitId,
}

/// Everything the history panel needs to draw the graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryView {
    #[ts(type = "number")]
    pub image_id: ImageId,
    /// Commits reachable from some branch, oldest first (parents precede children).
    pub commits: Vec<CommitInfo>,
    /// Branches in creation order; the first is the original `main`.
    pub branches: Vec<BranchInfo>,
    pub head: Head,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// Result of any history operation: the new graph plus the state at HEAD.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Checkout {
    pub view: HistoryView,
    pub state: EditState,
}

// ---------------------------------------------------------------------------------------------
// Low-level row access (free functions so they work on a Connection or a Transaction)
// ---------------------------------------------------------------------------------------------

struct CommitRow {
    id: CommitId,
    parent: Option<CommitId>,
    state_hash: String,
    key: Option<String>,
    updated_at: i64,
}

fn commit_row(c: &Connection, id: CommitId) -> Result<CommitRow> {
    c.query_row(
        "SELECT id, parent, state_hash, key, updated_at FROM commits WHERE id = ?1",
        [id],
        |r| {
            Ok(CommitRow {
                id: r.get(0)?,
                parent: r.get(1)?,
                state_hash: r.get(2)?,
                key: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| CatalogError::NotFound(format!("commit {id}")))
}

fn load_state(c: &Connection, hash: &str) -> Result<EditState> {
    let json: String = c
        .query_row(
            "SELECT json FROM edit_states WHERE hash = ?1",
            [hash],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| CatalogError::NotFound(format!("edit state {hash}")))?;
    Ok(serde_json::from_str(&json)?)
}

pub(crate) fn store_state(c: &Connection, state: &EditState) -> Result<String> {
    let hash = state.hash();
    c.execute(
        "INSERT OR IGNORE INTO edit_states (hash, json) VALUES (?1, ?2)",
        params![hash, state.to_canonical_json()],
    )?;
    Ok(hash)
}

pub(crate) fn head(c: &Connection, image: ImageId) -> Result<Option<Head>> {
    Ok(c.query_row(
        "SELECT branch, commit_id FROM heads WHERE image_id = ?1",
        [image],
        |r| {
            Ok(Head {
                branch: r.get(0)?,
                commit: r.get(1)?,
            })
        },
    )
    .optional()?)
}

pub(crate) fn set_head(
    c: &Connection,
    image: ImageId,
    branch: &str,
    commit: CommitId,
) -> Result<()> {
    c.execute(
        "INSERT INTO heads (image_id, branch, commit_id) VALUES (?1, ?2, ?3)
         ON CONFLICT(image_id) DO UPDATE SET branch = excluded.branch, commit_id = excluded.commit_id",
        params![image, branch, commit],
    )?;
    Ok(())
}

pub(crate) fn branches(c: &Connection, image: ImageId) -> Result<Vec<BranchInfo>> {
    let mut stmt = c.prepare("SELECT name, commit_id FROM refs WHERE image_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map([image], |r| {
        Ok(BranchInfo {
            name: r.get(0)?,
            tip: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn branch_tip(c: &Connection, image: ImageId, name: &str) -> Result<CommitId> {
    c.query_row(
        "SELECT commit_id FROM refs WHERE image_id = ?1 AND name = ?2",
        params![image, name],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| CatalogError::NotFound(format!("branch “{name}”")))
}

pub(crate) fn set_branch(
    c: &Connection,
    image: ImageId,
    name: &str,
    commit: CommitId,
) -> Result<()> {
    c.execute(
        "INSERT INTO refs (image_id, name, commit_id) VALUES (?1, ?2, ?3)
         ON CONFLICT(image_id, name) DO UPDATE SET commit_id = excluded.commit_id",
        params![image, name, commit],
    )?;
    Ok(())
}

pub(crate) fn parents(
    c: &Connection,
    image: ImageId,
) -> Result<HashMap<CommitId, Option<CommitId>>> {
    let mut stmt = c.prepare("SELECT id, parent FROM commits WHERE image_id = ?1")?;
    let rows = stmt.query_map([image], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// `tip` and all its ancestors, tip first.
pub(crate) fn lineage(
    parents: &HashMap<CommitId, Option<CommitId>>,
    tip: CommitId,
) -> Vec<CommitId> {
    let mut out = vec![tip];
    let mut cur = tip;
    while let Some(Some(p)) = parents.get(&cur) {
        out.push(*p);
        cur = *p;
    }
    out
}

fn has_children(c: &Connection, commit: CommitId) -> Result<bool> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM commits WHERE parent = ?1)",
        [commit],
        |r| r.get(0),
    )?)
}

fn insert_commit(
    c: &Connection,
    image: ImageId,
    parent: Option<CommitId>,
    state_hash: &str,
    summary: &str,
    key: Option<&str>,
    now: i64,
) -> Result<CommitId> {
    c.execute(
        "INSERT INTO commits (image_id, parent, state_hash, summary, key, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![image, parent, state_hash, summary, key, now],
    )?;
    Ok(c.last_insert_rowid())
}

pub(crate) fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CatalogError::Invalid("Branch name can’t be empty".into()));
    }
    if name.chars().count() > MAX_BRANCH_NAME {
        return Err(CatalogError::Invalid(format!(
            "Branch names are limited to {MAX_BRANCH_NAME} characters"
        )));
    }
    Ok(name.to_string())
}

/// Next free `base-N` name, where `base` is `from` without any existing `-N` suffix.
pub(crate) fn auto_branch_name(existing: &[BranchInfo], from: &str) -> String {
    let base = match from.rsplit_once('-') {
        Some((b, n)) if !b.is_empty() && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => {
            b
        }
        _ => from,
    };
    let taken: HashSet<&str> = existing.iter().map(|b| b.name.as_str()).collect();
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|n| !taken.contains(n.as_str()))
        .expect("unbounded")
}

// ---------------------------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------------------------

impl Catalog {
    /// Open an image's history, creating `main` with an "Original" commit on first use.
    pub fn history_open(&mut self, image: ImageId) -> Result<Checkout> {
        let tx = self.conn.transaction()?;
        if head(&tx, image)?.is_none() {
            let hash = store_state(&tx, &EditState::default())?;
            let root = insert_commit(&tx, image, None, &hash, "Original", None, (self.clock)())?;
            set_branch(&tx, image, DEFAULT_BRANCH, root)?;
            set_head(&tx, image, DEFAULT_BRANCH, root)?;
        }
        tx.commit()?;
        self.checkout_result(image)
    }

    pub fn history_view(&self, image: ImageId) -> Result<HistoryView> {
        let c = &self.conn;
        let head = head(c, image)?.ok_or_else(|| CatalogError::NotFound("history".into()))?;
        let branches = branches(c, image)?;
        let parents = parents(c, image)?;
        let visible: HashSet<CommitId> = branches
            .iter()
            .flat_map(|b| lineage(&parents, b.tip))
            .collect();

        let mut stmt = c.prepare(
            "SELECT id, parent, summary, updated_at FROM commits WHERE image_id = ?1 ORDER BY id",
        )?;
        let commits = stmt
            .query_map([image], |r| {
                Ok(CommitInfo {
                    id: r.get(0)?,
                    parent: r.get(1)?,
                    summary: r.get(2)?,
                    time: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|c| visible.contains(&c.id))
            .collect();

        let can_undo = matches!(parents.get(&head.commit), Some(Some(_)));
        let can_redo = branches
            .iter()
            .any(|b| b.name == head.branch && b.tip != head.commit);
        Ok(HistoryView {
            image_id: image,
            commits,
            branches,
            head,
            can_undo,
            can_redo,
        })
    }

    pub fn state_at(&self, commit: CommitId) -> Result<EditState> {
        load_state(&self.conn, &commit_row(&self.conn, commit)?.state_hash)
    }

    fn checkout_result(&self, image: ImageId) -> Result<Checkout> {
        let view = self.history_view(image)?;
        let state = self.state_at(view.head.commit)?;
        Ok(Checkout { view, state })
    }

    fn require_head(&self, image: ImageId) -> Result<Head> {
        head(&self.conn, image)?.ok_or_else(|| CatalogError::NotFound("history".into()))
    }

    /// Record `state` as the result of an edit. `key` identifies the control that made the
    /// change, for coalescing; pass `None` for one-off actions (e.g. "As Shot").
    pub fn commit(
        &mut self,
        image: ImageId,
        state: &EditState,
        key: Option<&str>,
    ) -> Result<Checkout> {
        self.commit_inner(image, state, key, None)
    }

    /// Copy `panels` from the tip of branch `source` onto HEAD, as one commit
    /// ("Color from B&W"). A no-op if nothing differs.
    pub fn apply_from_branch(
        &mut self,
        image: ImageId,
        source: &str,
        panels: &[Panel],
    ) -> Result<Checkout> {
        if panels.is_empty() {
            return Err(CatalogError::Invalid("choose at least one panel".into()));
        }
        let h = self.require_head(image)?;
        if h.branch == source {
            return Err(CatalogError::Invalid(
                "can't copy a branch onto itself".into(),
            ));
        }
        let tip = branch_tip(&self.conn, image, source)?;
        let src = self.state_at(tip)?;
        let cur = self.state_at(h.commit)?;
        let next = cur.with_panels_from(&src, panels);
        let what = if panels.len() == Panel::ALL.len() {
            "All settings".to_string()
        } else {
            Panel::ALL
                .iter()
                .filter(|p| panels.contains(p))
                .map(|p| p.label())
                .collect::<Vec<_>>()
                .join(", ")
        };
        self.commit_inner(image, &next, None, Some(format!("{what} from {source}")))
    }

    fn commit_inner(
        &mut self,
        image: ImageId,
        state: &EditState,
        key: Option<&str>,
        summary: Option<String>,
    ) -> Result<Checkout> {
        let now = self.now();
        let tx = self.conn.transaction()?;
        let h = head(&tx, image)?.ok_or_else(|| CatalogError::NotFound("history".into()))?;
        let cur = commit_row(&tx, h.commit)?;
        let tip = branch_tip(&tx, image, &h.branch)?;
        let at_tip = tip == h.commit;
        let new_hash = state.hash();

        // Coalesce into the tip commit?
        let other_refs: i64 = tx.query_row(
            "SELECT COUNT(*) FROM refs WHERE image_id = ?1 AND commit_id = ?2 AND name != ?3",
            params![image, cur.id, h.branch],
            |r| r.get(0),
        )?;
        let coalesce = at_tip
            && key.is_some()
            && cur.key.as_deref() == key
            && now - cur.updated_at < COALESCE_MS
            && other_refs == 0
            && cur.parent.is_some()
            && !has_children(&tx, cur.id)?;

        if coalesce {
            let parent = cur.parent.expect("checked above");
            let parent_row = commit_row(&tx, parent)?;
            if parent_row.state_hash == new_hash {
                // The gesture ended where it started: drop the commit entirely.
                set_branch(&tx, image, &h.branch, parent)?;
                set_head(&tx, image, &h.branch, parent)?;
                tx.execute("DELETE FROM commits WHERE id = ?1", [cur.id])?;
            } else {
                let prev = load_state(&tx, &parent_row.state_hash)?;
                store_state(&tx, state)?;
                tx.execute(
                    "UPDATE commits SET state_hash = ?2, summary = ?3, updated_at = ?4 WHERE id = ?1",
                    params![cur.id, new_hash, describe_change(&prev, state), now],
                )?;
            }
        } else if cur.state_hash != new_hash {
            let prev = load_state(&tx, &cur.state_hash)?;
            store_state(&tx, state)?;
            let summary = summary.unwrap_or_else(|| describe_change(&prev, state));
            let id = insert_commit(&tx, image, Some(cur.id), &new_hash, &summary, key, now)?;
            let branch = if at_tip {
                h.branch.clone()
            } else {
                // Editing from the past forks; the old branch keeps its tip.
                auto_branch_name(&branches(&tx, image)?, &h.branch)
            };
            set_branch(&tx, image, &branch, id)?;
            set_head(&tx, image, &branch, id)?;
        }
        tx.commit()?;
        self.checkout_result(image)
    }

    /// Step HEAD back one commit along the current branch.
    pub fn undo(&mut self, image: ImageId) -> Result<Checkout> {
        let h = self.require_head(image)?;
        if let Some(parent) = commit_row(&self.conn, h.commit)?.parent {
            set_head(&self.conn, image, &h.branch, parent)?;
        }
        self.checkout_result(image)
    }

    /// Step HEAD forward one commit towards the current branch's tip.
    pub fn redo(&mut self, image: ImageId) -> Result<Checkout> {
        let h = self.require_head(image)?;
        let tip = branch_tip(&self.conn, image, &h.branch)?;
        let line = lineage(&parents(&self.conn, image)?, tip);
        if let Some(i) = line.iter().position(|&c| c == h.commit) {
            if i > 0 {
                set_head(&self.conn, image, &h.branch, line[i - 1])?;
            }
        }
        self.checkout_result(image)
    }

    /// Move HEAD to any visible commit. Stays on the current branch if the commit is on it,
    /// otherwise switches to a branch that ends there, otherwise to the oldest branch containing it.
    pub fn checkout(&mut self, image: ImageId, commit: CommitId) -> Result<Checkout> {
        let h = self.require_head(image)?;
        let branches = branches(&self.conn, image)?;
        let parents = parents(&self.conn, image)?;
        let contains = |b: &BranchInfo| lineage(&parents, b.tip).contains(&commit);
        let current = branches
            .iter()
            .find(|b| b.name == h.branch)
            .filter(|b| contains(b));
        let branch = current
            .or_else(|| branches.iter().find(|b| b.tip == commit))
            .or_else(|| branches.iter().find(|b| contains(b)))
            .ok_or_else(|| CatalogError::NotFound(format!("commit {commit}")))?;
        set_head(&self.conn, image, &branch.name, commit)?;
        self.checkout_result(image)
    }

    /// Switch to a branch's tip.
    pub fn switch_branch(&mut self, image: ImageId, name: &str) -> Result<Checkout> {
        let tip = branch_tip(&self.conn, image, name)?;
        set_head(&self.conn, image, name, tip)?;
        self.checkout_result(image)
    }

    /// Create a branch at `at` (auto-named after the current branch if `name` is `None`)
    /// and switch to it.
    pub fn create_branch(
        &mut self,
        image: ImageId,
        at: CommitId,
        name: Option<&str>,
    ) -> Result<Checkout> {
        let h = self.require_head(image)?;
        let existing = branches(&self.conn, image)?;
        let parents = parents(&self.conn, image)?;
        if !existing
            .iter()
            .any(|b| lineage(&parents, b.tip).contains(&at))
        {
            return Err(CatalogError::NotFound(format!("commit {at}")));
        }
        let name = match name {
            Some(n) => validate_name(n)?,
            None => auto_branch_name(&existing, &h.branch),
        };
        if existing.iter().any(|b| b.name == name) {
            return Err(CatalogError::Invalid(format!(
                "A branch named “{name}” already exists"
            )));
        }
        let tx = self.conn.transaction()?;
        set_branch(&tx, image, &name, at)?;
        set_head(&tx, image, &name, at)?;
        tx.commit()?;
        self.checkout_result(image)
    }

    pub fn rename_branch(&mut self, image: ImageId, old: &str, new: &str) -> Result<Checkout> {
        let new = validate_name(new)?;
        if new != old {
            branch_tip(&self.conn, image, old)?;
            if branch_tip(&self.conn, image, &new).is_ok() {
                return Err(CatalogError::Invalid(format!(
                    "A branch named “{new}” already exists"
                )));
            }
            let tx = self.conn.transaction()?;
            tx.execute(
                "UPDATE refs SET name = ?3 WHERE image_id = ?1 AND name = ?2",
                params![image, old, new],
            )?;
            tx.execute(
                "UPDATE heads SET branch = ?3 WHERE image_id = ?1 AND branch = ?2",
                params![image, old, new],
            )?;
            tx.commit()?;
        }
        self.checkout_result(image)
    }

    /// Delete a branch. Its commits stay in the catalog but disappear from the graph unless
    /// another branch shares them. The last branch can't be deleted.
    pub fn delete_branch(&mut self, image: ImageId, name: &str) -> Result<Checkout> {
        let h = self.require_head(image)?;
        let all = branches(&self.conn, image)?;
        if !all.iter().any(|b| b.name == name) {
            return Err(CatalogError::NotFound(format!("branch “{name}”")));
        }
        let Some(fallback) = all.iter().find(|b| b.name != name) else {
            return Err(CatalogError::Invalid(
                "The only branch can’t be deleted".into(),
            ));
        };
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM refs WHERE image_id = ?1 AND name = ?2",
            params![image, name],
        )?;
        if h.branch == name {
            set_head(&tx, image, &fallback.name, fallback.tip)?;
        }
        // The commits only that branch had are unreachable now: delete them (children first,
        // which have the higher ids), and any edit state no commit uses any more.
        let par = parents(&tx, image)?;
        let keep: HashSet<CommitId> = branches(&tx, image)?
            .iter()
            .flat_map(|b| lineage(&par, b.tip))
            .collect();
        let mut gone: Vec<CommitId> = par.keys().filter(|c| !keep.contains(c)).copied().collect();
        gone.sort_unstable_by(|a, b| b.cmp(a));
        for c in &gone {
            tx.execute("DELETE FROM commits WHERE id = ?1", [c])?;
        }
        if !gone.is_empty() {
            tx.execute(
                "DELETE FROM edit_states WHERE hash NOT IN (SELECT state_hash FROM commits)",
                [],
            )?;
        }
        tx.commit()?;
        self.checkout_result(image)
    }

    /// Take photos back to as-shot and delete **all** their history: every branch and commit,
    /// and any edit state no other commit uses. Can't be undone. The photos look as if they
    /// were never edited (history is created again on the next open); their shape is forgotten
    /// so the library relearns it from the plain thumbnail. Ratings, flags and albums stay.
    pub fn reset_history(&mut self, images: &[ImageId]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for image in images {
            tx.execute("DELETE FROM heads WHERE image_id = ?1", [image])?;
            tx.execute("DELETE FROM refs WHERE image_id = ?1", [image])?;
            // One statement: the parent links are only checked once it has finished.
            tx.execute("DELETE FROM commits WHERE image_id = ?1", [image])?;
            tx.execute("UPDATE images SET aspect = NULL WHERE id = ?1", [image])?;
        }
        tx.execute(
            "DELETE FROM edit_states WHERE hash NOT IN (SELECT state_hash FROM commits)",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;

    use super::*;

    fn catalog() -> (Catalog, Arc<AtomicI64>, ImageId) {
        let t = Arc::new(AtomicI64::new(1_000_000));
        let tc = t.clone();
        let cat = Catalog::open_in_memory()
            .unwrap()
            .with_clock(Box::new(move || tc.load(Ordering::SeqCst)));
        let img = cat.test_image("x");
        (cat, t, img)
    }

    fn exp(v: f32) -> EditState {
        let mut s = EditState::default();
        s.light.exposure = v;
        s
    }

    fn summaries(c: &Checkout) -> Vec<&str> {
        c.view.commits.iter().map(|c| c.summary.as_str()).collect()
    }

    #[test]
    fn reset_removes_all_history_of_only_those_photos() {
        let (mut cat, _, img) = catalog();
        let other = cat.test_image("y");
        let root = cat.history_open(img).unwrap().view.head.commit;
        cat.commit(img, &exp(1.0), None).unwrap();
        cat.create_branch(img, root, Some("bw")).unwrap();
        cat.commit(img, &exp(2.0), None).unwrap();
        cat.history_open(other).unwrap();
        cat.commit(other, &exp(1.0), None).unwrap(); // shares a state with `img`
        cat.set_aspect(img, 1.0, false).unwrap();

        cat.reset_history(&[img]).unwrap();

        let count = |sql: &str| -> i64 { cat.conn.query_row(sql, [img], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT COUNT(*) FROM commits WHERE image_id = ?1"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM refs WHERE image_id = ?1"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM heads WHERE image_id = ?1"), 0);
        assert_eq!(
            count("SELECT COUNT(*) FROM images WHERE id = ?1 AND aspect IS NULL"),
            1
        );
        // exp(2) was only `img`'s: gone. exp(1) and the default are still used by `other`.
        let states: i64 = cat
            .conn
            .query_row("SELECT COUNT(*) FROM edit_states", [], |r| r.get(0))
            .unwrap();
        assert_eq!(states, 2);
        assert_eq!(cat.history_view(other).unwrap().commits.len(), 2);

        // Opening it again starts over from as-shot, on a single `main`.
        let c = cat.history_open(img).unwrap();
        assert_eq!(summaries(&c), ["Original"]);
        assert_eq!(c.view.branches.len(), 1);
        assert_eq!(c.state, EditState::default());
    }

    #[test]
    fn applies_panels_from_another_branch() {
        let (mut cat, _, img) = catalog();
        let root = cat.history_open(img).unwrap().view.head.commit;
        // main: exposure +1. bw: saturation -100, contrast +20.
        cat.commit(img, &exp(1.0), None).unwrap();
        cat.create_branch(img, root, Some("bw")).unwrap();
        let mut bw = EditState::default();
        bw.color.saturation = -100.0;
        bw.light.contrast = 20.0;
        cat.commit(img, &bw, None).unwrap();
        cat.switch_branch(img, "main").unwrap();

        let c = cat.apply_from_branch(img, "bw", &[Panel::Color]).unwrap();
        assert_eq!(c.view.head.branch, "main");
        assert_eq!(c.state.color.saturation, -100.0);
        assert_eq!(c.state.light.exposure, 1.0, "Light was not copied");
        assert_eq!(c.state.light.contrast, 0.0);
        assert_eq!(summaries(&c).last(), Some(&"Color from bw"));

        let c = cat.apply_from_branch(img, "bw", &Panel::ALL).unwrap();
        assert_eq!(c.state, bw);
        assert_eq!(summaries(&c).last(), Some(&"All settings from bw"));
        assert!(cat.apply_from_branch(img, "main", &[Panel::Light]).is_err());
        assert!(cat.apply_from_branch(img, "bw", &[]).is_err());
    }

    #[test]
    fn opens_with_original_on_main() {
        let (mut cat, _, img) = catalog();
        let c = cat.history_open(img).unwrap();
        assert_eq!(summaries(&c), ["Original"]);
        assert_eq!(c.view.head.branch, "main");
        assert_eq!(c.state, EditState::default());
        assert!(!c.view.can_undo && !c.view.can_redo);
        // Idempotent.
        assert_eq!(cat.history_open(img).unwrap(), c);
    }

    #[test]
    fn commit_undo_redo() {
        let (mut cat, t, img) = catalog();
        cat.history_open(img).unwrap();
        cat.commit(img, &exp(1.0), Some("exposure")).unwrap();
        t.fetch_add(5000, Ordering::SeqCst);
        let c = cat.commit(img, &exp(1.5), Some("exposure")).unwrap();
        assert_eq!(
            summaries(&c),
            ["Original", "Exposure +1.00", "Exposure +1.50"]
        );
        let c = cat.undo(img).unwrap();
        assert_eq!(c.state, exp(1.0));
        assert!(c.view.can_redo);
        let c = cat.redo(img).unwrap();
        assert_eq!(c.state, exp(1.5));
        assert!(!c.view.can_redo);
        // Committing an unchanged state is a no-op.
        assert_eq!(cat.commit(img, &exp(1.5), None).unwrap(), c);
    }

    #[test]
    fn same_control_coalesces_within_window() {
        let (mut cat, t, img) = catalog();
        cat.history_open(img).unwrap();
        cat.commit(img, &exp(0.1), Some("exposure")).unwrap();
        t.fetch_add(300, Ordering::SeqCst);
        cat.commit(img, &exp(0.2), Some("exposure")).unwrap();
        t.fetch_add(300, Ordering::SeqCst);
        let c = cat.commit(img, &exp(0.3), Some("exposure")).unwrap();
        assert_eq!(summaries(&c), ["Original", "Exposure +0.30"]);
        // Different control: new commit.
        let mut s = exp(0.3);
        s.light.contrast = 10.0;
        let c = cat.commit(img, &s, Some("contrast")).unwrap();
        assert_eq!(c.view.commits.len(), 3);
        // Nudging back to the parent state drops the coalesced commit.
        t.fetch_add(100, Ordering::SeqCst);
        let c = cat.commit(img, &exp(0.3), Some("contrast")).unwrap();
        assert_eq!(summaries(&c), ["Original", "Exposure +0.30"]);
    }

    #[test]
    fn editing_the_past_forks_a_branch() {
        let (mut cat, t, img) = catalog();
        cat.history_open(img).unwrap();
        cat.commit(img, &exp(1.0), Some("exposure")).unwrap();
        t.fetch_add(5000, Ordering::SeqCst);
        let main_tip = cat
            .commit(img, &exp(2.0), Some("exposure"))
            .unwrap()
            .view
            .head
            .commit;
        let c = cat.undo(img).unwrap();
        let fork_point = c.view.head.commit;
        let c = cat.commit(img, &exp(-1.0), Some("exposure")).unwrap();
        assert_eq!(c.view.head.branch, "main-2");
        assert_eq!(c.view.commits.last().unwrap().parent, Some(fork_point));
        let main = c.view.branches.iter().find(|b| b.name == "main").unwrap();
        assert_eq!(main.tip, main_tip, "main is untouched");
        // Forking from main-2 continues the numbering rather than nesting suffixes.
        let c = cat.undo(img).unwrap();
        let c2 = cat.commit(img, &exp(3.0), None).unwrap();
        assert_eq!(c2.view.head.branch, "main-3");
        assert_eq!(c.view.head.branch, "main-2");
    }

    #[test]
    fn checkout_prefers_current_branch_then_tip_owner() {
        let (mut cat, _, img) = catalog();
        let root = cat.history_open(img).unwrap().view.head.commit;
        let a = cat.commit(img, &exp(1.0), None).unwrap().view.head.commit;
        let c = cat.create_branch(img, root, Some("B&W")).unwrap();
        assert_eq!(
            c.view.head,
            Head {
                branch: "B&W".into(),
                commit: root
            }
        );
        let b = cat.commit(img, &exp(-1.0), None).unwrap().view.head.commit;
        // Root is on B&W: stay on B&W.
        assert_eq!(cat.checkout(img, root).unwrap().view.head.branch, "B&W");
        // `a` is only on main.
        assert_eq!(cat.checkout(img, a).unwrap().view.head.branch, "main");
        assert_eq!(cat.checkout(img, b).unwrap().view.head.branch, "B&W");
    }

    #[test]
    fn rename_and_delete() {
        let (mut cat, _, img) = catalog();
        let root = cat.history_open(img).unwrap().view.head.commit;
        cat.create_branch(img, root, Some("HDR")).unwrap();
        assert!(cat.rename_branch(img, "HDR", "main").is_err());
        assert!(cat.rename_branch(img, "HDR", "  ").is_err());
        let c = cat.rename_branch(img, "HDR", " Creative ").unwrap();
        assert_eq!(c.view.head.branch, "Creative");
        cat.commit(img, &exp(1.0), None).unwrap();
        let c = cat.delete_branch(img, "Creative").unwrap();
        assert_eq!(c.view.head.branch, "main");
        assert_eq!(
            summaries(&c),
            ["Original"],
            "commits only on the deleted branch are gone"
        );
        assert!(cat.delete_branch(img, "main").is_err());
        let rows: i64 = cat
            .conn
            .query_row(
                "SELECT COUNT(*) FROM commits WHERE image_id = ?1",
                [img],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1, "and deleted, not just hidden");
        let states: i64 = cat
            .conn
            .query_row("SELECT COUNT(*) FROM edit_states", [], |r| r.get(0))
            .unwrap();
        assert_eq!(states, 1);
        // Still fully usable.
        cat.commit(img, &exp(2.0), None).unwrap();
    }

    #[test]
    fn auto_names() {
        let b = |n: &[&str]| {
            n.iter()
                .map(|n| BranchInfo {
                    name: n.to_string(),
                    tip: 0,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(auto_branch_name(&b(&["main"]), "main"), "main-2");
        assert_eq!(
            auto_branch_name(&b(&["main", "main-2"]), "main-2"),
            "main-3"
        );
        assert_eq!(auto_branch_name(&b(&["B&W"]), "B&W"), "B&W-2");
        assert_eq!(auto_branch_name(&b(&["-5"]), "-5"), "-5-2");
    }

    // -----------------------------------------------------------------------------------------
    // Property tests: random operation sequences preserve the graph invariants.
    // -----------------------------------------------------------------------------------------

    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Op {
        Commit {
            exposure: u8,
            key: Option<u8>,
            dt: i64,
        },
        Undo,
        Redo,
        Checkout(usize),
        Switch(usize),
        Branch(usize),
        Rename(usize, u8),
        Delete(usize),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            4 => (0u8..6, proptest::option::of(0u8..2), prop_oneof![Just(0i64), Just(200), Just(5000)])
                .prop_map(|(exposure, key, dt)| Op::Commit { exposure, key, dt }),
            2 => Just(Op::Undo),
            1 => Just(Op::Redo),
            2 => any::<usize>().prop_map(Op::Checkout),
            1 => any::<usize>().prop_map(Op::Switch),
            1 => any::<usize>().prop_map(Op::Branch),
            1 => (any::<usize>(), 0u8..4).prop_map(|(i, n)| Op::Rename(i, n)),
            1 => any::<usize>().prop_map(Op::Delete),
        ]
    }

    fn check_invariants(cat: &Catalog, v: &HistoryView) {
        let ids: HashSet<CommitId> = v.commits.iter().map(|c| c.id).collect();
        // Parents precede children and are visible.
        let mut seen = HashSet::new();
        for c in &v.commits {
            if let Some(p) = c.parent {
                assert!(seen.contains(&p), "parent {p} of {} not before it", c.id);
            }
            seen.insert(c.id);
        }
        assert_eq!(
            v.commits.iter().filter(|c| c.parent.is_none()).count(),
            1,
            "single root"
        );
        // Refs resolve; names unique.
        let names: HashSet<&str> = v.branches.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names.len(), v.branches.len());
        assert!(!v.branches.is_empty());
        for b in &v.branches {
            assert!(ids.contains(&b.tip));
        }
        // HEAD is on its branch.
        let b = v
            .branches
            .iter()
            .find(|b| b.name == v.head.branch)
            .expect("head branch exists");
        let parents = parents(&cat.conn, v.image_id).unwrap();
        assert!(
            lineage(&parents, b.tip).contains(&v.head.commit),
            "head commit on head branch"
        );
        assert_eq!(v.can_redo, b.tip != v.head.commit);
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 300, ..ProptestConfig::default() })]
        #[test]
        fn random_histories_keep_invariants(ops in proptest::collection::vec(op(), 1..40)) {
            let (mut cat, t, img) = catalog();
            let mut cur = cat.history_open(img).unwrap();
            for op in ops {
                let before = cur.view.clone();
                let visible_before: HashSet<CommitId> = before.commits.iter().map(|c| c.id).collect();
                let pick = |i: usize, n: usize| i % n.max(1);
                let next = match &op {
                    Op::Commit { exposure, key, dt } => {
                        t.fetch_add(*dt, Ordering::SeqCst);
                        let state = exp(*exposure as f32);
                        let key = key.map(|k| format!("k{k}"));
                        let r = cat.commit(img, &state, key.as_deref()).unwrap();
                        let at_tip = before.branches.iter().any(|b| b.name == before.head.branch && b.tip == before.head.commit);
                        if cur.state != state && r.state == state {
                            if at_tip {
                                prop_assert_eq!(&r.view.head.branch, &before.head.branch);
                            } else {
                                // Forked: every pre-existing branch keeps its tip.
                                prop_assert_ne!(&r.view.head.branch, &before.head.branch);
                                for b in &before.branches {
                                    prop_assert!(r.view.branches.contains(b));
                                }
                            }
                        }
                        // Only a coalesced tip commit may disappear.
                        let now: HashSet<CommitId> = r.view.commits.iter().map(|c| c.id).collect();
                        let lost: Vec<_> = visible_before.difference(&now).collect();
                        prop_assert!(lost.is_empty() || (lost == [&before.head.commit] && at_tip), "lost {:?}", lost);
                        // The state at HEAD is what was committed, unless the commit vanished
                        // because the coalesced gesture returned to the parent state (same thing).
                        prop_assert_eq!(&r.state, &state);
                        r
                    }
                    Op::Undo => cat.undo(img).unwrap(),
                    Op::Redo => cat.redo(img).unwrap(),
                    Op::Checkout(i) => {
                        let id = before.commits[pick(*i, before.commits.len())].id;
                        let r = cat.checkout(img, id).unwrap();
                        prop_assert_eq!(r.view.head.commit, id);
                        r
                    }
                    Op::Switch(i) => {
                        let b = &before.branches[pick(*i, before.branches.len())];
                        cat.switch_branch(img, &b.name).unwrap()
                    }
                    Op::Branch(i) => {
                        let id = before.commits[pick(*i, before.commits.len())].id;
                        cat.create_branch(img, id, None).unwrap()
                    }
                    Op::Rename(i, n) => {
                        let b = &before.branches[pick(*i, before.branches.len())];
                        match cat.rename_branch(img, &b.name, &format!("name{n}")) {
                            Ok(r) => r,
                            Err(CatalogError::Invalid(_)) => cat.history_open(img).unwrap(),
                            Err(e) => panic!("{e}"),
                        }
                    }
                    Op::Delete(i) => {
                        let b = &before.branches[pick(*i, before.branches.len())];
                        match cat.delete_branch(img, &b.name) {
                            Ok(r) => r,
                            Err(CatalogError::Invalid(_)) => {
                                prop_assert_eq!(before.branches.len(), 1);
                                cat.history_open(img).unwrap()
                            }
                            Err(e) => panic!("{e}"),
                        }
                    }
                };
                check_invariants(&cat, &next.view);
                // Nothing but commit/delete changes the set of visible commits.
                if !matches!(op, Op::Commit { .. } | Op::Delete(_)) {
                    let now: HashSet<CommitId> = next.view.commits.iter().map(|c| c.id).collect();
                    prop_assert_eq!(&now, &visible_before);
                }
                // HEAD state is always the stored state of the HEAD commit.
                prop_assert_eq!(&next.state, &cat.state_at(next.view.head.commit).unwrap());
                cur = next;
            }
        }
    }
}
