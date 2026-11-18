//! Per-photo maps for Clarity and Dehaze, computed once from a small copy of the photo.
//!
//! Both effects compare a pixel with its surroundings over tens of pixels. Rather than blur at
//! full resolution (slow in the preview, and wrong at export tile edges), the smooth part is
//! computed here on a copy whose long edge is [`MAP_EDGE`], with the **fast guided filter**
//! (He & Sun, 2015): the filter is fitted at low resolution as `q = a·I + b`, and the shader
//! rebuilds `q` at full resolution from the full-resolution guide `I` and bilinearly sampled
//! `a`, `b`. Edges in `I` stay sharp in `q`, so there are no halos.
//!
//! Everything here depends only on the photo (as-shot white balance, no edits), so the maps are
//! made once per photo and serve the preview and the export alike.
//!
//! - **Clarity:** the guided filter of the as-shot log2 luminance by itself (an edge-preserving
//!   blur). The shader boosts `L − q` (local contrast) in the mid-tones.
//! - **Dehaze:** the dark channel prior (He, Sun & Tang, 2009). The haze colour ("airlight") is
//!   the average of the brightest 0.1% of the dark channel; transmission is
//!   `1 − ω · dark(I / A)`, refined by the guided filter with the normalised luminance as guide.

use half::f16;
use tonality_core::color::{mul, CameraColor};

/// Long edge of the maps, in pixels.
pub const MAP_EDGE: usize = 512;
/// Clarity's neighbourhood and edge threshold: radius as a share of the long edge, and the
/// variance (in stops²) below which detail counts as texture to smooth rather than an edge.
const CLARITY_RADIUS: f32 = 0.02;
const CLARITY_EPS: f32 = 0.25;
/// Dehaze: dark-channel patch and guided-filter radius (share of the long edge), how much haze
/// counts (ω), and the guided filter's regulariser (normalised luminance²).
const DARK_RADIUS: f32 = 0.002;
const HAZE_RADIUS: f32 = 0.015;
const OMEGA: f32 = 0.95;
const HAZE_EPS: f32 = 1e-4;

/// Rec.2020 luminance weights.
const LUMA_2020: [f32; 3] = [0.2627, 0.6780, 0.0593];

#[derive(Debug, Clone, PartialEq)]
pub struct PresenceMaps {
    pub width: u32,
    pub height: u32,
    /// Per pixel: clarity `a`, `b` (guide: as-shot log2 luminance), haze `a`, `b` (guide:
    /// as-shot luminance ÷ the airlight's).
    pub data: Vec<[f32; 4]>,
    /// The haze colour in camera RGB, before white balance (developed like a pixel).
    pub airlight: [f32; 3],
}

/// What the shader needs to rebuild the guides: as-shot luminance of camera RGB is
/// `dot(weights, clamp(raw · wb, 0, clip))`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    pub weights: [f32; 3],
    pub wb: [f32; 3],
    pub clip: f32,
}

impl Guide {
    pub fn new(color: &CameraColor) -> Self {
        let m = color.cam_to_working();
        let weights = [0, 1, 2].map(|j| (0..3).map(|i| LUMA_2020[i] * m[i][j]).sum());
        Guide {
            weights,
            wb: color.as_shot_multipliers(),
            clip: 1.0,
        }
    }

    pub fn luminance(&self, raw: [f32; 3]) -> f32 {
        (0..3)
            .map(|k| self.weights[k] * (raw[k] * self.wb[k]).clamp(0.0, self.clip))
            .sum()
    }
}

/// Whether `edit` needs the maps (Clarity or Dehaze, globally or in an enabled mask).
pub fn uses_maps(edit: &tonality_core::EditState) -> bool {
    let e = &edit.effects;
    e.clarity != 0.0
        || e.dehaze != 0.0
        || edit
            .masks
            .iter()
            .any(|m| m.enabled && (m.adjust.clarity != 0.0 || m.adjust.dehaze != 0.0))
}

/// Log2 luminance as the clarity guide sees it (floored, so black stays finite).
pub fn log_luminance(l: f32) -> f32 {
    l.max(1e-5).log2()
}

