//! Masks: where on the photo a local adjustment applies.
//!
//! Gradients and brightness ranges are simple enough for the develop shader
//! to work out for every pixel as it draws. Brush strokes are not: they are
//! painted here into a coverage map, one for each brush part, which the
//! shader reads like a picture. While you paint, only the newest part of the
//! stroke is painted again.

use std::ops::Range;

use rayon::prelude::*;

use crate::edit::{LocalAdjustments, Mask, Mode, Shape, Stroke};

/// The most masks a recipe can use; any more are left out of the picture.
pub const MAX_MASKS: usize = 8;
/// The most parts all the masks together can have.
pub const MAX_PARTS: usize = 32;
/// The most brush parts all the masks together can have: one coverage map each.
pub const MAX_BRUSHES: usize = 8;
/// The long edge of a coverage map, in its own pixels.
pub const COVERAGE_EDGE: u32 = 3072;

/// Masks laid out the way the develop shader reads them. Mirrors the mask
/// half of `Params` in develop.wgsl.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Packed {
    /// Masks in use, parts in use, the mask whose coverage to show (or -1), and a spare.
    pub counts: [f32; 4],
    /// Per mask: its sliders as light, tone, color and detail (laid out like
    /// the photo's own), then invert and how much it applies.
    pub masks: [[[f32; 4]; 5]; MAX_MASKS],
    /// Per part: kind, mode, which mask and which coverage map; then two
    /// rows of numbers that depend on the kind.
    pub parts: [[[f32; 4]; 3]; MAX_PARTS],
}

impl Default for Packed {
    fn default() -> Self {
        Self { counts: [0.0, 0.0, -1.0, 0.0], masks: [[[0.0; 4]; 5]; MAX_MASKS], parts: [[[0.0; 4]; 3]; MAX_PARTS] }
    }
}

/// Part kinds, as the shader numbers them.
const BRUSH: f32 = 1.0;
const LINEAR: f32 = 2.0;
const RADIAL: f32 = 3.0;
const LUMINANCE: f32 = 4.0;

/// A mask's sliders in the units the shader works in: stops for exposure,
/// -1..1 for the rest. Laid out like the photo's own.
fn local_rows(a: &LocalAdjustments) -> [[f32; 4]; 4] {
    let unit = |value: f32| (value / 100.0).clamp(-1.0, 1.0);
    [
        [a.exposure.clamp(-5.0, 5.0), unit(a.contrast), unit(a.highlights), unit(a.shadows)],
        [unit(a.whites), unit(a.blacks), unit(a.temperature), unit(a.tint)],
        [unit(a.vibrance), unit(a.saturation), unit(a.clarity), unit(a.dehaze)],
        [unit(a.sharpening), unit(a.noise_reduction), 0.0, 0.0],
    ]
}

