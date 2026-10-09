//! Healing: covering dust, hairs and scratches with texture from another
//! part of the photo.
//!
//! A spot (`edit::Spot`) covers an area and names a patch to take texture
//! from. The heal pass (`heal` in shaders/prepare.wgsl) lays the patch's
//! texture over the area but keeps the brightness and colour of what
//! surrounds it: at the area's edge the fix matches its surroundings
//! exactly, and inside, that difference is blended in from every side. The
//! pass runs as the working image is made, straight after a negative is
//! turned into a positive, so everything after it (the develop shader, its
//! blurs, the histogram, thumbnails and exports) sees the healed photo.
//!
//! Here: laying spots out for that pass, choosing a patch for a new spot,
//! and finding specks of dust to offer for healing.

use std::collections::{HashMap, VecDeque};

use bytemuck::{Pod, Zeroable};
use rayon::prelude::*;
use serde::Serialize;

use crate::edit::{Adjustments, Spot};
use crate::geometry;

/// The most spots a photo can have; any more are left out of the picture.
pub const MAX_SPOTS: usize = 500;
/// The most points one spot's line can have. Longer lines keep this many,
/// spread along them.
pub const MAX_POINTS: usize = 64;
/// The share of a spot's radius over which it fades into the photo.
const FEATHER: f32 = 0.3;
/// The most specks one look for dust offers.
pub const MAX_SPECKS: usize = 200;
/// The longest side of the picture dust is looked for in, in pixels: about
/// a full-size photo's, while keeping a large one quick to look through.
pub const DUST_EDGE: u32 = 4096;

/// One spot as the heal pass reads it. Mirrors `Spot` in prepare.wgsl.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Packed {
    /// Points in use, the radius, and how far away the patch is (x, y), all in photo pixels.
    pub info: [f32; 4],
    /// How far in from the edge the spot fades, in pixels; the rest is spare.
    pub edge: [f32; 4],
    /// The points in photo pixels, two to a row.
    pub points: [[f32; 4]; MAX_POINTS / 2],
}

/// A spot ready for the heal pass, and the parts of the photo it changes and reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub packed: Packed,
    /// Left, top, width and height in photo pixels: all the pass needs to draw.
    pub area: [u32; 4],
    /// The same for what the pass reads: around the spot, and around its patch.
    pub reads: [[u32; 4]; 2],
}

/// The part of a `width` x `height` photo inside `bounds` (left, top, right,
/// bottom), as left, top, width and height in whole pixels; empty if none of it is.
fn on_photo([left, top, right, bottom]: [f32; 4], width: u32, height: u32) -> [u32; 4] {
    let (left, top) = ((left.max(0.0) as u32).min(width), (top.max(0.0) as u32).min(height));
    let (right, bottom) = ((right.ceil().max(0.0) as u32).min(width), (bottom.ceil().max(0.0) as u32).min(height));
    [left, top, right.saturating_sub(left), bottom.saturating_sub(top)]
}

/// Keeps at most `limit` of `points`, spread evenly, the first and last among them.
fn thin(points: &[[f32; 2]], limit: usize) -> Vec<[f32; 2]> {
    if points.len() <= limit {
        return points.to_vec();
    }
    let last = points.len() - 1;
    (0..limit).map(|i| points[(i * last + (limit - 1) / 2) / (limit - 1)]).collect()
}

/// A spot's area in photo pixels: a line of points and how far it reaches.
#[derive(Debug, Clone)]
struct Path {
    points: Vec<[f32; 2]>,
    radius: f32,
}

impl Path {
    /// `spot` on a `width` x `height` photo; None if it covers nothing.
    fn of(spot: &Spot, (width, height): (u32, u32)) -> Option<Path> {
        let radius = spot.radius * width.max(height) as f32;
        let points: Vec<[f32; 2]> =
            thin(&spot.points, MAX_POINTS).iter().map(|[x, y]| [x * width as f32, y * height as f32]).collect();
        let finite = radius.is_finite() && points.iter().flatten().all(|v| v.is_finite());
        (finite && radius >= 0.5 && !points.is_empty()).then_some(Path { points, radius })
    }

    /// How far `q` is from the line through the points.
    fn distance(&self, q: [f32; 2]) -> f32 {
        let mut nearest = dist(q, self.points[0]);
        for pair in self.points.windows(2) {
            nearest = nearest.min(dist(q, closest(q, pair[0], pair[1])));
        }
        nearest
    }

    /// Points every `step` or so along the line, both ends included.
    fn along(&self, step: f32) -> Vec<[f32; 2]> {
        let mut out = vec![self.points[0]];
        for pair in self.points.windows(2) {
            let n = (dist(pair[0], pair[1]) / step).ceil().max(1.0) as usize;
            out.extend((1..=n).map(|i| lerp(pair[0], pair[1], i as f32 / n as f32)));
        }
        out
    }

