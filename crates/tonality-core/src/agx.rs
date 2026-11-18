//! AgX, the display transform, as Blender ships it: scene-linear Rec.2020 → an SDR display
//! (AgX Base, for sRGB or Display P3) or an HDR one (AgX HDR, 1000 nits).
//!
//! See `docs/agx.md` for the steps, every constant's source, and how this was checked against
//! Blender. The matrices are computed here from AgX's published parameters; the shader
//! (`agx()` in `develop.wgsl`) receives them through [`Agx::uniform`] and repeats
//! [`Agx::display`] / [`Agx::hdr_light`] step for step in f32.

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

/// HDR: peak over reference white (1000 nits over 100). The shoulder's power is multiplied by
/// `HDR_RATIO ^ log10(2)` (= 2), middle grey is darkened by the ratio, and the darkening keeps
/// `HDR_PURITY` of its hue and saturation change.
pub const HDR_RATIO: f64 = 10.0;
pub const HDR_PURITY: f64 = 0.5;
/// The darkening curve's log2 range: −20 stops below middle grey up to linear 1.0.
const DARK_MIN: f64 = -20.0;

/// Inset: primaries rotated (degrees, counter-clockwise in CIE xy) and pulled toward white.
pub const INSET_ROTATE: V3 = [2.13976149, -1.22827335, -3.05174246];
pub const INSET_SCALE: V3 = [0.32965205, 0.28051336, 0.12475368];
/// Outset (inverted): purity restored, no rotation.
pub const OUTSET_SCALE: V3 = [0.32317438, 0.28325605, 0.0374326];

/// Guard-rail luminance weights (Rec.2020). Eary Chow's scripts moved from the first set to the
/// second in June 2025; Blender's AgX Base LUTs (sRGB, P3) were made before, its HDR LUT
/// (August 2025) after, and each set reproduces only its own.
pub const LUMA_BASE: V3 = [0.2658180370250449, 0.59846986045365, 0.1357121025213052];
pub const LUMA_HDR: V3 = [0.2589235355689848, 0.6104985346066525, 0.13057792982436284];

const WHITE_D65: [f64; 2] = [0.3127, 0.3290];
const REC2020: [[f64; 2]; 3] = [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046]];
const REC709: [[f64; 2]; 3] = [[0.640, 0.330], [0.300, 0.600], [0.150, 0.060]];
const P3: [[f64; 2]; 3] = [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]];
/// FilmLight E-Gamut → XYZ (D65).
const EGAMUT_TO_XYZ: M3 = [
    [0.7053968501, 0.1640413283, 0.08101774865],
    [0.2801307241, 0.8202066415, -0.1003373656],
    [-0.1037815116, -0.07290725703, 1.265746519],
];

/// An SDR display's primaries. Both use the sRGB curve (Display P3 is P3 primaries, D65, sRGB
/// transfer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Display {
    Srgb,
    DisplayP3,
}

impl Display {
    fn primaries(self) -> &'static [[f64; 2]; 3] {
        match self {
            Display::Srgb => &REC709,
            Display::DisplayP3 => &P3,
        }
    }

    /// Rec.2020 → this display's primaries (linear).
    pub fn from_2020(self) -> M3 {
        mul(
            &inv(&npm(self.primaries(), WHITE_D65)),
            &npm(&REC2020, WHITE_D65),
        )
    }
}

/// Jed Smith's tunable sigmoid (lengths 0, limits (0, 0) and (1, 1)): through the pivot
/// `(px, py)` with `slope` there, toe and shoulder powers `powers`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sigmoid {
    pub pivot: [f64; 2],
    pub slope: f64,
    pub powers: [f64; 2],
    /// Toe and shoulder scales.
    pub scales: [f64; 2],
}

impl Sigmoid {
    pub fn new(pivot: [f64; 2], slope: f64, powers: [f64; 2]) -> Self {
        let [px, py] = pivot;
        let scale = |tx: f64, ty: f64, p: f64| {
            let a = slope * (1.0 - tx);
            (a.powf(-p) * ((a / (1.0 - ty)).powf(p) - 1.0)).powf(-1.0 / p)
        };
        Sigmoid {
            pivot,
            slope,
            powers,
            scales: [
                -scale(1.0 - px, 1.0 - py, powers[0]),
                scale(px, py, powers[1]),
            ],
        }
    }