/// Lays masks out for the shader, for a photo `width` x `height` (any scale).
/// `overlay` names the mask whose coverage should be shown in red. Returns
/// the strokes of each brush part too, in the order of their coverage maps.
pub fn pack(masks: &[Mask], width: u32, height: u32, overlay: Option<u32>) -> (Packed, Vec<&[Stroke]>) {
    let mut packed = Packed::default();
    let mut brushes = Vec::new();
    let long = width.max(height).max(1) as f32;
    // Shapes are measured in shares of the long side, both ways, so that
    // circles stay round on a photo that isn't square.
    let (sx, sy) = (width as f32 / long, height as f32 / long);
    let on_photo = |[x, y]: [f32; 2]| [x * sx, y * sy];

    let mut parts = 0;
    for (index, mask) in masks.iter().take(MAX_MASKS).enumerate() {
        let [light, tone, color, detail] = local_rows(&mask.adjustments);
        let info = [mask.invert as u8 as f32, mask.visible as u8 as f32, 0.0, 0.0];
        packed.masks[index] = [light, tone, color, detail, info];
        if overlay == Some(mask.id) {
            packed.counts[2] = index as f32;
        }
        for part in &mask.parts {
            if parts == MAX_PARTS {
                break;
            }
            let mode = match part.mode {
                Mode::Add => 0.0,
                Mode::Subtract => 1.0,
                Mode::Intersect => 2.0,
            };
            let row = match &part.shape {
                Shape::Brush { strokes } => {
                    if brushes.len() == MAX_BRUSHES {
                        continue;
                    }
                    brushes.push(strokes.as_slice());
                    [[BRUSH, mode, index as f32, (brushes.len() - 1) as f32], [0.0; 4], [0.0; 4]]
                }
                Shape::Linear { from, to } => {
                    let ([fx, fy], [tx, ty]) = (on_photo(*from), on_photo(*to));
                    [[LINEAR, mode, index as f32, 0.0], [fx, fy, tx, ty], [0.0; 4]]
                }
                Shape::Radial { center, radius, angle, feather } => {
                    let [cx, cy] = on_photo(*center);
                    let radius = radius.map(|r| r.max(1e-4));
                    [
                        [RADIAL, mode, index as f32, 0.0],
                        [cx, cy, radius[0], radius[1]],
                        [angle.to_radians(), feather.clamp(0.0, 1.0), 0.0, 0.0],
                    ]
                }
                Shape::Luminance { low, high, smoothness } => [
                    [LUMINANCE, mode, index as f32, 0.0],
                    [low.clamp(0.0, 1.0), high.clamp(0.0, 1.0), smoothness.clamp(0.0, 1.0), 0.0],
                    [0.0; 4],
                ],
            };
            packed.parts[parts] = row;
            parts += 1;
        }
        packed.counts[0] = (index + 1) as f32;
    }
    packed.counts[1] = parts as f32;
    (packed, brushes)
}

/// The strokes of one brush part, painted: how much of each pixel they cover, 0..1.
pub struct Coverage {
    pub width: u32,
    pub height: u32,
    pub values: Vec<f32>,
    /// What `values` shows.
    painted: Vec<Stroke>,
}

