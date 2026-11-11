use std::path::Path;

use half::f16;
use image::{imageops, ImageBuffer, Rgb};
use rawler::decoders::{Orientation, RawDecodeParams};
use rawler::rawsource::RawSource;
use rawler::imgop::xyz::Illuminant;
use rawler::rawimage::{RawImage, RawImageData, RawPhotometricInterpretation};
use rayon::prelude::*;
use tonality_core::color::{CameraColor, Mat3};
use tonality_core::lens::LensProfile;

use crate::{Metadata, RawError, Result};

type RgbF32 = ImageBuffer<Rgb<f32>, Vec<f32>>;

/// A display-sized, camera-space linear image ready for upload as an RGB16F texture.
pub struct Preview {
    pub width: u32,
    pub height: u32,
    /// Interleaved RGB, IEEE half floats (bit patterns), row-major, top row first.
    pub pixels: Vec<u16>,
    pub color: CameraColor,
    /// The lens's correction profile, if known.
    pub lens: Option<LensProfile>,
    pub metadata: Metadata,
    /// Size of the full-resolution image ([`decode_full`]), after orientation.
    pub full_width: u32,
    pub full_height: u32,
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
    let mut d = Decoded::open(path)?;
    let (_, _, cw, ch) = crop_rect(&d.raw);
    let (full_width, full_height) =
        if transposes(d.orientation) { (ch as u32, cw as u32) } else { (cw as u32, ch as u32) };

    let mut img = match &d.raw.photometric {
        RawPhotometricInterpretation::Cfa(cfg) if d.raw.cpp == 1 && cfg.cfa.is_rgb() => {
            superpixel(&d.raw, &cfg.cfa)?
        }
        RawPhotometricInterpretation::LinearRaw if d.raw.cpp == 3 => linear_rgb(&d.raw),
        other => return Err(RawError::Unsupported(format!("{other:?} with {} cpp", d.raw.cpp))),
    };
    d.raw.data = RawImageData::Float(Vec::new()); // free the sensor data early

    let long = img.width().max(img.height()) as usize;
    if long > max_edge {
        let s = max_edge as f32 / long as f32;
        let w = ((img.width() as f32 * s).round() as u32).max(1);
        let h = ((img.height() as f32 * s).round() as u32).max(1);
        img = imageops::resize(&img, w, h, imageops::FilterType::Triangle);
    }

    img = orient(img, d.orientation);

    let pixels = img
        .as_raw()
        .par_iter()
        .map(|&v| f16::from_f32(v.clamp(0.0, 65504.0)).to_bits())
        .collect();

    Ok(Preview {
        width: img.width(),
        height: img.height(),
        pixels,
        color: d.color,
        lens: profile(path),
        metadata: d.metadata,
        full_width,
        full_height,
    })
}

/// A full-resolution, demosaiced, oriented camera-space image (RGB half floats).
pub struct FullImage {
    pub width: u32,
    pub height: u32,
    /// Interleaved RGB, IEEE half floats (bit patterns), row-major, top row first.
    pub pixels: Vec<u16>,
    pub color: CameraColor,
    pub lens: Option<LensProfile>,
}

/// The lens profile for a RAW (embedded data, else lensfun).
fn profile(path: &Path) -> Option<LensProfile> {
    crate::lens_profile(path, &crate::read_meta(path).ok()?)
}

