//! tonality-core: the edit model, the git-like history graph and the catalog.
//! Has no GUI or GPU dependencies so it can be tested in isolation.

pub mod color;
pub mod edit;

pub use edit::EditState;
