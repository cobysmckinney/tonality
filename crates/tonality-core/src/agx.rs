//! AgX Base, the display transform: scene-linear Rec.2020 → sRGB, as Blender ships it.
//!
//! See `docs/agx.md` for the steps, every constant's source, and how this was checked against
//! Blender. The matrices are computed here from AgX's published parameters; the shader
//! (`agx()` in `develop.wgsl`) receives them through [`Agx::uniform`] and repeats [`Agx::display`]
//! step for step in f32.

#![allow(clippy::excessive_precision)] // constants are quoted verbatim from their sources

use std::sync::OnceLock;

pub type M3 = [[f64; 3]; 3];
pub type V3 = [f64; 3];

/// Middle grey, and the log range the sigmoid spans (stops around middle grey).
pub const GREY: f64 = 0.18;
pub const LOG_MIN: f64 = -10.0;
pub const LOG_MAX: f64 = 6.5;
/// Blender's LUT input: E-Gamut log2 over −10…+15 stops around middle grey.
pub const DOMAIN_STOPS: [f64; 2] = [-10.0, 15.0];
/// Sigmoid slope at the pivot, and its toe and shoulder powers.
pub const SLOPE: f64 = 2.4;
pub const POWER: f64 = 1.5;
/// The display encoding the sigmoid's output is in (a pure power).
pub const SIGMOID_GAMMA: f64 = 2.4;
/// Share of the sigmoid's per-channel hue shift that is kept.
pub const HUE_KEEP: f64 = 0.4;

/// Inset: primaries rotated (degrees, counter-clockwise in CIE xy) and pulled toward white.
pub const INSET_ROTATE: V3 = [2.13976149, -1.22827335, -3.05174246];
pub const INSET_SCALE: V3 = [0.32965205, 0.28051336, 0.12475368];
/// Outset (inverted): purity restored, no rotation.
pub const OUTSET_SCALE: V3 = [0.32317438, 0.28325605, 0.0374326];

/// Guard-rail luminance weights (Rec.2020). The ones Blender's LUTs were made with; Eary Chow's
/// scripts have since moved to [0.2589, 0.6105, 0.1306] (June 2025).
pub const LUMA: V3 = [0.2658180370250449, 0.59846986045365, 0.1357121025213052];

const WHITE_D65: [f64; 2] = [0.3127, 0.3290];
const REC2020: [[f64; 2]; 3] = [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]];
const REC709: [[f64; 2]; 3] = [[0.640, 0.330], [0.300, 0.600], [0.150, 0.060]];
/// FilmLight E-Gamut → XYZ (D65).
const EGAMUT_TO_XYZ: M3 = [
    [0.7053968501, 0.1640413283, 0.08101774865],
    [0.2801307241, 0.8202066415, -0.1003373656],
    [-0.1037815116, -0.07290725703, 1.265746519],
];

/// The constants the transform needs, derived once.
#[derive(Debug, Clone, PartialEq)]
pub struct Agx {
    /// Rec.2020 → E-Gamut and back (the input domain clamp).
    pub to_egamut: M3,
    pub from_egamut: M3,
    pub inset: M3,
    /// Outset, then Rec.2020 → the display's primaries (Rec.709).
    pub outset_display: M3,
    /// Luminance weights for the guard rails, for Rec.2020 and display RGB.
    pub luma: V3,
    pub luma_display: V3,
    /// Sigmoid pivot (x, y) and the toe and shoulder scales.
    pub pivot: [f64; 2],
    pub scale_toe: f64,
    pub scale_shoulder: f64,
}

