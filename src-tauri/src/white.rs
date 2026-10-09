//! White balance: the light the camera balanced a RAW for, and the
//! Temperature and Tint that make a colour neutral.
//!
//! Recipes store Temperature and Tint relative to the photo as opened: 0 is
//! as shot, so a recipe or preset means the same on any photo. The develop
//! shader turns them into gains on linear sRGB: each 100 of Temperature
//! moves red and blue `TEMPERATURE_STOPS` stops apart, each 100 of Tint moves
//! green `TINT_STOPS` stops against them. The interface shows them in Kelvin
//! from the as-shot white found here (`whiteBalance.ts`).

use std::collections::HashMap;

use rawler::imgop::xyz::Illuminant;
use rawler::RawImage;
use serde::Serialize;

/// Stops red and blue move apart for each 100 of Temperature. The same in develop.wgsl.
pub const TEMPERATURE_STOPS: f32 = 1.2;
/// Stops green moves against red and blue for each 100 of Tint. The same in develop.wgsl.
pub const TINT_STOPS: f32 = 0.35;

/// Daylight, 6500 K: the white sRGB is balanced for.
pub const D65: [f32; 2] = [0.31271, 0.32902];
const D50: [f32; 2] = [0.34567, 0.35850];

/// A colour darker than this in any channel is too dark to judge.
const DARKEST: f32 = 0.0005;
/// A RAW channel this bright may have clipped, which changes its colour.
const CLIPPED: f32 = 0.97;

/// Why a colour can't be made neutral.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unjudgeable {
    TooDark,
    TooBright,
}

impl std::fmt::Display for Unjudgeable {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooDark => "That spot is too dark to judge its colour. Click something grey or white in good light.",
            Self::TooBright => "That spot is too bright: the camera lost its colour there. Click something grey or white that isn't blown out.",
        })
    }
}

impl std::error::Error for Unjudgeable {}

/// The Temperature and Tint (slider units, relative to as shot) that make
/// `colour` grey. `colour` is linear light as the photo was opened, before
/// any white balance; `clips` says whether it is a RAW's, whose brightest
/// values may have clipped.
pub fn neutralising(colour: [f32; 3], clips: bool) -> Result<(f32, f32), Unjudgeable> {
    let [r, g, b] = colour;
    if !colour.iter().all(|v| v.is_finite() && *v > DARKEST) {
        return Err(Unjudgeable::TooDark);
    }
    if clips && r.max(g).max(b) >= CLIPPED {
        return Err(Unjudgeable::TooBright);
    }
    let temperature = (b / r).log2() / TEMPERATURE_STOPS * 100.0;
    let tint = (g / (r * b).sqrt()).log2() / TINT_STOPS * 100.0;
    Ok((temperature, tint))
}

/// What the interface needs to show a RAW's white balance in Kelvin.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Camera {
    /// The light the camera balanced for, as a CIE xy chromaticity.
    pub as_shot: [f32; 2],
    /// What a white card under other light looks like in the photo as
    /// opened: for each light the camera's colour is measured under (its
    /// colour temperature), the matrix from the light's XYZ to the card's
    /// linear sRGB. Between them, interpolate by inverse temperature.
    pub lights: Vec<Measured>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Measured {
    pub kelvin: f32,
    pub matrix: [[f32; 3]; 3],
}

/// A RAW's white balance, as `Camera` describes it. None when the file
/// doesn't say enough.
pub fn camera(raw: &RawImage) -> Option<Camera> {
    camera_for(raw.wb_coeffs, &raw.color_matrix)
}

