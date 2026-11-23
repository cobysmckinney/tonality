//! Colour grading: a tint and a brightness change for each of the shadows, midtones and
//! highlights (see docs/colour-grading.md).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The edit's colour grading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ColorGrading {
    pub shadows: GradeRange,
    pub midtones: GradeRange,
    pub highlights: GradeRange,
    /// 0..=100. How far the three ranges reach into each other.
    pub blending: f32,
    /// -100..=100. Positive treats more of the photo as highlights, negative as shadows.
    pub balance: f32,
    /// The relationship the panel holds the three hues in while they are linked. It only
    /// guides the panel: the picture is made from the three ranges alone.
    pub link: Option<Harmony>,
}

impl Default for ColorGrading {
    fn default() -> Self {
        Self {
            shadows: GradeRange::default(),
            midtones: GradeRange::default(),
            highlights: GradeRange::default(),
            blending: 50.0,
            balance: 0.0,
            link: None,
        }
    }
}

/// One range's place on the colour wheel, and its brightness.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct GradeRange {
    /// Degrees, 0..360: an OkLCh hue, as the colour mixer's.
    pub hue: f32,
    /// 0..=100: the distance from the wheel's centre. 0 is no tint.
    pub saturation: f32,
    /// -100..=100.
    pub luminance: f32,
}

/// How three linked hues sit around the wheel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Harmony {
    /// Three neighbours, 30° apart.
    Analogous,
    /// Shadows opposite highlights; midtones at the centre.
    Complementary,
    /// 120° apart.
    Triadic,
    /// Shadows alone; midtones and highlights 30° either side of its opposite.
    SplitComplementary,
}

impl Harmony {
    pub fn label(self) -> &'static str {
        match self {
            Harmony::Analogous => "Analogous",
            Harmony::Complementary => "Complementary",
            Harmony::Triadic => "Triadic",
            Harmony::SplitComplementary => "Split-Complementary",
        }
    }
}

/// The strongest tint: OkLab chroma added per unit of lightness, at saturation 100.
pub const GRADE_CHROMA: f32 = 0.2;

impl ColorGrading {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn ranges(&self) -> [&GradeRange; 3] {
        [&self.shadows, &self.midtones, &self.highlights]
    }

    /// What the shader needs, as four vec4s. 0–2, for shadows, midtones and highlights:
    /// xy: the tint (OkLab a, b per unit of lightness), z: brightness in stops. 3: x: blending
    /// (0..1), y: balance (-1..1), z: 1 if any of it changes the picture.
    ///
    /// Clamped, and finite whatever a sidecar from elsewhere holds.
    pub fn uniform(&self) -> [[f32; 4]; 4] {
        let f = |v: f32, lo: f32, hi: f32| {
            if v.is_finite() {
                (v / 100.0).clamp(lo, hi)
            } else {
                0.0
            }
        };
        let mut out = [[0.0; 4]; 4];
        let mut active = false;
        for (row, r) in out.iter_mut().zip(self.ranges()) {
            let c = GRADE_CHROMA * f(r.saturation, 0.0, 1.0);
            let h = if r.hue.is_finite() {
                r.hue.rem_euclid(360.0).to_radians()
            } else {
                0.0
            };
            *row = [c * h.cos(), c * h.sin(), f(r.luminance, -1.0, 1.0), 0.0];
            active |= c != 0.0 || row[2] != 0.0;
        }
        out[3] = [
            if self.blending.is_finite() {
                (self.blending / 100.0).clamp(0.0, 1.0)
            } else {
                0.5
            },
            f(self.balance, -1.0, 1.0),
            if active { 1.0 } else { 0.0 },
            0.0,
        ];
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untouched_grading_does_nothing() {
        let u = ColorGrading::default().uniform();
        assert_eq!(u[3], [0.5, 0.0, 0.0, 0.0]);
        // Blending, balance and the link change nothing on their own.
        let g = ColorGrading {
            blending: 80.0,
            balance: -40.0,
            link: Some(Harmony::Triadic),
            ..Default::default()
        };
        assert_eq!(g.uniform()[3], [0.8, -0.4, 0.0, 0.0]);
        assert!(!g.is_default());
    }

    #[test]
    fn a_tint_points_along_its_hue() {
        let g = ColorGrading {
            highlights: GradeRange {
                hue: 90.0,
                saturation: 50.0,
                luminance: -25.0,
            },
            ..Default::default()
        };
        let u = g.uniform();
        assert!(u[2][0].abs() < 1e-6, "{u:?}");
        assert!((u[2][1] - 0.5 * GRADE_CHROMA).abs() < 1e-6, "{u:?}");
        assert_eq!(u[2][2], -0.25);
        assert_eq!(u[3][2], 1.0, "it changes the picture");
        assert_eq!(u[0], [0.0; 4]);
    }

    #[test]
    fn values_from_elsewhere_are_clamped_and_finite() {
        let g = ColorGrading {
            shadows: GradeRange {
                hue: f32::NAN,
                saturation: 1e9,
                luminance: f32::INFINITY,
            },
            midtones: GradeRange {
                hue: -90.0,
                saturation: 100.0,
                luminance: 0.0,
            },
            blending: f32::NAN,
            balance: -1e9,
            ..Default::default()
        };
        let u = g.uniform();
        assert!(u.iter().flatten().all(|v| v.is_finite()), "{u:?}");
        assert_eq!(u[0], [GRADE_CHROMA, 0.0, 0.0, 0.0]);
        assert!((u[1][1] + GRADE_CHROMA).abs() < 1e-6, "-90° wraps to 270°");
        assert_eq!(u[3], [0.5, -1.0, 1.0, 0.0]);
    }
}
