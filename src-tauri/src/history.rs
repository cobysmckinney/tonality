//! Edit history, shaped like a small git repository per photo.
//!
//! Every change is a *step* that points at the step before it, so the steps
//! form a tree. A *branch* is a named line through that tree: it remembers
//! its newest step (the tip) and where you currently are on it (the head,
//! which undo moves back from the tip). Each photo has one current branch,
//! and that branch's head is what the library shows.

use std::collections::HashSet;

use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::edit::Adjustments;
use crate::library::Library;

pub(crate) const SCHEMA: &str = "
CREATE TABLE edit_steps (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id    INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    -- NULL only for the first step, the photo as shot.
    parent_id   INTEGER REFERENCES edit_steps(id),
    -- What changed, in words: 'Exposure +0.50'.
    label       TEXT NOT NULL,
    -- The whole recipe after this step; NULL means as shot.
    recipe      TEXT,
    created_at  INTEGER NOT NULL
);
CREATE INDEX edit_steps_photo ON edit_steps(photo_id);

CREATE TABLE edit_branches (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id    INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    tip_id      INTEGER NOT NULL REFERENCES edit_steps(id),
    -- An ancestor of the tip, or the tip itself.
    head_id     INTEGER NOT NULL REFERENCES edit_steps(id),
    created_at  INTEGER NOT NULL
);
CREATE INDEX edit_branches_photo ON edit_branches(photo_id);

-- The photo's current branch; NULL until it is first opened for editing.
ALTER TABLE photos ADD COLUMN branch_id INTEGER;
";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: i64,
    pub label: String,
    pub created_at: i64,
    /// Other branches that part ways with the current one right after this step.
    pub forks: Vec<String>,
    /// The files exported from exactly this step, oldest first.
    pub exports: Vec<ExportMark>,
}

/// A file an export wrote. See export.rs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMark {
    pub path: String,
    pub created_at: i64,
}

/// A photo's history as seen from its current branch.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub branches: Vec<Branch>,
    pub branch_id: i64,
    /// The current branch from the original to its tip, oldest first.
    pub steps: Vec<Step>,
    /// Where you are on the branch; steps after it have been undone.
    pub head_id: i64,
    /// The recipe at the head.
    pub adjustments: Adjustments,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

struct Current {
    branch_id: i64,
    tip_id: i64,
    head_id: i64,
}

fn insert_step(db: &Connection, photo_id: i64, parent: Option<i64>, label: &str, recipe: Option<&str>) -> Result<i64> {
    db.execute(
        "INSERT INTO edit_steps (photo_id, parent_id, label, recipe, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![photo_id, parent, label, recipe, now()],
    )?;
    Ok(db.last_insert_rowid())
}

fn insert_branch(db: &Connection, photo_id: i64, name: &str, step: i64) -> Result<i64> {
    db.execute(
        "INSERT INTO edit_branches (photo_id, name, tip_id, head_id, created_at) VALUES (?1, ?2, ?3, ?3, ?4)",
        params![photo_id, name, step, now()],
    )?;
    Ok(db.last_insert_rowid())
}