/// `camera` from a camera's white balance `wb` and its colour matrices
/// (XYZ to camera, by the light each was measured under).
fn camera_for(wb: [f32; 4], color_matrix: &HashMap<Illuminant, Vec<f32>>) -> Option<Camera> {
    if !wb[..3].iter().all(|v| v.is_finite() && *v > 0.0) {
        return None;
    }
    let neutral = [1.0 / wb[0], 1.0 / wb[1], 1.0 / wb[2]];
    let as_matrix = |values: &Vec<f32>| {
        let rows = values.as_chunks::<3>().0;
        (rows.len() == 3).then(|| [rows[0], rows[1], rows[2]])
    };
    let mut matrices: Vec<(f32, [[f32; 3]; 3])> = color_matrix
        .iter()
        .filter_map(|(illuminant, matrix)| Some((temperature_of(*illuminant)?, as_matrix(matrix)?)))
        .collect();
    matrices.sort_by(|a, b| a.0.total_cmp(&b.0));
    if matrices.is_empty() {
        return None;
    }

    // Which matrix applies depends on the light, which is what is being
    // found: start from D50 and settle on an answer.
    let mut xy = D50;
    for _ in 0..30 {
        let xyz = mul(invert(interpolate(&matrices, temperature(xy)))?, neutral);
        let sum: f32 = xyz.iter().sum();
        let next = [xyz[0] / sum, xyz[1] / sum];
        if !next.iter().all(|v| v.is_finite()) {
            return None;
        }
        let settled = (next[0] - xy[0]).abs() + (next[1] - xy[1]).abs() < 1e-7;
        xy = next;
        if settled {
            break;
        }
    }
    // A light no camera would balance for means the numbers are wrong.
    if !((0.2..0.6).contains(&xy[0]) && (0.2..0.5).contains(&xy[1])) {
        return None;
    }

    // How rawler takes the balanced camera colour to linear sRGB: through
    // the daylight matrix, scaled so the camera's white is sRGB's.
    let daylight = as_matrix(color_matrix.get(&Illuminant::D65)?)?;
    let to_camera = multiply(daylight, SRGB_TO_XYZ).map(|row| {
        let sum: f32 = row.iter().sum();
        row.map(|v| v / sum)
    });
    let to_srgb = invert(to_camera)?;
    let balanced = |m: [[f32; 3]; 3]| std::array::from_fn(|r| m[r].map(|v| v * wb[r]));
    let lights = matrices.iter().map(|&(kelvin, m)| Measured { kelvin, matrix: multiply(to_srgb, balanced(m)) }).collect();
    Some(Camera { as_shot: xy, lights })
}

/// The colour temperature a colour matrix is measured under.
fn temperature_of(illuminant: Illuminant) -> Option<f32> {
    use Illuminant::*;
    Some(match illuminant {
        A | Tungsten => 2856.0,
        IsoStudioTungsten => 3200.0,
        WhiteFluorescent | Fluorescent => 3450.0,
        CoolWhiteFluorescent => 4150.0,
        B => 4874.0,
        D50 | DaylightWhiteFluorescent => 5003.0,
        D55 | Daylight | FineWeather | Flash => 5503.0,
        DaylightFluorescent => 6430.0,
        D65 | CloudyWeather => 6504.0,
        C => 6774.0,
        D75 | Shade => 7504.0,
        Unknown => return None,
    })
}

/// McCamy's approximation of a colour's temperature: close enough to
/// choose between colour matrices.
fn temperature([x, y]: [f32; 2]) -> f32 {
    let n = (x - 0.3320) / (0.1858 - y);
    (449.0 * n.powi(3) + 3525.0 * n.powi(2) + 6823.3 * n + 5520.33).clamp(1000.0, 50000.0)
}

/// The colour matrix for light of `kelvin`: between the two measured either
/// side of it, weighted by inverse temperature as the DNG specification says.
fn interpolate(matrices: &[(f32, [[f32; 3]; 3])], kelvin: f32) -> [[f32; 3]; 3] {
    let (first, last) = (matrices[0], matrices[matrices.len() - 1]);
    if kelvin <= first.0 {
        return first.1;
    }
    if kelvin >= last.0 {
        return last.1;
    }
    let above = matrices.iter().position(|m| m.0 >= kelvin).expect("kelvin is below the last");
    let ((low, m1), (high, m2)) = (matrices[above - 1], matrices[above]);
    let weight = (1.0 / kelvin - 1.0 / high) / (1.0 / low - 1.0 / high);
    std::array::from_fn(|r| std::array::from_fn(|c| weight * m1[r][c] + (1.0 - weight) * m2[r][c]))
}

