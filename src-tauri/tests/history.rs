//! Edit history: steps, undo and redo, and branching.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use image::RgbImage;
use tempfile::TempDir;
use tonality_lib::edit::Adjustments;
use tonality_lib::history::History;
use tonality_lib::import;
use tonality_lib::library::{Library, View};

struct Fixture {
    _dir: TempDir,
    library: Library,
    photo: i64,
}

/// A library holding one photo.
fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let library = Library::open(&dir.path().join("Tonality")).unwrap();
    let source = dir.path().join("in/a.png");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    RgbImage::new(8, 8).save(&source).unwrap();
    import_into(&library, source.parent().unwrap());
    let photo = library.list_photos(View::Library).unwrap()[0].id;
    Fixture { _dir: dir, library, photo }
}

fn import_into(library: &Library, folder: &Path) {
    let session = import::scan(library, 1, &[folder.to_path_buf()], None, &|_, _| {}).unwrap();
    let all: Vec<usize> = (0..session.items.len()).collect();
    import::run(library, &session, &all, &AtomicBool::new(false), &|_, _| {}).unwrap();
}

fn exposure(stops: f32) -> Adjustments {
    Adjustments { exposure: stops, ..Default::default() }
}

fn labels(history: &History) -> Vec<&str> {
    history.steps.iter().map(|step| step.label.as_str()).collect()
}

impl Fixture {
    fn commit(&self, stops: f32) -> History {
        self.library.history_commit(self.photo, &exposure(stops), &format!("Exposure {stops}")).unwrap()
    }

    /// What the grid would show for the photo.
    fn shown(&self) -> Adjustments {
        Adjustments::from_json(self.library.edits(self.photo).unwrap().as_deref())
    }
}

#[test]
fn a_photo_starts_with_its_original_on_a_main_branch() {
    let f = fixture();
    let history = f.library.history(f.photo).unwrap();
    assert_eq!(labels(&history), ["Original"]);
    assert_eq!(history.head_id, history.steps[0].id);
    assert_eq!(history.branches.len(), 1);
    assert_eq!(history.branches[0].name, "Main");
    assert!(history.adjustments.is_default());
    assert_eq!(f.library.list_photos(View::Library).unwrap()[0].branches, 1);
}

#[test]
fn steps_accumulate_and_undo_and_redo_move_along_them() {
    let f = fixture();
    f.commit(0.5);
    let history = f.commit(1.0);
    assert_eq!(labels(&history), ["Original", "Exposure 0.5", "Exposure 1"]);
    assert_eq!(history.head_id, history.steps[2].id);
    assert_eq!(f.shown(), exposure(1.0));

    // Undo: back one step. Nothing is removed, so redo can return.
    let undone = f.library.history_goto(f.photo, history.steps[1].id).unwrap();
    assert_eq!(undone.steps.len(), 3);
    assert_eq!(undone.head_id, history.steps[1].id);
    assert_eq!(undone.adjustments, exposure(0.5));
    assert_eq!(f.shown(), exposure(0.5), "the grid follows the head");

    let redone = f.library.history_goto(f.photo, history.steps[2].id).unwrap();
    assert_eq!(redone.adjustments, exposure(1.0));

    // All the way back is the photo as shot, which stores no recipe at all.
    f.library.history_goto(f.photo, history.steps[0].id).unwrap();
    assert_eq!(f.library.edits(f.photo).unwrap(), None);
}

#[test]
fn editing_after_an_undo_replaces_the_undone_steps() {
    let f = fixture();
    f.commit(0.5);
    let history = f.commit(1.0);
    f.library.history_goto(f.photo, history.steps[1].id).unwrap();

    let history = f.commit(-1.0);
    assert_eq!(labels(&history), ["Original", "Exposure 0.5", "Exposure -1"]);
    assert_eq!(f.shown(), exposure(-1.0));
}

