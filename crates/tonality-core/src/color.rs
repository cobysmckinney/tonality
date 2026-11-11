//! Colour science shared by the decoder, the preview and the exporter.
//!
//! Conventions:
//! - Working space is **linear Rec.2020, D65**.
//! - Matrices are row-major `[[row0], [row1], [row2]]` and multiply column vectors.
//! - White balance happens in *camera* space as per-channel multipliers (like dcraw/RawTherapee),
//!   then a row-normalised camera→Rec.2020 matrix maps camera-white to D65-white.

#![allow(clippy::excessive_precision)] // coefficients are quoted verbatim from their specs

use serde::{Deserialize, Serialize};

pub type Mat3 = [[f32; 3]; 3];
pub type Vec3 = [f32; 3];

pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub const REC2020_TO_XYZ: Mat3 = [
    [0.636_958_0, 0.144_616_9, 0.168_881_0],
    [0.262_700_2, 0.677_998_1, 0.059_301_7],
    [0.000_000_0, 0.028_072_7, 1.060_985_1],
];

/// Per-image colour description captured at decode time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraColor {
    /// XYZ (D65) → camera RGB, as found in the RAW/DNG metadata.
    pub xyz_to_cam: Mat3,
    /// As-shot white balance multipliers (camera space). Any scale; only ratios matter.
    pub as_shot_wb: Vec3,
}

pub fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            out[r][c] = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

pub fn mul_vec(m: &Mat3, v: Vec3) -> Vec3 {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

pub fn invert(m: &Mat3) -> Option<Mat3> {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 {
        return None;
    }
    let s = 1.0 / det;
    Some([
        [(e * i - f * h) * s, (c * h - b * i) * s, (b * f - c * e) * s],
        [(f * g - d * i) * s, (a * i - c * g) * s, (c * d - a * f) * s],
        [(d * h - e * g) * s, (b * g - a * h) * s, (a * e - b * d) * s],
    ])
}

// ---------------------------------------------------------------------------------------------
// Chromaticity helpers
// ---------------------------------------------------------------------------------------------

/// CIE 1931 xy of a Planckian (black body) radiator. Kim et al. cubic spline, 1667K..25000K.
pub fn planckian_xy(t: f32) -> (f32, f32) {
    let t = t.clamp(1667.0, 25000.0) as f64;
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.266_123_9e9 / t3 - 0.234_358_9e6 / t2 + 0.877_695_6e3 / t + 0.179_910
    } else {
        -3.025_846_9e9 / t3 + 2.107_037_9e6 / t2 + 0.222_634_7e3 / t + 0.240_390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.106_381_4 * x3 - 1.348_110_20 * x2 + 2.185_558_32 * x - 0.202_196_83
    } else if t <= 4000.0 {
        -0.954_947_6 * x3 - 1.374_185_93 * x2 + 2.091_370_15 * x - 0.167_488_67
    } else {
        3.081_758_0 * x3 - 5.873_386_70 * x2 + 3.751_129_97 * x - 0.370_014_83
    };
    (x as f32, y as f32)
}

fn xy_to_uv(x: f32, y: f32) -> (f32, f32) {
    let d = -2.0 * x + 12.0 * y + 3.0;
    (4.0 * x / d, 6.0 * y / d)
}

fn uv_to_xy(u: f32, v: f32) -> (f32, f32) {
    let d = 2.0 * u - 8.0 * v + 4.0;
    (3.0 * u / d, 2.0 * v / d)
}

