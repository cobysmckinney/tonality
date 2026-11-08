use std::path::Path;

use half::f16;
use image::{imageops, ImageBuffer, Rgb};
use rawler::decoders::{Orientation, RawDecodeParams};
use rawler::rawsource::RawSource;
use rawler::imgop::xyz::Illuminant;
use rawler::rawimage::{RawImage, RawImageData, RawPhotometricInterpretation};
use rayon::prelude::*;
use tonality_core::color::{CameraColor, Mat3};

use crate::{Metadata, RawError, Result};

type RgbF32 = ImageBuffer<Rgb<f32>, Vec<f32>>;

/// A display-sized, camera-space linear image ready for upload as an RGB16F texture.
pub struct Preview {
    pub width: u32,
    pub height: u32,
    /// Interleaved RGB, IEEE half floats (bit patterns), row-major, top row first.
    pub pixels: Vec<u16>,
    pub color: CameraColor,
    pub metadata: Metadata,
}

impl Preview {
    /// Little-endian bytes of `pixels`, for IPC.
    pub fn pixel_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 2);
        for p in &self.pixels {
            out.extend_from_slice(&p.to_le_bytes());
        }
        out
    }
}

/// Decode a RAW file into a preview whose long edge is at most `max_edge`.
///
/// Demosaicing uses CFA "superpixels" (2×2 for Bayer, 3×3 for X-Trans): every output pixel is
/// the per-colour average of one CFA block. At preview sizes this is both fast and artefact-free;
/// a full-resolution demosaic is used for export.
pub fn decode_preview(path: &Path, max_edge: usize) -> Result<Preview> {
    let err = |e: rawler::RawlerError| RawError::Decode(e.to_string());
    let src = RawSource::new(path).map_err(|e| RawError::Decode(e.to_string()))?;
    let decoder = rawler::get_decoder(&src).map_err(err)?;
    let params = RawDecodeParams::default();
    let mut raw = decoder.raw_image(&src, &params, false).map_err(err)?;
    raw.apply_scaling().map_err(err)?;

    // Not every decoder fills `RawImage::orientation` (e.g. Sony ARW), so prefer EXIF.
    let orientation = exif_orientation(decoder.as_ref(), &src).unwrap_or(raw.orientation);

    let color = camera_color(&raw);
    let metadata = Metadata { make: raw.clean_make.clone(), model: raw.clean_model.clone() };

    let mut img = match &raw.photometric {
        RawPhotometricInterpretation::Cfa(cfg) if raw.cpp == 1 && cfg.cfa.is_rgb() => {
            superpixel(&raw, &cfg.cfa)?
        }
        RawPhotometricInterpretation::LinearRaw if raw.cpp == 3 => linear_rgb(&raw),
        other => return Err(RawError::Unsupported(format!("{other:?} with {} cpp", raw.cpp))),
    };

    let long = img.width().max(img.height()) as usize;
    if long > max_edge {
        let s = max_edge as f32 / long as f32;
        let w = ((img.width() as f32 * s).round() as u32).max(1);
        let h = ((img.height() as f32 * s).round() as u32).max(1);
        img = imageops::resize(&img, w, h, imageops::FilterType::Triangle);
    }

    img = orient(img, orientation);

    let pixels = img
        .as_raw()
        .par_iter()
        .map(|&v| f16::from_f32(v.clamp(0.0, 65504.0)).to_bits())
        .collect();

    Ok(Preview { width: img.width(), height: img.height(), pixels, color, metadata })
}

pub(crate) fn exif_orientation(decoder: &dyn rawler::decoders::Decoder, src: &RawSource) -> Option<Orientation> {
    decoder
        .raw_metadata(src, &RawDecodeParams::default())
        .ok()
        .and_then(|m| m.exif.orientation)
        .map(Orientation::from_u16)
        .filter(|o| *o != Orientation::Unknown)
}

fn float_data(raw: &RawImage) -> &[f32] {
    match &raw.data {
        RawImageData::Float(d) => d,
        // apply_scaling() always produces Float.
        RawImageData::Integer(_) => unreachable!("apply_scaling converts to float"),
    }
}

