//! Stopping long work part-way: an export or an import the user no longer wants.

use std::sync::atomic::{AtomicBool, Ordering};

/// A request to stop, raised from the interface and checked by the work between photos (never
/// in the middle of one, so nothing is left half-written). There is one per kind of work, and
/// whatever starts the work clears it first.
#[derive(Default)]
pub struct Cancel(AtomicBool);

impl Cancel {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    pub fn request(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn clear(&self) {
        self.0.store(false, Ordering::Relaxed);
    }

    pub fn requested(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