    /// The box around the area, `margin` further out on every side: left, top, right, bottom.
    fn bounds(&self, margin: f32) -> [f32; 4] {
        let reach = self.radius + margin;
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for &[x, y] in &self.points {
            b = [b[0].min(x - reach), b[1].min(y - reach), b[2].max(x + reach), b[3].max(y + reach)];
        }
        b
    }
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn plus(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

/// The point of the segment from `a` to `b` nearest `q`.
fn closest(q: [f32; 2], a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let length = ab[0] * ab[0] + ab[1] * ab[1];
    if length <= 1e-12 {
        return a;
    }
    lerp(a, b, (((q[0] - a[0]) * ab[0] + (q[1] - a[1]) * ab[1]) / length).clamp(0.0, 1.0))
}

/// Lays spots out for the heal pass, on a `width` x `height` photo (the size
/// the working image is held at). Spots that cover nothing are left out.
pub fn pack(spots: &[Spot], width: u32, height: u32) -> Vec<Placed> {
    spots
        .iter()
        .take(MAX_SPOTS)
        .filter_map(|spot| {
            let path = Path::of(spot, (width, height))?;
            let first = path.points[0];
            // Whole pixels, so the patch's grain is copied as sharp as it is.
            let offset = [(spot.source[0] * width as f32 - first[0]).round(), (spot.source[1] * height as f32 - first[1]).round()];
            if !offset.iter().all(|v| v.is_finite()) {
                return None;
            }
            let area = on_photo(path.bounds(1.0), width, height);
            if area[2] == 0 || area[3] == 0 {
                return None;
            }
            // The pass reads around the edge as far as a third of the radius and a few pixels more.
            let [left, top, right, bottom] = path.bounds(path.radius * 0.35 + 4.0);
            let around = on_photo([left, top, right, bottom], width, height);
            let patch = on_photo([left + offset[0], top + offset[1], right + offset[0], bottom + offset[1]], width, height);
            let mut points = [[0.0; 4]; MAX_POINTS / 2];
            for (i, p) in path.points.iter().enumerate() {
                points[i / 2][(i % 2) * 2] = p[0];
                points[i / 2][(i % 2) * 2 + 1] = p[1];
            }
            let packed = Packed {
                info: [path.points.len() as f32, path.radius, offset[0], offset[1]],
                edge: [(path.radius * FEATHER).max(1.0), 0.0, 0.0, 0.0],
                points,
            };
            Some(Placed { packed, area, reads: [around, patch] })
        })
        .collect()
}

/// Part of the working image, read back to be looked at: `width` x `height`
/// pixels, each `scale` photo pixels across and down, the first one's
/// corner at `origin` (in photo pixels).
pub struct Area {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 3]>,
    pub origin: [f32; 2],
    pub scale: [f32; 2],
}

impl Area {
    /// How the picture looks at a point in photo pixels, blended between
    /// its pixels: brightness on a scale closer to the eye's than light's.
    /// None off its edge.
    fn at(&self, [x, y]: [f32; 2]) -> Option<[f32; 3]> {
        let u = (x - self.origin[0]) / self.scale[0] - 0.5;
        let v = (y - self.origin[1]) / self.scale[1] - 0.5;
        let (w, h) = (self.width as f32, self.height as f32);
        if !(u >= 0.0 && v >= 0.0 && u <= w - 1.0 && v <= h - 1.0) {
            return None;
        }
        let (x0, y0) = (u.floor() as usize, v.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width as usize - 1), (y0 + 1).min(self.height as usize - 1));
        let (fx, fy) = (u - x0 as f32, v - y0 as f32);
        let row = self.width as usize;
        let p = |x: usize, y: usize| self.pixels[y * row + x];
        let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
        Some(std::array::from_fn(|i| {
            let value = (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy;
            value.clamp(0.0, 16.0).sqrt()
        }))
    }
}

/// How far around a spot of `radius` photo pixels a patch is looked for:
/// the area read for `find_source` should reach this far past the spot.
pub fn search_reach(radius: f32) -> f32 {
    radius * 8.5
}

/// The mean and variance of each channel.
fn spread(values: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let n = values.len().max(1) as f32;
    let mean: [f32; 3] = std::array::from_fn(|c| values.iter().map(|v| v[c]).sum::<f32>() / n);
    let variance = std::array::from_fn(|c| values.iter().map(|v| (v[c] - mean[c]).powi(2)).sum::<f32>() / n);
    (mean, variance)
}

/// Where a new spot's patch should be, on the photo file (0..1 across and
/// down), for the spot's first point: near the spot, clear of it and of
/// `others`, on the photo, where the surroundings look most like the spot's
/// own and nothing in the patch stands out more than they do.
///
/// `area` must hold the spot and `search_reach` around it. The photo is
/// `width` x `height` pixels.
pub fn find_source(area: &Area, spot: &Spot, (width, height): (u32, u32), others: &[Spot]) -> [f32; 2] {
    let Some(path) = Path::of(spot, (width, height)) else { return spot.points.first().copied().unwrap_or(spot.source) };
    let r = path.radius;
    let first = path.points[0];
    let to_file = |p: [f32; 2]| [p[0] / width as f32, p[1] / height as f32];
    let centres = path.along((r * 0.5).max(0.5));

    // A ring around the area, and the middle of it.
    let mut ring = Vec::new();
    let mut inside = Vec::new();
    for &c in &centres {
        for k in 0..12 {
            let angle = (k as f32 + 0.5) * std::f32::consts::TAU / 12.0;
            let way = [angle.cos(), angle.sin()];
            for reach in [1.3, 1.65] {
                let q = plus(c, [way[0] * r * reach, way[1] * r * reach]);
                if path.distance(q) >= 1.15 * r {
                    ring.push(q);
                }
            }
            if k % 2 == 0 {
                inside.push(plus(c, [way[0] * r * 0.6, way[1] * r * 0.6]));
            }
        }
        inside.push(c);
    }
    let ring = thin(&ring, 600);
    let inside = thin(&inside, 300);
    let ring: Vec<([f32; 2], [f32; 3])> = ring.into_iter().filter_map(|q| Some((q, area.at(q)?))).collect();
    let others: Vec<Path> = others.iter().filter_map(|other| Path::of(other, (width, height))).collect();
    let on_photo = |q: [f32; 2]| q[0] >= r && q[1] >= r && q[0] <= width as f32 - r && q[1] <= height as f32 - r;

    let around: Vec<[f32; 3]> = ring.iter().map(|(_, value)| *value).collect();
    let (mean, variance) = spread(&around);
    let mut best: Option<(f32, [f32; 2])> = None;
    for (n, reach) in [2.3f32, 2.8, 3.5, 4.5, 6.0].into_iter().enumerate() {
        for k in 0..24 {
            let angle = (k as f32 + 0.5 * (n % 2) as f32) * std::f32::consts::TAU / 24.0;
            let offset = [angle.cos() * r * reach, angle.sin() * r * reach];
            let clear = centres.iter().all(|&c| {
                let q = plus(c, offset);
                on_photo(q) && path.distance(q) >= 2.05 * r && others.iter().all(|o| o.distance(q) >= o.radius + r)
            });
            if !clear || ring.is_empty() {
                continue;
            }
            let Some(patch) = ring.iter().map(|(q, _)| area.at(plus(*q, offset))).collect::<Option<Vec<_>>>() else { continue };
            let Some(middle) = inside.iter().map(|q| area.at(plus(*q, offset))).collect::<Option<Vec<_>>>() else { continue };
            let (patch_mean, _) = spread(&patch);
            let (_, middle_variance) = spread(&middle);
            // Texture: the surroundings, each with its own brightness taken off.
            let texture = around
                .iter()
                .zip(&patch)
                .map(|(a, b)| (0..3).map(|c| ((a[c] - mean[c]) - (b[c] - patch_mean[c])).powi(2)).sum::<f32>())
                .sum::<f32>()
                / around.len() as f32;
            // Something in the patch standing out more than anything around the spot.
            let busy: f32 = (0..3).map(|c| (middle_variance[c].sqrt() - variance[c].sqrt()).max(0.0).powi(2)).sum();
            let shade: f32 = (0..3).map(|c| (mean[c] - patch_mean[c]).powi(2)).sum();
            let cost = (texture + 2.0 * busy + 0.25 * shade + 1e-6) * (1.0 + 0.04 * reach);
            if best.is_none_or(|(lowest, _)| cost < lowest) {
                best = Some((cost, offset));
            }
        }
    }
    let offset = match best {
        Some((_, offset)) => offset,
        // Nowhere suits (a spot on the very edge, crowded by others): beside it, towards the middle.
        None => {
            let towards = |v: f32, size: u32| if v < size as f32 / 2.0 { 1.0 } else { -1.0 };
            [towards(first[0], width) * r * 2.5, 0.0]
        }
    };
    let source = to_file(plus(first, offset));
    source.map(|v| (v.clamp(0.0, 1.0) * 1e5).round() / 1e5)
}

/// A speck that looks like dust, offered for healing: where it is and how
/// big, on the photo file (0..1, the radius a share of the longer side),
/// and the patch that would cover it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Speck {
    pub center: [f32; 2],
    pub radius: f32,
    pub source: [f32; 2],
}