#[test]
fn a_branch_keeps_both_directions() {
    let f = fixture();
    let first = f.commit(0.5);
    let main = f.commit(1.0);
    let main_branch = main.branch_id;

    // Branch off from the first edit and go a different way.
    let fork = f.library.history_branch(f.photo, first.head_id, "  Moody  ").unwrap();
    assert_eq!(fork.branches.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Main", "Moody"]);
    assert_ne!(fork.branch_id, main_branch, "the new branch is checked out");
    assert_eq!(labels(&fork), ["Original", "Exposure 0.5"]);
    assert_eq!(fork.adjustments, exposure(0.5));

    let moody = f.commit(-2.0);
    assert_eq!(labels(&moody), ["Original", "Exposure 0.5", "Exposure -2"]);
    assert_eq!(moody.steps[1].forks, ["Main"], "Main parts ways after the shared step");
    assert!(moody.steps[0].forks.is_empty() && moody.steps[2].forks.is_empty());
    assert_eq!(f.shown(), exposure(-2.0));
    assert_eq!(f.library.list_photos(View::Library).unwrap()[0].branches, 2);

    // Main is exactly as it was left.
    let back = f.library.history_switch(f.photo, main_branch).unwrap();
    assert_eq!(labels(&back), ["Original", "Exposure 0.5", "Exposure 1"]);
    assert_eq!(back.steps[1].forks, ["Moody"]);
    assert_eq!(f.shown(), exposure(1.0));
}

#[test]
fn undone_steps_survive_if_another_branch_still_needs_them() {
    let f = fixture();
    f.commit(0.5);
    let main = f.commit(1.0);
    // Branch from the tip, then switch back to Main, undo and edit: Main drops
    // "Exposure 1", but the other branch still stands on it.
    let fork = f.library.history_branch(f.photo, main.head_id, "Keep").unwrap();
    f.library.history_switch(f.photo, main.branch_id).unwrap();
    f.library.history_goto(f.photo, main.steps[1].id).unwrap();
    let rewritten = f.commit(3.0);
    assert_eq!(labels(&rewritten), ["Original", "Exposure 0.5", "Exposure 3"]);

    let kept = f.library.history_switch(f.photo, fork.branch_id).unwrap();
    assert_eq!(labels(&kept), ["Original", "Exposure 0.5", "Exposure 1"]);
    assert_eq!(kept.adjustments, exposure(1.0));
}

#[test]
fn each_branch_remembers_where_you_were_on_it() {
    let f = fixture();
    f.commit(0.5);
    let main = f.commit(1.0);
    f.library.history_goto(f.photo, main.steps[1].id).unwrap();
    let fork = f.library.history_branch(f.photo, main.steps[0].id, "").unwrap();
    assert_eq!(fork.branches[1].name, "Branch 2", "unnamed branches are numbered");
    assert!(fork.adjustments.is_default());

    let back = f.library.history_switch(f.photo, main.branch_id).unwrap();
    assert_eq!(back.head_id, main.steps[1].id, "still one step undone");
    assert_eq!(back.steps.len(), 3, "and the undone step is still there to redo");
}

#[test]
fn a_new_unnamed_branch_never_takes_a_name_already_in_use() {
    fn names(history: &History) -> Vec<&str> {
        history.branches.iter().map(|branch| branch.name.as_str()).collect()
    }
    let f = fixture();
    let start = f.commit(0.5).head_id;
    let two = f.library.history_branch(f.photo, start, "").unwrap();
    let three = f.library.history_branch(f.photo, start, "").unwrap();
    assert_eq!(names(&three), ["Main", "Branch 2", "Branch 3"]);

    f.library.history_delete_branch(f.photo, two.branch_id).unwrap();
    let four = f.library.history_branch(f.photo, start, "").unwrap();
    assert_eq!(names(&four), ["Main", "Branch 3", "Branch 4"], "not a second Branch 3");

    // A branch renamed to look like a numbered one is counted too.
    let g = fixture();
    let main = g.commit(0.5);
    g.library.history_rename_branch(g.photo, main.branch_id, "Branch 2").unwrap();
    let next = g.library.history_branch(g.photo, main.head_id, "").unwrap();
    assert_eq!(names(&next), ["Branch 2", "Branch 3"]);
}

#[test]
fn branches_can_be_renamed_and_deleted_but_one_always_remains() {
    let f = fixture();
    let main = f.commit(0.5);
    let fork = f.library.history_branch(f.photo, main.head_id, "Draft").unwrap();
    f.commit(2.0);

    let renamed = f.library.history_rename_branch(f.photo, fork.branch_id, "Final").unwrap();
    assert_eq!(renamed.branches[1].name, "Final");
    assert_eq!(f.library.history_rename_branch(f.photo, fork.branch_id, "   ").unwrap().branches[1].name, "Final");

    // Deleting the branch you are on moves you to another one.
    let left = f.library.history_delete_branch(f.photo, fork.branch_id).unwrap();
    assert_eq!(left.branch_id, main.branch_id);
    assert_eq!(left.branches.len(), 1);
    assert_eq!(labels(&left), ["Original", "Exposure 0.5"]);
    assert_eq!(f.shown(), exposure(0.5));

    assert!(f.library.history_delete_branch(f.photo, main.branch_id).is_err());
    assert_eq!(f.library.history(f.photo).unwrap().branches.len(), 1);
}

#[test]
fn history_cannot_reach_into_other_branches_or_photos() {
    let f = fixture();
    let main = f.commit(0.5);
    let fork = f.library.history_branch(f.photo, main.steps[0].id, "Other").unwrap();
    let other_step = f.commit(2.0).head_id;
    f.library.history_switch(f.photo, main.branch_id).unwrap();
    assert!(f.library.history_goto(f.photo, other_step).is_err(), "a step from another branch");
    assert!(f.library.history_goto(f.photo, 9999).is_err());
    assert!(f.library.history_switch(f.photo, 9999).is_err());
    assert!(f.library.history_branch(f.photo, 9999, "x").is_err());
    assert!(f.library.history(9999).is_err(), "an unknown photo");
    let _ = fork;
}

#[test]
fn deleting_a_photo_takes_its_history_with_it() {
    let f = fixture();
    let main = f.commit(0.5);
    f.library.history_branch(f.photo, main.head_id, "B").unwrap();
    f.library.trash(&[f.photo]).unwrap();
    assert_eq!(f.library.purge(&[f.photo]).unwrap(), 1);
    assert!(f.library.history(f.photo).is_err());
}
