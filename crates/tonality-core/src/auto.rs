//! Auto settings: a starting point for Light (exposure, contrast, highlights, shadows, whites,
//! blacks) and for white balance, from the photo itself (its preview, inside the crop).
//!
//! **Tone** predicts what the screen will show: the shader's `tone()` (ported as [`tone_ev`],
//! kept in step by a GPU test) followed by AgX Base on grey, which is exact for luminance. It
//! reads the scene's luminance percentiles (sensor-clipped pixels left out: no setting brings
//! them back), and searches the six sliders for the settings whose predicted percentiles land on
//! targets, one region of the range per slider: the median a little under middle grey
//! (exposure), the brightest 0.5% just short of white (whites), the 90th percentile clear of the
//! top (highlights, recovering only), the 10th clear of the bottom (shadows, lifting only), the
//! darkest 0.5% just above black (blacks), and the middle half spread enough (contrast). Small
//! penalties on each slider keep ordinary photos to gentle settings. The search is a plain
//! coordinate descent from a fixed start, so the same photo always gets the same values.
//!
//! **White balance** estimates the light's colour from the photo's grey pixels (after Huo et
//! al. 2006): starting from the camera's own estimate (As Shot), it takes the pixels that look
//! neutral under the current estimate, averages their colour, and repeats with a tightening
//! tolerance, so a large coloured surface doesn't vote. With nothing that looks grey it stays As
//! Shot. The result is named with temperature and tint the way As Shot is named
//! ([`CameraColor::temperature_tint_of`]). See [`white_balance`] and `docs/auto.md`.

use std::sync::OnceLock;

use crate::agx::{Agx, Display, GREY};
use crate::color::{CameraColor, Vec3};
use crate::edit::{Crop, Light};
use crate::geometry::Geometry;
use crate::lens::{LensTable, UvToSource};

/// Rec.2020 luminance weights, as the shader's `REC2020_LUMA`.
const LUMA: Vec3 = [0.2627, 0.6780, 0.0593];

/// Camera RGB of the photo inside the crop on an `n × n` grid over the framed output (nearest
/// pixel), lens-corrected as the shader shows it (`fs_main`: each corrected position reads the
/// source through the lens table, times its vignetting gain). `pixel(x, y)` reads the
/// `width × height` preview; `full_w × full_h` is the full-size image the crop and `lens` are
/// defined on.
#[allow(clippy::too_many_arguments)]
pub fn sample(
    width: u32,
    height: u32,
    pixel: impl Fn(u32, u32) -> Vec3,
    crop: &Crop,
    lens: &LensTable,
    full_w: u32,
    full_h: u32,
    n: u32,
) -> Vec<Vec3> {
    let g = Geometry::new(crop, full_w as f32, full_h as f32);
    let to_src = UvToSource::new(lens, full_w, full_h);
    let mut out = Vec::with_capacity((n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            let (fx, fy) = ((i as f32 + 0.5) / n as f32, (j as f32 + 0.5) / n as f32);
            let u = g.m[0][0] * fx + g.m[0][1] * fy + g.m[0][2];
            let v = g.m[1][0] * fx + g.m[1][1] * fy + g.m[1][2];
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            let ([u, v], gain) = to_src.map_with_gain([u, v]);
            let (x, y) = ((u * width as f32) as u32, (v * height as f32) as u32);
            out.push(pixel(x.min(width - 1), y.min(height - 1)).map(|c| c * gain));
        }
    }
    out
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The shader's `tone()` on log2 luminance relative to middle grey. Slider values as the edit
/// stores them (−100…100).
pub fn tone_ev(ev: f32, l: &Light) -> f32 {
    let n = |v: f32| (v / 100.0).clamp(-1.0, 1.0);
    let mut ev = ev * (1.0 + 0.6 * n(l.contrast));
    ev += 1.5 * n(l.highlights) * smoothstep(-0.5, 3.5, ev);
    ev += 1.5 * n(l.shadows) * (1.0 - smoothstep(-6.0, -0.5, ev));
    ev += n(l.whites) * smoothstep(1.0, 4.5, ev);
    ev += 1.5 * n(l.blacks) * (1.0 - smoothstep(-10.0, -4.0, ev));
    ev
}

/// What the screen shows (sRGB code value) for grey at `ev` stops from middle grey, after AgX.
pub fn display_of_ev(ev: f32) -> f32 {
    const LO: f32 = -24.0;
    const HI: f32 = 12.0;
    const STEPS: usize = 36 * 32;
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let agx = Agx::sdr(Display::Srgb);
        (0..=STEPS)
            .map(|k| {
                let ev = LO as f64 + (HI - LO) as f64 * k as f64 / STEPS as f64;
                agx.display([GREY * ev.exp2(); 3])[0] as f32
            })
            .collect()
    });
    let x = ((ev.clamp(LO, HI) - LO) / (HI - LO)) * STEPS as f32;
    let i = (x as usize).min(STEPS - 1);
    t[i] + (t[i + 1] - t[i]) * (x - i as f32)
}