/// Whether a point on the photo file (0..1) is inside the crop.
pub fn inside_crop(width: u32, height: u32, adjustments: &Adjustments) -> impl Fn([f32; 2]) -> bool {
    let frame = geometry::frame(width, height, adjustments, false);
    let to_frame = geometry::frame_to_source(width, height, adjustments, &frame).inverse();
    move |[x, y]| {
        let [u, v] = to_frame.apply([x as f64, y as f64]);
        (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)
    }
}

/// One pass of a running minimum or maximum (`take`) over windows of
/// `2 * reach + 1` values along each row of `width`, in O(1) a value
/// (van Herk and Gil-Werman). Past the ends counts as `pad`.
fn run_rows(values: &[f32], width: usize, reach: usize, pad: f32, take: fn(f32, f32) -> f32) -> Vec<f32> {
    let window = 2 * reach + 1;
    let mut out = vec![0.0; values.len()];
    out.par_chunks_mut(width).zip(values.par_chunks(width)).for_each(|(out, row)| {
        let len = (row.len() + 2 * reach).div_ceil(window) * window;
        let mut padded = vec![pad; len];
        padded[reach..reach + row.len()].copy_from_slice(row);
        let (mut ahead, mut behind) = (padded.clone(), padded.clone());
        for i in 1..len {
            if i % window != 0 {
                ahead[i] = take(ahead[i - 1], padded[i]);
            }
        }
        for i in (0..len - 1).rev() {
            if (i + 1) % window != 0 {
                behind[i] = take(behind[i + 1], padded[i]);
            }
        }
        for (i, value) in out.iter_mut().enumerate() {
            *value = take(behind[i], ahead[i + 2 * reach]);
        }
    });
    out
}

