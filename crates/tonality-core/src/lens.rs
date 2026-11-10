//! Lens correction: distortion, lateral chromatic aberration and vignetting.
//!
//! A [`LensProfile`] describes a lens (as measured by the camera maker and stored in the RAW, or
//! from the lensfun database) as radial functions sampled on a fixed grid. [`lens_table`] combines
//! it with the edit's [`LensCorrection`] settings into the table the shader reads.
//!
//! Units: radius `r` is distance from the image centre in half-diagonals (r = 1 at the corners),
//! measured on the full source image. Correction happens before framing, so crop, straighten
//! and masks all work on the corrected image.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Samples per radial function.
pub const LENS_KNOTS: usize = 32;
/// Radius covered by the tables (a little past the corners, for sources beyond them).
pub const LENS_RMAX: f32 = 1.2;

/// Radius of knot `i`.
pub fn knot_radius(i: usize) -> f32 {
    i as f32 * LENS_RMAX / (LENS_KNOTS - 1) as f32
}

/// A function of radius, sampled at [`knot_radius`].
pub type Radial = [f32; LENS_KNOTS];

/// Linear interpolation in a [`Radial`], clamped at both ends.
pub fn sample(f: &Radial, r: f32) -> f32 {
    let t = (r / LENS_RMAX).clamp(0.0, 1.0) * (LENS_KNOTS - 1) as f32;
    let i = (t as usize).min(LENS_KNOTS - 2);
    f[i] + (f[i + 1] - f[i]) * (t - i as f32)
}

/// Tabulate `f` at the knots.
pub fn radial(f: impl Fn(f32) -> f32) -> Radial {
    std::array::from_fn(|i| f(knot_radius(i)))
}

/// Where a lens profile comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LensSource {
    /// Correction data the camera wrote into the RAW file.
    Embedded,
    /// The lensfun database.
    Lensfun,
}

/// How one lens bends and darkens the image.
#[derive(Debug, Clone, PartialEq)]
pub struct LensProfile {
    pub source: LensSource,
    /// Human description, e.g. "Sony E 18-135mm F3.5-5.6 OSS".
    pub name: String,
    /// Per channel (red, green, blue): source radius ÷ output radius, at output radius `r`.
    /// Green alone carries the distortion; red and blue add chromatic aberration.
    pub distortion: Option<[Radial; 3]>,
    /// Whether red/blue differ from green (lateral chromatic aberration data).
    pub chromatic: bool,
    /// Gain that undoes the lens's falloff, at source radius `r` (≥ 1 towards the corners).
    pub vignetting: Option<Radial>,
}

/// The edit's lens settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct LensCorrection {
    /// Apply the lens profile (the camera's own data, or lensfun's).
    pub profile: bool,
    /// Strength of the profile's distortion correction, 0..=200 (%).
    pub distortion: f32,
    /// Strength of the profile's vignetting correction, 0..=200 (%).
    pub vignetting: f32,
    /// Remove the profile's lateral chromatic aberration (colour fringes towards the corners).
    pub chromatic: bool,
    /// Manual distortion, -100..=100: positive straightens barrel distortion (lines bowing
    /// outwards), negative straightens pincushion.
    pub manual_distortion: f32,
}

impl Default for LensCorrection {
    fn default() -> Self {
        Self { profile: false, distortion: 100.0, vignetting: 100.0, chromatic: true, manual_distortion: 0.0 }
    }
}