impl FullImage {
    /// The pixels of `(x, y, w, h)` (clamped to the image), box-downsampled by `2^level`.
    /// Returns the region actually covered (full-res pixels), the output size, and RGB halves.
    pub fn region(&self, x: u32, y: u32, w: u32, h: u32, level: u32) -> Region {
        let f = 1u32 << level.min(6);
        // Align to the downsampling factor so neighbouring requests line up.
        let x0 = (x.min(self.width.saturating_sub(1)) / f) * f;
        let y0 = (y.min(self.height.saturating_sub(1)) / f) * f;
        let x1 = x.saturating_add(w).min(self.width);
        let y1 = y.saturating_add(h).min(self.height);
        let (ow, oh) = ((x1.saturating_sub(x0) / f).max(1), (y1.saturating_sub(y0) / f).max(1));
        let stride = self.width as usize * 3;
        let mut out = vec![0u16; ow as usize * oh as usize * 3];
        out.par_chunks_mut(ow as usize * 3).enumerate().for_each(|(oy, row)| {
            let sy = y0 as usize + oy * f as usize;
            if f == 1 {
                let start = sy * stride + x0 as usize * 3;
                row.copy_from_slice(&self.pixels[start..start + row.len()]);
                return;
            }
            for ox in 0..ow as usize {
                let sx = x0 as usize + ox * f as usize;
                let mut acc = [0f32; 3];
                // Boxes at the image edge can be partly outside it: average what's inside.
                let ys = sy..(sy + f as usize).min(self.height as usize);
                let xs = sx..(sx + f as usize).min(self.width as usize);
                let n = (ys.len() * xs.len()).max(1) as f32;
                for yy in ys {
                    for xx in xs.clone() {
                        let i = yy * stride + xx * 3;
                        for (c, a) in acc.iter_mut().enumerate() {
                            *a += f16::from_bits(self.pixels[i + c]).to_f32();
                        }
                    }
                }
                for c in 0..3 {
                    row[ox * 3 + c] = f16::from_f32(acc[c] / n).to_bits();
                }
            }
        });
        Region { x: x0, y: y0, w: ow * f, h: oh * f, width: ow, height: oh, pixels: out }
    }
}

impl FullImage {
    /// A copy scaled down so its long edge is `long_edge` (never up). Returns `(w, h, pixels)`.
    pub fn scaled(&self, long_edge: u32) -> (u32, u32, Vec<u16>) {
        let long = self.width.max(self.height);
        if long_edge >= long {
            return (self.width, self.height, self.pixels.clone());
        }
        let ratio = long as f32 / long_edge as f32;
        // Cheap power-of-two box reduction first, leaving at least 2× for the proper filter.
        let level = if ratio >= 4.0 { (ratio / 2.0).log2().floor() as u32 } else { 0 };
        let r = self.region(0, 0, self.width, self.height, level);
        let f32s: Vec<f32> = r.pixels.par_iter().map(|&v| f16::from_bits(v).to_f32()).collect();
        let img = RgbF32::from_raw(r.width, r.height, f32s).expect("buffer size matches");
        let s = long_edge as f32 / long as f32;
        let w = ((self.width as f32 * s).round() as u32).max(1);
        let h = ((self.height as f32 * s).round() as u32).max(1);
        let out = imageops::resize(&img, w, h, imageops::FilterType::CatmullRom);
        let pixels = out.as_raw().par_iter().map(|&v| f16::from_f32(v.clamp(0.0, 65504.0)).to_bits()).collect();
        (w, h, pixels)
    }
}

/// A piece of a [`FullImage`]: `(x, y, w, h)` in full-resolution pixels, `width × height` samples.
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u16>,
}

/// Decode and demosaic a RAW file at full resolution (PPG for Bayer, Markesteijn for X-Trans).
pub fn decode_full(path: &Path) -> Result<FullImage> {
    use rawler::imgop::sensor::bayer::ppg::PPGDemosaic;
    use rawler::imgop::sensor::xtrans::markesteijn::XTransMarkesteijnDemosaic;
    use rawler::imgop::sensor::{Demosaic, SensorType};
    use rawler::imgop::{Dim2, Point, Rect};
    use rawler::pixarray::PixF32;

    let mut d = Decoded::open(path)?;
    let (cx, cy, cw, ch) = crop_rect(&d.raw);
    let (w, h, rgb): (usize, usize, Vec<f32>) = match &d.raw.photometric {
        RawPhotometricInterpretation::Cfa(cfg) if d.raw.cpp == 1 && cfg.cfa.is_rgb() => {
            let cfg = cfg.clone();
            let data = match std::mem::replace(&mut d.raw.data, RawImageData::Float(Vec::new())) {
                RawImageData::Float(v) => v,
                RawImageData::Integer(_) => unreachable!("apply_scaling converts to float"),
            };
            let pixels = PixF32::new_with(data, d.raw.width, d.raw.height);
            let roi = Rect::new(Point::new(cx, cy), Dim2::new(cw, ch));
            let out = match cfg.sensor {
                SensorType::Bayer => PPGDemosaic::new().demosaic(&pixels, &cfg.cfa, &cfg.colors, roi),
                SensorType::Xtrans => {
                    XTransMarkesteijnDemosaic::new_pass_1().demosaic(&pixels, &cfg.cfa, &cfg.colors, roi)
                }
            };
            (out.width, out.height, out.data.into_flattened())
        }
        RawPhotometricInterpretation::LinearRaw if d.raw.cpp == 3 => {
            let img = linear_rgb(&d.raw);
            (img.width() as usize, img.height() as usize, img.into_raw())
        }
        other => return Err(RawError::Unsupported(format!("{other:?} with {} cpp", d.raw.cpp))),
    };
    drop(d.raw);

    let halves: Vec<u16> = rgb.par_iter().map(|&v| f16::from_f32(v.clamp(0.0, 65504.0)).to_bits()).collect();
    drop(rgb);
    let img = ImageBuffer::<Rgb<u16>, Vec<u16>>::from_raw(w as u32, h as u32, halves).expect("buffer size matches");
    let img = orient(img, d.orientation);
    Ok(FullImage { width: img.width(), height: img.height(), pixels: img.into_raw(), color: d.color, lens: profile(path) })
}