fn transpose(values: &[f32], width: usize, height: usize) -> Vec<f32> {
    let mut out = vec![0.0; values.len()];
    out.par_chunks_mut(height).enumerate().for_each(|(x, column)| {
        for (y, value) in column.iter_mut().enumerate() {
            *value = values[y * width + x];
        }
    });
    out
}

/// A running minimum or maximum over squares of `2 * reach + 1` pixels.
fn square(values: &[f32], width: usize, height: usize, reach: usize, pad: f32, take: fn(f32, f32) -> f32) -> Vec<f32> {
    let across = run_rows(values, width, reach, pad, take);
    let down = run_rows(&transpose(&across, width, height), height, reach, pad, take);
    transpose(&down, height, width)
}

fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let middle = values.len() / 2;
    *values.select_nth_unstable_by(middle, f32::total_cmp).1
}

/// The side of the blocks the photo's noise is measured in, in pixels.
const BLOCK: usize = 64;
/// How far past the noise a speck has to stand out...
const STANDS_OUT: f32 = 5.0;
/// ...and at least by how much, on a scale from black (0) to white (1).
const LEAST_CONTRAST: f32 = 0.04;

/// What a speck found in the picture came to.
struct Blob {
    pixels: usize,
    /// Sums of x, y, x², y² and xy over its pixels.
    sums: [f64; 5],
    bounds: [usize; 4],
    /// How far it stands out at most, in thresholds.
    strength: f32,
}