/// Linear sRGB to XYZ, as rawler has it.
const SRGB_TO_XYZ: [[f32; 3]; 3] = [[0.4124564, 0.3575761, 0.1804375], [0.2126729, 0.7151522, 0.072175], [0.0193339, 0.1191920, 0.9503041]];

fn multiply(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

fn mul(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

fn invert(m: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 {
        return None;
    }
    Some([
        [(e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det],
        [(f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det],
        [(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
    ])
}

/// The camera's white balance for light of chromaticity `xy`, seen through
/// `matrix` (XYZ to camera, nine numbers): what to multiply each channel
/// by, with green left alone. None if the matrix makes no sense of it.
pub fn balance_for(xy: [f32; 2], matrix: &[f32]) -> Option<[f32; 3]> {
    let [x, y] = xy;
    let xyz = [x / y, 1.0, (1.0 - x - y) / y];
    let rows = matrix.as_chunks::<3>().0;
    if rows.len() != 3 {
        return None;
    }
    let neutral = mul([rows[0], rows[1], rows[2]], xyz);
    neutral.iter().all(|v| v.is_finite() && *v > 0.0).then(|| [neutral[1] / neutral[0], 1.0, neutral[1] / neutral[2]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the develop shader does with Temperature and Tint.
    fn balanced(colour: [f32; 3], temperature: f32, tint: f32) -> [f32; 3] {
        let (t, g) = (temperature / 100.0, tint / 100.0);
        let gains = [(t * TEMPERATURE_STOPS / 2.0).exp2(), (-g * TINT_STOPS).exp2(), (-t * TEMPERATURE_STOPS / 2.0).exp2()];
        std::array::from_fn(|c| colour[c] * gains[c])
    }

    #[test]
    fn the_shader_balances_as_these_numbers_say() {
        let shader = include_str!("shaders/develop.wgsl");
        assert!(shader.contains(&format!("const TEMPERATURE_STOPS = {TEMPERATURE_STOPS};")));
        assert!(shader.contains(&format!("const TINT_STOPS = {TINT_STOPS};")));
    }

    #[test]
    fn a_picked_colour_comes_out_grey() {
        for colour in [[0.4, 0.3, 0.15], [0.1, 0.2, 0.3], [0.05, 0.08, 0.05], [0.3, 0.3, 0.3]] {
            let (temperature, tint) = neutralising(colour, true).unwrap();
            let [r, g, b] = balanced(colour, temperature, tint);
            assert!((r / g - 1.0).abs() < 1e-4 && (b / g - 1.0).abs() < 1e-4, "{colour:?} came out {:?}", [r, g, b]);
        }
        // A warm colour is cooled, a green one made more magenta.
        let (temperature, _) = neutralising([0.4, 0.3, 0.15], true).unwrap();
        assert!(temperature < 0.0);
        let (_, tint) = neutralising([0.2, 0.3, 0.2], true).unwrap();
        assert!(tint > 0.0);
    }

    #[test]
    fn black_and_blown_out_spots_cant_be_judged() {
        assert_eq!(neutralising([0.0, 0.1, 0.1], true), Err(Unjudgeable::TooDark));
        assert_eq!(neutralising([0.99, 0.8, 0.5], true), Err(Unjudgeable::TooBright));
        // Only a RAW clips: a JPEG's white is just white.
        assert!(neutralising([0.99, 0.8, 0.5], false).is_ok());
    }

    /// A camera's colour matrices, and its white balance for `light`.
    fn balanced_for(light: [f32; 2], matrices: &[(Illuminant, [[f32; 3]; 3])]) -> ([f32; 4], HashMap<Illuminant, Vec<f32>>) {
        let mut sorted: Vec<_> = matrices.iter().map(|(i, m)| (temperature_of(*i).unwrap(), *m)).collect();
        sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
        let wb = balance_for(light, interpolate(&sorted, temperature(light)).as_flattened()).unwrap();
        let color_matrix = matrices.iter().map(|(illuminant, m)| (*illuminant, m.as_flattened().to_vec())).collect();
        ([wb[0], wb[1], wb[2], f32::NAN], color_matrix)
    }

    const XYZ_TO_SRGB: [[f32; 3]; 3] = [[3.2406, -1.5372, -0.4986], [-0.9689, 1.8758, 0.0415], [0.0557, -0.2040, 1.0570]];

    fn scaled(m: [[f32; 3]; 3], by: [f32; 3]) -> [[f32; 3]; 3] {
        std::array::from_fn(|r| m[r].map(|v| v * by[r]))
    }

    #[test]
    fn the_as_shot_light_is_found_from_the_white_balance() {
        let one = [(Illuminant::D65, scaled(XYZ_TO_SRGB, [0.5, 1.0, 0.7]))];
        // Two matrices, as camera makers measure them, under tungsten and daylight.
        let two = [(Illuminant::A, scaled(XYZ_TO_SRGB, [0.45, 1.0, 0.8])), (Illuminant::D65, scaled(XYZ_TO_SRGB, [0.5, 1.0, 0.7]))];
        for matrices in [&one[..], &two[..]] {
            for light in [D65, [0.4476, 0.4074], [0.3457, 0.3585]] {
                let (wb, color_matrix) = balanced_for(light, matrices);
                let found = camera_for(wb, &color_matrix).unwrap().as_shot;
                assert!((found[0] - light[0]).abs() < 1e-3 && (found[1] - light[1]).abs() < 1e-3, "{light:?} found as {found:?}");
            }
        }
    }

    #[test]
    fn a_white_card_under_the_as_shot_light_comes_out_white() {
        let two = [(Illuminant::A, scaled(XYZ_TO_SRGB, [0.45, 1.0, 0.8])), (Illuminant::D65, scaled(XYZ_TO_SRGB, [0.5, 1.0, 0.7]))];
        let warm = [0.4476, 0.4074];
        let (wb, color_matrix) = balanced_for(warm, &two);
        let camera = camera_for(wb, &color_matrix).unwrap();
        assert_eq!(camera.lights.iter().map(|light| light.kelvin).collect::<Vec<_>>(), [2856.0, 6504.0]);
        // Under tungsten (A), the card is white; under daylight, blue.
        let card = |light: &Measured, xy: [f32; 2]| mul(light.matrix, [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]]);
        let [r, g, b] = card(&camera.lights[0], warm);
        assert!((r / g - 1.0).abs() < 1e-3 && (b / g - 1.0).abs() < 1e-3, "{:?}", [r, g, b]);
        let [r, _, b] = card(&camera.lights[1], D65);
        assert!(b > r * 1.5, "{:?}", [r, b]);
    }

    #[test]
    fn a_raw_without_white_balance_or_matrices_has_no_as_shot_light() {
        let (wb, color_matrix) = balanced_for(D65, &[(Illuminant::D65, XYZ_TO_SRGB)]);
        assert!(camera_for(wb, &color_matrix).is_some());
        assert_eq!(camera_for([f32::NAN; 4], &color_matrix), None);
        assert_eq!(camera_for(wb, &HashMap::new()), None);
        // A matrix for no particular light can't be placed between others.
        let unknown = HashMap::from([(Illuminant::Unknown, XYZ_TO_SRGB.as_flattened().to_vec())]);
        assert_eq!(camera_for(wb, &unknown), None);
    }
}
