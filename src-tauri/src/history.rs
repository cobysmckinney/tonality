//! History commands: thin wrappers over `tonality_core::history` on the shared catalog.

use std::sync::{Arc, Mutex, OnceLock};

use tauri::State;
use tonality_core::{Catalog, Checkout, CommitId, EditState, ImageId, Panel};

use crate::CmdResult;

type ChangeHook = Box<dyn Fn(&[ImageId]) + Send + Sync>;

/// The shared catalog. Cheap to clone (a handle), so commands can hand it to a blocking thread.
#[derive(Clone)]
pub struct CatalogState {
    catalog: Arc<Mutex<Catalog>>,
    /// Told which images changed after every `edit` (keeps sidecars in step).
    on_change: Arc<OnceLock<ChangeHook>>,
}

impl CatalogState {
    pub fn new(catalog: Catalog) -> Self {
        Self { catalog: Arc::new(Mutex::new(catalog)), on_change: Arc::new(OnceLock::new()) }
    }

    /// [`with`](Self::with) on a blocking thread: commands are async, and catalog work (SQLite,
    /// or waiting for the lock behind an import) mustn't stall the async runtime's workers.
    pub(crate) async fn read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Catalog) -> tonality_core::catalog::Result<T> + Send + 'static,
    ) -> CmdResult<T> {
        let me = self.clone();
        crate::blocking(move || me.with(f)).await
    }

    /// [`edit`](Self::edit) on a blocking thread.
    pub(crate) async fn change<T: Send + 'static>(
        &self,
        ids: Vec<ImageId>,
        f: impl FnOnce(&mut Catalog) -> tonality_core::catalog::Result<T> + Send + 'static,
    ) -> CmdResult<T> {
        let me = self.clone();
        crate::blocking(move || me.edit(&ids, f)).await
    }

    pub fn on_change(&self, hook: ChangeHook) {
        let _ = self.on_change.set(hook);
    }

    /// Read (or change something that sidecars don't record).
    pub(crate) fn with<T>(&self, f: impl FnOnce(&mut Catalog) -> tonality_core::catalog::Result<T>) -> CmdResult<T> {
        let mut cat = self.catalog.lock().map_err(|_| "catalog lock poisoned")?;
        f(&mut cat).map_err(|e| e.to_string())
    }

    /// Change the history, rating or flag of `ids`.
    pub(crate) fn edit<T>(
        &self,
        ids: &[ImageId],
        f: impl FnOnce(&mut Catalog) -> tonality_core::catalog::Result<T>,
    ) -> CmdResult<T> {
        let out = self.with(f)?;
        if let Some(hook) = self.on_change.get() {
            hook(ids);
        }
        Ok(out)
    }
}

/// An image's history, created with an "Original" commit on first open.
#[tauri::command]
pub async fn history_open(image_id: ImageId, catalog: State<'_, CatalogState>) -> CmdResult<Checkout> {
    catalog.change(vec![image_id], move |c| c.history_open(image_id)).await
}

#[tauri::command]
pub async fn history_commit(
    image_id: ImageId,
    state: EditState,
    key: Option<String>,
    catalog: State<'_, CatalogState>,
) -> CmdResult<Checkout> {
    catalog.change(vec![image_id], move |c| c.commit(image_id, &state, key.as_deref())).await
}

#[tauri::command]
pub async fn history_undo(image_id: ImageId, catalog: State<'_, CatalogState>) -> CmdResult<Checkout> {
    catalog.change(vec![image_id], move |c| c.undo(image_id)).await
}

#[tauri::command]
pub async fn history_redo(image_id: ImageId, catalog: State<'_, CatalogState>) -> CmdResult<Checkout> {
    catalog.change(vec![image_id], move |c| c.redo(image_id)).await
}

#[tauri::command]
pub async fn history_checkout(
    image_id: ImageId,
    commit: CommitId,
    catalog: State<'_, CatalogState>,
) -> CmdResult<Checkout> {
    catalog.change(vec![image_id], move |c| c.checkout(image_id, commit)).await
}

#[tauri::command]
pub async fn history_switch(image_id: ImageId, name: String, catalog: State<'_, CatalogState>) -> CmdResult<Checkout> {
    catalog.change(vec![image_id], move |c| c.switch_branch(image_id, &name)).await
}

#[tauri::command]
pub async fn history_branch(
    image_id: ImageId,
    at: CommitId,
    name: Option<String>,
    catalog: State<'_, CatalogState>,
) -> CmdResult<Checkout> {
    catalog.change(vec![image_id], move |c| c.create_branch(image_id, at, name.as_deref())).await
}

#[tauri::command]
pub async fn history_rename(
    image_id: ImageId,
    old: String,
    new: String,
    catalog: State<'_, CatalogState>,
) -> CmdResult<Checkout> {
    catalog.change(vec![image_id], move |c| c.rename_branch(image_id, &old, &new)).await
}

#[tauri::command]
pub async fn history_delete(image_id: ImageId, name: String, catalog: State<'_, CatalogState>) -> CmdResult<Checkout> {
    catalog.change(vec![image_id], move |c| c.delete_branch(image_id, &name)).await
}

/// Copy `panels` from the tip of `source` onto the current version, as one commit.
#[tauri::command]
pub async fn history_apply_from(
    image_id: ImageId,
    source: String,
    panels: Vec<Panel>,
    catalog: State<'_, CatalogState>,
) -> CmdResult<Checkout> {
    catalog.change(vec![image_id], move |c| c.apply_from_branch(image_id, &source, &panels)).await
}

/// The edit recorded at `commit` (e.g. a branch tip, for its thumbnail or for compare).
#[tauri::command]
pub async fn history_state(commit: CommitId, catalog: State<'_, CatalogState>) -> CmdResult<EditState> {
    catalog.read(move |c| c.state_at(commit)).await
}