/// Specks in `area` (the whole working image of a `width` x `height`
/// photo) that look like dust: small and compact, standing out from their
/// surroundings by more than the photo's grain or noise does. On a
/// negative's positive dust is lighter than what is around it (`bright`);
/// on a slide or a digital photo, darker.
///
/// Fine detail (a texture, an edge, a line) is passed over as best it can
/// be: a speck must be roundish, not one of a crowd, and stand out from the
/// noise around it. Only those `keep` keeps (the ones inside the crop, say)
/// and that no spot already covers are offered, the clearest first, each
/// with a patch to cover it.
pub fn find_dust(area: &Area, (width, height): (u32, u32), bright: bool, spots: &[Spot], keep: impl Fn([f32; 2]) -> bool) -> Vec<Speck> {
    let (w, h) = (area.width as usize, area.height as usize);
    if w < 8 || h < 8 {
        return Vec::new();
    }
    // Brightness on a scale closer to the eye's, white at the photo's own white.
    let luminance: Vec<f32> = area.pixels.par_iter().map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]).collect();
    // Leaving out the far-past-white black of a film holder, as a negative's positive has it.
    let mut sampled: Vec<f32> = luminance.iter().step_by(7).copied().filter(|v| v.is_finite() && *v < 30.0).collect();
    let white = if sampled.is_empty() {
        1.0
    } else {
        let at = ((sampled.len() as f32 * 0.99) as usize).min(sampled.len() - 1);
        *sampled.select_nth_unstable_by(at, f32::total_cmp).1
    }
    .max(1e-4);
    let sign = if bright { 1.0 } else { -1.0 };
    let values: Vec<f32> = luminance.par_iter().map(|v| sign * (v / white).clamp(0.0, 4.0).powf(1.0 / 2.2)).collect();

    // What the picture would be without anything smaller than a square of
    // `reach` either way that stands out above it (a morphological opening):
    // edges and broad shapes stay, specks go.
    let reach = (w.max(h) / 128).clamp(6, 24);
    let opened = square(&square(&values, w, h, reach, f32::INFINITY, f32::min), w, h, reach, f32::NEG_INFINITY, f32::max);
    let lifted: Vec<f32> = values.par_iter().zip(&opened).map(|(v, o)| v - o).collect();
    // Pixel-to-pixel noise: each pixel against its four neighbours.
    let noise: Vec<f32> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let n = |dx: isize, dy: isize| {
                let (nx, ny) = ((x as isize + dx).clamp(0, w as isize - 1) as usize, (y as isize + dy).clamp(0, h as isize - 1) as usize);
                values[ny * w + nx]
            };
            (values[i] - (n(-1, 0) + n(1, 0) + n(0, -1) + n(0, 1)) / 4.0).abs()
        })
        .collect();

    // Block by block: how far the opening sits under the picture anyway
    // (it does, a little, wherever there is grain), and how busy it is: its
    // grain from pixel to pixel, or detail the size of a speck (leaves, say)
    // standing out of the opening, whichever is more.
    let (bw, bh) = (w.div_ceil(BLOCK), h.div_ceil(BLOCK));
    let blocks: Vec<(f32, f32)> = (0..bw * bh)
        .into_par_iter()
        .map(|b| {
            let (bx, by) = (b % bw * BLOCK, b / bw * BLOCK);
            let (mut lift, mut grain) = (Vec::with_capacity(BLOCK * BLOCK), Vec::with_capacity(BLOCK * BLOCK));
            for y in by..(by + BLOCK).min(h) {
                for x in bx..(bx + BLOCK).min(w) {
                    lift.push(lifted[y * w + x]);
                    grain.push(noise[y * w + x]);
                }
            }
            let bias = median(&mut lift);
            let mut spread: Vec<f32> = lift.iter().map(|v| (v - bias).abs()).collect();
            // Median absolute differences as standard deviations (the grain's against four neighbours).
            let (detail, grain) = (median(&mut spread) * 1.4826, median(&mut grain) * 1.4826 / 1.118);
            (bias, detail.max(grain))
        })
        .collect();
    // The noisiest of each block's neighbours counts, so a busy area's edge isn't taken for a quiet one.
    let thresholds: Vec<(f32, f32)> = (0..bw * bh)
        .map(|b| {
            let (bx, by) = ((b % bw) as isize, (b / bw) as isize);
            let mut sigma = 0.0f32;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (x, y) = (bx + dx, by + dy);
                    if x >= 0 && y >= 0 && (x as usize) < bw && (y as usize) < bh {
                        sigma = sigma.max(blocks[y as usize * bw + x as usize].1);
                    }
                }
            }
            (blocks[b].0, (STANDS_OUT * sigma).max(LEAST_CONTRAST))
        })
        .collect();
    // How far a pixel stands out, in thresholds: those of the blocks around
    // it blended by how near their middles are, so no seam shows where one
    // block meets the next.
    let above = |i: usize| {
        let place = |v: usize, blocks: usize| ((v as f32 + 0.5) / BLOCK as f32 - 0.5).clamp(0.0, (blocks - 1) as f32);
        let (fx, fy) = (place(i % w, bw), place(i / w, bh));
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(bw - 1), (y0 + 1).min(bh - 1));
        let at = |x: usize, y: usize| thresholds[y * bw + x];
        let mix = |a: (f32, f32), b: (f32, f32), t: f32| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let (bias, threshold) = mix(mix(at(x0, y0), at(x1, y0), tx), mix(at(x0, y1), at(x1, y1), tx), ty);
        (lifted[i] - bias) / threshold
    };

    // Specks: what rises over the threshold, with what rises half as far around it.
    let largest = (2 * reach + 1).pow(2);
    let mut seen = vec![false; w * h];
    let mut blobs = Vec::new();
    let mut queue = VecDeque::new();
    for start in 0..w * h {
        if seen[start] || above(start) < 1.0 {
            continue;
        }
        seen[start] = true;
        queue.push_back(start);
        let mut blob = Blob { pixels: 0, sums: [0.0; 5], bounds: [usize::MAX, usize::MAX, 0, 0], strength: 0.0 };
        while let Some(i) = queue.pop_front() {
            let (x, y) = (i % w, i / w);
            blob.pixels += 1;
            let (fx, fy) = (x as f64, y as f64);
            for (sum, value) in blob.sums.iter_mut().zip([fx, fy, fx * fx, fy * fy, fx * fy]) {
                *sum += value;
            }
            blob.bounds = [blob.bounds[0].min(x), blob.bounds[1].min(y), blob.bounds[2].max(x), blob.bounds[3].max(y)];
            blob.strength = blob.strength.max(above(i));
            for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let n = ny as usize * w + nx as usize;
                if !seen[n] && above(n) >= 0.5 {
                    seen[n] = true;
                    queue.push_back(n);
                }
            }
        }
        if blob.pixels <= largest * 2 {
            blobs.push(blob);
        }
    }

    // Compact and smaller than the square; touching the edge of the picture is something else.
    let compact: Vec<&Blob> = blobs
        .iter()
        .filter(|blob| {
            let [left, top, right, bottom] = blob.bounds;
            let (across, down) = (right - left + 1, bottom - top + 1);
            if blob.pixels < 3 || across > 2 * reach + 1 || down > 2 * reach + 1 {
                return false;
            }
            if left == 0 || top == 0 || right + 1 >= w || bottom + 1 >= h {
                return false;
            }
            if (blob.pixels as f32) < 0.25 * (across * down) as f32 {
                return false;
            }
            // Elongation, from how its pixels spread along their main axis and across it.
            let n = blob.pixels as f64;
            let [sx, sy, sxx, syy, sxy] = blob.sums;
            let (vx, vy, cxy) = (sxx / n - (sx / n).powi(2), syy / n - (sy / n).powi(2), sxy / n - sx * sy / (n * n));
            let (middle, half) = ((vx + vy) / 2.0, (((vx - vy) / 2.0).powi(2) + cxy * cxy).sqrt());
            let (long, short) = (middle + half, (middle - half).max(0.0));
            // No more than twice as long as it is wide: a piece of a line or an edge is longer.
            blob.pixels <= 6 || long <= 4.0 * short.max(0.05)
        })
        .collect();

    // Many specks close together are a texture (leaves, gravel, stars), not dust.
    let cell = (6 * reach) as f32;
    let centre = |blob: &Blob| [(blob.sums[0] / blob.pixels as f64) as f32, (blob.sums[1] / blob.pixels as f64) as f32];
    let mut grid: HashMap<(i32, i32), Vec<[f32; 2]>> = HashMap::new();
    for blob in &compact {
        let c = centre(blob);
        grid.entry(((c[0] / cell) as i32, (c[1] / cell) as i32)).or_default().push(c);
    }
    let crowded = |c: [f32; 2]| {
        let (gx, gy) = ((c[0] / cell) as i32, (c[1] / cell) as i32);
        let mut near = 0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(list) = grid.get(&(gx + dx, gy + dy)) {
                    near += list.iter().filter(|o| dist(**o, c) <= cell).count();
                }
            }
        }
        near > 7
    };

    let long = width.max(height) as f32;
    let existing: Vec<Path> = spots.iter().filter_map(|spot| Path::of(spot, (width, height))).collect();
    let mut specks: Vec<(f32, Speck)> = compact
        .into_iter()
        .filter(|blob| !crowded(centre(blob)))
        .filter_map(|blob| {
            let c = centre(blob);
            let [left, top, right, bottom] = blob.bounds;
            let extent = (right - left + 1).max(bottom - top + 1) as f32;
            // Room for the soft rim around a speck too, past where the spot fades out.
            let reach_px = ((blob.pixels as f32 / std::f32::consts::PI).sqrt() * 2.0).max(extent * 0.8) + 2.0;
            let at = [area.origin[0] + (c[0] + 0.5) * area.scale[0], area.origin[1] + (c[1] + 0.5) * area.scale[1]];
            let radius = (reach_px * area.scale[0].max(area.scale[1])).max(2.0);
            let center = [at[0] / width as f32, at[1] / height as f32];
            if !keep(center) || existing.iter().any(|spot| spot.distance(at) < spot.radius + radius * 0.5) {
                return None;
            }
            let tidy = |v: f32| (v * 1e5).round() / 1e5;
            Some((blob.strength, Speck { center: center.map(tidy), radius: tidy(radius / long), source: [0.0; 2] }))
        })
        .collect();
    specks.sort_by(|a, b| b.0.total_cmp(&a.0));
    specks.truncate(MAX_SPECKS);

    // Each speck's patch steers clear of the spots there are and of every other speck.
    let as_spot = |speck: &Speck| Spot { points: vec![speck.center], radius: speck.radius, source: speck.center };
    let all: Vec<Spot> = spots.iter().cloned().chain(specks.iter().map(|(_, speck)| as_spot(speck))).collect();
    specks
        .par_iter()
        .enumerate()
        .map(|(i, (_, speck))| {
            let others: Vec<Spot> =
                all.iter().enumerate().filter(|(j, _)| *j != spots.len() + i).map(|(_, spot)| spot.clone()).collect();
            Speck { source: find_source(area, &as_spot(speck), (width, height), &others), ..speck.clone() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_are_packed_in_photo_pixels_with_whole_pixel_patches() {
        let spots = [
            Spot { points: vec![[0.5, 0.25]], radius: 0.01, source: [0.6, 0.26] },
            // A line on the edge of the photo is cut to the photo.
            Spot { points: vec![[0.0, 0.5], [0.1, 0.5]], radius: 0.02, source: [0.0, 0.6] },
            // Nothing to cover.
            Spot { points: vec![], radius: 0.01, source: [0.5, 0.5] },
            Spot { points: vec![[0.5, 0.5]], radius: 0.0, source: [0.5, 0.5] },
        ];
        let placed = pack(&spots, 400, 200);
        assert_eq!(placed.len(), 2);
        let spot = &placed[0].packed;
        assert_eq!(spot.info, [1.0, 4.0, 40.0, 2.0]);
        assert_eq!(spot.points[0][..2], [200.0, 50.0]);
        assert_eq!(spot.edge[0], 1.2);
        assert_eq!(placed[0].area, [195, 45, 10, 10]);
        // It reads a little further out, around itself and its patch.
        assert_eq!(placed[0].reads, [[190, 40, 20, 20], [230, 42, 20, 20]]);
        let line = &placed[1];
        assert_eq!(line.packed.info[0], 2.0);
        assert_eq!(line.packed.points[0], [0.0, 100.0, 40.0, 100.0]);
        assert_eq!(line.area, [0, 91, 49, 18]);
    }

    #[test]
    fn a_long_line_keeps_its_ends() {
        let points: Vec<[f32; 2]> = (0..200).map(|i| [i as f32 / 200.0, 0.5]).collect();
        let kept = thin(&points, MAX_POINTS);
        assert_eq!(kept.len(), MAX_POINTS);
        assert_eq!((kept[0], kept[MAX_POINTS - 1]), (points[0], points[199]));
        let spot = Spot { points, radius: 0.01, source: [0.5, 0.6] };
        assert_eq!(pack(&[spot], 300, 300)[0].packed.info[0], MAX_POINTS as f32);
    }

    #[test]
    fn spots_past_the_limit_are_left_out() {
        let spot = Spot { points: vec![[0.5, 0.5]], radius: 0.01, source: [0.6, 0.5] };
        assert_eq!(pack(&vec![spot; MAX_SPOTS + 10], 200, 200).len(), MAX_SPOTS);
    }

    /// A picture `width` x `height` made by `paint`, as a whole-photo area.
    fn picture(width: u32, height: u32, paint: impl Fn(u32, u32) -> f32) -> Area {
        let pixels = (0..width * height).map(|i| [paint(i % width, i / width); 3]).collect();
        Area { width, height, pixels, origin: [0.0, 0.0], scale: [1.0, 1.0] }
    }

    /// Repeatable noise in -1..1.
    fn noise(x: u32, y: u32) -> f32 {
        let mut n = x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263);
        n = (n ^ (n >> 13)).wrapping_mul(1274126177);
        ((n ^ (n >> 16)) & 0xffff) as f32 / 32767.5 - 1.0
    }

    #[test]
    fn a_patch_is_taken_from_the_same_side_of_an_edge() {
        // Dark left, light right, the edge at x = 100; a spot on the light side near the edge.
        let area = picture(200, 200, |x, y| if x < 100 { 0.05 } else { 0.5 } + 0.01 * noise(x, y));
        let spot = Spot { points: vec![[0.6, 0.5]], radius: 0.03, source: [0.0; 2] };
        let source = find_source(&area, &spot, (200, 200), &[]);
        let r = 6.0;
        let (sx, sy) = (source[0] * 200.0, source[1] * 200.0);
        assert!(sx - r * 1.7 > 100.0, "the patch at ({sx}, {sy}) reaches over the edge");
        assert!(dist([sx, sy], [120.0, 100.0]) >= 2.0 * r, "it doesn't overlap the spot");
        assert!(dist([sx, sy], [120.0, 100.0]) <= 6.5 * r, "and it's close by");
    }

    #[test]
    fn a_patch_keeps_clear_of_other_spots_and_the_edge_of_the_photo() {
        let area = picture(120, 120, |x, y| 0.3 + 0.02 * noise(x, y));
        let spot = Spot { points: vec![[0.08, 0.5]], radius: 0.04, source: [0.0; 2] };
        let blocking: Vec<Spot> = (0..8)
            .map(|i| {
                let angle = i as f32 * std::f32::consts::TAU / 8.0;
                let at = [0.08 + 0.12 * angle.cos(), 0.5 + 0.12 * angle.sin()];
                Spot { points: vec![at], radius: 0.03, source: at }
            })
            .filter(|s| s.points[0][1] > 0.5)
            .collect();
        let source = find_source(&area, &spot, (120, 120), &blocking);
        let (sx, sy) = (source[0] * 120.0, source[1] * 120.0);
        let r = 0.04 * 120.0;
        assert!(sx >= r && sy >= r && sx <= 120.0 - r && sy <= 120.0 - r, "({sx}, {sy}) is off the photo");
        for other in &blocking {
            let at = [other.points[0][0] * 120.0, other.points[0][1] * 120.0];
            assert!(dist([sx, sy], at) >= other.radius * 120.0 + r, "({sx}, {sy}) covers another spot");
        }
    }

    #[test]
    fn a_line_takes_its_patch_from_beside_it_not_along_it() {
        let area = picture(200, 200, |x, y| 0.3 + 0.02 * noise(x, y));
        let spot = Spot { points: vec![[0.2, 0.5], [0.8, 0.5]], radius: 0.02, source: [0.0; 2] };
        let source = find_source(&area, &spot, (200, 200), &[]);
        let offset = [source[0] * 200.0 - 40.0, source[1] * 200.0 - 100.0];
        assert!(offset[1].abs() >= 2.0 * 4.0, "{offset:?} overlaps the line");
    }

    /// A photo with a soft speck of `contrast` planted at each of `specks`
    /// (x, y, radius), over fine noise and a gentle gradient.
    fn dusty(width: u32, height: u32, specks: &[(f32, f32, f32)], contrast: f32, texture: impl Fn(u32, u32) -> f32) -> Area {
        picture(width, height, |x, y| {
            let mut v = 0.15 + 0.2 * x as f32 / width as f32 + 0.004 * noise(x, y) + texture(x, y);
            for &(sx, sy, r) in specks {
                let d = ((x as f32 + 0.5 - sx).powi(2) + (y as f32 + 0.5 - sy).powi(2)).sqrt();
                v += contrast * (1.0 - ((d - r * 0.6) / (r * 0.4)).clamp(0.0, 1.0));
            }
            v.max(0.0)
        })
    }

    fn near(found: &[Speck], (x, y): (f32, f32), size: (u32, u32)) -> bool {
        found.iter().any(|s| dist([s.center[0] * size.0 as f32, s.center[1] * size.1 as f32], [x, y]) < 3.0)
    }

    #[test]
    fn dust_is_found_on_a_quiet_photo() {
        let planted = [(40.0, 50.0, 2.5), (150.0, 90.0, 4.0), (300.0, 200.0, 6.0), (220.0, 40.0, 1.6)];
        let area = dusty(400, 300, &planted, 0.25, |_, _| 0.0);
        let found = find_dust(&area, (400, 300), true, &[], |_| true);
        for speck in planted {
            assert!(near(&found, (speck.0, speck.1), (400, 300)), "missed the speck at {speck:?}: {found:?}");
        }
        assert_eq!(found.len(), planted.len(), "{found:?}");
        // Each covers its speck, and its patch is somewhere else.
        for speck in &found {
            let r = speck.radius * 400.0;
            let planted = planted.iter().find(|p| near(std::slice::from_ref(speck), (p.0, p.1), (400, 300))).unwrap();
            assert!(r >= planted.2, "a radius of {r} doesn't cover a speck of {}", planted.2);
            assert!(dist(speck.source.map(|v| v), speck.center) * 400.0 >= 2.0 * r);
        }

        // Dark specks are for slides and digital photos: on a negative's positive they aren't dust.
        let dark = dusty(400, 300, &planted, -0.12, |_, _| 0.0);
        assert!(find_dust(&dark, (400, 300), true, &[], |_| true).is_empty());
        let found = find_dust(&dark, (400, 300), false, &[], |_| true);
        assert_eq!(found.len(), planted.len(), "{found:?}");
    }

    #[test]
    fn edges_lines_and_fine_texture_are_not_dust() {
        let planted = [(100.0, 200.0, 3.0)];
        let area = dusty(400, 300, &planted, 0.3, |x, y| {
            // A hard edge, a thin bright line, and a patch of fine woven texture.
            let edge = if x > 250 { 0.3 } else { 0.0 };
            let line = if (x as i32 - y as i32 - 20).abs() <= 1 && x < 240 { 0.3 } else { 0.0 };
            let weave = if x > 270 && y < 120 { 0.15 * (((x / 3) + (y / 3)) % 2) as f32 } else { 0.0 };
            edge + line + weave
        });
        let found = find_dust(&area, (400, 300), true, &[], |_| true);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(near(&found, (100.0, 200.0), (400, 300)));
    }

    #[test]
    fn grain_alone_is_not_dust() {
        // Heavy grain, as a fast film shows, and a speck that stands well clear of it.
        let area = picture(300, 300, |x, y| {
            let d = ((x as f32 - 150.0).powi(2) + (y as f32 - 100.0).powi(2)).sqrt();
            0.3 + 0.06 * noise(x, y) + if d < 3.0 { 0.6 } else { 0.0 }
        });
        let found = find_dust(&area, (300, 300), true, &[], |_| true);
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn specks_already_covered_or_outside_the_crop_are_not_offered() {
        let planted = [(60.0, 60.0, 3.0), (200.0, 150.0, 3.0), (340.0, 240.0, 3.0)];
        let area = dusty(400, 300, &planted, 0.3, |_, _| 0.0);
        let covered = Spot { points: vec![[60.0 / 400.0, 60.0 / 300.0]], radius: 0.015, source: [0.3, 0.3] };
        let adjustments = Adjustments { crop: crate::edit::Crop { x: 0.4, y: 0.4, width: 0.8, height: 0.8 }, ..Default::default() };
        let found = find_dust(&area, (400, 300), true, &[covered], inside_crop(400, 300, &adjustments));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(near(&found, (200.0, 150.0), (400, 300)));
    }
}