fn xy_to_xyz(x: f32, y: f32) -> Vec3 {
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// CIE daylight locus (xy) at correlated colour temperature `t`, 4000K..25000K.
pub fn daylight_xy(t: f32) -> (f32, f32) {
    let t = t.clamp(4000.0, 25000.0) as f64;
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 7000.0 {
        -4.6070e9 / t3 + 2.9678e6 / t2 + 0.09911e3 / t + 0.244063
    } else {
        -2.0064e9 / t3 + 1.9018e6 / t2 + 0.24748e3 / t + 0.237040
    };
    let y = -3.0 * x * x + 2.87 * x - 0.275;
    (x as f32, y as f32)
}

/// Tint units: ±100 ≙ ±0.03 Duv. Positive tint = magenta (below the Planckian locus).
const TINT_SCALE: f32 = 0.0003;

/// White point (xy) for a temperature/tint pair.
pub fn white_xy(temperature: f32, tint: f32) -> (f32, f32) {
    // Within the locus fit's range, so the finite difference below never collapses to a point.
    let temperature = temperature.clamp(1667.0, 25000.0);
    let (x, y) = planckian_xy(temperature);
    let (u, v) = xy_to_uv(x, y);
    // Locus normal from a finite difference in mired space (one mired hotter, or cooler at the
    // hot end; the sign is fixed below).
    let mired = 1e6 / temperature;
    let other = if 1e6 / (mired - 1.0) <= 25000.0 { mired - 1.0 } else { mired + 1.0 };
    let (x2, y2) = planckian_xy(1e6 / other);
    let (u2, v2) = xy_to_uv(x2, y2);
    let (du, dv) = (u2 - u, v2 - v);
    let len = (du * du + dv * dv).sqrt().max(1e-12);
    // Normal pointing "up" (towards green), i.e. positive v.
    let (mut nu, mut nv) = (-dv / len, du / len);
    if nv < 0.0 {
        nu = -nu;
        nv = -nv;
    }
    let duv = -tint * TINT_SCALE;
    uv_to_xy(u + nu * duv, v + nv * duv)
}

/// Inverse of [`white_xy`]: nearest temperature (mired search) and the signed offset as tint.
pub fn temperature_tint(x: f32, y: f32) -> (f32, f32) {
    let (u, v) = xy_to_uv(x, y);
    let dist = |t: f32| {
        let (px, py) = planckian_xy(t);
        let (pu, pv) = xy_to_uv(px, py);
        ((u - pu).powi(2) + (v - pv).powi(2), pv)
    };
    // Coarse scan then golden-section refine, both in mireds (perceptually uniform-ish).
    let (lo, hi) = (1e6 / 25000.0, 1e6 / 1667.0);
    let mut best = lo;
    let mut best_d = f32::MAX;
    let steps = 200;
    for i in 0..=steps {
        let m = lo + (hi - lo) * i as f32 / steps as f32;
        let d = dist(1e6 / m).0;
        if d < best_d {
            best_d = d;
            best = m;
        }
    }
    let step = (hi - lo) / steps as f32;
    let (mut a, mut b) = ((best - step).max(lo), (best + step).min(hi));
    let g = 0.618_034;
    for _ in 0..40 {
        let c = b - g * (b - a);
        let d = a + g * (b - a);
        if dist(1e6 / c).0 < dist(1e6 / d).0 {
            b = d;
        } else {
            a = c;
        }
    }
    let t = 1e6 / ((a + b) / 2.0);
    let (d2, pv) = dist(t);
    let duv = d2.sqrt() * if v >= pv { 1.0 } else { -1.0 };
    (t, -duv / TINT_SCALE)
}

// ---------------------------------------------------------------------------------------------
// Camera transforms
// ---------------------------------------------------------------------------------------------

impl CameraColor {
    /// Camera RGB → linear Rec.2020, rows normalised so camera (1,1,1) maps to D65 white.
    pub fn cam_to_working(&self) -> Mat3 {
        let mut cam_rgb = mul(&self.xyz_to_cam, &REC2020_TO_XYZ);
        for row in cam_rgb.iter_mut() {
            let s: f32 = row.iter().sum();
            if s.abs() > 1e-9 {
                row.iter_mut().for_each(|v| *v /= s);
            }
        }
        invert(&cam_rgb).unwrap_or(IDENTITY)
    }

    /// White balance multipliers that neutralise an illuminant of the given temperature/tint.
    /// Normalised so the smallest multiplier is 1 (so clipping stays neutral).
    pub fn wb_for(&self, temperature: f32, tint: f32) -> Vec3 {
        let (x, y) = white_xy(temperature, tint);
        let cam = mul_vec(&self.xyz_to_cam, xy_to_xyz(x, y));
        normalize_min([1.0 / cam[0], 1.0 / cam[1], 1.0 / cam[2]])
    }

    /// As-shot multipliers, normalised so the smallest is 1.
    pub fn as_shot_multipliers(&self) -> Vec3 {
        normalize_min(self.as_shot_wb)
    }

    /// Temperature/tint corresponding to the as-shot white balance.
    pub fn as_shot_temperature_tint(&self) -> (f32, f32) {
        let wb = self.as_shot_wb;
        let cam_white = [1.0 / wb[0], 1.0 / wb[1], 1.0 / wb[2]];
        let Some(cam_to_xyz) = invert(&self.xyz_to_cam) else { return (6500.0, 0.0) };
        let xyz = mul_vec(&cam_to_xyz, cam_white);
        let s = xyz[0] + xyz[1] + xyz[2];
        if !(s.is_finite() && s > 0.0) {
            return (6500.0, 0.0);
        }
        temperature_tint(xyz[0] / s, xyz[1] / s)
    }
}

fn normalize_min(v: Vec3) -> Vec3 {
    let m = v.iter().cloned().fold(f32::MAX, f32::min);
    if !(m.is_finite() && m > 0.0) {
        return [1.0, 1.0, 1.0];
    }
    [v[0] / m, v[1] / m, v[2] / m]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tint_moves_the_white_point_across_the_whole_range() {
        for t in [1000.0, 1667.0, 6500.0, 24_500.0, 25_000.0, 40_000.0] {
            let (x0, y0) = white_xy(t, 0.0);
            let (x1, y1) = white_xy(t, 50.0);
            let d = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
            assert!(d > 1e-3, "tint ignored at {t} K ({d})");
            // Positive tint is magenta: below the locus, i.e. lower y here.
            assert!(y1 < y0, "{t} K");
        }
    }

    /// Sony ILCE-7M3 D65 matrix (Adobe DNG converter), scaled by 1/10000.
    fn a7m3() -> CameraColor {
        let m = [7374, -2389, -551, -5435, 13162, 2519, -1006, 1795, 6552];
        let f = |i: usize| m[i] as f32 / 10000.0;
        CameraColor {
            xyz_to_cam: [[f(0), f(1), f(2)], [f(3), f(4), f(5)], [f(6), f(7), f(8)]],
            as_shot_wb: [2.4, 1.0, 1.6],
        }
    }

    #[test]
    fn planckian_d65_is_close_to_6504k() {
        let (x, y) = planckian_xy(6504.0);
        // D65 sits slightly above the locus; xy should be within ~0.005.
        assert!((x - 0.3127).abs() < 0.005 && (y - 0.3290).abs() < 0.01, "{x} {y}");
    }

    #[test]
    fn temperature_tint_round_trips() {
        for &(t, tint) in &[(2800.0, 0.0), (5000.0, 20.0), (6500.0, -15.0), (9000.0, 40.0)] {
            let (x, y) = white_xy(t, tint);
            let (t2, tint2) = temperature_tint(x, y);
            assert!((t - t2).abs() / t < 0.01, "temp {t} -> {t2}");
            assert!((tint - tint2).abs() < 1.5, "tint {tint} -> {tint2}");
        }
    }

    #[test]
    fn cam_white_maps_to_working_white() {
        let w = mul_vec(&a7m3().cam_to_working(), [1.0, 1.0, 1.0]);
        for c in w {
            assert!((c - 1.0).abs() < 1e-4, "{w:?}");
        }
    }

    #[test]
    fn wb_for_as_shot_temperature_reproduces_as_shot_multipliers() {
        let cc = a7m3();
        let (t, tint) = cc.as_shot_temperature_tint();
        let a = cc.as_shot_multipliers();
        let b = cc.wb_for(t, tint);
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() / a[i] < 0.01, "{a:?} vs {b:?} at {t}K/{tint}");
        }
    }
}
