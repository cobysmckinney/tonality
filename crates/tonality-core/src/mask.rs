//! Masks and local adjustments.
//!
//! A [`Mask`] selects part of the image (a linear or radial gradient, or painted brush
//! strokes), optionally narrowed to a luminance range, and carries its own [`LocalAdjust`]
//! settings. These are added to the global settings, weighted by the mask.
//!
//! Positions are in source uv (0..1 across the unturned, uncropped image), so masks stay on
//! the same part of the photo whatever the framing. Sizes are fractions of the source's long
//! edge, so circles stay circles.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Mask {
    pub name: String,
    /// Hidden masks are kept but have no effect.
    pub enabled: bool,
    pub shape: MaskShape,
    /// Select everything except the shape.
    pub invert: bool,
    /// Overall strength, 0..=100.
    pub amount: f32,
    pub range: LuminanceRange,
    pub adjust: LocalAdjust,
}

impl Default for Mask {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            shape: MaskShape::default(),
            invert: false,
            amount: 100.0,
            range: LuminanceRange::default(),
            adjust: LocalAdjust::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum MaskShape {
    /// Full effect at `from`, fading to none at `to` (and beyond).
    Linear { from: [f32; 2], to: [f32; 2] },
    /// An ellipse: full effect inside, fading out over the outer `feather` (0..=1) of its radius.
    /// `radius` is in long-edge units; `angle` (degrees) turns it clockwise.
    Radial { center: [f32; 2], radius: [f32; 2], angle: f32, feather: f32 },
    /// Painted strokes, applied in order.
    Brush { strokes: Vec<Stroke> },
    /// Found by an on-device model. The raster itself is cached outside the edit (it is
    /// regenerated from the photo when missing); `refine` strokes are painted over it.
    Ai {
        target: AiTarget,
        /// Model that produced it (`"subject@1"`), so a model update never changes an edit.
        model: String,
        #[serde(default)]
        refine: Vec<Stroke>,
    },
}

/// What an AI mask selects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum AiTarget {
    /// The photo's main subject (inverted, the background).
    Subject,
    Sky,
    People,
    /// Whatever was circled: `lasso` is a closed loop in source uv.
    Object { lasso: Vec<[f32; 2]> },
}

impl AiTarget {
    pub fn label(&self) -> &'static str {
        match self {
            AiTarget::Subject => "Subject",
            AiTarget::Sky => "Sky",
            AiTarget::People => "People",
            AiTarget::Object { .. } => "Object",
        }
    }

    /// Stable name for the cached raster: the target, plus a hash of the lasso for objects.
    pub fn cache_key(&self) -> String {
        match self {
            AiTarget::Object { lasso } => {
                let mut h = blake3::Hasher::new();
                for p in lasso {
                    h.update(&p[0].to_le_bytes());
                    h.update(&p[1].to_le_bytes());
                }
                format!("object-{}", &h.finalize().to_hex()[..16])
            }
            t => t.label().to_ascii_lowercase(),
        }
    }
}

impl Default for MaskShape {
    fn default() -> Self {
        MaskShape::Radial { center: [0.5, 0.5], radius: [0.2, 0.2], angle: 0.0, feather: 0.5 }
    }
}

impl MaskShape {
    pub fn label(&self) -> &'static str {
        match self {
            MaskShape::Linear { .. } => "Linear",
            MaskShape::Radial { .. } => "Radial",
            MaskShape::Brush { .. } => "Brush",
            MaskShape::Ai { target, .. } => target.label(),
        }
    }

    /// Whether the mask is a raster layer (brush or AI) rather than a formula.
    pub fn is_raster(&self) -> bool {
        matches!(self, MaskShape::Brush { .. } | MaskShape::Ai { .. })
    }
}

/// One brush stroke: a polyline swept by a soft round brush.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Stroke {
    /// Source uv.
    pub points: Vec<[f32; 2]>,
    /// Long-edge units.
    pub radius: f32,
    /// Soft part of the brush, 0..=1 of its radius.
    pub feather: f32,
    /// 0..=1. Strokes don't build up: overlapping strokes take the stronger.
    pub strength: f32,
    /// Remove from the mask instead of adding.
    pub erase: bool,
}

impl Default for Stroke {
    fn default() -> Self {
        Self { points: Vec::new(), radius: 0.03, feather: 0.5, strength: 1.0, erase: false }
    }
}

/// Restrict a mask to a band of (display) luminance, 0..=100.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct LuminanceRange {
    pub low: f32,
    pub high: f32,
    /// Softness of both edges.
    pub smoothness: f32,
}

