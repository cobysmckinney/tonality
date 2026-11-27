//! `EditState` is a complete snapshot of every adjustment applied to an image.
//!
//! Snapshots (not deltas) are what the history graph stores, so checkout is O(1).
//! Every field has a neutral default and `#[serde(default)]`, so states written by
//! older versions of tonality keep loading as new adjustments are added.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::grading::ColorGrading;
use crate::lens::LensCorrection;
use crate::mask::{describe_masks, Mask};

/// What an edit state can hold, as a number that goes up whenever a build older than the
/// change would lose something reading a newer state: a new setting (it ignores the field, and
/// writes the state back without it) or a new value of an existing one (it can't read the state
/// at all). Nothing is refused on this number; it lets the updater tell that an older build
/// can't be trusted with this library (see [`crate::format`]).
///
/// 1: v0.1.1. 2: colour grading.
///
/// The test `the_stored_shape_is_the_one_edit_format_names` fails when the shape changes, as a
/// reminder to decide whether this needs raising.
pub const EDIT_FORMAT: u32 = 2;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct EditState {
    pub light: Light,
    pub white_balance: WhiteBalance,
    pub curve: ToneCurve,
    pub color: Color,
    pub detail: Detail,
    pub effects: Effects,
    pub crop: Crop,
    /// Local adjustments, applied in order. Omitted from the JSON when empty, so states saved
    /// before masks existed keep their hashes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[ts(optional = nullable)]
    pub masks: Vec<Mask>,
    /// Lens correction. Omitted from the JSON while untouched (older states keep their hashes).
    #[serde(skip_serializing_if = "LensCorrection::is_default")]
    #[ts(optional = nullable)]
    pub lens: LensCorrection,
    /// Colour grading. Omitted from the JSON while untouched (older states keep their hashes).
    #[serde(skip_serializing_if = "ColorGrading::is_default")]
    #[ts(optional = nullable)]
    pub grading: ColorGrading,
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
    /// The preset `temperature` and `tint` were set from, if any (`None`: as shot, or custom).
    /// A label only: the values are what's rendered.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub preset: Option<WbPreset>,
}

/// A named lighting condition for white balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum WbPreset {
    Daylight,
    Cloudy,
    Shade,
    Tungsten,
    Fluorescent,
    Flash,
    /// Measured from the photo (`auto::white_balance`); its values are whatever that found.
    Auto,
}

impl WbPreset {
    pub const ALL: [WbPreset; 6] = [
        WbPreset::Daylight,
        WbPreset::Cloudy,
        WbPreset::Shade,
        WbPreset::Tungsten,
        WbPreset::Fluorescent,
        WbPreset::Flash,
    ];

    /// The light's chromaticity (CIE 1931 xy): standard illuminants where there is one. `None`
    /// for Auto, which has no fixed light.
    pub fn xy(self) -> Option<(f32, f32)> {
        use crate::color::daylight_xy;
        Some(match self {
            WbPreset::Daylight => daylight_xy(5500.0),
            WbPreset::Cloudy => daylight_xy(6500.0),
            WbPreset::Shade => daylight_xy(7500.0),
            // Illuminant A (incandescent).
            WbPreset::Tungsten => (0.44757, 0.40745),
            // F2, cool white fluorescent.
            WbPreset::Fluorescent => (0.37208, 0.37529),
            WbPreset::Flash => daylight_xy(5900.0),
            WbPreset::Auto => return None,
        })
    }

    /// `(temperature, tint)` on the sliders' scales, rounded as the sliders show them.
    pub fn values(self) -> Option<(f32, f32)> {
        let (x, y) = self.xy()?;
        let (t, tint) = crate::color::temperature_tint(x, y);
        Some(((t / 50.0).round() * 50.0, tint.round()))
    }

    pub fn label(self) -> &'static str {
        match self {
            WbPreset::Daylight => "Daylight",
            WbPreset::Cloudy => "Cloudy",
            WbPreset::Shade => "Shade",
            WbPreset::Tungsten => "Tungsten",
            WbPreset::Fluorescent => "Fluorescent",
            WbPreset::Flash => "Flash",
            WbPreset::Auto => "Auto",
        }
    }
}

/// Tone curves in display space (0..1 in, 0..1 out). Each is a list of `[x, y]` control points
/// sorted by x; an empty list is the identity. Channel curves apply after the master `rgb` curve.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ToneCurve {
    pub rgb: Vec<[f32; 2]>,
    pub red: Vec<[f32; 2]>,
    pub green: Vec<[f32; 2]>,
    pub blue: Vec<[f32; 2]>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Color {
    /// -100..=100. Boosts muted colours more than saturated ones.
    pub vibrance: f32,
    /// -100..=100. -100 is monochrome.
    pub saturation: f32,
    pub mixer: ColorMixer,
}