    pub fn eval(&self, x: f64) -> f64 {
        let [px, py] = self.pivot;
        if x == px {
            return py;
        }
        let k = usize::from(x > px);
        let (s, p) = (self.scales[k], self.powers[k]);
        let u = self.slope * (x - px) / s;
        // `u` is negative only on a shoulder with a negative scale, whose power is 1 here.
        s * u / (1.0 + u.signum() * u.abs().powf(p)).powf(1.0 / p) + py
    }
}

/// The constants one AgX target needs, derived once.
#[derive(Debug, Clone, PartialEq)]
pub struct Agx {
    /// HDR (AgX's 1000-nit formation) rather than AgX Base.
    pub hdr: bool,
    /// Rec.2020 → E-Gamut and back (the input domain clamp).
    pub to_egamut: M3,
    pub from_egamut: M3,
    pub inset: M3,
    /// Outset, then Rec.2020 → the lower guard rail's gamut (the display's for SDR; P3 for HDR).
    pub outset_display: M3,
    /// Luminance weights for the guard rails, for Rec.2020 and the rail's gamut.
    pub luma: V3,
    pub luma_display: V3,
    pub sigmoid: Sigmoid,
    /// HDR: the darkening of middle grey (on [`dark_encode`]d values).
    pub darken: Sigmoid,
    /// HDR: P3 → Rec.2020 (back from the guard rail), then Rec.2020 → `display` (the SDR base
    /// image's primaries, which the gain map is computed in).
    pub rail_to_2020: M3,
    pub to_display: M3,
}