/// Percentiles the tone search looks at.
const PCTS: [f32; 7] = [0.5, 10.0, 25.0, 50.0, 75.0, 90.0, 99.5];

/// The photo's luminance in stops from middle grey, white-balanced with `wb` (camera
/// multipliers), unedited: sensor-clipped pixels left out (unless that's nearly all of them).
fn scene_evs(cam: &[Vec3], color: &CameraColor, wb: Vec3) -> Vec<f32> {
    let m = color.cam_to_working();
    let mut clipped = Vec::new();
    let mut evs = Vec::with_capacity(cam.len());
    for c in cam {
        // As the shader: white balance, then clip at the sensor's white (1).
        let b = [0, 1, 2].map(|k| c[k].max(0.0) * wb[k]);
        let clip = b.iter().any(|&v| v >= 0.999);
        let b = b.map(|v| v.min(1.0));
        let rgb = [0, 1, 2].map(|r| m[r][0] * b[0] + m[r][1] * b[1] + m[r][2] * b[2]);
        let max = rgb[0].max(rgb[1]).max(rgb[2]);
        let y = (LUMA[0] * rgb[0] + LUMA[1] * rgb[1] + LUMA[2] * rgb[2])
            .max(LUMA[2] * max)
            .max(1e-7);
        let ev = (y / GREY as f32).log2();
        if clip {
            clipped.push(ev);
        } else {
            evs.push(ev);
        }
    }
    if evs.len() < cam.len() / 20 {
        evs.extend(clipped);
    }
    evs.sort_by(f32::total_cmp);
    evs
}

fn percentiles(sorted: &[f32]) -> [f32; 7] {
    PCTS.map(|p| {
        let x = p / 100.0 * (sorted.len() - 1) as f32;
        let i = x as usize;
        let j = (i + 1).min(sorted.len() - 1);
        sorted[i] + (sorted[j] - sorted[i]) * (x - i as f32)
    })
}

/// How far `l` puts the photo (percentiles `q`, stops) from the targets. `median`: where the
/// median should land (exposure aims at mid-grey; the other sliders keep what exposure chose).
fn loss(q: &[f32; 7], l: &Light, median: f32) -> f32 {
    let d = q.map(|ev| display_of_ev(tone_ev(ev + l.exposure, l)));
    let [p005, p10, p25, p50, p75, p90, p995] = d;
    let sq = |v: f32| v * v;
    // Pulls toward `target`: `over` per unit² above it, `under` below.
    let pull = |v: f32, target: f32, over: f32, under: f32| {
        if v > target {
            over * sq(v - target)
        } else {
            under * sq(target - v)
        }
    };
    let mut e = 3.0 * sq(p50 - median);
    // Each region of the range has a target, placed from the median (so a night scene, held
    // dark, keeps dark surroundings), and each slider mostly moves one of them:
    // whites: the brightest 0.5% just short of white; clipping costs far more than dim.
    e += pull(p995, (median + 0.48).min(0.92), 12.0, 0.6);
    // Highlights: the upper tones (90th percentile) brought in from the top, if crowding it.
    e += pull(p90, (median + 0.3).min(0.76), 3.0, 0.5);
    // Shadows: the lower tones (10th percentile) opened up, if sunk.
    e += pull(p10, (median - 0.26).max(0.03), 0.5, 3.0);
    // Blacks: the darkest 0.5% just above black; crushing costs far more than lifting.
    e += 100.0 * sq((0.004 - p005).max(0.0)) + pull(p005, 0.02, 2.0, 2.0);
    // Contrast: the middle half of the tones spread enough, but not too far.
    e += pull(p75 - p25, 0.28, 1.0, 1.5);
    // Gentle settings: each slider costs a little, and exposure beyond two stops more (a night
    // scene stays a night scene).
    let n = |v: f32| v / 100.0;
    e += 0.01
        * (sq(n(l.contrast))
            + sq(n(l.highlights))
            + sq(n(l.shadows))
            + sq(n(l.whites))
            + sq(n(l.blacks)));
    e + 0.03 * sq((l.exposure.abs() - 2.0).max(0.0))
}

/// Mid-grey on screen, the median's target.
const MEDIAN: f32 = 0.42;