/// Per-hue adjustments ("HSL"), eight bands around the hue circle.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ColorMixer {
    pub red: HueBand,
    pub orange: HueBand,
    pub yellow: HueBand,
    pub green: HueBand,
    pub aqua: HueBand,
    pub blue: HueBand,
    pub purple: HueBand,
    pub magenta: HueBand,
}

impl ColorMixer {
    pub fn bands(&self) -> [&HueBand; 8] {
        [
            &self.red,
            &self.orange,
            &self.yellow,
            &self.green,
            &self.aqua,
            &self.blue,
            &self.purple,
            &self.magenta,
        ]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct HueBand {
    /// -100..=100, shifts towards the neighbouring hues.
    pub hue: f32,
    pub saturation: f32,
    pub luminance: f32,
}

/// Sharpening and noise reduction. Sizes are in full-resolution pixels, so the effect is only
/// fully visible at 1:1 (as in the export).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Detail {
    /// 0..=150
    pub sharpen_amount: f32,
    /// 0.5..=3.0 pixels.
    pub sharpen_radius: f32,
    /// 0..=100: restricts sharpening to edges (0 = everywhere).
    pub sharpen_masking: f32,
    /// 0..=100
    pub luminance_noise: f32,
    /// 0..=100
    pub color_noise: f32,
}

impl Default for Detail {
    fn default() -> Self {
        Self {
            sharpen_amount: 0.0,
            sharpen_radius: 1.0,
            sharpen_masking: 0.0,
            luminance_noise: 0.0,
            color_noise: 0.0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Effects {
    /// -100..=100: fine detail (a few pixels across) brought out, or smoothed when negative.
    /// Left out of the saved JSON at 0, so edits made before it existed keep their hash.
    #[serde(skip_serializing_if = "is_zero")]
    #[ts(as = "Option<f32>", optional)]
    pub texture: f32,
    /// -100..=100: mid-scale local contrast, edge-aware.
    #[serde(skip_serializing_if = "is_zero")]
    #[ts(as = "Option<f32>", optional)]
    pub clarity: f32,
    /// -100..=100: haze removed, or added when negative.
    #[serde(skip_serializing_if = "is_zero")]
    #[ts(as = "Option<f32>", optional)]
    pub dehaze: f32,
    pub vignette: Vignette,
    pub grain: Grain,
}

/// For `skip_serializing_if`: settings added after edits were first saved stay out of the
/// canonical JSON (and so out of the hash) until they're used.
pub(crate) fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Vignette {
    /// -100 (dark edges) ..= 100 (light edges).
    pub amount: f32,
    /// 0..=100: how far from the centre the falloff starts.
    pub midpoint: f32,
    /// 0..=100: softness of the falloff.
    pub feather: f32,
}

impl Default for Vignette {
    fn default() -> Self {
        Self {
            amount: 0.0,
            midpoint: 50.0,
            feather: 50.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Grain {
    /// 0..=100
    pub amount: f32,
    /// 0..=100, relative to the image so it looks the same at any output size.
    pub size: f32,
}

impl Default for Grain {
    fn default() -> Self {
        Self {
            amount: 0.0,
            size: 25.0,
        }
    }
}

/// Framing: quarter turns and flip (lossless), then a fine rotation, then a crop.
/// See [`crate::geometry`] for how this maps output pixels to the source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Crop {
    /// Clockwise quarter turns, 0..=3.
    pub turns: u8,
    /// Mirror left-right (after turning).
    pub flip: bool,
    /// Straighten angle in degrees, -45..=45; positive turns the image clockwise.
    pub angle: f32,
    /// Crop rectangle `[x, y, w, h]` as fractions of the turned frame (before straightening).
    /// Must lie inside the rotated image; `[0, 0, 1, 1]` with no angle is uncropped.
    pub rect: [f32; 4],
}

impl Default for Crop {
    fn default() -> Self {
        Self {
            turns: 0,
            flip: false,
            angle: 0.0,
            rect: [0.0, 0.0, 1.0, 1.0],
        }
    }
}

/// The sections of the Adjustments panel, for copying settings between versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Panel {
    WhiteBalance,
    Light,
    ToneCurve,
    Color,
    ColorGrading,
    Detail,
    Effects,
    Crop,
    Masks,
    Lens,
}

impl Panel {
    pub const ALL: [Panel; 10] = [
        Panel::WhiteBalance,
        Panel::Light,
        Panel::ToneCurve,
        Panel::Color,
        Panel::ColorGrading,
        Panel::Detail,
        Panel::Effects,
        Panel::Crop,
        Panel::Masks,
        Panel::Lens,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Panel::WhiteBalance => "White Balance",
            Panel::Light => "Light",
            Panel::ToneCurve => "Tone Curve",
            Panel::Color => "Color",
            Panel::ColorGrading => "Color Grading",
            Panel::Detail => "Detail",
            Panel::Effects => "Effects",
            Panel::Crop => "Crop",
            Panel::Masks => "Masks",
            Panel::Lens => "Lens Correction",
        }
    }
}

impl EditState {
    /// This state with `panels` taken from `src`.
    pub fn with_panels_from(&self, src: &EditState, panels: &[Panel]) -> EditState {
        let mut out = self.clone();
        for p in panels {
            match p {
                Panel::WhiteBalance => out.white_balance = src.white_balance.clone(),
                Panel::Light => out.light = src.light.clone(),
                Panel::ToneCurve => out.curve = src.curve.clone(),
                Panel::Color => out.color = src.color.clone(),
                Panel::ColorGrading => out.grading = src.grading.clone(),
                Panel::Detail => out.detail = src.detail.clone(),
                Panel::Effects => out.effects = src.effects.clone(),
                Panel::Crop => out.crop = src.crop.clone(),
                Panel::Masks => out.masks = src.masks.clone(),
                Panel::Lens => out.lens = src.lens.clone(),
            }
        }
        out
    }

    /// Canonical JSON used for hashing and storage. serde_json emits struct fields in
    /// declaration order, so this is deterministic for a given schema version.
    pub fn to_canonical_json(&self) -> String {
        serde_json::to_string(self).expect("EditState is always serializable")
    }

    /// Whether the state reads back from its own canonical JSON. It doesn't when a number in it
    /// isn't finite: a JSON number too large for an `f32` (`1e39`) is read as infinity, and
    /// infinity is written as `null`, which is not a number. Such a state must never be stored,
    /// or the step that holds it can't be opened again.
    pub fn is_storable(&self) -> bool {
        serde_json::from_str::<EditState>(&self.to_canonical_json()).is_ok()
    }

    /// Content address of this state (blake3 of canonical JSON, hex).
    pub fn hash(&self) -> String {
        blake3::hash(self.to_canonical_json().as_bytes())
            .to_hex()
            .to_string()
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
    /// Unsigned integer.
    Plain,
    /// Unsigned, with this many decimals.
    Decimal(usize),
    /// Signed degrees, one decimal.
    Degrees,
}

struct Field {
    label: &'static str,
    get: fn(&EditState) -> Option<f32>,
    fmt: Fmt,
}

/// Three fields (hue, saturation, luminance) for one colour-mixer band.
macro_rules! band {
    ($name:literal, $f:ident) => {
        [
            Field {
                label: concat!($name, " Hue"),
                get: |s| Some(s.color.mixer.$f.hue),
                fmt: Fmt::Signed(0),
            },
            Field {
                label: concat!($name, " Saturation"),
                get: |s| Some(s.color.mixer.$f.saturation),
                fmt: Fmt::Signed(0),
            },
            Field {
                label: concat!($name, " Luminance"),
                get: |s| Some(s.color.mixer.$f.luminance),
                fmt: Fmt::Signed(0),
            },
        ]
    };
}

const FIELDS: &[Field] = &[
    Field {
        label: "Temperature",
        get: |s| s.white_balance.temperature,
        fmt: Fmt::Kelvin,
    },
    Field {
        label: "Tint",
        get: |s| s.white_balance.tint,
        fmt: Fmt::SignedOpt,
    },
    Field {
        label: "Exposure",
        get: |s| Some(s.light.exposure),
        fmt: Fmt::Signed(2),
    },
    Field {
        label: "Contrast",
        get: |s| Some(s.light.contrast),
        fmt: Fmt::Signed(0),
    },
    Field {
        label: "Highlights",
        get: |s| Some(s.light.highlights),
        fmt: Fmt::Signed(0),
    },
    Field {
        label: "Shadows",
        get: |s| Some(s.light.shadows),
        fmt: Fmt::Signed(0),
    },
    Field {
        label: "Whites",
        get: |s| Some(s.light.whites),
        fmt: Fmt::Signed(0),
    },
    Field {
        label: "Blacks",
        get: |s| Some(s.light.blacks),
        fmt: Fmt::Signed(0),
    },
    Field {
        label: "Vibrance",
        get: |s| Some(s.color.vibrance),
        fmt: Fmt::Signed(0),
    },
    Field {
        label: "Saturation",
        get: |s| Some(s.color.saturation),
        fmt: Fmt::Signed(0),
    },
    Field {
        label: "Sharpening",
        get: |s| Some(s.detail.sharpen_amount),
        fmt: Fmt::Plain,
    },
    Field {
        label: "Sharpen Radius",
        get: |s| Some(s.detail.sharpen_radius),
        fmt: Fmt::Decimal(1),
    },
    Field {
        label: "Sharpen Masking",
        get: |s| Some(s.detail.sharpen_masking),
        fmt: Fmt::Plain,
    },
    Field {
        label: "Noise Reduction",
        get: |s| Some(s.detail.luminance_noise),
        fmt: Fmt::Plain,
    },
    Field {
        label: "Color Noise Reduction",
        get: |s| Some(s.detail.color_noise),
        fmt: Fmt::Plain,
    },
    Field {
        label: "Straighten",
        get: |s| Some(s.crop.angle),
        fmt: Fmt::Degrees,
    },
    Field {
        label: "Texture",
        get: |s| Some(s.effects.texture),
        fmt: Fmt::Signed(0),
    },
    Field {
        label: "Clarity",
        get: |s| Some(s.effects.clarity),
        fmt: Fmt::Signed(0),
    },
    Field {
        label: "Dehaze",
        get: |s| Some(s.effects.dehaze),
        fmt: Fmt::Signed(0),
    },
    Field {
        label: "Vignette",
        get: |s| Some(s.effects.vignette.amount),
        fmt: Fmt::Signed(0),
    },
    Field {
        label: "Vignette Midpoint",
        get: |s| Some(s.effects.vignette.midpoint),
        fmt: Fmt::Plain,
    },
    Field {
        label: "Vignette Feather",
        get: |s| Some(s.effects.vignette.feather),
        fmt: Fmt::Plain,
    },
    Field {
        label: "Grain",
        get: |s| Some(s.effects.grain.amount),
        fmt: Fmt::Plain,
    },
    Field {
        label: "Grain Size",
        get: |s| Some(s.effects.grain.size),
        fmt: Fmt::Plain,
    },
    Field {
        label: "Lens Distortion",
        get: |s| Some(s.lens.distortion),
        fmt: Fmt::Plain,
    },
    Field {
        label: "Lens Vignetting",
        get: |s| Some(s.lens.vignetting),
        fmt: Fmt::Plain,
    },
    Field {
        label: "Manual Distortion",
        get: |s| Some(s.lens.manual_distortion),
        fmt: Fmt::Signed(0),
    },
];

const BANDS: &[[Field; 3]] = &[
    band!("Red", red),
    band!("Orange", orange),
    band!("Yellow", yellow),
    band!("Green", green),
    band!("Aqua", aqua),
    band!("Blue", blue),
    band!("Purple", purple),
    band!("Magenta", magenta),
];

/// Curves aren't scalar; they're summarised by name only.
type CurveGetter = fn(&EditState) -> &Vec<[f32; 2]>;
const CURVES: &[(&str, CurveGetter)] = &[
    ("Tone Curve", |s| &s.curve.rgb),
    ("Red Curve", |s| &s.curve.red),
    ("Green Curve", |s| &s.curve.green),
    ("Blue Curve", |s| &s.curve.blue),
];

/// `v` with a sign and `d` decimals, without "-0" / "+0.00" noise.
pub(crate) fn signed(v: f32, d: usize) -> String {
    let s = format!("{v:.d$}");
    if s.trim_start_matches(['-', '0', '.']).is_empty() {
        format!("{:.d$}", 0.0)
    } else if v > 0.0 {
        format!("+{s}")
    } else {
        s
    }
}

fn format_value(fmt: &Fmt, v: Option<f32>) -> String {
    match (fmt, v) {
        (Fmt::Signed(d), v) => signed(v.unwrap_or(0.0), *d),
        (Fmt::Kelvin, Some(v)) => format!("{}K", v.round() as i64),
        (Fmt::SignedOpt, Some(v)) => format_value(&Fmt::Signed(0), Some(v)),
        (Fmt::Plain, v) => format!("{}", v.unwrap_or(0.0).round() as i64),
        (Fmt::Degrees, v) => format!("{}°", format_value(&Fmt::Signed(1), v)),
        (Fmt::Decimal(d), v) => format!("{:.d$}", v.unwrap_or(0.0), d = *d),
        (_, None) => "As Shot".into(),
    }
}

/// `FIELDS`, with each colour-mixer band expanded to its three fields.
fn all_fields() -> impl Iterator<Item = &'static Field> {
    FIELDS.iter().chain(BANDS.iter().flatten())
}

/// Human description of what changed from `prev` to `next`, e.g. `"Exposure +0.30"`,
/// `"White Balance As Shot"`, `"Shadows, Blacks"` or `"5 adjustments"`.
pub fn describe_change(prev: &EditState, next: &EditState) -> String {
    let mut changed: Vec<&Field> = all_fields()
        .filter(|f| (f.get)(prev) != (f.get)(next))
        .collect();
    let mut curves: Vec<&str> = CURVES
        .iter()
        .filter(|(_, c)| c(prev) != c(next))
        .map(|(l, _)| *l)
        .collect();
    // Picking a preset (or switching to Custom) names the mode, rather than the temperature
    // and tint it set.
    let mode = |w: &WhiteBalance| match (w.preset, w.temperature, w.tint) {
        (Some(p), ..) => Some(p.label()),
        (None, None, None) => None,
        _ => Some("Custom"),
    };
    let (was, now) = (mode(&prev.white_balance), mode(&next.white_balance));
    let preset_label = now
        .filter(|_| was != now)
        .map(|m| format!("White Balance {m}"));
    if let Some(label) = &preset_label {
        changed.retain(|f| f.label != "Temperature" && f.label != "Tint");
        curves.insert(0, label.as_str());
    }
    // Framing changes that aren't scalars are named, like curves.
    let (a, b) = (&prev.crop, &next.crop);
    if a.turns != b.turns {
        curves.push("Rotate");
    }
    if a.flip != b.flip {
        curves.push("Flip");
    }
    if a.rect != b.rect {
        curves.push("Crop");
    }
    let (a, b) = (&prev.lens, &next.lens);
    if a.profile != b.profile {
        curves.push(if b.profile {
            "Lens Profile On"
        } else {
            "Lens Profile Off"
        });
    }
    if a.chromatic != b.chromatic {
        curves.push(if b.chromatic {
            "Chromatic Aberration On"
        } else {
            "Chromatic Aberration Off"
        });
    }
    // Grading is named as one thing: a linked drag moves all three ranges at once.
    let (a, b) = (&prev.grading, &next.grading);
    let grading = match b.link {
        Some(h) if a.link != b.link => format!("Color Grading: {}", h.label()),
        None if a.link.is_some() => "Color Grading: Unlinked".into(),
        _ => "Color Grading".into(),
    };
    if a != b {
        curves.push(&grading);
    }
    let masks = describe_masks(&prev.masks, &next.masks);
    if let ([], [], [(_, full)]) = (changed.as_slice(), curves.as_slice(), masks.as_slice()) {
        return full.clone();
    }
    curves.extend(masks.iter().map(|(short, _)| short.as_str()));
    let wb_as_shot = next.white_balance == WhiteBalance::default();
    match (changed.as_slice(), curves.as_slice()) {
        ([], []) => "No change".into(),
        ([], [c]) => (*c).into(),
        ([f], []) => format!("{} {}", f.label, format_value(&f.fmt, (f.get)(next))),
        ([a, b], []) if a.label == "Temperature" && b.label == "Tint" && wb_as_shot => {
            "White Balance As Shot".into()
        }
        (fs, cs) if fs.len() + cs.len() <= 3 => fs
            .iter()
            .map(|f| f.label)
            .chain(cs.iter().copied())
            .collect::<Vec<_>>()
            .join(", "),
        (fs, cs) => format!("{} adjustments", fs.len() + cs.len()),
    }
}

/// One setting a step changed, for the history panel's step details.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Change {
    pub label: String,
    /// The value before and after, formatted as the step labels are. `None` for changes that
    /// have no single value to show (a curve, a crop, a mask).
    #[ts(optional = nullable)]
    pub from: Option<String>,
    #[ts(optional = nullable)]
    pub to: Option<String>,
}

/// Colour grading as scalars, for [`list_changes`] only: step labels name grading as one thing.
macro_rules! grade_range {
    ($name:literal, $r:ident) => {
        [
            Field {
                label: concat!($name, " Hue"),
                get: |s| Some(s.grading.$r.hue),
                fmt: Fmt::Plain,
            },
            Field {
                label: concat!($name, " Saturation"),
                get: |s| Some(s.grading.$r.saturation),
                fmt: Fmt::Plain,
            },
            Field {
                label: concat!($name, " Luminance"),
                get: |s| Some(s.grading.$r.luminance),
                fmt: Fmt::Signed(0),
            },
        ]
    };
}

const GRADE_RANGES: &[[Field; 3]] = &[
    grade_range!("Shadows", shadows),
    grade_range!("Midtones", midtones),
    grade_range!("Highlights", highlights),
];

const GRADE_FIELDS: &[Field] = &[
    Field {
        label: "Grading Blending",
        get: |s| Some(s.grading.blending),
        fmt: Fmt::Plain,
    },
    Field {
        label: "Grading Balance",
        get: |s| Some(s.grading.balance),
        fmt: Fmt::Signed(0),
    },
];

/// Every setting that differs from `prev` to `next`, with old and new values where there are
/// any: the long form of [`describe_change`], e.g. `Exposure +1.20 → +2.85`.
pub fn list_changes(prev: &EditState, next: &EditState) -> Vec<Change> {
    let value = |label: &str, from: String, to: String| Change {
        label: label.into(),
        from: Some(from),
        to: Some(to),
    };
    let named = |label: &str| Change {
        label: label.into(),
        from: None,
        to: None,
    };
    let on_off = |b: bool| if b { "On" } else { "Off" }.to_string();
    let mut out = Vec::new();

    let mode = |w: &WhiteBalance| match (w.preset, w.temperature, w.tint) {
        (Some(p), ..) => p.label(),
        (None, None, None) => "As Shot",
        _ => "Custom",
    };
    let (was, now) = (mode(&prev.white_balance), mode(&next.white_balance));
    if was != now {
        out.push(value("White Balance", was.into(), now.into()));
    }
    let scalars = |fields: &mut dyn Iterator<Item = &Field>, out: &mut Vec<Change>| {
        for f in fields {
            let (a, b) = ((f.get)(prev), (f.get)(next));
            if a != b {
                out.push(value(
                    f.label,
                    format_value(&f.fmt, a),
                    format_value(&f.fmt, b),
                ));
            }
        }
    };
    scalars(&mut FIELDS.iter(), &mut out);
    for (label, c) in CURVES {
        if c(prev) != c(next) {
            out.push(named(label));
        }
    }
    scalars(&mut BANDS.iter().flatten(), &mut out);

    let (a, b) = (&prev.grading, &next.grading);
    scalars(&mut GRADE_RANGES.iter().flatten(), &mut out);
    scalars(&mut GRADE_FIELDS.iter(), &mut out);
    if a.link != b.link {
        let link = |h: Option<crate::grading::Harmony>| h.map_or("Off", |h| h.label()).to_string();
        out.push(value("Grading Link", link(a.link), link(b.link)));
    }

    let (a, b) = (&prev.crop, &next.crop);
    if a.turns != b.turns {
        let deg = |t: u8| format!("{}°", u32::from(t) * 90);
        out.push(value("Rotate", deg(a.turns), deg(b.turns)));
    }
    if a.flip != b.flip {
        out.push(value("Flip", on_off(a.flip), on_off(b.flip)));
    }
    if a.rect != b.rect {
        out.push(named("Crop"));
    }
    let (a, b) = (&prev.lens, &next.lens);
    if a.profile != b.profile {
        out.push(value("Lens Profile", on_off(a.profile), on_off(b.profile)));
    }
    if a.chromatic != b.chromatic {
        out.push(value(
            "Chromatic Aberration",
            on_off(a.chromatic),
            on_off(b.chromatic),
        ));
    }
    out.extend(
        describe_masks(&prev.masks, &next.masks)
            .into_iter()
            .map(|(_, full)| named(&full)),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grading::Harmony;

    /// Every type an edit state is made of, with its fields and variants, as TypeScript (which
    /// is how the types are already written out for the interface).
    fn stored_shape() -> String {
        struct Walk {
            cfg: ts_rs::Config,
            seen: std::collections::BTreeMap<String, String>,
        }
        impl ts_rs::TypeVisitor for Walk {
            fn visit<T: TS + 'static + ?Sized>(&mut self) {
                let name = T::name(&self.cfg);
                if self.seen.contains_key(&name) {
                    return;
                }
                self.seen.insert(name, T::inline(&self.cfg));
                T::visit_dependencies(self);
            }
        }
        let mut walk = Walk {
            cfg: ts_rs::Config::default(),
            seen: Default::default(),
        };
        ts_rs::TypeVisitor::visit::<EditState>(&mut walk);
        let all: String = walk
            .seen
            .iter()
            .map(|(name, shape)| format!("{name} = {shape}\n"))
            .collect();
        // Without the doc comments (rewording one changes nothing that's stored), and with
        // every run of white space as one space.
        let mut shape = String::new();
        let mut rest = all.as_str();
        while let Some(at) = rest.find("/*") {
            shape.push_str(&rest[..at]);
            rest = rest[at..]
                .find("*/")
                .map_or("", |end| &rest[at + end + 2..]);
        }
        shape.push_str(rest);
        shape
            .lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The shape [`EDIT_FORMAT`] was last decided for (blake3 of [`stored_shape`]).
    const EDIT_SHAPE: &str = "a8450af3e601905795d21a9442f448af18c63824cb73521d0a765909bd679e44";

    #[test]
    fn the_stored_shape_is_the_one_edit_format_names() {
        let shape = stored_shape();
        assert!(
            shape.contains("ColorGrading = {"),
            "walks nested types: {shape}"
        );
        assert!(
            shape.contains("MaskShape = "),
            "and the ones inside lists: {shape}"
        );
        let hash = blake3::hash(shape.as_bytes()).to_hex().to_string();
        assert_eq!(
            hash, EDIT_SHAPE,
            "\nWhat an edit state can hold has changed.\n\
             1. If a build from before this change would lose something reading a state written \
             after it (a new setting, or a new value for one), raise EDIT_FORMAT and add a line \
             to its comment. A rename or a retyped field needs more than that: older states \
             still have to load.\n\
             2. Set EDIT_SHAPE to \"{hash}\".\n\
             The shape is now:\n{shape}"
        );
    }

    #[test]
    fn presence_settings_stay_out_of_the_hash_until_used() {
        let mut s = EditState::default();
        s.masks.push(crate::mask::Mask::default());
        let json = s.to_canonical_json();
        for k in ["texture", "clarity", "dehaze"] {
            assert!(!json.contains(k), "{k} in {json}");
        }
        s.effects.clarity = 20.0;
        s.masks[0].adjust.dehaze = 10.0;
        let json = s.to_canonical_json();
        assert!(
            json.contains("\"clarity\":20") && json.contains("\"dehaze\":10"),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<EditState>(&json).unwrap(), s);
    }

    #[test]
    fn wb_presets_sit_where_their_lights_do() {
        let v = |p: WbPreset| p.values().unwrap();
        assert_eq!(v(WbPreset::Daylight), (5500.0, -11.0));
        assert_eq!(
            v(WbPreset::Tungsten),
            (2850.0, 0.0),
            "illuminant A is on the Planckian locus"
        );
        let t: Vec<f32> = [
            WbPreset::Tungsten,
            WbPreset::Fluorescent,
            WbPreset::Daylight,
            WbPreset::Flash,
            WbPreset::Cloudy,
            WbPreset::Shade,
        ]
        .iter()
        .map(|p| p.values().unwrap().0)
        .collect();
        assert!(t.windows(2).all(|w| w[0] < w[1]), "warm to cool: {t:?}"); // Auto has no fixed light, and isn't one of the listed presets.
        assert_eq!(WbPreset::Auto.values(), None);
        assert!(!WbPreset::ALL.contains(&WbPreset::Auto));
    }

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
        assert_eq!(describe_change(&a, &c), "White Balance Custom");
        assert_eq!(describe_change(&c, &a), "Temperature As Shot");
        c.white_balance.tint = Some(-12.0);
        assert_eq!(describe_change(&c, &a), "White Balance As Shot");
        c.light.shadows = 40.0;
        assert_eq!(describe_change(&a, &c), "Shadows, White Balance Custom");
        c.light.blacks = -10.0;
        assert_eq!(
            describe_change(&a, &c),
            "Shadows, Blacks, White Balance Custom"
        );
        c.light.whites = 5.0;
        assert_eq!(describe_change(&a, &c), "4 adjustments");

        let mut w = a.clone();
        w.white_balance = WhiteBalance {
            temperature: Some(2850.0),
            tint: Some(0.0),
            preset: Some(WbPreset::Tungsten),
        };
        assert_eq!(describe_change(&a, &w), "White Balance Tungsten");
        let mut custom = w.clone();
        custom.white_balance.preset = None;
        assert_eq!(describe_change(&w, &custom), "White Balance Custom");
        let mut warmer = custom.clone();
        warmer.white_balance.temperature = Some(3000.0);
        assert_eq!(describe_change(&custom, &warmer), "Temperature 3000K");

        let mut d = a.clone();
        d.color.mixer.blue.saturation = -30.0;
        assert_eq!(describe_change(&a, &d), "Blue Saturation -30");
        d.curve.rgb = vec![[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]];
        assert_eq!(describe_change(&a, &d), "Blue Saturation, Tone Curve");
        let mut e = a.clone();
        e.curve.red = vec![[0.0, 0.1], [1.0, 1.0]];
        assert_eq!(describe_change(&a, &e), "Red Curve");
        e.effects.vignette.midpoint = 30.0;
        assert_eq!(
            describe_change(
                &EditState {
                    curve: e.curve.clone(),
                    ..a.clone()
                },
                &e
            ),
            "Vignette Midpoint 30"
        );
        let mut g = a.clone();
        g.crop.angle = 2.34;
        assert_eq!(describe_change(&a, &g), "Straighten +2.3°");
        g.crop.rect = [0.1, 0.1, 0.8, 0.8];
        assert_eq!(describe_change(&a, &g), "Straighten, Crop");
        let mut f = a.clone();
        f.detail.sharpen_radius = 1.5;
        assert_eq!(describe_change(&a, &f), "Sharpen Radius 1.5");
    }

    /// `list_changes`, one line per change: "Label from → to", or just "Label".
    fn lines(a: &EditState, b: &EditState) -> Vec<String> {
        list_changes(a, b)
            .into_iter()
            .map(|c| match (c.from, c.to) {
                (Some(f), Some(t)) => format!("{} {f} → {t}", c.label),
                _ => c.label,
            })
            .collect()
    }

    #[test]
    fn lists_every_change_with_old_and_new_values() {
        let a = EditState::default();
        assert!(lines(&a, &a).is_empty());
        let mut b = a.clone();
        b.light.exposure = 1.2;
        let mut c = b.clone();
        c.light.exposure = 2.85;
        c.light.shadows = 40.0;
        c.light.blacks = -10.0;
        c.light.whites = 5.0;
        c.color.mixer.blue.saturation = -30.0;
        c.curve.rgb = vec![[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]];
        assert_eq!(describe_change(&b, &c), "6 adjustments");
        assert_eq!(
            lines(&b, &c),
            [
                "Exposure +1.20 → +2.85",
                "Shadows 0 → +40",
                "Whites 0 → +5",
                "Blacks 0 → -10",
                "Tone Curve",
                "Blue Saturation 0 → -30",
            ]
        );
    }

    #[test]
    fn lists_white_balance_framing_lens_grading_and_masks() {
        let a = EditState::default();
        let mut w = a.clone();
        w.white_balance = WhiteBalance {
            temperature: Some(2850.0),
            tint: Some(0.0),
            preset: Some(WbPreset::Tungsten),
        };
        assert_eq!(
            lines(&a, &w),
            [
                "White Balance As Shot → Tungsten",
                "Temperature As Shot → 2850K",
                "Tint As Shot → 0",
            ]
        );

        let mut f = a.clone();
        f.crop.turns = 1;
        f.crop.flip = true;
        f.crop.rect = [0.1, 0.1, 0.8, 0.8];
        f.lens.profile = !a.lens.profile;
        assert_eq!(
            lines(&a, &f),
            [
                "Rotate 0° → 90°",
                "Flip Off → On",
                "Crop",
                format!(
                    "Lens Profile {} → {}",
                    if a.lens.profile { "On" } else { "Off" },
                    if f.lens.profile { "On" } else { "Off" }
                )
                .as_str(),
            ]
        );

        let mut g = a.clone();
        g.grading.shadows.hue = 210.0;
        g.grading.shadows.saturation = 30.0;
        g.grading.link = Some(Harmony::Complementary);
        assert_eq!(
            lines(&a, &g),
            [
                "Shadows Hue 0 → 210",
                "Shadows Saturation 0 → 30",
                "Grading Link Off → Complementary",
            ]
        );

        let mut m = a.clone();
        m.masks.push(crate::mask::Mask::default());
        assert_eq!(lines(&a, &m).len(), 1);
        assert_eq!(lines(&a, &m)[0], describe_change(&a, &m));
    }

    #[test]
    fn grading_stays_out_of_the_hash_until_used() {
        let mut s = EditState::default();
        assert!(!s.to_canonical_json().contains("grading"));
        let untouched = s.hash();
        s.grading.shadows.saturation = 40.0;
        s.grading.shadows.hue = 220.0;
        s.grading.link = Some(Harmony::Complementary);
        let json = s.to_canonical_json();
        assert!(json.contains("\"link\":\"complementary\""), "{json}");
        assert_ne!(s.hash(), untouched);
        assert_eq!(serde_json::from_str::<EditState>(&json).unwrap(), s);
        s.grading = ColorGrading::default();
        assert_eq!(s.hash(), untouched);
    }

    #[test]
    fn grading_changes_are_named() {
        let a = EditState::default();
        let mut b = a.clone();
        b.grading.shadows.saturation = 40.0;
        b.grading.highlights.saturation = 40.0;
        b.grading.highlights.hue = 180.0;
        assert_eq!(describe_change(&a, &b), "Color Grading");
        let mut c = b.clone();
        c.grading.link = Some(Harmony::Triadic);
        c.grading.midtones.saturation = 40.0;
        assert_eq!(describe_change(&b, &c), "Color Grading: Triadic");
        assert_eq!(describe_change(&c, &b), "Color Grading: Unlinked");
        b.light.exposure = 1.0;
        assert_eq!(describe_change(&a, &b), "Exposure, Color Grading");
    }

    #[test]
    fn grading_is_a_panel_to_copy() {
        let mut src = EditState::default();
        src.grading.midtones.luminance = 12.0;
        src.color.saturation = 30.0;
        let out = EditState::default().with_panels_from(&src, &[Panel::ColorGrading]);
        assert_eq!(out.grading, src.grading);
        assert_eq!(out.color.saturation, 0.0);
        assert!(Panel::ALL.contains(&Panel::ColorGrading));
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