/// (x, y, w, h) of the recommended crop within the full sensor image.
fn crop_rect(raw: &RawImage) -> (usize, usize, usize, usize) {
    match raw.crop_area.or(raw.active_area) {
        Some(r) => (r.p.x, r.p.y, r.d.w, r.d.h),
        None => (0, 0, raw.width, raw.height),
    }
}

fn superpixel(raw: &RawImage, cfa: &rawler::CFA) -> Result<RgbF32> {
    let block = match (cfa.width, cfa.height) {
        (2, 2) => 2,
        (6, 6) => 3, // X-Trans: every 3×3 tile contains R, G and B.
        (w, h) => return Err(RawError::Unsupported(format!("{w}×{h} CFA"))),
    };
    let data = float_data(raw);
    let stride = raw.width;
    let (cx, cy, cw, ch) = crop_rect(raw);
    let (ow, oh) = (cw / block, ch / block);
    let mut out = vec![0f32; ow * oh * 3];

    out.par_chunks_mut(ow * 3).enumerate().for_each(|(oy, row)| {
        let y0 = cy + oy * block;
        for ox in 0..ow {
            let x0 = cx + ox * block;
            let mut sum = [0f32; 3];
            let mut n = [0u32; 3];
            for y in y0..y0 + block {
                for x in x0..x0 + block {
                    let c = cfa.color_at(y, x);
                    if c < 3 {
                        sum[c] += data[y * stride + x];
                        n[c] += 1;
                    }
                }
            }
            for c in 0..3 {
                row[ox * 3 + c] = if n[c] > 0 { sum[c] / n[c] as f32 } else { 0.0 };
            }
        }
    });

    Ok(RgbF32::from_raw(ow as u32, oh as u32, out).expect("buffer size matches"))
}

fn linear_rgb(raw: &RawImage) -> RgbF32 {
    let data = float_data(raw);
    let (cx, cy, cw, ch) = crop_rect(raw);
    let mut out = Vec::with_capacity(cw * ch * 3);
    for y in cy..cy + ch {
        let start = (y * raw.width + cx) * 3;
        out.extend_from_slice(&data[start..start + cw * 3]);
    }
    RgbF32::from_raw(cw as u32, ch as u32, out).expect("buffer size matches")
}

pub(crate) fn orient<P: image::Pixel + 'static>(
    img: ImageBuffer<P, Vec<P::Subpixel>>,
    o: Orientation,
) -> ImageBuffer<P, Vec<P::Subpixel>> {
    match o {
        Orientation::Normal | Orientation::Unknown => img,
        Orientation::HorizontalFlip => imageops::flip_horizontal(&img),
        Orientation::Rotate180 => imageops::rotate180(&img),
        Orientation::VerticalFlip => imageops::flip_vertical(&img),
        Orientation::Transpose => imageops::flip_horizontal(&imageops::rotate90(&img)),
        Orientation::Rotate90 => imageops::rotate90(&img),
        Orientation::Transverse => imageops::flip_vertical(&imageops::rotate90(&img)),
        Orientation::Rotate270 => imageops::rotate270(&img),
    }
}

fn camera_color(raw: &RawImage) -> CameraColor {
    let preferred = [
        Illuminant::D65,
        Illuminant::D55,
        Illuminant::D50,
        Illuminant::Daylight,
        Illuminant::D75,
        Illuminant::Flash,
        Illuminant::A,
    ];
    let xyz_to_cam: Mat3 = match raw.color_matrix_find_first(preferred) {
        Some((_, m)) if m.len() >= 9 => [[m[0], m[1], m[2]], [m[3], m[4], m[5]], [m[6], m[7], m[8]]],
        _ => [raw.xyz_to_cam[0], raw.xyz_to_cam[1], raw.xyz_to_cam[2]],
    };
    let wb = if raw.wb_coeffs[..3].iter().all(|v| v.is_finite() && *v > 0.0) {
        [raw.wb_coeffs[0], raw.wb_coeffs[1], raw.wb_coeffs[2]]
    } else {
        let n = raw.neutralwb();
        [n[0], n[1], n[2]]
    };
    CameraColor { xyz_to_cam, as_shot_wb: wb }
}