impl Agx {
    /// AgX Base for an SDR display.
    pub fn sdr(display: Display) -> &'static Agx {
        static SRGB: OnceLock<Agx> = OnceLock::new();
        static P3: OnceLock<Agx> = OnceLock::new();
        match display {
            Display::Srgb => SRGB.get_or_init(|| Agx::new(false, display)),
            Display::DisplayP3 => P3.get_or_init(|| Agx::new(false, display)),
        }
    }

    /// AgX HDR (1000 nits, limited to P3), converted to `display`'s primaries at the end.
    pub fn hdr(display: Display) -> &'static Agx {
        static SRGB: OnceLock<Agx> = OnceLock::new();
        static P3: OnceLock<Agx> = OnceLock::new();
        match display {
            Display::Srgb => SRGB.get_or_init(|| Agx::new(true, display)),
            Display::DisplayP3 => P3.get_or_init(|| Agx::new(true, display)),
        }
    }

    fn new(hdr: bool, display: Display) -> Self {
        let to_2020 = inv(&npm(&REC2020, WHITE_D65));
        let to_egamut = mul(&inv(&EGAMUT_TO_XYZ), &npm(&REC2020, WHITE_D65));
        let inset = mul(
            &to_2020,
            &npm(&working_space(INSET_ROTATE, INSET_SCALE), WHITE_D65),
        );
        let outset = inv(&mul(
            &to_2020,
            &npm(&working_space([0.0; 3], OUTSET_SCALE), WHITE_D65),
        ));
        // HDR is guarded in P3 whatever the display, as Blender's "P3 limited" HDR LUT.
        let rail = if hdr { Display::DisplayP3 } else { display };
        let to_rail = rail.from_2020();
        let luma = if hdr { LUMA_HDR } else { LUMA_BASE };
        // Rail luminance measured back in Rec.2020: W · (rail→2020 · c) = (rail→2020)ᵀ W · c.
        let rail_to_2020 = inv(&to_rail);
        let luma_display = [0, 1, 2].map(|j| (0..3).map(|i| rail_to_2020[i][j] * luma[i]).sum());
        let pivot = [
            -LOG_MIN / (LOG_MAX - LOG_MIN),
            GREY.powf(1.0 / SIGMOID_GAMMA),
        ];
        let shoulder = if hdr {
            POWER * HDR_RATIO.powf(2f64.log10())
        } else {
            POWER
        };
        Agx {
            hdr,
            to_egamut,
            from_egamut: inv(&to_egamut),
            inset,
            outset_display: mul(&to_rail, &outset),
            luma,
            luma_display,
            sigmoid: Sigmoid::new(pivot, SLOPE, [POWER, shoulder]),
            darken: Sigmoid::new(
                [dark_encode(GREY), dark_encode(GREY / HDR_RATIO)],
                1.000001,
                [3.0, 1.0],
            ),
            rail_to_2020,
            to_display: display.from_2020(),
        }
    }

    /// Steps 1–11 of `docs/agx.md`: scene Rec.2020 → linear light in the guard rail's gamut
    /// (unclipped).
    fn form(&self, rgb: V3) -> V3 {
        // 1. Input domain: E-Gamut, −10…+15 stops around middle grey, per channel.
        let [lo, hi] = DOMAIN_STOPS.map(|s| GREY * s.exp2());
        let c = mv(
            &self.from_egamut,
            mv(&self.to_egamut, rgb).map(|v| v.clamp(lo, hi)),
        );
        // 2. Lower guard rail in Rec.2020.
        let c = low_side_2020(c, &self.luma);
        // 3. Inset, 4. its hue.
        let c = mv(&self.inset, c);
        let h0 = hue(c);
        // 5.–7. Log (clipped to the curve's range for HDR), sigmoid, linearise (per channel).
        let c = c.map(|v| {
            let x = ((v.max(1e-300) / GREY).log2() - LOG_MIN) / (LOG_MAX - LOG_MIN);
            let x = if self.hdr { x.clamp(0.0, 1.0) } else { x };
            self.sigmoid.eval(x).max(0.0).powf(SIGMOID_GAMMA)
        });
        // HDR: middle grey down to a tenth (reference white 100 nits of 1000).
        let c = if self.hdr { self.darken_grey(c) } else { c };
        // 8. Keep 40% of the curve's hue shift.
        let c = with_hue(c, lerp_hue(h0, hue(c), HUE_KEEP));
        // 9.–10. Outset, to the rail's primaries. 11. Lower guard rail there.
        low_side_display(mv(&self.outset_display, c), &self.luma_display)
    }

    /// AgXHLG.py's `darken_middle_grey`: each channel down along the darkening curve (on log2),
    /// then hue and saturation moved back halfway to where they were.
    fn darken_grey(&self, c: V3) -> V3 {
        let d = c.map(|v| {
            if v > 0.0 {
                dark_decode(self.darken.eval(dark_encode(v)))
            } else {
                0.0
            }
        });
        let h = lerp_hue(hue(c), hue(d), HDR_PURITY);
        let s = saturation(c) + HDR_PURITY * (saturation(d) - saturation(c));
        hsv(h, s, d[0].max(d[1]).max(d[2]))
    }

    /// AgX Base: scene-linear Rec.2020 → the display's code values (0–1, sRGB curve). The
    /// reference the shader follows.
    pub fn display(&self, rgb: V3) -> V3 {
        debug_assert!(!self.hdr);
        // 12. Clip, encode.
        self.form(rgb)
            .map(|v| srgb_encode(if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }))
    }

    /// AgX HDR: scene-linear Rec.2020 → display light, Rec.2020 primaries (limited to P3),
    /// 0–1 = 0–1000 nits.
    pub fn hdr_light(&self, rgb: V3) -> V3 {
        debug_assert!(self.hdr);
        mv(&self.rail_to_2020, self.form(rgb))
            .map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) })
    }

    /// AgX HDR in the display's primaries, relative to SDR white (0–[`HDR_RATIO`]): what the
    /// gain map compares with AgX Base. Colours outside the display's gamut clip.
    pub fn hdr_relative(&self, rgb: V3) -> V3 {
        mv(&self.to_display, self.hdr_light(rgb)).map(|v| v.max(0.0) * HDR_RATIO)
    }

    /// The shader's AgX parameter block: [`AGX_VEC4S`] vec4s (matrices as rows).
    pub fn uniform(&self) -> [[f32; 4]; AGX_VEC4S] {
        let row = |m: &M3, i: usize| [m[i][0] as f32, m[i][1] as f32, m[i][2] as f32, 0.0];
        let [lo, hi] = DOMAIN_STOPS.map(|s| (GREY * s.exp2()) as f32);
        let sig = |s: &Sigmoid| {
            [
                s.pivot[0] as f32,
                s.pivot[1] as f32,
                s.scales[0] as f32,
                s.scales[1] as f32,
            ]
        };
        let mut u = [[0.0f32; 4]; AGX_VEC4S];
        for i in 0..3 {
            u[i] = row(&self.to_egamut, i);
            u[3 + i] = row(&self.from_egamut, i);
            u[6 + i] = row(&self.inset, i);
            u[9 + i] = row(&self.outset_display, i);
            u[16 + i] = row(&self.rail_to_2020, i);
            u[19 + i] = row(&self.to_display, i);
        }
        u[12] = [
            self.luma[0] as f32,
            self.luma[1] as f32,
            self.luma[2] as f32,
            lo,
        ];
        u[13] = [
            self.luma_display[0] as f32,
            self.luma_display[1] as f32,
            self.luma_display[2] as f32,
            hi,
        ];
        u[14] = sig(&self.sigmoid);
        u[15] = [
            self.sigmoid.powers[0] as f32,
            self.sigmoid.powers[1] as f32,
            if self.hdr { 1.0 } else { 0.0 },
            0.0,
        ];
        u[22] = sig(&self.darken);
        u[23] = [
            self.darken.slope as f32,
            self.darken.powers[0] as f32,
            self.darken.powers[1] as f32,
            HDR_PURITY as f32,
        ];
        u
    }
}

