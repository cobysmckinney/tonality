//! Library thumbnails: the camera's embedded JPEG when there is one (fast, and matches what
//! the photographer saw on the back of the camera), else a quick neutral develop of the RAW.

use std::path::Path;

use image::{imageops, DynamicImage, RgbImage};
use rawler::decoders::RawDecodeParams;
use rawler::rawsource::RawSource;
use tonality_core::color::{mul, mul_vec, REC2020_TO_XYZ};

use crate::preview::{exif_orientation, orient};
use crate::{decode_preview, RawError, Result};

/// A thumbnail whose long edge is at most `max_edge`, correctly oriented, in sRGB.
pub fn thumbnail(path: &Path, max_edge: u32) -> Result<RgbImage> {
    match embedded(path, max_edge) {
        Ok(Some(img)) => Ok(img),
        _ => developed(path, max_edge),
    }
}

fn embedded(path: &Path, max_edge: u32) -> Result<Option<RgbImage>> {
    let src = RawSource::new(path).map_err(|e| RawError::Decode(e.to_string()))?;
    let decoder = rawler::get_decoder(&src).map_err(|e| RawError::Decode(e.to_string()))?;
    let params = RawDecodeParams::default();
    let mut images: Vec<DynamicImage> = [
        decoder.thumbnail_image(&src, &params),
        decoder.preview_image(&src, &params),
    ]
    .into_iter()
    .filter_map(|r| r.ok().flatten())
    .collect();
    let sizes: Vec<(u32, u32)> = images.iter().map(|i| (i.width(), i.height())).collect();
    let Some(chosen) = choose(&sizes, max_edge) else {
        return Ok(None);
    };
    let img = images.swap_remove(chosen);
    let img = img.thumbnail(max_edge, max_edge).to_rgb8();
    Ok(Some(match exif_orientation(decoder.as_ref(), &src) {
        Some(o) => orient(img, o),
        None => img,
    }))
}

/// Which of a file's embedded images (`sizes`, width × height, in the order found) to make a
/// `max_edge` thumbnail from: the smallest that is still big enough, else the largest there
/// is, and none at all if that is under half the size wanted (a 160 × 120 thumbnail scaled up
/// looks bad in the grid: the raw is developed instead).
fn choose(sizes: &[(u32, u32)], max_edge: u32) -> Option<usize> {
    let long = |i: usize| sizes[i].0.max(sizes[i].1);
    let mut best: Option<usize> = None;
    for i in 0..sizes.len() {
        let good = long(i) >= max_edge;
        let better = match best {
            None => true,
            Some(b) => {
                (good && (long(b) < max_edge || long(i) < long(b)))
                    || (!good && long(b) < max_edge && long(i) > long(b))
            }
        };
        if better {
            best = Some(i);
        }
    }
    best.filter(|&b| long(b) >= max_edge / 2)
}

/// Neutral CPU develop: as-shot WB, camera matrix, gentle tone curve.
fn developed(path: &Path, max_edge: u32) -> Result<RgbImage> {
    let p = decode_preview(path, max_edge as usize)?;
    let wb = p.color.as_shot_multipliers();
    let xyz_to_srgb = [
        [3.2406, -1.5372, -0.4986],
        [-0.9689, 1.8758, 0.0415],
        [0.0557, -0.2040, 1.0570],
    ];
    let m = mul(
        &mul(&xyz_to_srgb, &REC2020_TO_XYZ),
        &p.color.cam_to_working(),
    );
    let encode = |v: f32| {
        let v = (v * 1.6).max(0.0);
        let v = v / (1.0 + v * 0.35); // soft shoulder
        let v = if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
    };
    let mut out = RgbImage::new(p.width, p.height);
    for (px, c) in out.pixels_mut().zip(p.pixels.as_chunks::<3>().0) {
        let cam = [0, 1, 2].map(|i| (half::f16::from_bits(c[i]).to_f32() * wb[i]).min(1.0));
        px.0 = mul_vec(&m, cam).map(encode);
    }
    Ok(imageops::thumbnail(
        &out,
        out.width().min(max_edge),
        out.height().min(max_edge),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_smallest_embedded_image_that_is_big_enough_is_used() {
        // What cameras embed: a small thumbnail and a larger preview, either way up.
        let both = [(160, 120), (1616, 1080)];
        assert_eq!(choose(&both, 640), Some(1), "the thumbnail is too small");
        assert_eq!(choose(&both, 160), Some(0), "the thumbnail will do");
        assert_eq!(
            choose(&[(1616, 1080), (160, 120)], 160),
            Some(1),
            "in either order"
        );
        // Two that are big enough: the smaller, which is quicker to scale down.
        assert_eq!(choose(&[(6000, 4000), (1616, 1080)], 640), Some(1));
        assert_eq!(choose(&[(1616, 1080), (6000, 4000)], 640), Some(0));
        // Portrait counts by its long edge.
        assert_eq!(choose(&[(120, 160), (1080, 1616)], 1200), Some(1));
        // None big enough: the largest, if it is at least half the size wanted...
        assert_eq!(choose(&[(160, 120), (400, 300)], 640), Some(1));
        assert_eq!(choose(&[(400, 300), (160, 120)], 640), Some(0));
        assert_eq!(choose(&[(320, 240)], 640), Some(0), "exactly half");
        // ...and nothing otherwise: the raw is developed instead.
        assert_eq!(choose(&[(160, 120)], 640), None);
        assert_eq!(choose(&[(160, 120), (319, 200)], 640), None);
        assert_eq!(choose(&[], 640), None);
    }
}