/// A decoded (scaled, not demosaiced) RAW plus what's needed to develop it.
struct Decoded {
    raw: RawImage,
    orientation: Orientation,
    color: CameraColor,
    metadata: Metadata,
}

impl Decoded {
    fn open(path: &Path) -> Result<Self> {
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
        Ok(Self { raw, orientation, color, metadata })
    }
}

fn transposes(o: Orientation) -> bool {
    matches!(o, Orientation::Transpose | Orientation::Rotate90 | Orientation::Transverse | Orientation::Rotate270)
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
    let (x, y, w, h) = match raw.crop_area.or(raw.active_area) {
        Some(r) => (r.p.x, r.p.y, r.d.w, r.d.h),
        None => (0, 0, raw.width, raw.height),
    };
    clamp_rect((x, y, w, h), raw.width, raw.height)
}

/// `(x, y, w, h)` limited to a `width × height` image (a damaged file can claim anything), and
/// the whole image if nothing would be left.
fn clamp_rect((x, y, w, h): (usize, usize, usize, usize), width: usize, height: usize) -> (usize, usize, usize, usize) {
    let (x, y) = (x.min(width), y.min(height));
    let (w, h) = (w.min(width - x), h.min(height - y));
    if w == 0 || h == 0 {
        (0, 0, width, height)
    } else {
        (x, y, w, h)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_rects_stay_inside_the_sensor() {
        assert_eq!(clamp_rect((10, 20, 100, 50), 200, 100), (10, 20, 100, 50));
        assert_eq!(clamp_rect((150, 80, 100, 50), 200, 100), (150, 80, 50, 20));
        assert_eq!(clamp_rect((500, 0, 10, 10), 200, 100), (0, 0, 200, 100));
        assert_eq!(clamp_rect((0, 0, usize::MAX, usize::MAX), 200, 100), (0, 0, 200, 100));
    }

    fn image(w: u32, h: u32) -> FullImage {
        // Each pixel's red channel encodes x, green encodes y.
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                pixels.extend([f16::from_f32(x as f32).to_bits(), f16::from_f32(y as f32).to_bits(), 0]);
            }
        }
        FullImage {
            width: w,
            height: h,
            pixels,
            color: CameraColor { xyz_to_cam: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], as_shot_wb: [1.0; 3] },
            lens: None,
        }
    }

    #[test]
    fn region_is_clamped_and_exact_at_level_0() {
        let img = image(40, 30);
        let r = img.region(30, 20, 50, 50, 0);
        assert_eq!((r.x, r.y, r.w, r.h, r.width, r.height), (30, 20, 10, 10, 10, 10));
        assert_eq!(f16::from_bits(r.pixels[0]).to_f32(), 30.0);
        assert_eq!(f16::from_bits(r.pixels[1]).to_f32(), 20.0);
    }

    #[test]
    fn region_downsamples_with_aligned_boxes() {
        let img = image(40, 30);
        let r = img.region(5, 5, 20, 20, 1);
        // Aligned down to even coordinates; each sample averages a 2×2 box.
        assert_eq!((r.x, r.y, r.width, r.height), (4, 4, 10, 10));
        assert_eq!(f16::from_bits(r.pixels[0]).to_f32(), 4.5);
    }

    #[test]
    fn scaled_keeps_aspect() {
        let img = image(40, 30);
        let (w, h, px) = img.scaled(8);
        assert_eq!((w, h), (8, 6));
        assert_eq!(px.len(), 8 * 6 * 3);
        assert_eq!(img.scaled(100).0, 40);
    }
}
