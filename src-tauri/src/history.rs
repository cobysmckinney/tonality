//! History commands: thin wrappers over `tonality_core::history` on the shared catalog.

use std::sync::Mutex;

use tauri::State;
use tonality_core::{Catalog, Checkout, CommitId, EditState, ImageId, Panel};

use crate::CmdResult;

pub struct CatalogState(Mutex<Catalog>);

impl CatalogState {
    pub fn new(catalog: Catalog) -> Self {
        Self(Mutex::new(catalog))
    }

    pub(crate) fn with<T>(&self, f: impl FnOnce(&mut Catalog) -> tonality_core::catalog::Result<T>) -> CmdResult<T> {
        let mut cat = self.0.lock().map_err(|_| "catalog lock poisoned")?;
        f(&mut cat).map_err(|e| e.to_string())
    }
}

/// An image's history, created with an "Original" commit on first open.
#[tauri::command]
pub fn history_open(image_id: ImageId, catalog: State<'_, CatalogState>) -> CmdResult<Checkout> {
    catalog.with(|c| c.history_open(image_id))
}

#[tauri::command]
pub fn history_commit(
    image_id: ImageId,
    state: EditState,
    key: Option<String>,
    catalog: State<'_, CatalogState>,
) -> CmdResult<Checkout> {
    catalog.with(|c| c.commit(image_id, &state, key.as_deref()))
}

#[tauri::command]
pub fn history_undo(image_id: ImageId, catalog: State<'_, CatalogState>) -> CmdResult<Checkout> {
    catalog.with(|c| c.undo(image_id))
}

#[tauri::command]
pub fn history_redo(image_id: ImageId, catalog: State<'_, CatalogState>) -> CmdResult<Checkout> {
    catalog.with(|c| c.redo(image_id))
}

#[tauri::command]
pub fn history_checkout(
    image_id: ImageId,
    commit: CommitId,
    catalog: State<'_, CatalogState>,
) -> CmdResult<Checkout> {
    catalog.with(|c| c.checkout(image_id, commit))
}

#[tauri::command]
pub fn history_switch(image_id: ImageId, name: String, catalog: State<'_, CatalogState>) -> CmdResult<Checkout> {
    catalog.with(|c| c.switch_branch(image_id, &name))
}

#[tauri::command]
pub fn history_branch(
    image_id: ImageId,
    at: CommitId,
    name: Option<String>,
    catalog: State<'_, CatalogState>,
) -> CmdResult<Checkout> {
    catalog.with(|c| c.create_branch(image_id, at, name.as_deref()))
}

#[tauri::command]
pub fn history_rename(
    image_id: ImageId,
    old: String,
    new: String,
    catalog: State<'_, CatalogState>,
) -> CmdResult<Checkout> {
    catalog.with(|c| c.rename_branch(image_id, &old, &new))
}

#[tauri::command]
pub fn history_delete(image_id: ImageId, name: String, catalog: State<'_, CatalogState>) -> CmdResult<Checkout> {
    catalog.with(|c| c.delete_branch(image_id, &name))
}

/// Copy `panels` from the tip of `source` onto the current version, as one commit.
#[tauri::command]
pub fn history_apply_from(
    image_id: ImageId,
    source: String,
    panels: Vec<Panel>,
    catalog: State<'_, CatalogState>,
) -> CmdResult<Checkout> {
    catalog.with(|c| c.apply_from_branch(image_id, &source, &panels))
}

/// The edit recorded at `commit` (e.g. a branch tip, for its thumbnail or for compare).
#[tauri::command]
pub fn history_state(commit: CommitId, catalog: State<'_, CatalogState>) -> CmdResult<EditState> {
    catalog.with(|c| c.state_at(commit))
}
