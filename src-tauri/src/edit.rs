//! The edit recipe for a photo. Editing never touches the original file:
//! the recipe is stored next to it in the library and applied on display.

use serde::{Deserialize, Serialize};

use crate::film::Film;

/// A tone curve as control points from (0, 0) black to (1, 1) white.
pub type CurvePoints = Vec<[f32; 2]>;

fn straight() -> CurvePoints {
    vec![[0.0, 0.0], [1.0, 1.0]]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Curves {
    pub master: CurvePoints,
    pub red: CurvePoints,
    pub green: CurvePoints,
    pub blue: CurvePoints,
}

impl Default for Curves {
    fn default() -> Self {
        Self { master: straight(), red: straight(), green: straight(), blue: straight() }
    }
}

/// One colour range in the colour mixer. Each value runs -100..100.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Band {
    pub hue: f32,
    pub saturation: f32,
    pub luminance: f32,
}

/// The part of the photo that is kept, on the photo after its quarter-turns
/// and flips: centre and size as fractions of that picture. The rectangle is
/// tilted by the straighten angle about its own centre.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for Crop {
    fn default() -> Self {
        Self { x: 0.5, y: 0.5, width: 1.0, height: 1.0 }
    }
}

/// The sliders a mask can carry. Each is added to the photo's own setting
/// wherever the mask covers it, in the same units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LocalAdjustments {
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub temperature: f32,
    pub tint: f32,
    pub vibrance: f32,
    pub saturation: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub sharpening: f32,
    pub noise_reduction: f32,
}

/// One brush stroke. Positions are on the photo file, 0..1 across and down.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Stroke {
    pub points: Vec<[f32; 2]>,
    /// The brush's radius, as a share of the photo's longer side.
    pub radius: f32,
    /// 0..1: how much of the radius fades out rather than covering fully.
    pub feather: f32,
    /// 0..1: how much the stroke covers at its centre.
    pub strength: f32,
    /// Takes coverage away instead of adding it.
    pub erase: bool,
}

impl Default for Stroke {
    fn default() -> Self {
        Self { points: Vec::new(), radius: 0.02, feather: 0.5, strength: 1.0, erase: false }
    }
}

/// What a part of a mask covers. Positions are on the photo file, 0..1
/// across and down; sizes are shares of its longer side; angles are degrees
/// clockwise as the file is stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Shape {
    /// Painted by hand.
    Brush { strokes: Vec<Stroke> },
    /// Full effect at `from`, fading to none at `to`, in parallel bands.
    Linear { from: [f32; 2], to: [f32; 2] },
    /// An ellipse, fading out over its outer `feather` (0..1) share.
    Radial { center: [f32; 2], radius: [f32; 2], angle: f32, feather: f32 },
    /// The parts of the photo within a range of brightness (0 black, 1 white),
    /// fading out over `smoothness` either side.
    Luminance { low: f32, high: f32, smoothness: f32 },
    /// The photo's main subject, found by a segmentation model. Nothing is
    /// stored: each photo's own subject is found, so a pasted mask fits it.
    Subject,
    /// The photo's sky, found the same way.
    Sky,
    /// The object inside a circle drawn on the photo (`points`, a closed
    /// outline), found by the subject model looking only there.
    Object { points: Vec<[f32; 2]> },
}

/// How a part combines with the parts before it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    Add,
    Subtract,
    /// Keeps only what is covered both by the parts before and by this one.
    Intersect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskPart {
    #[serde(default)]
    pub mode: Mode,
    pub shape: Shape,
}

/// A local adjustment: an area of the photo, and the sliders that apply there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Mask {
    /// Tells masks apart while they are edited; unique within a recipe.
    pub id: u32,
    pub name: String,
    /// A hidden mask stays in the recipe but has no effect.
    pub visible: bool,
    /// Applies everywhere the parts do not cover instead.
    pub invert: bool,
    pub parts: Vec<MaskPart>,
    pub adjustments: LocalAdjustments,
}