impl Agx {
    pub fn get() -> &'static Agx {
        static AGX: OnceLock<Agx> = OnceLock::new();
        AGX.get_or_init(Agx::new)
    }

    fn new() -> Self {
        let to_2020 = inv(&npm(&REC2020, WHITE_D65));
        let r2020_to_709 = mul(&inv(&npm(&REC709, WHITE_D65)), &npm(&REC2020, WHITE_D65));
        let to_egamut = mul(&inv(&EGAMUT_TO_XYZ), &npm(&REC2020, WHITE_D65));
        let inset = mul(&to_2020, &npm(&working_space(INSET_ROTATE, INSET_SCALE), WHITE_D65));
        let outset = inv(&mul(&to_2020, &npm(&working_space([0.0; 3], OUTSET_SCALE), WHITE_D65)));
        // Display luminance measured back in Rec.2020: W · (709→2020 · c) = (709→2020)ᵀ W · c.
        let r709_to_2020 = inv(&r2020_to_709);
        let luma_display = [0, 1, 2].map(|j| (0..3).map(|i| r709_to_2020[i][j] * LUMA[i]).sum());
        let px = -LOG_MIN / (LOG_MAX - LOG_MIN);
        let py = GREY.powf(1.0 / SIGMOID_GAMMA);
        Agx {
            to_egamut,
            from_egamut: inv(&to_egamut),
            inset,
            outset_display: mul(&r2020_to_709, &outset),
            luma: LUMA,
            luma_display,
            pivot: [px, py],
            scale_toe: -sigmoid_scale(1.0 - px, 1.0 - py),
            scale_shoulder: sigmoid_scale(px, py),
        }
    }

    /// Scene-linear Rec.2020 → sRGB code values (0–1). The reference the shader follows.
    pub fn display(&self, rgb: V3) -> V3 {
        // 1. Input domain: E-Gamut, −10…+15 stops around middle grey, per channel.
        let [lo, hi] = DOMAIN_STOPS.map(|s| GREY * s.exp2());
        let c = mv(&self.from_egamut, mv(&self.to_egamut, rgb).map(|v| v.clamp(lo, hi)));
        // 2. Lower guard rail in Rec.2020.
        let c = low_side_2020(c, &self.luma);
        // 3. Inset, 4. its hue.
        let c = mv(&self.inset, c);
        let h0 = hue(c);
        // 5.–7. Log, sigmoid, linearise (per channel).
        let c = c.map(|v| {
            let x = ((v.max(1e-300) / GREY).log2() - LOG_MIN) / (LOG_MAX - LOG_MIN);
            self.sigmoid(x).max(0.0).powf(SIGMOID_GAMMA)
        });
        // 8. Keep 40% of the curve's hue shift.
        let c = with_hue(c, lerp_hue(h0, hue(c), HUE_KEEP));
        // 9.–10. Outset, to the display's primaries.
        let c = mv(&self.outset_display, c);
        // 11. Lower guard rail in the display gamut. 12. Clip, encode.
        low_side_display(c, &self.luma_display).map(|v| srgb_encode(if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }))
    }

    /// Jed Smith's tunable sigmoid (lengths 0, limits (0,0) and (1,1)).
    pub fn sigmoid(&self, x: f64) -> f64 {
        let [px, py] = self.pivot;
        let s = if x < px { self.scale_toe } else { self.scale_shoulder };
        if x == px {
            return py;
        }
        let u = SLOPE * (x - px) / s;
        s * u / (1.0 + u.powf(POWER)).powf(1.0 / POWER) + py
    }

    /// The shader's `agx` uniform block: 15 vec4s (matrices as rows).
    pub fn uniform(&self) -> [[f32; 4]; 15] {
        let row = |m: &M3, i: usize| [m[i][0] as f32, m[i][1] as f32, m[i][2] as f32, 0.0];
        let [lo, hi] = DOMAIN_STOPS.map(|s| (GREY * s.exp2()) as f32);
        let mut u = [[0.0f32; 4]; 15];
        for i in 0..3 {
            u[i] = row(&self.to_egamut, i);
            u[3 + i] = row(&self.from_egamut, i);
            u[6 + i] = row(&self.inset, i);
            u[9 + i] = row(&self.outset_display, i);
        }
        u[12] = [self.luma[0] as f32, self.luma[1] as f32, self.luma[2] as f32, lo];
        u[13] = [self.luma_display[0] as f32, self.luma_display[1] as f32, self.luma_display[2] as f32, hi];
        u[14] = [self.pivot[0] as f32, self.pivot[1] as f32, self.scale_toe as f32, self.scale_shoulder as f32];
        u
    }
}

/// `scale()` of the tunable sigmoid for a segment from the pivot `(tx, ty)` to the limit (1, 1).
fn sigmoid_scale(tx: f64, ty: f64) -> f64 {
    let a = SLOPE * (1.0 - tx);
    (a.powf(-POWER) * ((a / (1.0 - ty)).powf(POWER) - 1.0)).powf(-1.0 / POWER)
}

/// Compensated luminance: `max(x) − Y(max(x) − x) + Y(x)`.
fn compensated(x: V3, w: &V3) -> f64 {
    let m = x[0].max(x[1]).max(x[2]);
    let inv = x.map(|v| m - v);
    inv[0].max(inv[1]).max(inv[2]) - dot(inv, w) + dot(x, w)
}