impl Coverage {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, values: vec![0.0; (width * height) as usize], painted: Vec::new() }
    }

    /// Brings the map up to date with `strokes`. Returns the rows that
    /// changed, if any.
    ///
    /// Painting a stroke twice leaves the same result as painting it once,
    /// so a stroke that has grown since it was last painted only needs its
    /// new length painted, as long as no stroke came after it.
    pub fn update(&mut self, strokes: &[Stroke]) -> Option<Range<u32>> {
        if strokes == self.painted.as_slice() {
            return None;
        }
        let had = self.painted.len();
        let grown = |old: &Stroke, new: &Stroke| {
            let same_brush = (old.radius, old.feather, old.strength, old.erase) == (new.radius, new.feather, new.strength, new.erase);
            same_brush && new.points.starts_with(&old.points)
        };
        let carries_on = strokes.len() >= had
            && (had == 0 || (strokes[..had - 1] == self.painted[..had - 1] && grown(&self.painted[had - 1], &strokes[had - 1])));

        let mut rows: Option<Range<u32>> = None;
        let mut note = |touched: Option<Range<u32>>| {
            if let Some(touched) = touched {
                rows = Some(match rows.take() {
                    Some(r) => r.start.min(touched.start)..r.end.max(touched.end),
                    None => touched,
                });
            }
        };
        if carries_on {
            for (index, stroke) in strokes.iter().enumerate().skip(had.saturating_sub(1)) {
                // Of the stroke already painted, only its last point is needed to join on.
                let from = if index + 1 == had { self.painted[index].points.len().saturating_sub(1) } else { 0 };
                note(self.paint(stroke, from));
            }
        } else {
            self.values.fill(0.0);
            note(Some(0..self.height));
            for stroke in strokes {
                self.paint(stroke, 0);
            }
        }
        self.painted = strokes.to_vec();
        rows
    }

    /// Paints `stroke` from its point `from` on. Returns the rows it touched.
    fn paint(&mut self, stroke: &Stroke, from: usize) -> Option<Range<u32>> {
        let (width, height) = (self.width as f32, self.height as f32);
        let radius = stroke.radius * width.max(height);
        if radius <= 0.0 || stroke.points.is_empty() {
            return None;
        }
        let points: Vec<[f32; 2]> = stroke.points[from.min(stroke.points.len() - 1)..]
            .iter()
            .map(|[x, y]| [x * width, y * height])
            .collect();
        // A point on its own is a dab: a segment from it to itself.
        let segments: Vec<([f32; 2], [f32; 2])> = match points.as_slice() {
            [only] => vec![(*only, *only)],
            _ => points.windows(2).map(|pair| (pair[0], pair[1])).collect(),
        };
        // At least a pixel and a half of softness, so a hard brush is still smooth-edged.
        let soft = stroke.feather.clamp(0.0, 1.0).max(1.5 / radius);
        let strength = stroke.strength.clamp(0.0, 1.0);

        let reach = |a: f32, b: f32, limit: f32| {
            let low = (a.min(b) - radius).floor().max(0.0) as u32;
            let high = ((a.max(b) + radius).ceil().max(0.0) as u32).min(limit as u32);
            low..high
        };
        let mut touched: Option<Range<u32>> = None;
        for (a, b) in segments {
            let (xs, ys) = (reach(a[0], b[0], width), reach(a[1], b[1], height));
            if xs.is_empty() || ys.is_empty() {
                continue;
            }
            let length_sq = (b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2);
            let row_len = self.width as usize;
            let rows = &mut self.values[ys.start as usize * row_len..ys.end as usize * row_len];
            rows.par_chunks_mut(row_len).enumerate().for_each(|(i, row)| {
                let y = (ys.start as usize + i) as f32 + 0.5;
                for x in xs.clone() {
                    let p = [x as f32 + 0.5, y];
                    // Distance from the pixel to the segment, as a share of the radius.
                    let t = if length_sq > 0.0 {
                        (((p[0] - a[0]) * (b[0] - a[0]) + (p[1] - a[1]) * (b[1] - a[1])) / length_sq).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let near = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                    let d = ((p[0] - near[0]).powi(2) + (p[1] - near[1]).powi(2)).sqrt() / radius;
                    if d >= 1.0 {
                        continue;
                    }
                    let value = strength * (1.0 - smoothstep(1.0 - soft, 1.0, d));
                    let cell = &mut row[x as usize];
                    *cell = if stroke.erase { cell.min(1.0 - value) } else { cell.max(value) };
                }
            });
            touched = Some(match touched {
                Some(r) => r.start.min(ys.start)..r.end.max(ys.end),
                None => ys,
            });
        }
        touched
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::MaskPart;

    fn stroke(points: &[[f32; 2]]) -> Stroke {
        Stroke { points: points.to_vec(), radius: 0.1, feather: 0.3, strength: 1.0, erase: false }
    }

    fn at(map: &Coverage, x: f32, y: f32) -> f32 {
        map.values[(y * map.height as f32) as usize * map.width as usize + (x * map.width as f32) as usize]
    }

    #[test]
    fn a_stroke_covers_its_path_and_nothing_far_from_it() {
        let mut map = Coverage::new(200, 100);
        map.update(&[stroke(&[[0.2, 0.5], [0.8, 0.5]])]);
        assert!(at(&map, 0.5, 0.5) > 0.99);
        assert!(at(&map, 0.2, 0.5) > 0.99, "the ends are round, not cut off");
        // The radius is a tenth of the long side: 20 of these pixels, both ways.
        assert!(at(&map, 0.5, 0.65) > 0.0 && at(&map, 0.5, 0.65) < 1.0, "the edge is feathered");
        assert_eq!(at(&map, 0.5, 0.75), 0.0);
        assert_eq!(at(&map, 0.95, 0.5), 0.0);
    }

    #[test]
    fn painting_as_you_go_matches_painting_all_at_once() {
        let path = [[0.1, 0.2], [0.3, 0.4], [0.5, 0.3], [0.7, 0.7], [0.9, 0.6]];
        let eraser = Stroke { erase: true, strength: 0.7, ..stroke(&[[0.5, 0.0], [0.5, 1.0]]) };
        let mut live = Coverage::new(160, 120);
        for n in 1..=path.len() {
            live.update(&[stroke(&path[..n])]);
        }
        for n in 1..=2 {
            live.update(&[stroke(&path), Stroke { points: eraser.points[..n].to_vec(), ..eraser.clone() }]);
        }
        let mut whole = Coverage::new(160, 120);
        whole.update(&[stroke(&path), eraser.clone()]);
        assert_eq!(live.values, whole.values);

        // Undoing the eraser paints everything again, without it.
        let rows = live.update(&[stroke(&path)]);
        assert_eq!(rows, Some(0..120));
        let mut plain = Coverage::new(160, 120);
        plain.update(&[stroke(&path)]);
        assert_eq!(live.values, plain.values);
        assert_eq!(live.update(&[stroke(&path)]), None, "nothing to do when nothing changed");
    }

    #[test]
    fn erasing_takes_coverage_away_and_painting_again_brings_it_back() {
        let mut map = Coverage::new(100, 100);
        let across = stroke(&[[0.0, 0.5], [1.0, 0.5]]);
        let eraser = Stroke { erase: true, ..stroke(&[[0.5, 0.0], [0.5, 1.0]]) };
        map.update(&[across.clone(), eraser.clone()]);
        assert_eq!(at(&map, 0.5, 0.5), 0.0);
        assert!(at(&map, 0.2, 0.5) > 0.99);
        map.update(&[across.clone(), eraser, across]);
        assert!(at(&map, 0.5, 0.5) > 0.99);
    }

    #[test]
    fn masks_are_packed_in_shares_of_the_long_side() {
        let masks = vec![
            Mask {
                id: 7,
                parts: vec![
                    MaskPart { mode: Mode::Add, shape: Shape::Radial { center: [0.5, 0.5], radius: [0.1, 0.2], angle: 90.0, feather: 0.5 } },
                    MaskPart { mode: Mode::Subtract, shape: Shape::Brush { strokes: vec![stroke(&[[0.5, 0.5]])] } },
                ],
                adjustments: LocalAdjustments { exposure: 1.5, contrast: 50.0, ..Default::default() },
                ..Default::default()
            },
            Mask { id: 9, visible: false, invert: true, parts: vec![MaskPart { mode: Mode::Intersect, shape: Shape::Luminance { low: 0.2, high: 0.8, smoothness: 0.1 } }], ..Default::default() },
        ];
        let (packed, brushes) = pack(&masks, 300, 200, Some(9));
        assert_eq!(packed.counts, [2.0, 3.0, 1.0, 0.0]);
        assert_eq!(packed.masks[0][0], [1.5, 0.5, 0.0, 0.0]);
        assert_eq!(packed.masks[0][4], [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(packed.masks[1][4], [1.0, 0.0, 0.0, 0.0], "inverted, and hidden");
        // The centre of a 300 x 200 photo is (0.5, 1/3) in shares of its width.
        assert_eq!(packed.parts[0][1], [0.5, 0.5 * 200.0 / 300.0, 0.1, 0.2]);
        assert!((packed.parts[0][2][0] - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(packed.parts[1][0], [BRUSH, 1.0, 0.0, 0.0]);
        assert_eq!(packed.parts[2][0], [LUMINANCE, 2.0, 1.0, 0.0]);
        assert_eq!(brushes.len(), 1);
    }
}
