//! `EditState` is a complete snapshot of every adjustment applied to an image.
//!
//! Snapshots (not deltas) are what the history graph stores, so checkout is O(1).
//! Every field has a neutral default and `#[serde(default)]`, so states written by
//! older versions of tonality keep loading as new adjustments are added.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct EditState {
    pub light: Light,
    pub white_balance: WhiteBalance,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Light {
    /// Exposure in stops, -5..=5.
    pub exposure: f32,
    /// -100..=100
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct WhiteBalance {
    /// Correlated colour temperature in Kelvin. `None` = as shot.
    pub temperature: Option<f32>,
    /// Green(-)/magenta(+) shift, -150..=150. `None` = as shot.
    pub tint: Option<f32>,
}

impl EditState {
    /// Canonical JSON used for hashing and storage. serde_json emits struct fields in
    /// declaration order, so this is deterministic for a given schema version.
    pub fn to_canonical_json(&self) -> String {
        serde_json::to_string(self).expect("EditState is always serializable")
    }

    /// Content address of this state (blake3 of canonical JSON, hex).
    pub fn hash(&self) -> String {
        blake3::hash(self.to_canonical_json().as_bytes()).to_hex().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trips() {
        let s = EditState::default();
        let back: EditState = serde_json::from_str(&s.to_canonical_json()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let s: EditState = serde_json::from_str(r#"{"light":{"exposure":1.5}}"#).unwrap();
        assert_eq!(s.light.exposure, 1.5);
        assert_eq!(s.light.contrast, 0.0);
        assert_eq!(s.white_balance, WhiteBalance::default());
    }

    #[test]
    fn hash_is_stable_and_content_sensitive() {
        let a = EditState::default();
        let mut b = EditState::default();
        assert_eq!(a.hash(), b.hash());
        b.light.exposure = 0.3;
        assert_ne!(a.hash(), b.hash());
    }
}