fn lift(c: V3) -> V3 {
    let off = (-c[0].min(c[1]).min(c[2])).max(0.0);
    c.map(|v| v + off)
}

fn low_side_2020(c: V3, w: &V3) -> V3 {
    let y = compensated(c, w);
    let lifted = lift(c);
    let y_new = compensated(lifted, w);
    let ratio = if y_new > y { y / y_new } else { 1.0 };
    lifted.map(|v| v * ratio)
}

/// The display-gamut guard rail: compensation blended in by `min(Y^0.08, 1)`. A negative
/// luminance leaves the colour unscaled (NaN in the original script, whose comparison fails).
fn low_side_display(c: V3, w: &V3) -> V3 {
    let blend = |x: V3| {
        let y = dot(x, w);
        if y < 0.0 {
            return f64::NAN;
        }
        let t = y.powf(0.08).min(1.0);
        (1.0 - t) * compensated(x, w) + t * y
    };
    let y = blend(c);
    let lifted = lift(c);
    let y_new = blend(lifted);
    let ratio = if y_new > y { y / y_new.max(1e-100) } else { 1.0 };
    lifted.map(|v| v * ratio)
}

/// HSV hue (hexcone), 0–1; 0 for greys.
pub fn hue(c: V3) -> f64 {
    let [r, g, b] = c;
    let mx = r.max(g).max(b);
    let d = mx - r.min(g).min(b);
    if d == 0.0 {
        return 0.0;
    }
    let h = if mx == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h / 6.0
}

