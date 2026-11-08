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

// ---------------------------------------------------------------------------------------------
// Change summaries (history commit messages)
// ---------------------------------------------------------------------------------------------

enum Fmt {
    /// Signed, with this many decimals.
    Signed(usize),
    /// Kelvin; `None` reads "As Shot".
    Kelvin,
    /// Signed integer; `None` reads "As Shot".
    SignedOpt,
}

struct Field {
    label: &'static str,
    get: fn(&EditState) -> Option<f32>,
    fmt: Fmt,
}

const FIELDS: &[Field] = &[
    Field { label: "Temperature", get: |s| s.white_balance.temperature, fmt: Fmt::Kelvin },
    Field { label: "Tint", get: |s| s.white_balance.tint, fmt: Fmt::SignedOpt },
    Field { label: "Exposure", get: |s| Some(s.light.exposure), fmt: Fmt::Signed(2) },
    Field { label: "Contrast", get: |s| Some(s.light.contrast), fmt: Fmt::Signed(0) },
    Field { label: "Highlights", get: |s| Some(s.light.highlights), fmt: Fmt::Signed(0) },
    Field { label: "Shadows", get: |s| Some(s.light.shadows), fmt: Fmt::Signed(0) },
    Field { label: "Whites", get: |s| Some(s.light.whites), fmt: Fmt::Signed(0) },
    Field { label: "Blacks", get: |s| Some(s.light.blacks), fmt: Fmt::Signed(0) },
];

fn format_value(fmt: &Fmt, v: Option<f32>) -> String {
    match (fmt, v) {
        (Fmt::Signed(d), v) => {
            let v = v.unwrap_or(0.0);
            let s = format!("{v:.d$}", d = *d);
            // Avoid "-0" / "+0.00" noise.
            if s.trim_start_matches(['-', '0', '.']).is_empty() {
                format!("{:.d$}", 0.0, d = *d)
            } else if v > 0.0 {
                format!("+{s}")
            } else {
                s
            }
        }
        (Fmt::Kelvin, Some(v)) => format!("{}K", v.round() as i64),
        (Fmt::SignedOpt, Some(v)) => format_value(&Fmt::Signed(0), Some(v)),
        (_, None) => "As Shot".into(),
    }
}

/// Human description of what changed from `prev` to `next`, e.g. `"Exposure +0.30"`,
/// `"White Balance As Shot"`, `"Shadows, Blacks"` or `"5 adjustments"`.
pub fn describe_change(prev: &EditState, next: &EditState) -> String {
    let changed: Vec<&Field> =
        FIELDS.iter().filter(|f| (f.get)(prev) != (f.get)(next)).collect();
    let wb_as_shot = next.white_balance == WhiteBalance::default();
    match changed.as_slice() {
        [] => "No change".into(),
        [f] => format!("{} {}", f.label, format_value(&f.fmt, (f.get)(next))),
        [a, b] if a.label == "Temperature" && b.label == "Tint" && wb_as_shot => {
            "White Balance As Shot".into()
        }
        fs if fs.len() <= 3 => fs.iter().map(|f| f.label).collect::<Vec<_>>().join(", "),
        fs => format!("{} adjustments", fs.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_changes() {
        let a = EditState::default();
        let mut b = a.clone();
        assert_eq!(describe_change(&a, &b), "No change");
        b.light.exposure = 0.3;
        assert_eq!(describe_change(&a, &b), "Exposure +0.30");
        b.light.exposure = -0.0001;
        assert_eq!(describe_change(&a, &b), "Exposure 0.00");
        let mut c = a.clone();
        c.white_balance.temperature = Some(5200.4);
        assert_eq!(describe_change(&a, &c), "Temperature 5200K");
        assert_eq!(describe_change(&c, &a), "Temperature As Shot");
        c.white_balance.tint = Some(-12.0);
        assert_eq!(describe_change(&c, &a), "White Balance As Shot");
        c.light.shadows = 40.0;
        assert_eq!(describe_change(&a, &c), "Temperature, Tint, Shadows");
        c.light.blacks = -10.0;
        assert_eq!(describe_change(&a, &c), "4 adjustments");
    }

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