impl Default for Mask {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            visible: true,
            invert: false,
            parts: Vec::new(),
            adjustments: LocalAdjustments::default(),
        }
    }
}

/// How far Exposure goes either way, in stops: the photo's own slider, each
/// mask's, and all of them added together.
pub const MAX_EXPOSURE: f32 = 5.0;

/// How far the photo's own Temperature and Tint go either way. Past 100 is
/// only reached on RAWs, whose sliders read in Kelvin (`white.rs`): enough
/// for 2000 K to 50000 K from whatever the camera balanced for.
pub const MAX_WHITE_BALANCE: f32 = 2000.0;

/// Every slider in the editor. Zero everywhere means "as shot".
/// Exposure is in stops (-5..5); Temperature and Tint are relative to the
/// camera's white balance (-2000..2000, `MAX_WHITE_BALANCE`); everything
/// else runs -100..100 or 0..100.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Adjustments {
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,

    pub temperature: f32,
    pub tint: f32,
    pub vibrance: f32,
    pub saturation: f32,

    pub curves: Curves,
    /// Red, orange, yellow, green, aqua, blue, purple, magenta.
    pub mixer: [Band; 8],

    pub clarity: f32,
    pub dehaze: f32,
    pub sharpening: f32,
    pub noise_reduction: f32,

    pub vignette: f32,
    pub grain: f32,

    pub crop: Crop,
    /// Degrees, -45..45; positive turns the picture clockwise.
    pub straighten: f32,
    /// Quarter-turns clockwise, 0..3.
    pub rotation: u8,
    /// Mirrors applied after the quarter-turns, so they flip what you see.
    pub flip_horizontal: bool,
    pub flip_vertical: bool,

    /// Local adjustments, laid over the rest in order.
    pub masks: Vec<Mask>,

    /// For scans of film negatives: turns them into positives before anything else.
    pub film: Film,
}

impl Adjustments {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Takes the crop, straightening, turns and flips from `other`, leaving
    /// the tone and colour adjustments as they are.
    pub fn keep_framing_of(&mut self, other: &Adjustments) {
        self.crop = other.crop;
        self.straighten = other.straighten;
        self.rotation = other.rotation;
        self.flip_horizontal = other.flip_horizontal;
        self.flip_vertical = other.flip_vertical;
    }

    /// Reads a stored recipe. Unknown or damaged recipes fall back to "as
    /// shot" rather than making the photo unopenable.
    pub fn from_json(json: Option<&str>) -> Self {
        json.and_then(|json| serde_json::from_str(json).ok()).unwrap_or_default()
    }

    /// The form stored in the library: nothing at all for an unedited photo.
    pub fn to_json(&self) -> Option<String> {
        (!self.is_default()).then(|| serde_json::to_string(self).expect("adjustments are plain data"))
    }
}