impl Default for LuminanceRange {
    fn default() -> Self {
        Self { low: 0.0, high: 100.0, smoothness: 20.0 }
    }
}

impl LuminanceRange {
    pub fn is_full(&self) -> bool {
        self.low <= 0.0 && self.high >= 100.0
    }
}

/// Settings a mask applies, added to the global ones.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct LocalAdjust {
    /// Stops, -4..=4.
    pub exposure: f32,
    /// -100..=100 from here on.
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    /// Cooler (-) / warmer (+).
    pub temperature: f32,
    /// Green (-) / magenta (+).
    pub tint: f32,
    pub saturation: f32,
    /// Saturation that favours muted colours.
    pub vibrance: f32,
    /// Hue shift: -100..=100 is -30°..=30°.
    pub hue: f32,
    /// Sharpening added to the global amount; negative softens.
    pub sharpness: f32,
    /// Luminance noise reduction added to (or, negative, taken from) the global amount.
    pub noise: f32,
    /// Colour tint: hue in degrees (0..360, OkLCh, as the colour mixer), …
    pub color_hue: f32,
    /// … and how strongly it tints, 0..=100 (0: off).
    pub color_amount: f32,
}

// ---- brush rasterisation ------------------------------------------------------------------------
// Brush masks are rasterised to a small single-channel image per mask, which the shader samples.
// The preview paints live strokes with the same formula (`ui/lib/edit/brush.ts`); committed
// strokes are always rasterised here, so preview and export share one source.

/// Long edge of a brush mask raster.
pub const MASK_EDGE: u32 = 1024;

/// Raster size for a source of `src_w × src_h`.
pub fn mask_size(src_w: u32, src_h: u32) -> (u32, u32) {
    let long = src_w.max(src_h).max(1) as f32;
    let s = MASK_EDGE as f32 / long;
    (((src_w as f32 * s).round() as u32).max(1), ((src_h as f32 * s).round() as u32).max(1))
}

/// The rasters of every raster mask in `masks` (brush and AI, in order), as layers of one
/// `width × height` 8-bit image. Hidden masks are included, so a mask's layer is its index
/// among raster masks.
pub struct BrushLayers {
    pub width: u32,
    pub height: u32,
    pub layers: u32,
    pub data: Vec<u8>,
    /// Indices (into `masks`) of AI masks whose model raster wasn't available: their layer
    /// holds only the refine strokes until the raster is generated.
    pub missing: Vec<usize>,
}

/// Brush layers only (AI masks get empty layers and are reported missing).
pub fn brush_layers(masks: &[Mask], src_w: u32, src_h: u32) -> BrushLayers {
    mask_layers(masks, src_w, src_h, |_, _| None)
}

/// Every raster layer. `ai(index, mask)` supplies an AI mask's model raster, already
/// [`mask_size`] and in the masks' uv space (see [`crate::lens::warp_to_uv`]).
pub fn mask_layers(
    masks: &[Mask],
    src_w: u32,
    src_h: u32,
    mut ai: impl FnMut(usize, &Mask) -> Option<Vec<u8>>,
) -> BrushLayers {
    let (width, height) = mask_size(src_w, src_h);
    let len = (width * height) as usize;
    let mut data = Vec::new();
    let mut layers = 0;
    let mut missing = Vec::new();
    for (i, m) in masks.iter().enumerate() {
        let (mut layer, strokes) = match &m.shape {
            MaskShape::Brush { strokes } => (vec![0u8; len], strokes),
            MaskShape::Ai { refine, .. } => {
                let base = ai(i, m).filter(|r| r.len() == len);
                if base.is_none() {
                    missing.push(i);
                }
                (base.unwrap_or_else(|| vec![0u8; len]), refine)
            }
            _ => continue,
        };
        for s in strokes {
            paint(&mut layer, width, height, s);
        }
        data.extend_from_slice(&layer);
        layers += 1;
    }
    BrushLayers { width, height, layers, data, missing }
}

/// Paint one stroke into `layer`.
pub fn paint(layer: &mut [u8], w: u32, h: u32, s: &Stroke) {
    let n = s.points.len();
    for i in 0..n {
        let a = s.points[i.saturating_sub(1)];
        let b = s.points[i];
        if i > 0 || n == 1 {
            stamp_segment(layer, w, h, a, b, s);
        }
    }
}