/// Maps for camera-space RGB16F `pixels` (`width × height`, interleaved, before white balance).
pub fn presence_maps(pixels: &[u16], width: u32, height: u32, color: &CameraColor) -> PresenceMaps {
    let (w, h) = (width.max(1) as usize, height.max(1) as usize);
    let s = (MAP_EDGE as f32 / w.max(h) as f32).min(1.0);
    let (mw, mh) = (
        ((w as f32 * s).round() as usize).max(1),
        ((h as f32 * s).round() as usize).max(1),
    );
    let raw = downsample(pixels, w, h, mw, mh);
    let guide = Guide::new(color);
    let to_working = mul(
        &color.cam_to_working(),
        &[
            [guide.wb[0], 0.0, 0.0],
            [0.0, guide.wb[1], 0.0],
            [0.0, 0.0, guide.wb[2]],
        ],
    );
    let edge = mw.max(mh) as f32;
    let radius = |share: f32| ((share * edge).round() as usize).max(1);

    // Clarity: self-guided filter of log luminance.
    let lum: Vec<f32> = raw.iter().map(|&c| guide.luminance(c)).collect();
    let log: Vec<f32> = lum.iter().map(|&l| log_luminance(l)).collect();
    let (ca, cb) = guided(&log, &log, mw, mh, radius(CLARITY_RADIUS), CLARITY_EPS);

    // Dehaze: dark channel of as-shot working-space colour.
    let rgb: Vec<[f32; 3]> = raw
        .iter()
        .map(|c| {
            let c = c.map(|v| v.max(0.0));
            [0, 1, 2].map(|i| {
                (0..3)
                    .map(|k| to_working[i][k] * c[k])
                    .sum::<f32>()
                    .clamp(0.0, guide.clip.max(1.0))
            })
        })
        .collect();
    let dr = radius(DARK_RADIUS);
    let dark = min_filter(
        &rgb.iter()
            .map(|c| c[0].min(c[1]).min(c[2]))
            .collect::<Vec<_>>(),
        mw,
        mh,
        dr,
    );
    let mut order: Vec<usize> = (0..dark.len()).collect();
    order.sort_by(|&a, &b| dark[b].total_cmp(&dark[a]));
    let top = &order[..(dark.len() / 1000).max(1)];
    let n = top.len() as f32;
    let mean = |f: &dyn Fn(usize) -> [f32; 3]| {
        let mut acc = [0.0f32; 3];
        for &i in top {
            let v = f(i);
            (0..3).for_each(|k| acc[k] += v[k] / n);
        }
        acc
    };
    let airlight = mean(&|i| raw[i].map(|v| v.max(0.0)));
    let a_rgb = mean(&|i| rgb[i]).map(|v| v.max(1e-4));
    let a_lum = guide.luminance(airlight).max(1e-6);
    let dark_norm = min_filter(
        &rgb.iter()
            .map(|c| (c[0] / a_rgb[0]).min(c[1] / a_rgb[1]).min(c[2] / a_rgb[2]))
            .collect::<Vec<_>>(),
        mw,
        mh,
        dr,
    );
    let t: Vec<f32> = dark_norm
        .iter()
        .map(|&d| (1.0 - OMEGA * d).clamp(0.0, 1.0))
        .collect();
    let i_norm: Vec<f32> = lum.iter().map(|&l| l / a_lum).collect();
    let (ha, hb) = guided(&i_norm, &t, mw, mh, radius(HAZE_RADIUS), HAZE_EPS);

    let data = (0..mw * mh).map(|i| [ca[i], cb[i], ha[i], hb[i]]).collect();
    PresenceMaps {
        width: mw as u32,
        height: mh as u32,
        data,
        airlight,
    }
}

impl PresenceMaps {
    /// RGBA16F little-endian bytes, for a texture upload.
    pub fn rgba16f(&self) -> Vec<u8> {
        self.data
            .iter()
            .flat_map(|p| p.iter().flat_map(|&v| f16::from_f32(v).to_le_bytes()))
            .collect()
    }
}

/// Box-average camera RGB16F down to `mw × mh`.
fn downsample(pixels: &[u16], w: usize, h: usize, mw: usize, mh: usize) -> Vec<[f32; 3]> {
    let mut out = vec![[0.0f32; 3]; mw * mh];
    for (oy, row) in out.chunks_mut(mw).enumerate() {
        let (y0, y1) = (oy * h / mh, ((oy + 1) * h / mh).max(oy * h / mh + 1));
        for (ox, o) in row.iter_mut().enumerate() {
            let (x0, x1) = (ox * w / mw, ((ox + 1) * w / mw).max(ox * w / mw + 1));
            let mut acc = [0.0f32; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = (y * w + x) * 3;
                    for k in 0..3 {
                        acc[k] += f16::from_bits(pixels[p + k]).to_f32();
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as f32;
            *o = acc.map(|v| v / n);
        }
    }
    out
}

/// Mean over the `(2r+1)²` window around each pixel (clamped at the borders), via a summed-area
/// table.
fn box_mean(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut sat = vec![0.0f64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0.0f64;
        for x in 0..w {
            row += v[y * w + x] as f64;
            sat[(y + 1) * (w + 1) + x + 1] = sat[y * (w + 1) + x + 1] + row;
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            let s = sat[y1 * (w + 1) + x1] - sat[y0 * (w + 1) + x1] - sat[y1 * (w + 1) + x0]
                + sat[y0 * (w + 1) + x0];
            out[y * w + x] = (s / ((y1 - y0) * (x1 - x0)) as f64) as f32;
        }
    }
    out
}

/// Minimum over the `(2r+1)²` window (separable).
fn min_filter(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            tmp[y * w + x] = v[y * w + x0..y * w + x1]
                .iter()
                .copied()
                .fold(f32::INFINITY, f32::min);
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            out[y * w + x] = (y0..y1)
                .map(|yy| tmp[yy * w + x])
                .fold(f32::INFINITY, f32::min);
        }
    }
    out
}

