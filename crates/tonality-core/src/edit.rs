//! `EditState` is a complete snapshot of every adjustment applied to an image.
//!
//! Snapshots (not deltas) are what the history graph stores, so checkout is O(1).
//! Every field has a neutral default and `#[serde(default)]`, so states written by
//! older versions of tonality keep loading as new adjustments are added.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::lens::LensCorrection;
use crate::mask::{describe_masks, Mask};

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
}

impl WbPreset {
    pub const ALL: [WbPreset; 6] =
        [WbPreset::Daylight, WbPreset::Cloudy, WbPreset::Shade, WbPreset::Tungsten, WbPreset::Fluorescent, WbPreset::Flash];

    /// The light's chromaticity (CIE 1931 xy): standard illuminants where there is one.
    pub fn xy(self) -> (f32, f32) {
        use crate::color::daylight_xy;
        match self {
            WbPreset::Daylight => daylight_xy(5500.0),
            WbPreset::Cloudy => daylight_xy(6500.0),
            WbPreset::Shade => daylight_xy(7500.0),
            // Illuminant A (incandescent).
            WbPreset::Tungsten => (0.44757, 0.40745),
            // F2, cool white fluorescent.
            WbPreset::Fluorescent => (0.37208, 0.37529),
            WbPreset::Flash => daylight_xy(5900.0),
        }
    }

    /// `(temperature, tint)` on the sliders' scales, rounded as the sliders show them.
    pub fn values(self) -> (f32, f32) {
        let (x, y) = self.xy();
        let (t, tint) = crate::color::temperature_tint(x, y);
        ((t / 50.0).round() * 50.0, tint.round())
    }