/// Samples a curve at `N` evenly spaced inputs.
///
/// Points are joined with a monotone cubic spline: smooth through every
/// point, and never overshooting between them, so a gentle S stays an S.
pub fn curve_table<const N: usize>(points: &[[f32; 2]]) -> [f32; N] {
    let mut points: Vec<[f32; 2]> =
        points.iter().filter(|p| p[0].is_finite() && p[1].is_finite()).map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)]).collect();
    points.sort_by(|a, b| a[0].total_cmp(&b[0]));
    points.dedup_by(|b, a| b[0] - a[0] < 1e-4);
    let identity = |i: usize| i as f32 / (N - 1) as f32;
    match points.len() {
        0 => return std::array::from_fn(identity),
        1 => return [points[0][1]; N],
        _ => {}
    }

    // Fritsch-Carlson tangents.
    let n = points.len();
    let slopes: Vec<f32> = points.windows(2).map(|w| (w[1][1] - w[0][1]) / (w[1][0] - w[0][0])).collect();
    let mut tangents = vec![0.0f32; n];
    tangents[0] = slopes[0];
    tangents[n - 1] = slopes[n - 2];
    for i in 1..n - 1 {
        tangents[i] = if slopes[i - 1] * slopes[i] <= 0.0 { 0.0 } else { (slopes[i - 1] + slopes[i]) / 2.0 };
    }
    for i in 0..n - 1 {
        if slopes[i] == 0.0 {
            tangents[i] = 0.0;
            tangents[i + 1] = 0.0;
            continue;
        }
        let (a, b) = (tangents[i] / slopes[i], tangents[i + 1] / slopes[i]);
        let length = a * a + b * b;
        if length > 9.0 {
            let shrink = 3.0 / length.sqrt();
            tangents[i] = shrink * a * slopes[i];
            tangents[i + 1] = shrink * b * slopes[i];
        }
    }

    std::array::from_fn(|i| {
        let x = identity(i);
        if x <= points[0][0] {
            return points[0][1];
        }
        if x >= points[n - 1][0] {
            return points[n - 1][1];
        }
        let k = points.partition_point(|p| p[0] <= x) - 1;
        let width = points[k + 1][0] - points[k][0];
        let t = (x - points[k][0]) / width;
        let (t2, t3) = (t * t, t * t * t);
        let y = (2.0 * t3 - 3.0 * t2 + 1.0) * points[k][1]
            + (t3 - 2.0 * t2 + t) * width * tangents[k]
            + (-2.0 * t3 + 3.0 * t2) * points[k + 1][1]
            + (t3 - t2) * width * tangents[k + 1];
        y.clamp(0.0, 1.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unedited_photos_store_nothing() {
        assert_eq!(Adjustments::default().to_json(), None);
        let edited = Adjustments { exposure: 0.5, ..Default::default() };
        let json = edited.to_json().unwrap();
        assert_eq!(Adjustments::from_json(Some(&json)), edited);
        assert!(Adjustments::from_json(Some("not json")).is_default());
        // Recipes saved before a slider existed still load.
        assert_eq!(Adjustments::from_json(Some(r#"{"exposure":0.5}"#)), edited);
    }

    #[test]
    fn film_settings_are_kept_and_old_recipes_have_none() {
        use crate::film::{Kind, Range};
        assert_eq!(Adjustments::from_json(Some(r#"{"exposure":0.5}"#)).film, Film::default());
        let range = Range { low: [0.1, 0.2, 0.3], high: [1.0, 1.1, 1.2] };
        let film = Film { kind: Kind::BlackAndWhite, base: Some([0.6, 0.5, 0.55]), range: Some(range) };
        let scan = Adjustments { film, ..Default::default() };
        let json = scan.to_json().unwrap();
        assert!(json.contains(r#""kind":"blackAndWhite""#), "{json}");
        assert_eq!(Adjustments::from_json(Some(&json)), scan);
        // The interface sends a guessed base and an unbalanced range as null.
        let guessed = Adjustments::from_json(Some(r#"{"film":{"kind":"colour","base":null,"range":null}}"#));
        assert_eq!(guessed.film, Film { kind: Kind::Colour, base: None, range: None });
    }

    #[test]
    fn a_straight_curve_changes_nothing() {
        let table: [f32; 256] = curve_table(&[[0.0, 0.0], [1.0, 1.0]]);
        for (i, value) in table.iter().enumerate() {
            assert!((value - i as f32 / 255.0).abs() < 1e-5);
        }
    }

    #[test]
    fn curves_pass_through_their_points_without_overshooting() {
        let points = [[0.0, 0.0], [0.25, 0.1], [0.5, 0.5], [0.75, 0.95], [1.0, 1.0]];
        let table: [f32; 1001] = curve_table(&points);
        for [x, y] in points {
            assert!((table[(x * 1000.0) as usize] - y).abs() < 1e-4, "passes through ({x}, {y})");
        }
        assert!(table.windows(2).all(|w| w[1] >= w[0] - 1e-6), "rising points give a rising curve");

        // A lifted black point and lowered white point hold flat outside the ends.
        let faded: [f32; 256] = curve_table(&[[0.1, 0.2], [0.9, 0.8]]);
        assert_eq!((faded[0], faded[255]), (0.2, 0.8));
    }
}
