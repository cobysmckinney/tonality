//! Model input: a neutral, display-referred sRGB rendering of a photo.
//!
//! Segmentation models were trained on ordinary JPEGs, so they see the photo the way a camera
//! would show it: as-shot white balance, camera → sRGB, auto exposure and a gentle tone curve.
//! This runs on the CPU from the decoded preview, so it is deterministic and needs no GPU (it
//! also runs during export).

use half::f16;
use image::{imageops, ImageBuffer, Rgb, RgbImage};
use rayon::prelude::*;
use tonality_core::color::{mul, mul_vec, CameraColor, Mat3, REC2020_TO_XYZ};

/// XYZ (D65) → linear sRGB.
const XYZ_TO_SRGB: Mat3 = [
    [3.2404542, -1.5371385, -0.4985314],
    [-0.969_266, 1.876_010_8, 0.041_556],
    [0.0556434, -0.2040259, 1.0572252],
];

/// Middle grey the auto exposure aims the median at.
const TARGET_MEDIAN: f32 = 0.18;

fn srgb_encode(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Soft shoulder: linear in the mids, rolling off highlights instead of clipping them.
fn tone(v: f32) -> f32 {
    let v = v.max(0.0);
    v * (1.0 + v / 4.0) / (1.0 + v)
}

/// Render camera-space linear RGB (half-float bit patterns, interleaved) as sRGB.
pub fn neutral_srgb(pixels: &[u16], width: u32, height: u32, color: &CameraColor) -> RgbImage {
    let wb = color.as_shot_multipliers();
    let cam_to_srgb = mul(&mul(&XYZ_TO_SRGB, &REC2020_TO_XYZ), &color.cam_to_working());
    let n = (width * height) as usize;
    let lin: Vec<[f32; 3]> = (0..n)
        .into_par_iter()
        .map(|i| {
            // Clip after white balance, so blown highlights go white instead of magenta.
            let c = [0, 1, 2].map(|k| (f16::from_bits(pixels[i * 3 + k]).to_f32() * wb[k]).min(1.0));
            mul_vec(&cam_to_srgb, c).map(|v| v.max(0.0))
        })
        .collect();

    // Auto exposure from the median luminance (robust to small bright areas such as the sun).
    let mut lum: Vec<f32> = lin.iter().step_by(7).map(|c| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).collect();
    let mid = lum.len() / 2;
    let median = if lum.is_empty() { 0.0 } else { *lum.select_nth_unstable_by(mid, |a, b| a.total_cmp(b)).1 };
    let gain = if median > 1e-6 { (TARGET_MEDIAN / median).clamp(0.25, 64.0) } else { 1.0 };

    let mut out = vec![0u8; n * 3];
    out.par_chunks_mut(3).zip(lin.par_iter()).for_each(|(o, c)| {
        for k in 0..3 {
            o[k] = (srgb_encode(tone(c[k] * gain)) * 255.0 + 0.5) as u8;
        }
    });
    ImageBuffer::from_raw(width, height, out).expect("sized above")
}

/// How a model wants its input tensor.
#[derive(Debug, Clone, Copy)]
pub struct Normalize {
    pub mean: [f32; 3],
    pub std: [f32; 3],
}

impl Normalize {
    /// The usual ImageNet statistics (on 0..1 values).
    pub const IMAGENET: Normalize = Normalize { mean: [0.485, 0.456, 0.406], std: [0.229, 0.224, 0.225] };
    /// 0..1, no shift.
    pub const UNIT: Normalize = Normalize { mean: [0.0; 3], std: [1.0; 3] };
}

/// Resize to exactly `w × h` and lay out as a normalised NCHW float tensor (N = 1).
pub fn to_nchw(img: &RgbImage, w: u32, h: u32, norm: Normalize) -> Vec<f32> {
    let r = if img.width() == w && img.height() == h {
        img.clone()
    } else {
        imageops::resize(img, w, h, imageops::FilterType::Triangle)
    };
    let plane = (w * h) as usize;
    let mut out = vec![0.0f32; plane * 3];
    for (i, Rgb(p)) in r.pixels().enumerate() {
        for k in 0..3 {
            out[k * plane + i] = (p[k] as f32 / 255.0 - norm.mean[k]) / norm.std[k];
        }
    }
    out
}

/// Luminance of an sRGB image, 0..1, resized to `w × h` (the guide for [`crate::refine`]).
pub fn guide(img: &RgbImage, w: u32, h: u32) -> Vec<f32> {
    let r = imageops::resize(img, w, h, imageops::FilterType::Triangle);
    r.pixels().map(|Rgb(p)| (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonality_core::color::invert;

    fn color() -> CameraColor {
        // An identity-ish camera: XYZ → camera = XYZ → Rec.2020⁻¹, neutral white balance.
        let xyz_to_rec2020 = invert(&REC2020_TO_XYZ).unwrap();
        CameraColor { xyz_to_cam: xyz_to_rec2020, as_shot_wb: [1.0, 1.0, 1.0] }
    }

    #[test]
    fn grey_stays_grey_and_lands_mid() {
        let px: Vec<u16> = std::iter::repeat_n(f16::from_f32(0.05).to_bits(), 4 * 4 * 3).collect();
        let img = neutral_srgb(&px, 4, 4, &color());
        let p = img.get_pixel(1, 1).0;
        assert!(p[0].abs_diff(p[1]) <= 1 && p[1].abs_diff(p[2]) <= 1, "{p:?}");
        assert!((100..140).contains(&p[1]), "median → mid grey, got {}", p[1]);
    }

    #[test]
    fn clipped_highlights_go_white() {
        let mut c = color();
        c.as_shot_wb = [2.0, 1.0, 1.6];
        // Every channel at the sensor's clip point.
        let px: Vec<u16> = std::iter::repeat_n(f16::from_f32(1.0).to_bits(), 3).chain(std::iter::repeat_n(f16::from_f32(0.02).to_bits(), 15 * 3)).collect();
        let img = neutral_srgb(&px, 4, 4, &c);
        let p = img.get_pixel(0, 0).0;
        assert!(p[0].abs_diff(p[1]) <= 2 && p[1].abs_diff(p[2]) <= 2, "neutral, got {p:?}");
    }

    #[test]
    fn nchw_layout_and_normalisation() {
        let img = RgbImage::from_fn(2, 1, |x, _| if x == 0 { Rgb([255, 0, 0]) } else { Rgb([0, 0, 255]) });
        let t = to_nchw(&img, 2, 1, Normalize::UNIT);
        assert_eq!(t, [1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    }
}