    pub fn label(self) -> &'static str {
        match self {
            WbPreset::Daylight => "Daylight",
            WbPreset::Cloudy => "Cloudy",
            WbPreset::Shade => "Shade",
            WbPreset::Tungsten => "Tungsten",
            WbPreset::Fluorescent => "Fluorescent",
            WbPreset::Flash => "Flash",
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
        [&self.red, &self.orange, &self.yellow, &self.green, &self.aqua, &self.blue, &self.purple, &self.magenta]
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
        Self { sharpen_amount: 0.0, sharpen_radius: 1.0, sharpen_masking: 0.0, luminance_noise: 0.0, color_noise: 0.0 }
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
        Self { amount: 0.0, midpoint: 50.0, feather: 50.0 }
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
        Self { amount: 0.0, size: 25.0 }
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
        Self { turns: 0, flip: false, angle: 0.0, rect: [0.0, 0.0, 1.0, 1.0] }
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
    Detail,
    Effects,
    Crop,
    Masks,
    Lens,
}

impl Panel {
    pub const ALL: [Panel; 9] = [
        Panel::WhiteBalance,
        Panel::Light,
        Panel::ToneCurve,
        Panel::Color,
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
            Field { label: concat!($name, " Hue"), get: |s| Some(s.color.mixer.$f.hue), fmt: Fmt::Signed(0) },
            Field { label: concat!($name, " Saturation"), get: |s| Some(s.color.mixer.$f.saturation), fmt: Fmt::Signed(0) },
            Field { label: concat!($name, " Luminance"), get: |s| Some(s.color.mixer.$f.luminance), fmt: Fmt::Signed(0) },
        ]
    };
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
    Field { label: "Vibrance", get: |s| Some(s.color.vibrance), fmt: Fmt::Signed(0) },
    Field { label: "Saturation", get: |s| Some(s.color.saturation), fmt: Fmt::Signed(0) },
    Field { label: "Sharpening", get: |s| Some(s.detail.sharpen_amount), fmt: Fmt::Plain },
    Field { label: "Sharpen Radius", get: |s| Some(s.detail.sharpen_radius), fmt: Fmt::Decimal(1) },
    Field { label: "Sharpen Masking", get: |s| Some(s.detail.sharpen_masking), fmt: Fmt::Plain },
    Field { label: "Noise Reduction", get: |s| Some(s.detail.luminance_noise), fmt: Fmt::Plain },
    Field { label: "Color Noise Reduction", get: |s| Some(s.detail.color_noise), fmt: Fmt::Plain },
    Field { label: "Straighten", get: |s| Some(s.crop.angle), fmt: Fmt::Degrees },
    Field { label: "Texture", get: |s| Some(s.effects.texture), fmt: Fmt::Signed(0) },
    Field { label: "Clarity", get: |s| Some(s.effects.clarity), fmt: Fmt::Signed(0) },
    Field { label: "Dehaze", get: |s| Some(s.effects.dehaze), fmt: Fmt::Signed(0) },
    Field { label: "Vignette", get: |s| Some(s.effects.vignette.amount), fmt: Fmt::Signed(0) },
    Field { label: "Vignette Midpoint", get: |s| Some(s.effects.vignette.midpoint), fmt: Fmt::Plain },
    Field { label: "Vignette Feather", get: |s| Some(s.effects.vignette.feather), fmt: Fmt::Plain },
    Field { label: "Grain", get: |s| Some(s.effects.grain.amount), fmt: Fmt::Plain },
    Field { label: "Grain Size", get: |s| Some(s.effects.grain.size), fmt: Fmt::Plain },
    Field { label: "Lens Distortion", get: |s| Some(s.lens.distortion), fmt: Fmt::Plain },
    Field { label: "Lens Vignetting", get: |s| Some(s.lens.vignetting), fmt: Fmt::Plain },
    Field { label: "Manual Distortion", get: |s| Some(s.lens.manual_distortion), fmt: Fmt::Signed(0) },
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
    let mut changed: Vec<&Field> = all_fields().filter(|f| (f.get)(prev) != (f.get)(next)).collect();
    let mut curves: Vec<&str> = CURVES.iter().filter(|(_, c)| c(prev) != c(next)).map(|(l, _)| *l).collect();
    // Picking a preset (or switching to Custom) names the mode, rather than the temperature
    // and tint it set.
    let mode = |w: &WhiteBalance| match (w.preset, w.temperature, w.tint) {
        (Some(p), ..) => Some(p.label()),
        (None, None, None) => None,
        _ => Some("Custom"),
    };
    let (was, now) = (mode(&prev.white_balance), mode(&next.white_balance));
    let preset_label = now.filter(|_| was != now).map(|m| format!("White Balance {m}"));
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
        curves.push(if b.profile { "Lens Profile On" } else { "Lens Profile Off" });
    }
    if a.chromatic != b.chromatic {
        curves.push(if b.chromatic { "Chromatic Aberration On" } else { "Chromatic Aberration Off" });
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
        (fs, cs) if fs.len() + cs.len() <= 3 => {
            fs.iter().map(|f| f.label).chain(cs.iter().copied()).collect::<Vec<_>>().join(", ")
        }
        (fs, cs) => format!("{} adjustments", fs.len() + cs.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(json.contains("\"clarity\":20") && json.contains("\"dehaze\":10"), "{json}");
        assert_eq!(serde_json::from_str::<EditState>(&json).unwrap(), s);
    }

    #[test]
    fn wb_presets_sit_where_their_lights_do() {
        let v = |p: WbPreset| p.values();
        assert_eq!(v(WbPreset::Daylight), (5500.0, -11.0));
        assert_eq!(v(WbPreset::Tungsten), (2850.0, 0.0), "illuminant A is on the Planckian locus");
        let t: Vec<f32> = [WbPreset::Tungsten, WbPreset::Fluorescent, WbPreset::Daylight, WbPreset::Flash, WbPreset::Cloudy, WbPreset::Shade]
            .iter()
            .map(|p| p.values().0)
            .collect();
        assert!(t.windows(2).all(|w| w[0] < w[1]), "warm to cool: {t:?}");
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
        assert_eq!(describe_change(&a, &c), "Shadows, Blacks, White Balance Custom");
        c.light.whites = 5.0;
        assert_eq!(describe_change(&a, &c), "4 adjustments");

        let mut w = a.clone();
        w.white_balance = WhiteBalance { temperature: Some(2850.0), tint: Some(0.0), preset: Some(WbPreset::Tungsten) };
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
        assert_eq!(describe_change(&EditState { curve: e.curve.clone(), ..a.clone() }, &e), "Vignette Midpoint 30");
        let mut g = a.clone();
        g.crop.angle = 2.34;
        assert_eq!(describe_change(&a, &g), "Straighten +2.3°");
        g.crop.rect = [0.1, 0.1, 0.8, 0.8];
        assert_eq!(describe_change(&a, &g), "Straighten, Crop");
        let mut f = a.clone();
        f.detail.sharpen_radius = 1.5;
        assert_eq!(describe_change(&a, &f), "Sharpen Radius 1.5");
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