/// Auto Light for the photo sampled as `cam` (camera RGB, see [`sample`]), white-balanced with
/// `wb`.
pub fn tone(cam: &[Vec3], color: &CameraColor, wb: Vec3) -> Light {
    let evs = scene_evs(cam, color, wb);
    if evs.is_empty() {
        return Light::default();
    }
    let q = percentiles(&evs);
    // 1. Exposure sets the brightness: over its whole range, the other sliders at zero.
    let mut l = Light::default();
    let mut best = f32::MAX;
    for k in -100..=100 {
        let e = k as f32 * 0.05;
        let c = loss(
            &q,
            &Light {
                exposure: e,
                ..Light::default()
            },
            MEDIAN,
        );
        if c < best {
            (best, l.exposure) = (c, e);
        }
    }
    // 2. The rest shape the range around it, holding the median where exposure put it (so they
    // don't stand in for exposure where its cost stopped it, as in a night scene).
    let median = display_of_ev(q[3] + l.exposure);
    let mut best = loss(&q, &l, median);
    for step in [20.0, 10.0, 5.0, 2.0, 1.0] {
        loop {
            let mut moved = false;
            for k in 0..5 {
                for dir in [-1.0, 1.0] {
                    let mut t = l.clone();
                    let (v, lo, hi) = match k {
                        0 => (&mut t.contrast, -40.0, 40.0),
                        // Highlights only recover and shadows only lift: the other way they'd
                        // stand in for contrast.
                        1 => (&mut t.highlights, -80.0, 0.0),
                        2 => (&mut t.shadows, 0.0, 60.0),
                        3 => (&mut t.whites, -60.0, 60.0),
                        _ => (&mut t.blacks, -60.0, 60.0),
                    };
                    *v = (*v + dir * step).clamp(lo, hi);
                    let c = loss(&q, &t, median);
                    if c < best - 1e-7 {
                        (best, l, moved) = (c, t, true);
                    }
                }
            }
            if !moved {
                break;
            }
        }
    }
    Light {
        exposure: (l.exposure * 100.0).round() / 100.0,
        contrast: l.contrast.round(),
        highlights: l.highlights.round(),
        shadows: l.shadows.round(),
        whites: l.whites.round(),
        blacks: l.blacks.round(),
    }
}

/// Auto white balance: `(temperature, tint)` on the sliders' scales, rounded as they show them.
///
/// Grey pixels (after Huo et al. 2006): starting from the camera's own estimate (As Shot), take
/// the pixels that look neutral under the current estimate, average their colour, and repeat
/// with a tightening tolerance. Coloured surfaces, however large, don't vote, so a blue car or a
/// lawn doesn't tint the result. With no neutral-looking pixels it stays As Shot. `None` when
/// too little of the photo is usable (nearly all clipped or black).
pub fn white_balance(cam: &[Vec3], color: &CameraColor) -> Option<(f32, f32)> {
    // Unclipped pixels bright enough for their colour to mean something.
    let mut bright: Vec<f32> = cam.iter().map(|c| c[0].max(c[1]).max(c[2])).collect();
    bright.sort_by(f32::total_cmp);
    let floor = bright.get(bright.len() * 99 / 100).copied().unwrap_or(0.0) * 0.01;
    let usable: Vec<&Vec3> = cam
        .iter()
        .filter(|c| {
            let m = c[0].max(c[1]).max(c[2]);
            m < 0.98 && m > floor.max(1e-4) && c.iter().all(|&v| v > 0.0)
        })
        .collect();
    if usable.len() < 64 {
        return None;
    }
    let m = color.cam_to_working();
    let saturation = |c: &Vec3, g: Vec3| {
        let b = [0, 1, 2].map(|k| c[k] * g[k]);
        let w = [0, 1, 2].map(|r| m[r][0] * b[0] + m[r][1] * b[1] + m[r][2] * b[2]);
        let (hi, lo) = (w[0].max(w[1]).max(w[2]), w[0].min(w[1]).min(w[2]));
        if hi > 0.0 {
            (hi - lo) / hi
        } else {
            1.0
        }
    };
    let need = (usable.len() / 100).max(32);
    let mut g = color.as_shot_multipliers();
    let mut found = false;
    for it in 0..16 {
        // Tolerance from loose to tight; widened (to 0.4) if too few pixels qualify.
        let mut tol = (0.4 - 0.025 * it as f32).max(0.08);
        let greys = loop {
            let greys: Vec<(&&Vec3, f32)> = usable
                .iter()
                .map(|c| (c, saturation(c, g)))
                .filter(|(_, s)| *s < tol)
                .collect();
            if greys.len() >= need || tol >= 0.4 {
                break greys;
            }
            tol *= 1.25;
        };
        if greys.len() < need {
            break;
        }
        // Their mean chromaticity (each pixel counts by how grey it looks, however bright) is
        // the light.
        let mut light = [0.0f64; 3];
        let mut total = 0.0f64;
        for (c, s) in &greys {
            let w = ((1.0 - s / tol) as f64).powi(2);
            let sum = (c[0] + c[1] + c[2]) as f64;
            for k in 0..3 {
                light[k] += w * c[k] as f64 / sum;
            }
            total += w;
        }
        let next = [0, 1, 2].map(|k| (total / light[k]) as f32);
        let lo = next.iter().copied().fold(f32::MAX, f32::min);
        let next = next.map(|v| v / lo);
        let moved = (0..3)
            .map(|k| (next[k] / g[k] - 1.0).abs())
            .fold(0.0, f32::max);
        (g, found) = (next, true);
        if moved < 1e-4 && it >= 8 {
            break;
        }
    }
    let (shot_t, shot_tint) = color.as_shot_temperature_tint();
    let (t, tint) = if found {
        let (t, tint) = color.temperature_tint_of([1.0 / g[0], 1.0 / g[1], 1.0 / g[2]]);
        // A scene that fools this (a sky, a lawn) mostly shifts the tint; the camera's tint is
        // seldom far off, so keep within ±15 of it. Only when it has one: `clamp` panics on a
        // NaN bound, and an out-of-gamut As Shot white can name a NaN or infinite tint.
        if shot_tint.is_finite() {
            (t, tint.clamp(shot_tint - 15.0, shot_tint + 15.0))
        } else {
            (t, tint)
        }
    } else {
        (shot_t, shot_tint)
    };
    on_slider_scale(t, tint)
}