/// Guided filter of `p` with guide `i`: the averaged coefficients `(ā, b̄)`, so that
/// `q = ā·I + b̄` for the guide at any resolution.
fn guided(i: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> (Vec<f32>, Vec<f32>) {
    let mean_i = box_mean(i, w, h, r);
    let mean_p = box_mean(p, w, h, r);
    let ii: Vec<f32> = i.iter().map(|v| v * v).collect();
    let ip: Vec<f32> = i.iter().zip(p).map(|(a, b)| a * b).collect();
    let corr_i = box_mean(&ii, w, h, r);
    let corr_ip = box_mean(&ip, w, h, r);
    let mut a = vec![0.0f32; w * h];
    let mut b = vec![0.0f32; w * h];
    for k in 0..w * h {
        let var = (corr_i[k] - mean_i[k] * mean_i[k]).max(0.0);
        let cov = corr_ip[k] - mean_i[k] * mean_p[k];
        a[k] = cov / (var + eps);
        b[k] = mean_p[k] - a[k] * mean_i[k];
    }
    (box_mean(&a, w, h, r), box_mean(&b, w, h, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color() -> CameraColor {
        CameraColor {
            xyz_to_cam: tonality_core::color::IDENTITY,
            as_shot_wb: [1.0; 3],
        }
    }

    fn image(w: usize, h: usize, f: impl Fn(usize, usize) -> [f32; 3]) -> Vec<u16> {
        let mut out = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                out.extend(f(x, y).map(|v| f16::from_f32(v).to_bits()));
            }
        }
        out
    }

    #[test]
    fn a_flat_photo_has_no_local_contrast() {
        let px = image(64, 48, |_, _| [0.2, 0.2, 0.2]);
        let m = presence_maps(&px, 64, 48, &color());
        let g = Guide::new(&color());
        let l = log_luminance(g.luminance([0.2; 3]));
        for p in &m.data {
            assert!(
                (p[0] * l + p[1] - l).abs() < 1e-3,
                "base equals the photo: {p:?}"
            );
        }
    }

    #[test]
    fn clarity_base_keeps_a_strong_edge() {
        // Left half dark, right half bright (4 stops apart): the base follows the edge.
        let px = image(128, 64, |x, _| if x < 64 { [0.02; 3] } else { [0.32; 3] });
        let m = presence_maps(&px, 128, 64, &color());
        let g = Guide::new(&color());
        let at = |x: usize, v: f32| {
            let p = m.data[32 * m.width as usize + x];
            let l = log_luminance(g.luminance([v; 3]));
            p[0] * l + p[1] - l
        };
        assert!(
            at(60, 0.02).abs() < 0.2 && at(68, 0.32).abs() < 0.2,
            "{} {}",
            at(60, 0.02),
            at(68, 0.32)
        );
    }

    #[test]
    fn airlight_is_the_haze_not_the_shadows() {
        // Hazy bright top (grey-blue), clear dark bottom with saturated colours.
        let px = image(96, 96, |_, y| {
            if y < 40 {
                [0.7, 0.75, 0.8]
            } else {
                [0.3, 0.05, 0.02]
            }
        });
        let m = presence_maps(&px, 96, 96, &color());
        assert!((m.airlight[2] - 0.8).abs() < 0.02, "{:?}", m.airlight);
        // Transmission: low in the haze, high in the clear part.
        let g = Guide::new(&color());
        let a_lum = g.luminance(m.airlight);
        let t = |y: usize, c: [f32; 3]| {
            let p = m.data[y * m.width as usize + 48];
            p[2] * g.luminance(c) / a_lum + p[3]
        };
        assert!(t(10, [0.7, 0.75, 0.8]) < 0.2, "{}", t(10, [0.7, 0.75, 0.8]));
        assert!(
            t(80, [0.3, 0.05, 0.02]) > 0.8,
            "{}",
            t(80, [0.3, 0.05, 0.02])
        );
    }
}