impl LensCorrection {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// What the shader needs: per knot (red, green, blue distortion; vignetting gain).
#[derive(Debug, Clone, PartialEq)]
pub struct LensTable {
    pub knots: [[f32; 4]; LENS_KNOTS],
    /// Anything to do at all.
    pub active: bool,
    /// How far (in source uv, per axis) a corrected pixel can read from its own position:
    /// tiles of the source must be this much larger.
    pub margin: f32,
}

impl LensTable {
    pub fn identity() -> Self {
        Self { knots: [[1.0; 4]; LENS_KNOTS], active: false, margin: 0.0 }
    }
}

/// Manual distortion's strength at full slider: source radius shrinks by this much at the corners.
const MANUAL_K: f32 = 0.12;

/// Combine `profile` (if any) with the settings, for a `w × h` source. Distortion correction
/// also zooms just enough that no empty corners show.
pub fn lens_table(profile: Option<&LensProfile>, s: &LensCorrection, w: f32, h: f32) -> LensTable {
    let use_profile = s.profile && profile.is_some();
    let p = profile.filter(|_| use_profile);
    let amount = |v: f32| (v / 100.0).clamp(0.0, 2.0);
    let ad = amount(s.distortion);
    let k = (s.manual_distortion / 100.0).clamp(-1.0, 1.0) * MANUAL_K;

    // dr(c, r) before auto-zoom.
    let dist = p.and_then(|p| p.distortion.as_ref());
    let chroma = s.chromatic && p.is_some_and(|p| p.chromatic);
    let base = |c: usize, r: f32| -> f32 {
        let mut v = 1.0;
        if let Some(d) = dist {
            let g = sample(&d[1], r);
            v = 1.0 + ad * (g - 1.0);
            if chroma && c != 1 {
                v *= sample(&d[c], r) / g.max(1e-6);
            }
        }
        v * (1.0 - k * r * r)
    };
    let has_dist = (dist.is_some() && ad > 0.0) || k != 0.0 || chroma;

    // Smallest zoom z for which every point on the frame's edge reads inside the source:
    // output point p (half-diagonals) reads source point dr(|p|/z)/z · p.
    let d = (w * w + h * h).sqrt().max(1e-6);
    let (ax, ay) = (w / d, h / d);
    let inside = |z: f32| -> bool {
        const N: usize = 48;
        (0..=N).all(|i| {
            let t = i as f32 / N as f32;
            [[ax * (2.0 * t - 1.0), ay], [ax, ay * (2.0 * t - 1.0)]].iter().all(|&[x, y]| {
                let r = (x * x + y * y).sqrt();
                (0..3).all(|c| {
                    let f = base(c, r / z) / z;
                    (f * x).abs() <= ax * 1.0005 && (f * y).abs() <= ay * 1.0005
                })
            })
        })
    };
    let zoom = if has_dist {
        let (mut lo, mut hi) = (0.6_f32, 1.6_f32);
        if inside(lo) {
            lo
        } else {
            for _ in 0..30 {
                let mid = 0.5 * (lo + hi);
                if inside(mid) { hi = mid } else { lo = mid }
            }
            hi
        }
    } else {
        1.0
    };

    let av = amount(s.vignetting);
    let vig = p.and_then(|p| p.vignetting.as_ref()).filter(|_| av > 0.0);
    let mut out = LensTable::identity();
    for (i, row) in out.knots.iter_mut().enumerate() {
        let r = knot_radius(i);
        if has_dist {
            for (c, v) in row.iter_mut().take(3).enumerate() {
                *v = base(c, r / zoom) / zoom;
            }
        }
        if let Some(v) = vig {
            row[3] = sample(v, r).max(1e-3).powf(av);
        }
    }
    out.active = has_dist || vig.is_some();
    out.margin = (0..LENS_KNOTS)
        .filter(|&i| knot_radius(i) <= 1.0)
        .flat_map(|i| out.knots[i][..3].iter().map(|v| (v - 1.0).abs() * 0.5))
        .fold(0.0, f32::max)
        + 0.002;
    if !out.active {
        out.margin = 0.0;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn barrel() -> LensProfile {
        // A lens whose image is squeezed towards the edges: correcting reads closer to the centre.
        LensProfile {
            source: LensSource::Embedded,
            name: "test".into(),
            distortion: Some([radial(|r| 1.0 - 0.06 * r * r), radial(|r| 1.0 - 0.05 * r * r), radial(|r| 1.0 - 0.04 * r * r)]),
            chromatic: true,
            vignetting: Some(radial(|r| 1.0 + 0.8 * r * r)),
        }
    }

    fn on() -> LensCorrection {
        LensCorrection { profile: true, ..Default::default() }
    }

    #[test]
    fn off_is_identity() {
        let t = lens_table(Some(&barrel()), &LensCorrection::default(), 6000.0, 4000.0);
        assert_eq!(t, LensTable::identity());
        assert!(!lens_table(None, &on(), 6000.0, 4000.0).active, "no profile, nothing to do");
    }

    #[test]
    fn zooms_so_corners_stay_filled() {
        let (w, h) = (6000.0, 4000.0);
        let t = lens_table(Some(&barrel()), &on(), w, h);
        assert!(t.active);
        // Every output edge point reads inside the source, and tightly so somewhere (no waste).
        let d = f32::hypot(w, h);
        let (ax, ay) = (w / d, h / d);
        let read = |x: f32, y: f32, c: usize| {
            let r = f32::hypot(x, y);
            let f = {
                let col: Radial = std::array::from_fn(|i| t.knots[i][c]);
                sample(&col, r)
            };
            (f * x, f * y)
        };
        let mut tight: f32 = 0.0;
        for i in 0..=20 {
            let s = i as f32 / 10.0 - 1.0;
            for (x, y) in [(ax * s, ay), (ax, ay * s)] {
                for c in 0..3 {
                    let (u, v) = read(x, y, c);
                    assert!(u.abs() <= ax * 1.002 && v.abs() <= ay * 1.002, "({x}, {y}) reads ({u}, {v})");
                    tight = tight.max((u.abs() / ax).max(v.abs() / ay));
                }
            }
        }
        assert!(tight > 0.99, "zoomed in more than needed: {tight}");
        // Barrel correction pulls the edges in, so it zooms slightly *out* (uses more of the sensor).
        assert!(t.knots[0][1] > 1.0);
        assert!(t.margin > 0.0);
    }

    #[test]
    fn amounts_and_toggles() {
        let p = barrel();
        let full = lens_table(Some(&p), &on(), 600.0, 400.0);
        let no_ca = lens_table(Some(&p), &LensCorrection { chromatic: false, ..on() }, 600.0, 400.0);
        let i = LENS_KNOTS - 8;
        assert_ne!(full.knots[i][0], full.knots[i][1]);
        assert_eq!(no_ca.knots[i][0], no_ca.knots[i][1]);
        let none = lens_table(Some(&p), &LensCorrection { distortion: 0.0, chromatic: false, ..on() }, 600.0, 400.0);
        assert_eq!(none.knots[i][1], 1.0, "only vignetting left");
        assert!(none.knots[i][3] > 1.5);
        let half = lens_table(Some(&p), &LensCorrection { vignetting: 50.0, ..on() }, 600.0, 400.0);
        assert!((half.knots[i][3] - full.knots[i][3].sqrt()).abs() < 1e-5);
        // Manual distortion works without a profile.
        let m = lens_table(None, &LensCorrection { manual_distortion: 50.0, ..Default::default() }, 600.0, 400.0);
        assert!(m.active && m.knots[0][1] != m.knots[LENS_KNOTS - 1][1]);
    }

    #[test]
    fn default_settings_stay_out_of_saved_states() {
        let json = crate::EditState::default().to_canonical_json();
        assert!(!json.contains("lens"), "{json}");
    }
}
