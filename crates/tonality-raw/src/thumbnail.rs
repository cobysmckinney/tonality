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
    // Smallest candidate that is still big enough; fall back to the largest available.
    let mut best: Option<DynamicImage> = None;
    for img in [decoder.thumbnail_image(&src, &params), decoder.preview_image(&src, &params)]
        .into_iter()
        .filter_map(|r| r.ok().flatten())
    {
        let long = img.width().max(img.height());
        let good = long >= max_edge;
        let better = match &best {
            None => true,
            Some(b) => {
                let blong = b.width().max(b.height());
                (good && (blong < max_edge || long < blong)) || (!good && blong < max_edge && long > blong)
            }
        };
        if better {
            best = Some(img);
        }
    }
    let Some(img) = best else { return Ok(None) };
    // Tiny embedded thumbnails (e.g. 160×120) look bad in the grid; develop instead.
    if img.width().max(img.height()) < max_edge / 2 {
        return Ok(None);
    }
    let img = img.thumbnail(max_edge, max_edge).to_rgb8();
    Ok(Some(match exif_orientation(decoder.as_ref(), &src) {
        Some(o) => orient(img, o),
        None => img,
    }))
}

/// Neutral CPU develop: as-shot WB, camera matrix, gentle tone curve.
fn developed(path: &Path, max_edge: u32) -> Result<RgbImage> {
    let p = decode_preview(path, max_edge as usize)?;
    let wb = p.color.as_shot_multipliers();
    let xyz_to_srgb = [[3.2406, -1.5372, -0.4986], [-0.9689, 1.8758, 0.0415], [0.0557, -0.2040, 1.0570]];
    let m = mul(&mul(&xyz_to_srgb, &REC2020_TO_XYZ), &p.color.cam_to_working());
    let encode = |v: f32| {
        let v = (v * 1.6).max(0.0);
        let v = v / (1.0 + v * 0.35); // soft shoulder
        let v = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
    };
    let mut out = RgbImage::new(p.width, p.height);
    for (px, c) in out.pixels_mut().zip(p.pixels.as_chunks::<3>().0) {
        let cam = [0, 1, 2].map(|i| (half::f16::from_bits(c[i]).to_f32() * wb[i]).min(1.0));
        px.0 = mul_vec(&m, cam).map(encode);
    }
    Ok(imageops::thumbnail(&out, out.width().min(max_edge), out.height().min(max_edge)))
}