/// Sweep the brush from `a` to `b` (uv). Painting a polyline segment by segment gives the same
/// result as painting it at once, because the brush only ever takes the max (or, erasing, min).
pub fn stamp_segment(layer: &mut [u8], w: u32, h: u32, a: [f32; 2], b: [f32; 2], s: &Stroke) {
    let long = w.max(h) as f32;
    let r = (s.radius * long).max(0.5);
    let inner = (r * (1.0 - s.feather.clamp(0.0, 1.0))).min(r - 1.0);
    let (ax, ay) = (a[0] * w as f32, a[1] * h as f32);
    let (bx, by) = (b[0] * w as f32, b[1] * h as f32);
    let x0 = ((ax.min(bx) - r - 1.0).floor().max(0.0)) as u32;
    let y0 = ((ay.min(by) - r - 1.0).floor().max(0.0)) as u32;
    let x1 = ((ax.max(bx) + r + 1.0).ceil().min(w as f32)) as u32;
    let y1 = ((ay.max(by) + r + 1.0).ceil().min(h as f32)) as u32;
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let strength = s.strength.clamp(0.0, 1.0);
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = (x as f32 + 0.5 - ax, y as f32 + 0.5 - ay);
            let t = if len2 > 0.0 { ((px * dx + py * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let (ex, ey) = (px - t * dx, py - t * dy);
            let d = (ex * ex + ey * ey).sqrt();
            if d >= r {
                continue;
            }
            let v = strength * (1.0 - smoothstep(inner, r, d));
            let q = (v * 255.0).round() as u8;
            let p = &mut layer[(y * w + x) as usize];
            *p = if s.erase { (*p).min(255 - q) } else { (*p).max(q) };
        }
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---- change summaries ------------------------------------------------------------------------

/// What changed in the masks, as `(short, full)` labels: the short form names the mask, for
/// lists ("Radial 1, Exposure"); the full one says what happened ("Radial 1 Exposure +0.50").
pub(crate) fn describe_masks(prev: &[Mask], next: &[Mask]) -> Vec<(String, String)> {
    if prev == next {
        return Vec::new();
    }
    let one = |s: String| vec![(s.clone(), s)];
    if next.len() == prev.len() + 1 && next[..prev.len()] == *prev {
        return one(format!("Add {}", next[prev.len()].name));
    }
    if next.len() + 1 == prev.len() {
        let i = (0..next.len()).find(|&i| prev[i] != next[i]).unwrap_or(next.len());
        if prev[..i] == next[..i] && prev[i + 1..] == next[i..] {
            return one(format!("Delete {}", prev[i].name));
        }
    }
    if prev.len() != next.len() {
        return one("Masks".into());
    }
    prev.iter().zip(next).filter(|(a, b)| a != b).map(|(a, b)| (b.name.clone(), describe_mask(a, b))).collect()
}

fn describe_mask(a: &Mask, b: &Mask) -> String {
    let name = &b.name;
    if a.name != b.name {
        return format!("Rename {} to {}", a.name, b.name);
    }
    if a.enabled != b.enabled {
        return format!("{} {name}", if b.enabled { "Show" } else { "Hide" });
    }
    let mut what: Vec<String> = Vec::new();
    if a.shape != b.shape {
        what.push(match (&a.shape, &b.shape) {
            (MaskShape::Brush { strokes: sa }, MaskShape::Brush { strokes: sb })
            | (MaskShape::Ai { refine: sa, .. }, MaskShape::Ai { refine: sb, .. })
                if sb.len() > sa.len() =>
            {
                if sb.last().is_some_and(|s| s.erase) { "Erase".into() } else { "Paint".into() }
            }
            (MaskShape::Ai { target: ta, .. }, MaskShape::Ai { target: tb, .. }) if ta != tb => "Reselect".into(),
            _ => "Shape".into(),
        });
    }
    if a.invert != b.invert {
        what.push(if b.invert { "Invert".into() } else { "Uninvert".into() });
    }
    if a.amount != b.amount {
        what.push(format!("Amount {}", b.amount.round() as i64));
    }
    if a.range != b.range {
        what.push("Luminance Range".into());
    }
    type Getter = fn(&LocalAdjust) -> f32;
    const FIELDS: &[(&str, Getter, usize)] = &[
        ("Exposure", |l| l.exposure, 2),
        ("Contrast", |l| l.contrast, 0),
        ("Highlights", |l| l.highlights, 0),
        ("Shadows", |l| l.shadows, 0),
        ("Whites", |l| l.whites, 0),
        ("Blacks", |l| l.blacks, 0),
        ("Temperature", |l| l.temperature, 0),
        ("Tint", |l| l.tint, 0),
        ("Saturation", |l| l.saturation, 0),
        ("Vibrance", |l| l.vibrance, 0),
        ("Hue", |l| l.hue, 0),
        ("Sharpness", |l| l.sharpness, 0),
        ("Noise", |l| l.noise, 0),
        ("Color", |l| l.color_amount, 0),
    ];
    for (label, get, decimals) in FIELDS {
        let v = get(&b.adjust);
        if get(&a.adjust) != v {
            what.push(format!("{label} {}", crate::edit::signed(v, *decimals)));
        }
    }
    if a.adjust.color_hue != b.adjust.color_hue {
        what.push(format!("Color Hue {}°", b.adjust.color_hue.round() as i64));
    }
    match what.as_slice() {
        [w] if w == "Paint" || w == "Erase" || w == "Invert" || w == "Uninvert" || w == "Reselect" => {
            format!("{w} {name}")
        }
        [w] => format!("{name} {w}"),
        ws => format!("{name}: {} adjustments", ws.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(points: Vec<[f32; 2]>) -> Stroke {
        Stroke { points, radius: 0.05, feather: 0.5, strength: 1.0, erase: false }
    }

    fn brush(strokes: Vec<Stroke>) -> Mask {
        Mask { name: "Brush 1".into(), shape: MaskShape::Brush { strokes }, ..Mask::default() }
    }

    #[test]
    fn raster_size_follows_the_source() {
        assert_eq!(mask_size(6000, 4000), (1024, 683));
        assert_eq!(mask_size(4000, 6000), (683, 1024));
    }

    #[test]
    fn strokes_paint_and_erase() {
        let (w, h) = (200, 100);
        let mut layer = vec![0u8; (w * h) as usize];
        paint(&mut layer, w, h, &stroke(vec![[0.2, 0.5], [0.8, 0.5]]));
        let at = |l: &[u8], x: f32, y: f32| l[((y * h as f32) as u32 * w + (x * w as f32) as u32) as usize];
        assert_eq!(at(&layer, 0.5, 0.5), 255);
        assert_eq!(at(&layer, 0.5, 0.1), 0);
        // The soft edge falls off monotonically.
        let edge: Vec<u8> = (0..12).map(|i| layer[((50 + i) * w + 100) as usize]).collect();
        assert!(edge.windows(2).all(|p| p[0] >= p[1]), "{edge:?}");
        assert!(edge.iter().any(|&v| v > 0 && v < 255));
        let erase = Stroke { erase: true, ..stroke(vec![[0.5, 0.5]]) };
        paint(&mut layer, w, h, &erase);
        assert_eq!(at(&layer, 0.5, 0.5), 0);
        assert_eq!(at(&layer, 0.25, 0.5), 255);
    }

    #[test]
    fn segment_by_segment_matches_whole_stroke() {
        let (w, h) = (128, 96);
        let s = stroke(vec![[0.1, 0.2], [0.4, 0.6], [0.7, 0.3], [0.9, 0.8]]);
        let mut whole = vec![0u8; (w * h) as usize];
        paint(&mut whole, w, h, &s);
        let mut parts = vec![0u8; (w * h) as usize];
        for i in 1..s.points.len() {
            stamp_segment(&mut parts, w, h, s.points[i - 1], s.points[i], &s);
        }
        assert_eq!(whole, parts);
    }

    /// The same numbers are checked by `ui/lib/edit/brush.test.ts`, so the live preview paints
    /// exactly what this rasteriser produces.
    #[test]
    fn reference_values_for_the_preview() {
        let (w, h) = (64, 48);
        let mut layer = vec![0u8; (w * h) as usize];
        let s = Stroke { points: vec![[0.25, 0.5], [0.75, 0.25]], radius: 0.1, feather: 0.7, strength: 0.8, erase: false };
        paint(&mut layer, w, h, &s);
        let sum: u32 = layer.iter().map(|&v| v as u32).sum();
        let samples: Vec<u8> = [(16, 24), (16, 27), (20, 26), (32, 21), (40, 19), (16, 30), (50, 9), (11, 24)]
            .iter()
            .map(|&(x, y)| layer[(y * w + x) as usize])
            .collect();
        assert_eq!((sum, samples), (REF_SUM, REF_SAMPLES.to_vec()));
    }
    const REF_SUM: u32 = 69740;
    const REF_SAMPLES: [u8; 8] = [204, 144, 118, 149, 86, 0, 144, 77];

    #[test]
    fn layers_in_mask_order() {
        let masks = vec![
            brush(vec![stroke(vec![[0.5, 0.5]])]),
            Mask::default(),
            brush(vec![]),
        ];
        let l = brush_layers(&masks, 600, 400);
        assert_eq!((l.width, l.height, l.layers), (1024, 683, 2));
        assert_eq!(l.data.len(), 1024 * 683 * 2);
        assert!(l.data[..1024 * 683].contains(&255));
        assert!(l.data[1024 * 683..].iter().all(|&v| v == 0));
    }

    #[test]
    fn describes_mask_changes() {
        let r = Mask { name: "Radial 1".into(), ..Mask::default() };
        let full = |a: Vec<Mask>, b: Vec<Mask>| describe_masks(&a, &b).into_iter().map(|(_, f)| f).collect::<Vec<_>>();
        assert_eq!(full(vec![], vec![r.clone()]), ["Add Radial 1"]);
        assert_eq!(full(vec![r.clone()], vec![]), ["Delete Radial 1"]);
        let mut e = r.clone();
        e.adjust.exposure = 0.5;
        assert_eq!(full(vec![r.clone()], vec![e.clone()]), ["Radial 1 Exposure +0.50"]);
        e.invert = true;
        assert_eq!(full(vec![r.clone()], vec![e.clone()]), ["Radial 1: 2 adjustments"]);
        let b0 = brush(vec![]);
        let b1 = brush(vec![stroke(vec![[0.5, 0.5]])]);
        assert_eq!(full(vec![b0.clone()], vec![b1.clone()]), ["Paint Brush 1"]);
        let mut hidden = r.clone();
        hidden.enabled = false;
        assert_eq!(full(vec![r.clone(), b0.clone()], vec![hidden, b0.clone()]), ["Hide Radial 1"]);
        assert_eq!(full(vec![r.clone(), b0.clone(), b1.clone()], vec![r.clone(), b1.clone()]), ["Delete Brush 1"]);
    }

    #[test]
    fn json_shape() {
        let m = Mask { name: "Linear 1".into(), shape: MaskShape::Linear { from: [0.5, 0.0], to: [0.5, 0.5] }, ..Mask::default() };
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains(r#""shape":{"kind":"linear","from":[0.5,0.0],"to":[0.5,0.5]}"#), "{j}");
        let back: Mask = serde_json::from_str(&j).unwrap();
        assert_eq!(back, m);
    }

    fn ai(target: AiTarget) -> Mask {
        Mask { shape: MaskShape::Ai { target, model: "subject@1".into(), refine: vec![] }, ..Mask::default() }
    }

    #[test]
    fn ai_json_shape() {
        let m = ai(AiTarget::Sky);
        let j = serde_json::to_string(&m.shape).unwrap();
        assert_eq!(j, r#"{"kind":"ai","target":{"kind":"sky"},"model":"subject@1","refine":[]}"#);
        // `refine` may be absent.
        let back: MaskShape = serde_json::from_str(r#"{"kind":"ai","target":{"kind":"object","lasso":[[0.1,0.2]]},"model":"sam@1"}"#).unwrap();
        assert!(matches!(back, MaskShape::Ai { ref refine, .. } if refine.is_empty()));
    }

    #[test]
    fn cache_keys_are_stable_and_lasso_specific() {
        assert_eq!(AiTarget::Sky.cache_key(), "sky");
        let a = AiTarget::Object { lasso: vec![[0.1, 0.2], [0.3, 0.4]] };
        let b = AiTarget::Object { lasso: vec![[0.1, 0.2], [0.3, 0.41]] };
        assert_eq!(a.cache_key(), a.clone().cache_key());
        assert_ne!(a.cache_key(), b.cache_key());
        assert!(a.cache_key().starts_with("object-"));
    }

    #[test]
    fn ai_layers_sit_between_brush_layers_and_report_missing() {
        let masks = vec![brush(vec![stroke(vec![[0.5, 0.5]])]), Mask::default(), ai(AiTarget::Subject), ai(AiTarget::Sky)];
        let (w, h) = mask_size(64, 32);
        let len = (w * h) as usize;
        let l = mask_layers(&masks, 64, 32, |i, _| (i == 2).then(|| vec![200u8; len]));
        assert_eq!(l.layers, 3, "brush, subject, sky; the radial has no layer");
        assert_eq!(l.missing, [3]);
        assert!(l.data[len..2 * len].iter().all(|&v| v == 200), "subject raster");
        assert!(l.data[2 * len..].iter().all(|&v| v == 0), "missing sky is empty");
        // Refine strokes paint over the model raster.
        let mut refined = ai(AiTarget::Subject);
        if let MaskShape::Ai { refine, .. } = &mut refined.shape {
            refine.push(Stroke { erase: true, ..stroke(vec![[0.5, 0.5]]) });
        }
        let l = mask_layers(&[refined], 64, 32, |_, _| Some(vec![255u8; len]));
        let centre = (h / 2 * w + w / 2) as usize;
        assert_eq!(l.data[centre], 0, "erased");
        assert_eq!(l.data[0], 255, "untouched corner");
    }
}
