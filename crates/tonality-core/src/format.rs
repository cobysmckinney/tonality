//! What this build of tonality writes to disk, as three numbers that two builds can compare.
//!
//! A newer build always reads what an older one wrote. The other way round it depends: an older
//! build refuses a catalog or sidecar with a higher version than its own, and it *drops* any
//! setting in an edit state that it doesn't know (the field is ignored on read, and the state
//! is written back without it). So before the app replaces itself with an older build
//! (switching from Nightly back to Stable), it has to know that build reads what this one has
//! written. Each release says what it reads in its update feed (`latest.json`, under
//! `tonality`), from `tonality --formats`.

use serde::{Deserialize, Serialize};

use crate::catalog::CATALOG_VERSION;
use crate::edit::EDIT_FORMAT;
use crate::sidecar::SIDECAR_VERSION;

/// The on-disk formats a build reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Formats {
    /// The catalog's schema ([`CATALOG_VERSION`]).
    pub catalog: u32,
    /// The sidecar file format ([`SIDECAR_VERSION`]).
    pub sidecar: u32,
    /// The settings an edit state can hold ([`EDIT_FORMAT`]).
    pub edits: u32,
}

impl Formats {
    /// This build's.
    pub const CURRENT: Formats = Formats {
        catalog: CATALOG_VERSION,
        sidecar: SIDECAR_VERSION,
        edits: EDIT_FORMAT,
    };

    /// Whether a build with these formats can open a library that a build with `written_by`
    /// has used, without refusing it and without losing anything in it.
    pub fn reads(&self, written_by: &Formats) -> bool {
        self.catalog >= written_by.catalog
            && self.sidecar >= written_by.sidecar
            && self.edits >= written_by.edits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_reads_what_it_and_older_builds_wrote() {
        let now = Formats::CURRENT;
        assert!(now.reads(&now));
        for older in [
            Formats {
                catalog: now.catalog - 1,
                ..now
            },
            Formats {
                edits: now.edits - 1,
                ..now
            },
        ] {
            assert!(now.reads(&older), "{older:?}");
            assert!(!older.reads(&now), "{older:?} would refuse or lose {now:?}");
        }
        // Ahead in one, behind in another: still can't.
        let mixed = Formats {
            catalog: now.catalog + 1,
            edits: now.edits - 1,
            ..now
        };
        assert!(!mixed.reads(&now));
    }

    #[test]
    fn the_feed_spells_them_this_way() {
        // `latest.json` carries this object; a release already published can't be respelled.
        let json = serde_json::to_string(&Formats {
            catalog: 8,
            sidecar: 1,
            edits: 2,
        })
        .unwrap();
        assert_eq!(json, r#"{"catalog":8,"sidecar":1,"edits":2}"#);
    }
}