/// vec4s in one AgX parameter block (see `agx()` in `develop.wgsl` for the layout).
pub const AGX_VEC4S: usize = 24;

/// Ultra HDR gain map: per channel, `log2((HDR + k) / (SDR + k))` with `k` = [`GAIN_OFFSET`]
/// (HDR relative to SDR white, both linear in the base image's primaries), stored over
/// [`GAIN_LOG2`]. The range covers AgX's HDR over AgX Base for every colour (a test checks): up
/// to the 10× peak and a little more where SDR clips a channel; down where SDR's guard rail
/// lifts a channel HDR leaves dark.
pub const GAIN_LOG2: [f64; 2] = [-4.5, 3.75];
pub const GAIN_OFFSET: f64 = 1.0 / 64.0;

/// The darkening curve's log encoding: −20 stops → 0, linear 1.0 → 1.
pub fn dark_encode(v: f64) -> f64 {
    ((v / GREY).log2() - DARK_MIN) / ((1.0 / GREY).log2() - DARK_MIN)
}

pub fn dark_decode(x: f64) -> f64 {
    GREY * (x * ((1.0 / GREY).log2() - DARK_MIN) + DARK_MIN).exp2()
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
    let ratio = if y_new > y {
        y / y_new.max(1e-100)
    } else {
        1.0
    };
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

/// HSV saturation; 0 for black.
pub fn saturation(c: V3) -> f64 {
    let v = c[0].max(c[1]).max(c[2]);
    if v <= 0.0 {
        0.0
    } else {
        (v - c[0].min(c[1]).min(c[2])) / v
    }
}

/// HSV → RGB.
pub fn hsv(h: f64, s: f64, v: f64) -> V3 {
    [0.0, 2.0 / 3.0, 1.0 / 3.0].map(|o| {
        let k = ((((h + o).fract() + 1.0).fract() * 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0);
        v * (1.0 - s + s * k)
    })
}

/// `c` with its HSV hue replaced by `h` (saturation and value kept).
pub fn with_hue(c: V3, h: f64) -> V3 {
    let v = c[0].max(c[1]).max(c[2]);
    if v <= 0.0 {
        return c;
    }
    hsv(h, saturation(c), v)
}

/// Hue `a` moved `t` of the way to `b`, the short way round.
pub fn lerp_hue(a: f64, b: f64, t: f64) -> f64 {
    let d = b - a;
    let b = if d > 0.5 {
        b - 1.0
    } else if d < -0.5 {
        b + 1.0
    } else {
        b
    };
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

pub fn mv(m: &M3, v: V3) -> V3 {
    m.map(|r| dot(v, &r))
}

fn mul(a: &M3, b: &M3) -> M3 {
    [0, 1, 2].map(|r| [0, 1, 2].map(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

pub fn inv(m: &M3) -> M3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    [
        [
            (e * i - f * h) / det,
            (c * h - b * i) / det,
            (b * f - c * e) / det,
        ],
        [
            (f * g - d * i) / det,
            (a * i - c * g) / det,
            (c * d - a * f) / det,
        ],
        [
            (d * h - e * g) / det,
            (b * g - a * h) / det,
            (a * e - b * d) / det,
        ],
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
        blender_srgb: V3,
        exact_srgb: V3,
        blender_p3: V3,
        exact_p3: V3,
        blender_hlg: V3,
        exact_hlg: V3,
    }

    fn reference() -> Reference {
        serde_json::from_str(include_str!("../tests/data/agx_reference.json")).unwrap()
    }

    fn max_diff(a: &M3, b: &M3) -> f64 {
        (0..9)
            .map(|k| (a[k / 3][k % 3] - b[k / 3][k % 3]).abs())
            .fold(0.0, f64::max)
    }

    /// Display light (Rec.2020, 1 = 1000 nits) → Rec.2100 HLG signal (BT.2100: the inverse
    /// OOTF with system gamma 1.2, then the OETF), as Blender's HDR view writes it.
    fn hlg_encode(c: V3) -> V3 {
        let c = c.map(|v| v.clamp(0.0, 1.0));
        let y = 0.2627 * c[0] + 0.6780 * c[1] + 0.0593 * c[2];
        let gamma = 1.2;
        let scene = c.map(|v| {
            if y == 0.0 {
                0.0
            } else {
                y.powf((1.0 - gamma) / gamma) * v
            }
        });
        let (a, b): (f64, f64) = (0.17883277, 1.0 - 4.0 * 0.17883277);
        let c0 = 0.5 - a * (4.0 * a).ln();
        scene.map(|e| {
            if e <= 1.0 / 12.0 {
                (3.0 * e).sqrt()
            } else {
                a * (12.0 * e - b).ln() + c0
            }
            .clamp(0.0, 1.0)
        })
    }

    #[test]
    fn matrices_match_blender() {
        let agx = Agx::sdr(Display::Srgb);
        // The inset as written in Blender's config.ocio ("AgX Log").
        let blender: M3 = [
            [0.856627153315983, 0.0951212405381588, 0.0482516061458583],
            [0.137318972929847, 0.761241990602591, 0.101439036467562],
            [0.11189821299995, 0.0767994186031903, 0.811302368396859],
        ];
        assert!(max_diff(&agx.inset, &blender) < 1e-7, "{:?}", agx.inset);
        let r = reference();
        assert!(max_diff(&agx.inset, &r.inset) < 1e-7);
        for (agx, display) in [
            (agx, Display::Srgb),
            (Agx::sdr(Display::DisplayP3), Display::DisplayP3),
        ] {
            let outset = mul(&inv(&display.from_2020()), &agx.outset_display);
            assert!(max_diff(&outset, &r.outset) < 1e-7, "{outset:?}");
        }
    }

    #[test]
    fn sigmoid_passes_its_pivot_and_ends() {
        for agx in [Agx::sdr(Display::Srgb), Agx::hdr(Display::Srgb)] {
            let s = &agx.sigmoid;
            let [px, py] = s.pivot;
            assert!(s.eval(0.0).abs() < 1e-12);
            assert!((s.eval(1.0) - 1.0).abs() < 1e-12);
            assert_eq!(s.eval(px), py);
            // Slope at the pivot, from both sides.
            let e = 1e-7;
            assert!(((s.eval(px + e) - py) / e - SLOPE).abs() < 1e-4);
            assert!(((py - s.eval(px - e)) / e - SLOPE).abs() < 1e-4);
        }
        // HDR: twice the shoulder power, and the darkening takes middle grey to a tenth and keeps
        // 1.0 at 1.0.
        let hdr = Agx::hdr(Display::DisplayP3);
        assert!((hdr.sigmoid.powers[1] - 3.0).abs() < 1e-12);
        let dark = |v: f64| dark_decode(hdr.darken.eval(dark_encode(v)));
        assert!((dark(GREY) - GREY / HDR_RATIO).abs() < 1e-12);
        assert!((dark(1.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn middle_grey_is_display_middle_grey() {
        for display in [Display::Srgb, Display::DisplayP3] {
            let out = Agx::sdr(display).display([GREY; 3]);
            assert!((out[0] - srgb_encode(GREY)).abs() < 1e-9, "{out:?}");
            assert!(
                out.iter().all(|&v| (v - out[0]).abs() < 1e-12),
                "grey stays grey"
            );
            // HDR: 18 nits as well (0.18 of a 100-nit white), so the gain map is 1 there.
            let hdr = Agx::hdr(display).hdr_relative([GREY; 3]);
            assert!(hdr.iter().all(|&v| (v - GREY).abs() < 1e-6), "{hdr:?}");
        }
    }

    #[test]
    fn matches_the_exact_reference() {
        let (srgb, p3, hdr) = (
            Agx::sdr(Display::Srgb),
            Agx::sdr(Display::DisplayP3),
            Agx::hdr(Display::Srgb),
        );
        for r in reference().rows {
            // HLG's square-root toe magnifies rounding at black: a grey exactly 10 stops under
            // middle grey sits on the HDR curve's clip, where each channel lands either side.
            for (out, want, tol) in [
                (srgb.display(r.input), r.exact_srgb, 1e-5),
                (p3.display(r.input), r.exact_p3, 1e-5),
                (hlg_encode(hdr.hdr_light(r.input)), r.exact_hlg, 1e-4),
            ] {
                assert!(
                    (0..3).all(|k| (out[k] - want[k]).abs() < tol),
                    "{:?}: {out:?} vs {want:?}",
                    r.input
                );
            }
        }
    }

    #[test]
    fn matches_blender_within_its_lut_precision() {
        let (srgb, p3, hdr) = (
            Agx::sdr(Display::Srgb),
            Agx::sdr(Display::DisplayP3),
            Agx::hdr(Display::Srgb),
        );
        let rows = reference().rows;
        type Pair<'a> = &'a dyn Fn(&Row) -> (V3, V3);
        let targets: [(&str, Pair); 3] = [
            ("sRGB", &|r| (srgb.display(r.input), r.blender_srgb)),
            ("P3", &|r| (p3.display(r.input), r.blender_p3)),
            ("HLG", &|r| {
                (hlg_encode(hdr.hdr_light(r.input)), r.blender_hlg)
            }),
        ];
        for (name, f) in targets {
            let mut d: Vec<f64> = rows
                .iter()
                .flat_map(|r| {
                    let (out, want) = f(r);
                    (0..3).map(move |k| (out[k] - want[k]).abs())
                })
                .collect();
            d.sort_by(f64::total_cmp);
            let mean = d.iter().sum::<f64>() / d.len() as f64;
            let p99 = d[d.len() * 99 / 100];
            let max = d[d.len() - 1];
            assert!(
                mean < 0.002 && p99 < 0.02 && max < 0.07,
                "{name}: mean {mean} p99 {p99} max {max}"
            );
        }
    }

    #[test]
    fn gain_range_covers_hdr_over_sdr() {
        let k = GAIN_OFFSET;
        let linear = |v: f64| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let mut inputs: Vec<V3> = (0..=400)
            .map(|i| [GREY * (i as f64 / 400.0 * 26.0 - 12.0).exp2(); 3])
            .collect();
        inputs.extend(reference().rows.iter().map(|r| r.input));
        for display in [Display::Srgb, Display::DisplayP3] {
            let (sdr, hdr) = (Agx::sdr(display), Agx::hdr(display));
            for &c in &inputs {
                let (s, h) = (sdr.display(c).map(linear), hdr.hdr_relative(c));
                for i in 0..3 {
                    let g = ((h[i] + k) / (s[i] + k)).log2();
                    assert!(
                        (GAIN_LOG2[0]..=GAIN_LOG2[1]).contains(&g),
                        "{display:?} {c:?}: gain {g}"
                    );
                }
            }
            // Bright highlights reach the 10× peak.
            let white = hdr.hdr_relative([GREY * 128.0; 3]); // +7 stops: past the curve's end
            assert!(
                white.iter().all(|&v| (v - HDR_RATIO).abs() < 1e-6),
                "{white:?}"
            );
        }
    }

    #[test]
    fn hue_round_trips() {
        for c in [
            [0.9, 0.2, 0.1],
            [0.1, 0.8, 0.3],
            [0.2, 0.3, 0.9],
            [0.7, 0.1, 0.6],
        ] {
            let out = with_hue(c, hue(c));
            assert!(
                (0..3).all(|k| (out[k] - c[k]).abs() < 1e-12),
                "{c:?} → {out:?}"
            );
        }
        assert!((lerp_hue(0.95, 0.05, 0.5) - 0.0).abs() < 1e-12);
    }
}