/// The photo's current branch, starting its history if it has none yet.
fn current(db: &Connection, photo_id: i64) -> Result<Current> {
    let (branch, edits): (Option<i64>, Option<String>) = db
        .query_row("SELECT branch_id, edits FROM photos WHERE id = ?1", [photo_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .with_context(|| format!("photo {photo_id} is not in the library"))?;
    let branch_id = match branch {
        Some(branch_id) => branch_id,
        None => {
            let mut step = insert_step(db, photo_id, None, "Original", None)?;
            // Photos edited before history existed keep their edit as one step.
            if edits.is_some() {
                step = insert_step(db, photo_id, Some(step), "Earlier edits", edits.as_deref())?;
            }
            let branch_id = insert_branch(db, photo_id, "Main", step)?;
            db.execute("UPDATE photos SET branch_id = ?2 WHERE id = ?1", params![photo_id, branch_id])?;
            branch_id
        }
    };
    let (tip_id, head_id) = db.query_row(
        "SELECT tip_id, head_id FROM edit_branches WHERE id = ?1",
        [branch_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(Current { branch_id, tip_id, head_id })
}

/// The steps from the original up to `tip`, oldest first.
fn path(db: &Connection, tip: i64) -> Result<Vec<Step>> {
    let mut stmt = db.prepare_cached(
        "WITH RECURSIVE up(id, parent_id, label, created_at, depth) AS (
             SELECT id, parent_id, label, created_at, 0 FROM edit_steps WHERE id = ?1
             UNION ALL
             SELECT s.id, s.parent_id, s.label, s.created_at, up.depth + 1
             FROM edit_steps s JOIN up ON s.id = up.parent_id
         )
         SELECT id, label, created_at FROM up ORDER BY depth DESC",
    )?;
    let steps = stmt
        .query_map([tip], |r| {
            Ok(Step { id: r.get(0)?, label: r.get(1)?, created_at: r.get(2)?, forks: Vec::new(), exports: Vec::new() })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(steps)
}

/// Makes the library show the current branch's head.
fn show_head(db: &Connection, photo_id: i64) -> Result<()> {
    db.execute(
        "UPDATE photos SET edits = (
             SELECT s.recipe FROM edit_branches b JOIN edit_steps s ON s.id = b.head_id WHERE b.id = photos.branch_id
         ) WHERE id = ?1",
        [photo_id],
    )?;
    Ok(())
}

/// Deletes steps that no branch leads to any more.
fn forget_unreachable(db: &Connection, photo_id: i64) -> Result<()> {
    db.execute(
        "DELETE FROM edit_steps WHERE photo_id = ?1 AND id NOT IN (
             WITH RECURSIVE keep(id) AS (
                 SELECT tip_id FROM edit_branches WHERE photo_id = ?1
                 UNION
                 SELECT s.parent_id FROM edit_steps s JOIN keep ON s.id = keep.id WHERE s.parent_id IS NOT NULL
             )
             SELECT id FROM keep
         )",
        [photo_id],
    )?;
    Ok(())
}

fn read(db: &Connection, photo_id: i64) -> Result<History> {
    let now = current(db, photo_id)?;
    let mut steps = path(db, now.tip_id)?;
    let branches: Vec<(Branch, i64)> = db
        .prepare("SELECT id, name, tip_id FROM edit_branches WHERE photo_id = ?1 ORDER BY created_at, id")?
        .query_map([photo_id], |r| Ok((Branch { id: r.get(0)?, name: r.get(1)?, }, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;

    // Mark where each other branch leaves this one: the last step they share.
    let on_this_branch: HashSet<i64> = steps.iter().map(|step| step.id).collect();
    for (branch, tip) in branches.iter().filter(|(branch, _)| branch.id != now.branch_id) {
        let shared = path(db, *tip)?.into_iter().rev().find(|step| on_this_branch.contains(&step.id));
        if let Some(step) = shared.and_then(|shared| steps.iter_mut().find(|step| step.id == shared.id)) {
            step.forks.push(branch.name.clone());
        }
    }

    let exported: Vec<(i64, ExportMark)> = db
        .prepare("SELECT step_id, path, created_at FROM exports WHERE photo_id = ?1 ORDER BY created_at, id")?
        .query_map([photo_id], |r| Ok((r.get(0)?, ExportMark { path: r.get(1)?, created_at: r.get(2)? })))?
        .collect::<rusqlite::Result<_>>()?;
    for (step_id, mark) in exported {
        if let Some(step) = steps.iter_mut().find(|step| step.id == step_id) {
            step.exports.push(mark);
        }
    }

    let recipe: Option<String> =
        db.query_row("SELECT recipe FROM edit_steps WHERE id = ?1", [now.head_id], |r| r.get(0))?;
    Ok(History {
        branches: branches.into_iter().map(|(branch, _)| branch).collect(),
        branch_id: now.branch_id,
        steps,
        head_id: now.head_id,
        adjustments: Adjustments::from_json(recipe.as_deref()),
    })
}

fn tidy_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The name for a new unnamed branch: one past the highest "Branch N" the
/// photo has, starting at "Branch 2" (the first branch is "Main"). Counting
/// on from the highest, rather than filling a gap left by a deleted branch,
/// keeps the numbers in the order the branches were made, which is the order
/// the picker lists them in.
fn next_branch_name(db: &Connection, photo_id: i64) -> Result<String> {
    let names = db
        .prepare("SELECT name FROM edit_branches WHERE photo_id = ?1")?
        .query_map([photo_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let highest = names
        .iter()
        .filter_map(|name| name.strip_prefix("Branch ")?.parse::<u32>().ok())
        .max()
        .unwrap_or(1)
        .max(1);
    Ok(format!("Branch {}", u64::from(highest) + 1))
}

impl Library {
    /// Runs one history change as a single transaction and returns the result.
    fn change_history(&self, photo_id: i64, change: impl FnOnce(&Connection, &Current) -> Result<()>) -> Result<History> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let now = current(&tx, photo_id)?;
        change(&tx, &now)?;
        forget_unreachable(&tx, photo_id)?;
        show_head(&tx, photo_id)?;
        let history = read(&tx, photo_id)?;
        tx.commit()?;
        Ok(history)
    }

    /// The photo's history, started with an "Original" step if it had none.
    pub fn history(&self, photo_id: i64) -> Result<History> {
        self.change_history(photo_id, |_, _| Ok(()))
    }

    /// Records a new step on the current branch. Steps that had been undone
    /// are replaced by it, unless another branch still leads to them.
    pub fn history_commit(&self, photo_id: i64, adjustments: &Adjustments, label: &str) -> Result<History> {
        self.change_history(photo_id, |db, now| {
            let step = insert_step(db, photo_id, Some(now.head_id), label, adjustments.to_json().as_deref())?;
            db.execute("UPDATE edit_branches SET tip_id = ?2, head_id = ?2 WHERE id = ?1", params![now.branch_id, step])?;
            Ok(())
        })
    }

    /// Moves to another step of the current branch. Undo, redo and clicking a
    /// step in the list are all this.
    pub fn history_goto(&self, photo_id: i64, step_id: i64) -> Result<History> {
        self.change_history(photo_id, |db, now| {
            ensure!(path(db, now.tip_id)?.iter().any(|step| step.id == step_id), "that step is not on this branch");
            db.execute("UPDATE edit_branches SET head_id = ?2 WHERE id = ?1", params![now.branch_id, step_id])?;
            Ok(())
        })
    }

    /// Starts a new branch at `step_id` and switches to it. The branch you
    /// were on keeps everything it had.
    pub fn history_branch(&self, photo_id: i64, step_id: i64, name: &str) -> Result<History> {
        self.change_history(photo_id, |db, _| {
            let owner: Option<i64> =
                db.query_row("SELECT photo_id FROM edit_steps WHERE id = ?1", [step_id], |r| r.get(0)).optional()?;
            ensure!(owner == Some(photo_id), "that step belongs to another photo");
            let mut name = tidy_name(name);
            if name.is_empty() {
                name = next_branch_name(db, photo_id)?;
            }
            let branch = insert_branch(db, photo_id, &name, step_id)?;
            db.execute("UPDATE photos SET branch_id = ?2 WHERE id = ?1", params![photo_id, branch])?;
            Ok(())
        })
    }

    pub fn history_switch(&self, photo_id: i64, branch_id: i64) -> Result<History> {
        self.change_history(photo_id, |db, _| {
            let changed = db.execute(
                "UPDATE photos SET branch_id = ?2 WHERE id = ?1
                 AND EXISTS (SELECT 1 FROM edit_branches WHERE id = ?2 AND photo_id = ?1)",
                params![photo_id, branch_id],
            )?;
            ensure!(changed == 1, "that branch belongs to another photo");
            Ok(())
        })
    }

    pub fn history_rename_branch(&self, photo_id: i64, branch_id: i64, name: &str) -> Result<History> {
        self.change_history(photo_id, |db, _| {
            let name = tidy_name(name);
            if !name.is_empty() {
                db.execute(
                    "UPDATE edit_branches SET name = ?3 WHERE id = ?2 AND photo_id = ?1",
                    params![photo_id, branch_id, name],
                )?;
            }
            Ok(())
        })
    }

    /// Deletes a branch and the steps only it led to. A photo always keeps
    /// one branch; deleting the current one moves you to the oldest other.
    pub fn history_delete_branch(&self, photo_id: i64, branch_id: i64) -> Result<History> {
        self.change_history(photo_id, |db, now| {
            let other: Option<i64> = db
                .query_row(
                    "SELECT id FROM edit_branches WHERE photo_id = ?1 AND id != ?2 ORDER BY created_at, id LIMIT 1",
                    params![photo_id, branch_id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(other) = other else { bail!("a photo keeps at least one branch") };
            if now.branch_id == branch_id {
                db.execute("UPDATE photos SET branch_id = ?2 WHERE id = ?1", params![photo_id, other])?;
            }
            db.execute("DELETE FROM edit_branches WHERE id = ?2 AND photo_id = ?1", params![photo_id, branch_id])?;
            Ok(())
        })
    }
}