/// Camera RGB of the `size × size` preview pixels centred on frame position `at` (0..1 across
/// the framed output, as the editor shows it: crop, turns, flip, straighten and lens correction
/// mapped as [`sample`] maps them), for the white balance picker. Empty when `at` falls outside
/// the photo. The patch is square in preview pixels and clipped to the preview's edges.
#[allow(clippy::too_many_arguments)]
pub fn sample_around(
    width: u32,
    height: u32,
    pixel: impl Fn(u32, u32) -> Vec3,
    crop: &Crop,
    lens: &LensTable,
    full_w: u32,
    full_h: u32,
    at: [f32; 2],
    size: u32,
) -> Vec<Vec3> {
    let inside = |p: [f32; 2]| (0.0..=1.0).contains(&p[0]) && (0.0..=1.0).contains(&p[1]);
    if width == 0 || height == 0 || !inside(at) {
        return Vec::new();
    }
    let uv = Geometry::new(crop, full_w as f32, full_h as f32).apply(at);
    if !inside(uv) {
        return Vec::new();
    }
    let ([u, v], gain) = UvToSource::new(lens, full_w, full_h).map_with_gain(uv);
    let cx = ((u * width as f32) as i64).min(width as i64 - 1);
    let cy = ((v * height as f32) as i64).min(height as i64 - 1);
    let r = (size / 2) as i64;
    let mut out = Vec::with_capacity((size * size) as usize);
    for y in (cy - r).max(0)..=(cy + r).min(height as i64 - 1) {
        for x in (cx - r).max(0)..=(cx + r).min(width as i64 - 1) {
            out.push(pixel(x as u32, y as u32).map(|c| c * gain));
        }
    }
    out
}

/// White balance that makes the picked patch `cam` (camera RGB, see [`sample_around`]) neutral
/// grey: `(temperature, tint)` on the sliders' scales, rounded as they show them. The patch's
/// mean colour is taken as the light's colour; pixels clipped in any channel (their colour is
/// wrong) and black ones are left out. `None` when no pixel is usable.
pub fn white_balance_at(cam: &[Vec3], color: &CameraColor) -> Option<(f32, f32)> {
    let mut sum = [0.0f64; 3];
    let mut n = 0;
    for c in cam {
        if c.iter().all(|&v| v > 1e-4 && v < 0.98) {
            for k in 0..3 {
                sum[k] += c[k] as f64;
            }
            n += 1;
        }
    }
    if n == 0 {
        return None;
    }
    let (t, tint) = color.temperature_tint_of(sum.map(|v| (v / n as f64) as f32));
    on_slider_scale(t, tint)
}