/// `c` with its HSV hue replaced by `h` (saturation and value kept).
pub fn with_hue(c: V3, h: f64) -> V3 {
    let v = c[0].max(c[1]).max(c[2]);
    if v <= 0.0 {
        return c;
    }
    let s = (v - c[0].min(c[1]).min(c[2])) / v;
    [0.0, 2.0 / 3.0, 1.0 / 3.0].map(|o| {
        let k = ((((h + o).fract() + 1.0).fract() * 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0);
        v * (1.0 - s + s * k)
    })
}

/// Hue `a` moved `t` of the way to `b`, the short way round.
pub fn lerp_hue(a: f64, b: f64, t: f64) -> f64 {
    let d = b - a;
    let b = if d > 0.5 { b - 1.0 } else if d < -0.5 { b + 1.0 } else { b };
    (a + t * (b - a)).rem_euclid(1.0)
}

pub fn srgb_encode(v: f64) -> f64 {
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

// ---- working space (Sobotka's create_workingspace) ----

/// AgX's working-space primaries (CIE xy): each Rec.2020 primary's direction from white rotated
/// by `rotate` degrees, taken to where it leaves the Rec.2020 triangle, then pulled toward white
/// by `1 − scale`.
fn working_space(rotate: V3, scale: V3) -> [[f64; 2]; 3] {
    let [wx, wy] = WHITE_D65;
    [0, 1, 2].map(|i| {
        let (s, c) = rotate[i].to_radians().sin_cos();
        let (dx, dy) = (REC2020[i][0] - wx, REC2020[i][1] - wy);
        let d = [dx * c - dy * s, dx * s + dy * c];
        // Nearest exit of the ray w + t·d through an edge of the triangle.
        let mut t_exit = f64::INFINITY;
        for e in 0..3 {
            let (p0, p1) = (REC2020[e], REC2020[(e + 1) % 3]);
            let ex = [p1[0] - p0[0], p1[1] - p0[1]];
            let det = d[0] * -ex[1] - d[1] * -ex[0];
            if det.abs() < 1e-15 {
                continue;
            }
            let (qx, qy) = (p0[0] - wx, p0[1] - wy);
            let t = (qx * -ex[1] - qy * -ex[0]) / det;
            let u = (d[0] * qy - d[1] * qx) / det;
            if t > 0.0 && (-1e-12..=1.0 + 1e-12).contains(&u) {
                t_exit = t_exit.min(t);
            }
        }
        let k = t_exit * (1.0 - scale[i]);
        [wx + k * d[0], wy + k * d[1]]
    })
}

/// RGB → XYZ for primaries and white given in CIE xy (white maps to Y = 1).
fn npm(p: &[[f64; 2]; 3], white: [f64; 2]) -> M3 {
    let xyz = |q: [f64; 2]| [q[0] / q[1], 1.0, (1.0 - q[0] - q[1]) / q[1]];
    let cols = p.map(xyz);
    let m = [0, 1, 2].map(|r| [cols[0][r], cols[1][r], cols[2][r]]);
    let s = mv(&inv(&m), xyz(white));
    [0, 1, 2].map(|r| [m[r][0] * s[0], m[r][1] * s[1], m[r][2] * s[2]])
}

fn dot(a: V3, b: &V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn mv(m: &M3, v: V3) -> V3 {
    m.map(|r| dot(v, &r))
}

fn mul(a: &M3, b: &M3) -> M3 {
    [0, 1, 2].map(|r| [0, 1, 2].map(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

fn inv(m: &M3) -> M3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    [
        [(e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det],
        [(f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det],
        [(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Reference {
        inset: M3,
        outset: M3,
        rows: Vec<Row>,
    }
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "in")]
        input: V3,
        blender: V3,
        exact: V3,
    }

    fn reference() -> Reference {
        serde_json::from_str(include_str!("../tests/data/agx_reference.json")).unwrap()
    }

    fn max_diff(a: &M3, b: &M3) -> f64 {
        (0..9).map(|k| (a[k / 3][k % 3] - b[k / 3][k % 3]).abs()).fold(0.0, f64::max)
    }

    #[test]
    fn matrices_match_blender() {
        let agx = Agx::get();
        // The inset as written in Blender's config.ocio ("AgX Log").
        let blender: M3 = [
            [0.856627153315983, 0.0951212405381588, 0.0482516061458583],
            [0.137318972929847, 0.761241990602591, 0.101439036467562],
            [0.11189821299995, 0.0767994186031903, 0.811302368396859],
        ];
        assert!(max_diff(&agx.inset, &blender) < 1e-7, "{:?}", agx.inset);
        let r = reference();
        assert!(max_diff(&agx.inset, &r.inset) < 1e-7);
        let outset = mul(&inv(&mul(&inv(&npm(&REC709, WHITE_D65)), &npm(&REC2020, WHITE_D65))), &agx.outset_display);
        assert!(max_diff(&outset, &r.outset) < 1e-7, "{outset:?}");
    }

    #[test]
    fn sigmoid_passes_its_pivot_and_ends() {
        let agx = Agx::get();
        let [px, py] = agx.pivot;
        assert!(agx.sigmoid(0.0).abs() < 1e-12);
        assert!((agx.sigmoid(1.0) - 1.0).abs() < 1e-12);
        assert_eq!(agx.sigmoid(px), py);
        // Slope at the pivot, from both sides.
        let e = 1e-7;
        assert!(((agx.sigmoid(px + e) - py) / e - SLOPE).abs() < 1e-4);
        assert!(((py - agx.sigmoid(px - e)) / e - SLOPE).abs() < 1e-4);
    }

    #[test]
    fn middle_grey_is_display_middle_grey() {
        let out = Agx::get().display([GREY; 3]);
        assert!((out[0] - srgb_encode(GREY)).abs() < 1e-9, "{out:?}");
        assert!(out.iter().all(|&v| (v - out[0]).abs() < 1e-12), "grey stays grey");
    }

    #[test]
    fn matches_the_exact_reference() {
        let agx = Agx::get();
        for r in reference().rows {
            let out = agx.display(r.input);
            for k in 0..3 {
                assert!((out[k] - r.exact[k]).abs() < 1e-5, "{:?}: {out:?} vs {:?}", r.input, r.exact);
            }
        }
    }

    #[test]
    fn matches_blender_within_its_lut_precision() {
        let agx = Agx::get();
        let mut d: Vec<f64> = reference()
            .rows
            .iter()
            .flat_map(|r| {
                let out = agx.display(r.input);
                (0..3).map(move |k| (out[k] - r.blender[k]).abs())
            })
            .collect();
        d.sort_by(f64::total_cmp);
        let mean = d.iter().sum::<f64>() / d.len() as f64;
        let p99 = d[d.len() * 99 / 100];
        assert!(mean < 0.002 && p99 < 0.02 && d[d.len() - 1] < 0.07, "mean {mean} p99 {p99} max {}", d[d.len() - 1]);
    }

    #[test]
    fn hue_round_trips() {
        for c in [[0.9, 0.2, 0.1], [0.1, 0.8, 0.3], [0.2, 0.3, 0.9], [0.7, 0.1, 0.6]] {
            let out = with_hue(c, hue(c));
            assert!((0..3).all(|k| (out[k] - c[k]).abs() < 1e-12), "{c:?} → {out:?}");
        }
        assert!((lerp_hue(0.95, 0.05, 0.5) - 0.0).abs() < 1e-12);
    }
}