/// `(temperature, tint)` rounded and clamped as the sliders show them; `None` if not finite.
fn on_slider_scale(t: f32, tint: f32) -> Option<(f32, f32)> {
    if !(t.is_finite() && tint.is_finite()) {
        return None;
    }
    Some((
        ((t / 50.0).round() * 50.0).clamp(2000.0, 12000.0),
        tint.round().clamp(-150.0, 150.0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{invert, mul_vec, white_xy, REC2020_TO_XYZ};

    /// A camera whose RGB is Rec.2020 (as shot: no correction), so scenes can be written in
    /// stops directly.
    fn camera() -> CameraColor {
        CameraColor {
            xyz_to_cam: invert(&REC2020_TO_XYZ).unwrap(),
            as_shot_wb: [1.0; 3],
        }
    }

    /// Greys whose stops from middle grey follow `evs`, kept under the sensor's white.
    fn scene(evs: impl Iterator<Item = f32>) -> Vec<Vec3> {
        evs.map(|ev| [(GREY as f32 * ev.exp2()).min(0.95); 3])
            .collect()
    }

    /// A deterministic spread: `n` stops values evenly through a normal(mean, sd) by inverse CDF
    /// (approximated well enough with a logistic).
    fn normal(mean: f32, sd: f32, n: usize) -> impl Iterator<Item = f32> {
        (0..n).map(move |i| {
            let p = (i as f32 + 0.5) / n as f32;
            mean + sd * 0.5513 * (p / (1.0 - p)).ln()
        })
    }

    /// The display percentiles a result gives.
    fn shown(cam: &[Vec3], l: &Light) -> [f32; 7] {
        let q = percentiles(&scene_evs(cam, &camera(), [1.0; 3]));
        q.map(|ev| display_of_ev(tone_ev(ev + l.exposure, l)))
    }

    #[test]
    fn tone_ev_is_the_shader_formula() {
        let zero = Light::default();
        for ev in [-8.0, -2.0, 0.0, 1.5, 4.0] {
            assert_eq!(tone_ev(ev, &zero), ev);
        }
        // Contrast scales around grey; highlights only move the top, shadows only the bottom.
        let c = Light {
            contrast: 50.0,
            ..zero.clone()
        };
        assert!((tone_ev(2.0, &c) - 2.6).abs() < 1e-6 && tone_ev(0.0, &c) == 0.0);
        let h = Light {
            highlights: -100.0,
            ..zero.clone()
        };
        assert!(tone_ev(4.0, &h) < 3.0 && (tone_ev(-3.0, &h) + 3.0).abs() < 1e-6);
        let s = Light {
            shadows: 100.0,
            ..zero
        };
        assert!(tone_ev(-7.0, &s) > -6.0 && (tone_ev(2.0, &s) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn display_table_is_agx() {
        let agx = Agx::sdr(Display::Srgb);
        for ev in [-9.3, -4.0, 0.0, 0.37, 2.5, 6.0] {
            let want = agx.display([GREY * (ev as f64).exp2(); 3])[0] as f32;
            assert!(
                (display_of_ev(ev) - want).abs() < 2e-4,
                "{ev}: {} vs {want}",
                display_of_ev(ev)
            );
        }
    }

    #[test]
    fn a_dark_photo_gets_brighter() {
        let cam = scene(normal(-3.5, 1.2, 4000));
        let l = tone(&cam, &camera(), [1.0; 3]);
        assert!(l.exposure > 2.0, "{l:?}");
        // Most of the way to mid-grey (beyond two stops, exposure costs extra).
        assert!(shown(&cam, &l)[3] > 0.3, "{l:?}");
    }

    #[test]
    fn a_bright_photo_gets_darker() {
        let cam = scene(normal(1.5, 0.8, 4000));
        let l = tone(&cam, &camera(), [1.0; 3]);
        assert!(l.exposure < -1.0, "{l:?}");
        assert!(shown(&cam, &l)[6] <= 0.97, "{l:?}");
    }

    #[test]
    fn a_well_exposed_photo_is_left_mostly_alone() {
        let cam = scene(normal(-0.6, 1.6, 4000));
        let l = tone(&cam, &camera(), [1.0; 3]);
        assert!(l.exposure.abs() < 0.6, "{l:?}");
        for v in [l.contrast, l.highlights, l.shadows, l.whites, l.blacks] {
            assert!(v.abs() <= 30.0, "{l:?}");
        }
    }

    #[test]
    fn a_high_contrast_photo_keeps_both_ends() {
        // Deep shadow and bright sky, 10 stops apart.
        let cam = scene(normal(-6.0, 0.8, 2000).chain(normal(3.0, 0.6, 2000)));
        let l = tone(&cam, &camera(), [1.0; 3]);
        let d = shown(&cam, &l);
        assert!(d[6] <= 0.97, "highlights clip: {l:?} {d:?}");
        assert!(d[0] >= 0.003, "shadows crushed: {l:?} {d:?}");
        // (AgX spans 16.5 stops, so 10 needs no pulling in; both ends are what matters.)
    }

    #[test]
    fn a_flat_photo_gets_contrast() {
        let cam = scene(normal(-0.5, 0.35, 4000));
        let l = tone(&cam, &camera(), [1.0; 3]);
        assert!(l.contrast > 10.0, "{l:?}");
    }

    #[test]
    fn clipped_pixels_dont_drag_exposure_down() {
        // A dark room with a blown window: the window can't be recovered, so exposure follows
        // the room.
        let mut cam = scene(normal(-3.0, 1.0, 3000));
        cam.extend(std::iter::repeat_n([1.0; 3], 600));
        let with = tone(&cam, &camera(), [1.0; 3]);
        let without = tone(&cam[..3000], &camera(), [1.0; 3]);
        assert!(
            (with.exposure - without.exposure).abs() < 0.11,
            "{with:?} {without:?}"
        );
    }

    #[test]
    fn a_bright_sky_is_recovered_with_highlights() {
        // A darker foreground under a bright sky: exposure follows the foreground, highlights
        // bring the sky in, whites keep its top off white.
        let cam = scene(normal(-2.0, 1.2, 2500).chain(normal(1.5, 0.7, 1500)));
        let l = tone(&cam, &camera(), [1.0; 3]);
        assert!(l.exposure > 0.0, "{l:?}");
        assert!(l.highlights < -20.0, "{l:?}");
        assert!(shown(&cam, &l)[6] <= 0.92, "{l:?}");
    }

    #[test]
    fn deep_shadows_are_lifted_with_shadows() {
        // A wide-range scene whose darkest tenth sits far below the rest.
        let cam = scene(normal(-6.0, 0.8, 2000).chain(normal(3.0, 0.6, 2000)));
        let l = tone(&cam, &camera(), [1.0; 3]);
        assert!(l.shadows > 20.0, "{l:?}");
        let zero = Light {
            exposure: l.exposure,
            ..Light::default()
        };
        assert!(shown(&cam, &l)[1] > shown(&cam, &zero)[1] + 0.03, "{l:?}");
    }

    #[test]
    fn highlights_only_recover_and_shadows_only_lift() {
        for cam in [
            scene(normal(-0.5, 0.35, 4000)),
            scene(normal(1.5, 0.8, 4000)),
            scene(normal(-3.5, 1.2, 4000)),
        ] {
            let l = tone(&cam, &camera(), [1.0; 3]);
            assert!(l.highlights <= 0.0 && l.shadows >= 0.0, "{l:?}");
        }
    }

    #[test]
    fn auto_tone_is_deterministic() {
        let cam = scene(normal(-2.0, 2.0, 3000));
        assert_eq!(
            tone(&cam, &camera(), [1.0; 3]),
            tone(&cam, &camera(), [1.0; 3])
        );
    }

    /// The light of `t` kelvin in camera RGB.
    fn light(t: f32) -> Vec3 {
        let (x, y) = white_xy(t, 0.0);
        mul_vec(&camera().xyz_to_cam, [x / y, 1.0, (1.0 - x - y) / y])
    }

    /// A scene lit by `t` kelvin, in camera RGB: half of it one strong blue (a car, the sky: what
    /// fools an average), a quarter neutral surfaces, a quarter assorted colours.
    fn lit_scene(t: f32) -> Vec<Vec3> {
        let l = light(t);
        let k = 0.3 / l[1];
        (0..4000)
            .map(|i| {
                let a = 0.2 + 0.3 * ((i * 7919 % 1000) as f32 / 1000.0);
                let refl = match i % 4 {
                    0 | 1 => [0.25 * a, 0.45 * a, 1.0 * a],
                    2 => [a; 3],
                    _ => {
                        let h = i as f32 * 2.399; // golden-angle hues
                        [0.0, 2.094, 4.189].map(|o| a * (1.0 + 0.6 * (h + o).cos()))
                    }
                };
                [0, 1, 2].map(|j| refl[j] * l[j] * k)
            })
            .collect()
    }

    /// The camera's own guess was `shot` kelvin.
    fn shot_at(shot: f32) -> CameraColor {
        let l = light(shot);
        CameraColor {
            as_shot_wb: [1.0 / l[0], 1.0 / l[1], 1.0 / l[2]],
            ..camera()
        }
    }

    #[test]
    fn white_balance_finds_the_light() {
        // The camera's guess roughly right (as its own auto white balance usually is): the
        // neutral surfaces decide, the big blue doesn't. (Starting far off, with half the frame
        // one colour, some of that colour can look greyer than the greys: a known limit of
        // grey-pixel methods, and why the tint stays near the camera's.)
        for (t, shot) in [
            (2850.0, 3100.0),
            (4000.0, 4400.0),
            (6500.0, 5900.0),
            (5000.0, 5000.0),
        ] {
            let (got, tint) = white_balance(&lit_scene(t), &shot_at(shot)).unwrap();
            assert!((got - t).abs() <= 0.05 * t, "{t} K (shot {shot}) → {got} K");
            assert!(tint.abs() <= 5.0, "{t} K → tint {tint}");
        }
    }

    #[test]
    fn white_balance_without_neutrals_stays_as_shot() {
        // Only the strong blue: nothing looks grey, so the camera's estimate stands.
        let blue: Vec<Vec3> = lit_scene(4000.0).into_iter().step_by(4).collect();
        let c = shot_at(4500.0);
        let (t, tint) = c.as_shot_temperature_tint();
        assert_eq!(
            white_balance(&blue, &c),
            Some(((t / 50.0).round() * 50.0, tint.round()))
        );
    }

    #[test]
    fn white_balance_needs_usable_pixels() {
        assert_eq!(white_balance(&vec![[1.0; 3]; 1000], &camera()), None);
        assert_eq!(white_balance(&[], &camera()), None);
    }

    #[test]
    fn samples_only_inside_the_crop() {
        // Left half dark, right half bright; a crop of the right half sees only bright.
        let px = |x: u32, _: u32| if x < 50 { [0.01; 3] } else { [0.5; 3] };
        let all = sample(
            100,
            50,
            px,
            &Crop::default(),
            &LensTable::identity(),
            1000,
            500,
            20,
        );
        assert_eq!(all.len(), 400);
        assert!(all.iter().any(|c| c[0] < 0.1) && all.iter().any(|c| c[0] > 0.1));
        let crop = Crop {
            rect: [0.5, 0.0, 0.5, 1.0],
            ..Crop::default()
        };
        let right = sample(100, 50, px, &crop, &LensTable::identity(), 1000, 500, 20);
        assert!(!right.is_empty() && right.iter().all(|c| c[0] > 0.1));
    }

    #[test]
    fn samples_through_the_lens_as_the_shader_does() {
        use crate::lens::{lens_table, radial, LensCorrection, LensProfile, LensSource};
        // A barrel lens with strong falloff, on a 3:2 source; the preview is a quarter size.
        let profile = LensProfile {
            source: LensSource::Embedded,
            name: "test".into(),
            distortion: Some([radial(|r| 1.0 - 0.08 * r * r); 3]),
            chromatic: false,
            vignetting: Some(radial(|r| 1.0 + 0.8 * r * r)),
        };
        let s = LensCorrection {
            profile: true,
            ..Default::default()
        };
        let (fw, fh, w, h, n) = (1200u32, 800u32, 300u32, 200u32, 16u32);
        let table = lens_table(Some(&profile), &s, fw as f32, fh as f32);
        assert!(table.active);
        // Each pixel encodes its own position, so the samples say where they were read from.
        let px = |x: u32, y: u32| [x as f32, y as f32, 1.0];
        let got = sample(w, h, px, &Crop::default(), &table, fw, fh, n);
        assert_eq!(got.len(), (n * n) as usize);

        // `fs_main`, written out: r = |(uv − ½)·2·(w, h)/diag|, src = clamp(½ + k(r)·(uv − ½)),
        // gain = vignetting at r·k(r); `lens_at` interpolates the knots over [0, LENS_RMAX].
        let lens_at = |r: f32, c: usize| {
            let t = (r / crate::lens::LENS_RMAX).clamp(0.0, 1.0) * 31.0;
            let i = (t as usize).min(30);
            let (a, b) = (table.knots[i][c], table.knots[i + 1][c]);
            a + (b - a) * (t - i as f32)
        };
        let diag = (fw as f32).hypot(fh as f32);
        let (ax, ay) = (fw as f32 / diag, fh as f32 / diag);
        let mut corner_gain: f32 = 1.0;
        for j in 0..n {
            for i in 0..n {
                let uv = [(i as f32 + 0.5) / n as f32, (j as f32 + 0.5) / n as f32];
                let d = [uv[0] - 0.5, uv[1] - 0.5];
                let r = (d[0] * 2.0 * ax).hypot(d[1] * 2.0 * ay);
                let k = lens_at(r, 1);
                let src = [
                    (0.5 + k * d[0]).clamp(0.0, 1.0),
                    (0.5 + k * d[1]).clamp(0.0, 1.0),
                ];
                let gain = lens_at(r * k, 3);
                let x = ((src[0] * w as f32) as u32).min(w - 1) as f32;
                let y = ((src[1] * h as f32) as u32).min(h - 1) as f32;
                let c = got[(j * n + i) as usize];
                let at = format!("uv {uv:?}");
                assert!((c[2] - gain).abs() < 1e-5, "{at}: gain {} vs {gain}", c[2]);
                assert!(
                    (c[0] / c[2] - x).abs() < 1e-3,
                    "{at}: x {} vs {x}",
                    c[0] / c[2]
                );
                assert!(
                    (c[1] / c[2] - y).abs() < 1e-3,
                    "{at}: y {} vs {y}",
                    c[1] / c[2]
                );
                corner_gain = corner_gain.max(gain);
            }
        }
        // The corrected corners are brighter, and they read nearer the centre than uv itself.
        assert!(corner_gain > 1.4, "{corner_gain}");
        let first = got[0];
        assert!(first[0] / first[2] > 0.5 / n as f32 * w as f32);
    }

    #[test]
    fn white_balance_survives_an_as_shot_white_with_no_finite_tint() {
        // As Shot light whose XYZ is exactly (3, 0, −1) (camera RGB = XYZ): x = 1.5, y = 0 puts
        // the CIE 1960 uv denominator at 0, so it names no finite tint. The ±15 tint clamp
        // around it must not panic.
        let c = CameraColor {
            xyz_to_cam: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            as_shot_wb: [1.0 / 3.0, f32::INFINITY, -1.0],
        };
        let (_, shot_tint) = c.as_shot_temperature_tint();
        assert!(
            !shot_tint.is_finite(),
            "test needs a non-finite tint: {shot_tint}"
        );
        let greys = scene(normal(-0.5, 1.5, 2000));
        if let Some((t, tint)) = white_balance(&greys, &c) {
            assert!(t.is_finite() && tint.is_finite());
        }
    }

    #[test]
    fn picking_a_grey_makes_it_neutral() {
        // A grey card under 2850–9000 K, picked from a patch: its light comes back.
        for t in [2850.0, 4000.0, 5500.0, 6500.0, 9000.0] {
            let l = light(t);
            let patch = vec![l.map(|c| c * 0.4 / l[1]); 25];
            let (got, tint) = white_balance_at(&patch, &camera()).unwrap();
            assert!((got - t).abs() <= 0.02 * t + 25.0, "{t} K → {got} K");
            assert!(tint.abs() <= 1.0, "{t} K → tint {tint}");
        }
        // Brightness doesn't matter, only colour. Scale by the brightest channel so the bright
        // patch stays under the clip (at 4000 K red is above green).
        let l = light(4000.0);
        let top = l.iter().copied().fold(0.0f32, f32::max);
        let dim = vec![l.map(|c| c * 0.05 / top); 25];
        let bright = vec![l.map(|c| c * 0.8 / top); 25];
        assert!(white_balance_at(&bright, &camera()).is_some());
        assert_eq!(
            white_balance_at(&dim, &camera()),
            white_balance_at(&bright, &camera())
        );
    }

    #[test]
    fn picking_leaves_out_clipped_and_black_pixels() {
        let l = light(3500.0);
        let grey = l.map(|c| c * 0.3 / l[1]);
        let clean = white_balance_at(&[grey; 9], &camera());
        assert!(clean.is_some());
        // A blown pixel (green clipped, so it looks magenta) and a black one don't count.
        let mut patch = vec![grey; 7];
        patch.push([0.9, 1.0, 0.7]);
        patch.push([0.0; 3]);
        assert_eq!(white_balance_at(&patch, &camera()), clean);
        assert_eq!(white_balance_at(&[[1.0; 3]; 25], &camera()), None);
        assert_eq!(white_balance_at(&[[0.0; 3]; 25], &camera()), None);
        assert_eq!(white_balance_at(&[], &camera()), None);
    }

    /// Mean source position of a patch read from a preview whose pixels encode their position.
    fn centre(s: &[Vec3]) -> (f32, f32) {
        let n = s.len() as f32;
        (
            s.iter().map(|c| c[0]).sum::<f32>() / n,
            s.iter().map(|c| c[1]).sum::<f32>() / n,
        )
    }

    #[test]
    fn sample_around_reads_the_spot_under_the_cursor() {
        let px = |x: u32, y: u32| [x as f32, y as f32, 1.0];
        let id = LensTable::identity();
        let at = |crop: &Crop, f: [f32; 2]| sample_around(100, 50, px, crop, &id, 1000, 500, f, 5);
        // Uncropped: frame position = source position; 5 × 5 pixels around it.
        let s = at(&Crop::default(), [0.25, 0.5]);
        assert_eq!(s.len(), 25);
        assert_eq!(centre(&s), (25.0, 25.0));
        // Cropped to the right half: the frame's left edge is the middle of the source.
        let right = Crop {
            rect: [0.5, 0.0, 0.5, 1.0],
            ..Crop::default()
        };
        assert_eq!(centre(&at(&right, [0.1, 0.5])), (55.0, 25.0));
        // A quarter turn clockwise: the frame's top-left corner is the source's bottom-left.
        let turned = Crop {
            turns: 1,
            ..Crop::default()
        };
        let (x, y) = centre(&at(&turned, [0.1, 0.1]));
        assert!(x < 15.0 && y > 40.0, "({x}, {y})");
        // Flipped: left and right swap.
        let flipped = Crop {
            flip: true,
            ..Crop::default()
        };
        let (x, _) = centre(&at(&flipped, [0.1, 0.5]));
        assert!(x > 85.0, "{x}");
        // At a corner the patch is clipped to the preview; outside the frame, nothing.
        assert_eq!(at(&Crop::default(), [0.0, 0.0]).len(), 9);
        assert!(at(&Crop::default(), [1.2, 0.5]).is_empty());
    }

    #[test]
    fn sample_around_follows_a_straightened_crop() {
        // Straightened by 10°: each frame position reads the source pixel the shader shows there.
        let px = |x: u32, y: u32| [x as f32, y as f32, 1.0];
        let id = LensTable::identity();
        let crop = Crop {
            angle: 10.0,
            rect: [0.2, 0.2, 0.6, 0.6],
            ..Crop::default()
        };
        let g = Geometry::new(&crop, 1000.0, 500.0);
        for f in [[0.5, 0.5], [0.1, 0.2], [0.9, 0.8]] {
            let s = sample_around(100, 50, px, &crop, &id, 1000, 500, f, 1);
            assert_eq!(s.len(), 1);
            let [u, v] = g.apply(f);
            assert_eq!(s[0][0], (u * 100.0).floor(), "{f:?}");
            assert_eq!(s[0][1], (v * 50.0).floor(), "{f:?}");
        }
        // Off centre, straightening moves where a frame position reads from.
        let level = Crop {
            rect: crop.rect,
            ..Crop::default()
        };
        let a = sample_around(100, 50, px, &crop, &id, 1000, 500, [0.1, 0.1], 1);
        let b = sample_around(100, 50, px, &level, &id, 1000, 500, [0.1, 0.1], 1);
        assert_ne!(a, b);
    }
}
