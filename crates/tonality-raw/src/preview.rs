use std::path::Path;

use half::f16;
use image::{imageops, ImageBuffer, Rgb};
use rawler::decoders::{Orientation, RawDecodeParams};
use rawler::formats::tiff::Rational;
use rawler::imgop::xyz::Illuminant;
use rawler::rawimage::{RawImage, RawImageData, RawPhotometricInterpretation};
use rawler::rawsource::RawSource;
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
    let (_, _, cw, ch) = crop_rect(&d.raw)?;
    let (full_width, full_height) = if transposes(d.orientation) {
        (ch as u32, cw as u32)
    } else {
        (cw as u32, ch as u32)
    };

    let mut img = match &d.raw.photometric {
        RawPhotometricInterpretation::Cfa(cfg) if d.raw.cpp == 1 && cfg.cfa.is_rgb() => {
            superpixel(&d.raw, &cfg.cfa)?
        }
        RawPhotometricInterpretation::LinearRaw if d.raw.cpp == 3 => linear_rgb(&d.raw)?,
        other => {
            return Err(RawError::Unsupported(format!(
                "{other:?} with {} cpp",
                d.raw.cpp
            )))
        }
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
        .map(|&v| f16::from_f32(v.clamp(-65504.0, 65504.0)).to_bits())
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
        self.region_clipped(x, y, w, h, level, None)
    }

    /// [`region`](Self::region), with each sample held to `clip` (a level per channel) before
    /// it is averaged. See [`scaled_clipped`](Self::scaled_clipped).
    fn region_clipped(
        &self,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        level: u32,
        clip: Option<[f32; 3]>,
    ) -> Region {
        let clip = clip.unwrap_or([f32::INFINITY; 3]);
        let f = 1u32 << level.min(6);
        // Align to the downsampling factor so neighbouring requests line up.
        let x0 = (x.min(self.width.saturating_sub(1)) / f) * f;
        let y0 = (y.min(self.height.saturating_sub(1)) / f) * f;
        let x1 = x.saturating_add(w).min(self.width);
        let y1 = y.saturating_add(h).min(self.height);
        let (ow, oh) = (
            (x1.saturating_sub(x0) / f).max(1),
            (y1.saturating_sub(y0) / f).max(1),
        );
        let stride = self.width as usize * 3;
        let mut out = vec![0u16; ow as usize * oh as usize * 3];
        out.par_chunks_mut(ow as usize * 3)
            .enumerate()
            .for_each(|(oy, row)| {
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
                                *a += f16::from_bits(self.pixels[i + c]).to_f32().min(clip[c]);
                            }
                        }
                    }
                    for c in 0..3 {
                        row[ox * 3 + c] = f16::from_f32(acc[c] / n).to_bits();
                    }
                }
            });
        Region {
            x: x0,
            y: y0,
            w: ow * f,
            h: oh * f,
            width: ow,
            height: oh,
            pixels: out,
        }
    }
}

impl FullImage {
    /// A copy scaled down so its long edge is `long_edge` (never up). Returns `(w, h, pixels)`.
    pub fn scaled(&self, long_edge: u32) -> (u32, u32, Vec<u16>) {
        self.scaled_clipped(long_edge, None)
    }

    /// [`scaled`](Self::scaled) for developing with a known white balance: `clip` is the level,
    /// per channel of these camera-space pixels, at which white balance takes that channel to
    /// the develop shader's highlight clip (`clip level ÷ multiplier`).
    ///
    /// The shader keeps blown highlights neutral by clipping each channel *after* white
    /// balance. Averaging happens here, before it, so a pixel on the edge of a blown area is
    /// part sensor-clipped and part dark: it no longer reaches the clip, and white balance
    /// then tints it magenta, an outline as wide as an output pixel around every blown
    /// highlight. Holding each sample to the clip level first makes the average the one the
    /// shader would have got from clipped pixels: neutral.
    pub fn scaled_clipped(&self, long_edge: u32, clip: Option<[f32; 3]>) -> (u32, u32, Vec<u16>) {
        let long = self.width.max(self.height);
        if long_edge >= long {
            return (self.width, self.height, self.pixels.clone());
        }
        let ratio = long as f32 / long_edge as f32;
        // Cheap power-of-two box reduction first, leaving at least 2× for the proper filter.
        let level = if ratio >= 4.0 {
            (ratio / 2.0).log2().floor() as u32
        } else {
            0
        };
        let s = long_edge as f32 / long as f32;
        let w = ((self.width as f32 * s).round() as u32).max(1);
        let h = ((self.height as f32 * s).round() as u32).max(1);
        let pixels = if level == 0 {
            resample(
                &self.pixels,
                self.width as usize,
                self.height as usize,
                w as usize,
                h as usize,
                clip,
            )
        } else {
            let r = self.region_clipped(0, 0, self.width, self.height, level, clip);
            resample(
                &r.pixels,
                r.width as usize,
                r.height as usize,
                w as usize,
                h as usize,
                clip,
            )
        };
        (w, h, pixels)
    }
}

/// Catmull-Rom weights for resampling `src` samples to `dst`: per output sample, the first
/// source index and normalised weights (the kernel widened by the ratio when shrinking).
fn weights(src: usize, dst: usize) -> Vec<(usize, Vec<f32>)> {
    let ratio = src as f32 / dst as f32;
    let scale = ratio.max(1.0);
    let support = 2.0 * scale;
    let cubic = |x: f32| {
        let x = x.abs();
        if x < 1.0 {
            1.5 * x * x * x - 2.5 * x * x + 1.0
        } else if x < 2.0 {
            -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
        } else {
            0.0
        }
    };
    (0..dst)
        .map(|i| {
            let center = (i as f32 + 0.5) * ratio;
            let lo = ((center - support).floor().max(0.0)) as usize;
            let hi = ((center + support).ceil() as usize).min(src);
            let mut w: Vec<f32> = (lo..hi)
                .map(|j| cubic((j as f32 + 0.5 - center) / scale))
                .collect();
            let sum: f32 = w.iter().sum();
            if sum.abs() > 1e-8 {
                w.iter_mut().for_each(|v| *v /= sum);
            }
            (lo, w)
        })
        .collect()
}

/// Resize RGB halves `sw × sh` → `dw × dh` (separable Catmull-Rom, rows in parallel). With
/// `clip`, each source sample is first held to that level for its channel, and so is the
/// result (the filter's negative lobes can overshoot what went in).
fn resample(
    src: &[u16],
    sw: usize,
    sh: usize,
    dw: usize,
    dh: usize,
    clip: Option<[f32; 3]>,
) -> Vec<u16> {
    let clip = clip.unwrap_or([f32::INFINITY; 3]);
    let (wx, wy) = (weights(sw, dw), weights(sh, dh));
    // Horizontal: every source row to `dw` samples.
    let mut tmp = vec![0f32; dw * sh * 3];
    tmp.par_chunks_mut(dw * 3).enumerate().for_each(|(y, row)| {
        let line = &src[y * sw * 3..(y + 1) * sw * 3];
        for (o, (lo, w)) in row.as_chunks_mut::<3>().0.iter_mut().zip(&wx) {
            let mut acc = [0f32; 3];
            for (k, &wt) in w.iter().enumerate() {
                let i = (lo + k) * 3;
                for c in 0..3 {
                    acc[c] += wt * f16::from_bits(line[i + c]).to_f32().min(clip[c]);
                }
            }
            *o = acc;
        }
    });
    // Vertical.
    let mut out = vec![0u16; dw * dh * 3];
    out.par_chunks_mut(dw * 3).enumerate().for_each(|(y, row)| {
        let (lo, w) = &wy[y];
        let mut acc = vec![0f32; dw * 3];
        for (k, &wt) in w.iter().enumerate() {
            let line = &tmp[(lo + k) * dw * 3..][..dw * 3];
            for (a, &v) in acc.iter_mut().zip(line) {
                *a += wt * v;
            }
        }
        for (i, (o, a)) in row.iter_mut().zip(acc).enumerate() {
            *o = f16::from_f32(a.clamp(-65504.0, 65504.0).min(clip[i % 3])).to_bits();
        }
    });
    out
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
    let (cx, cy, cw, ch) = crop_rect(&d.raw)?;
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
                SensorType::Bayer => {
                    PPGDemosaic::new().demosaic(&pixels, &cfg.cfa, &cfg.colors, roi)
                }
                SensorType::Xtrans => XTransMarkesteijnDemosaic::new_pass_1().demosaic(
                    &pixels,
                    &cfg.cfa,
                    &cfg.colors,
                    roi,
                ),
            };
            (out.width, out.height, out.data.into_flattened())
        }
        RawPhotometricInterpretation::LinearRaw if d.raw.cpp == 3 => {
            let img = linear_rgb(&d.raw)?;
            (img.width() as usize, img.height() as usize, img.into_raw())
        }
        other => {
            return Err(RawError::Unsupported(format!(
                "{other:?} with {} cpp",
                d.raw.cpp
            )))
        }
    };
    drop(d.raw);

    let halves: Vec<u16> = rgb
        .par_iter()
        .map(|&v| f16::from_f32(v.clamp(-65504.0, 65504.0)).to_bits())
        .collect();
    drop(rgb);
    let img = ImageBuffer::<Rgb<u16>, Vec<u16>>::from_raw(w as u32, h as u32, halves)
        .expect("buffer size matches");
    let img = orient(img, d.orientation);
    Ok(FullImage {
        width: img.width(),
        height: img.height(),
        pixels: img.into_raw(),
        color: d.color,
        lens: profile(path),
    })
}

/// Fujifilm's Super CCD cameras (S2/S3/S5 Pro, F700, S100FS and a dozen more) have a sensor
/// turned 45°. The decoder hands their data over still laid out as a diamond, to be turned back
/// after demosaicing, and marks it with `fuji_rotation_width`. Nothing here does that turn yet,
/// so such a file would open as the scene rotated 45° on a black square, cropped in the wrong
/// place: an error says so instead.
fn fuji_diagonal_unsupported(raw: &RawImage) -> Result<()> {
    if raw.fuji_rotation_width.is_some() {
        return Err(RawError::Unsupported(format!(
            "the {} {} has a Fujifilm Super CCD sensor (laid out diagonally), which tonality \
             can't develop yet",
            raw.clean_make, raw.clean_model
        )));
    }
    Ok(())
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
        fuji_diagonal_unsupported(&raw)?;
        scale(&mut raw);
        apply_gain_maps(&mut raw, &crate::gainmap::read(path));
        // Not every decoder fills `RawImage::orientation` (e.g. Sony ARW), so prefer EXIF.
        let orientation = exif_orientation(decoder.as_ref(), &src).unwrap_or(raw.orientation);
        let color = camera_color(&raw);
        let metadata = Metadata {
            make: raw.clean_make.clone(),
            model: raw.clean_model.clone(),
        };
        Ok(Self {
            raw,
            orientation,
            color,
            metadata,
        })
    }
}

/// Apply DNG `GainMap` opcodes (OpcodeList2) to the scaled sensor data. They work on the active
/// area; rawler keeps the whole sensor, so they are offset by the active area's origin.
fn apply_gain_maps(raw: &mut RawImage, maps: &[crate::gainmap::GainMap]) {
    if maps.is_empty() {
        return;
    }
    let active = match raw.active_area {
        Some(r) => clamp_rect((r.p.x, r.p.y, r.d.w, r.d.h), raw.width, raw.height),
        None => (0, 0, raw.width, raw.height),
    };
    let (width, cpp) = (raw.width, raw.cpp);
    if let RawImageData::Float(px) = &mut raw.data {
        for m in maps {
            m.apply(px, width, cpp, active);
        }
    }
}

/// Black-subtract and normalise to 0..1 (white), like rawler's `apply_scaling` but keeping values
/// below black: sensor noise around black is symmetric, and clipping it here (before the
/// demosaic, downsampling and noise reduction average it) lifts and tints the shadows. The
/// shader clips black after noise reduction.
fn scale(raw: &mut RawImage) {
    let mut px = raw.data.as_f32().into_owned();
    match &raw.photometric {
        RawPhotometricInterpretation::Cfa(_) => {
            // Levels per position in the 2×2 CFA block: (0,0), (1,0), (0,1), (1,1).
            let black = raw.blacklevel.as_bayer_array();
            let white = raw.whitelevel.as_bayer_array();
            let range = [0, 1, 2, 3].map(|i| (white[i] - black[i]).max(1.0));
            let w = raw.width.max(1);
            px.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
                for (x, v) in row.iter_mut().enumerate() {
                    let i = (y % 2) * 2 + x % 2;
                    *v = (*v - black[i]) / range[i];
                }
            });
        }
        _ => {
            let black = raw.blacklevel.as_vec();
            let white = raw.whitelevel.as_vec();
            let ch = black.len().max(1);
            if white.len() == ch {
                let range: Vec<f32> = (0..ch).map(|i| (white[i] - black[i]).max(1.0)).collect();
                px.par_chunks_mut(ch).for_each(|b| {
                    for (i, v) in b.iter_mut().enumerate() {
                        *v = (*v - black[i]) / range[i];
                    }
                });
            }
        }
    }
    if std::env::var("TONALITY_DEBUG_BLACK").is_ok() {
        let neg = px.iter().filter(|v| **v < 0.0).count();
        let min = px.iter().copied().fold(f32::INFINITY, f32::min);
        eprintln!(
            "[scale] black {:?} white {:?} negatives {:.3}% min {min}",
            raw.blacklevel.as_vec(),
            raw.whitelevel.as_vec(),
            100.0 * neg as f64 / px.len() as f64
        );
    }
    raw.data = RawImageData::Float(px);
    raw.blacklevel
        .levels
        .iter_mut()
        .for_each(|x| *x = Rational::new(0, 1));
    raw.whitelevel.0.iter_mut().for_each(|x| *x = 1);
}

fn transposes(o: Orientation) -> bool {
    matches!(
        o,
        Orientation::Transpose
            | Orientation::Rotate90
            | Orientation::Transverse
            | Orientation::Rotate270
    )
}

pub(crate) fn exif_orientation(
    decoder: &dyn rawler::decoders::Decoder,
    src: &RawSource,
) -> Option<Orientation> {
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
        // `scale` always produces Float.
        RawImageData::Integer(_) => unreachable!("scale converts to float"),
    }
}

/// Smallest crop (each side) that is demosaiced: a damaged file can claim a 1-pixel crop, which
/// gives no CFA superpixels and is below what PPG and Markesteijn handle.
const MIN_CROP: usize = 16;

/// (x, y, w, h) of the recommended crop within the full sensor image, or an error if the sensor
/// itself is too small to develop.
fn crop_rect(raw: &RawImage) -> Result<(usize, usize, usize, usize)> {
    let (x, y, w, h) = match raw.crop_area.or(raw.active_area) {
        Some(r) => (r.p.x, r.p.y, r.d.w, r.d.h),
        None => (0, 0, raw.width, raw.height),
    };
    let r = clamp_rect((x, y, w, h), raw.width, raw.height);
    if r.2 < MIN_CROP || r.3 < MIN_CROP {
        return Err(RawError::Decode(format!(
            "image too small: {}×{}",
            raw.width, raw.height
        )));
    }
    Ok(r)
}

/// `(x, y, w, h)` limited to a `width × height` image (a damaged file can claim anything), and
/// the whole image if less than [`MIN_CROP`] would be left on either side.
fn clamp_rect(
    (x, y, w, h): (usize, usize, usize, usize),
    width: usize,
    height: usize,
) -> (usize, usize, usize, usize) {
    let (x, y) = (x.min(width), y.min(height));
    let (w, h) = (w.min(width - x), h.min(height - y));
    if w < MIN_CROP || h < MIN_CROP {
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
    let (cx, cy, cw, ch) = crop_rect(raw)?;
    let (ow, oh) = (cw / block, ch / block);
    let mut out = vec![0f32; ow * oh * 3];

    out.par_chunks_mut(ow * 3)
        .enumerate()
        .for_each(|(oy, row)| {
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

fn linear_rgb(raw: &RawImage) -> Result<RgbF32> {
    let data = float_data(raw);
    let (cx, cy, cw, ch) = crop_rect(raw)?;
    let mut out = Vec::with_capacity(cw * ch * 3);
    for y in cy..cy + ch {
        let start = (y * raw.width + cx) * 3;
        out.extend_from_slice(&data[start..start + cw * 3]);
    }
    Ok(RgbF32::from_raw(cw as u32, ch as u32, out).expect("buffer size matches"))
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
        Some((_, m)) if m.len() >= 9 => {
            [[m[0], m[1], m[2]], [m[3], m[4], m[5]], [m[6], m[7], m[8]]]
        }
        _ => [raw.xyz_to_cam[0], raw.xyz_to_cam[1], raw.xyz_to_cam[2]],
    };
    let wb = if raw.wb_coeffs[..3].iter().all(|v| v.is_finite() && *v > 0.0) {
        [raw.wb_coeffs[0], raw.wb_coeffs[1], raw.wb_coeffs[2]]
    } else {
        let n = raw.neutralwb();
        [n[0], n[1], n[2]]
    };
    CameraColor {
        xyz_to_cam,
        as_shot_wb: wb,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_matches_image_crate_catmull_rom() {
        let (sw, sh, dw, dh) = (97usize, 61usize, 40usize, 25usize);
        let f: Vec<f32> = (0..sw * sh * 3)
            .map(|i| ((i * 7919) % 1000) as f32 / 1000.0)
            .collect();
        let halves: Vec<u16> = f.iter().map(|&v| f16::from_f32(v).to_bits()).collect();
        let ours = resample(&halves, sw, sh, dw, dh, None);
        let img = RgbF32::from_raw(
            sw as u32,
            sh as u32,
            halves.iter().map(|&v| f16::from_bits(v).to_f32()).collect(),
        )
        .unwrap();
        let theirs = imageops::resize(&img, dw as u32, dh as u32, imageops::FilterType::CatmullRom);
        let max = ours
            .iter()
            .zip(theirs.as_raw())
            .map(|(&a, &b)| (f16::from_bits(a).to_f32() - b.clamp(-65504.0, 65504.0)).abs())
            .fold(0f32, f32::max);
        assert!(max < 0.02, "max difference {max}");
        // Flat stays flat.
        let flat = vec![f16::from_f32(0.5).to_bits(); sw * sh * 3];
        assert!(resample(&flat, sw, sh, dw, dh, None)
            .iter()
            .all(|&v| (f16::from_bits(v).to_f32() - 0.5).abs() < 1e-3));
    }

    #[test]
    fn crop_rects_stay_inside_the_sensor() {
        assert_eq!(clamp_rect((10, 20, 100, 50), 200, 100), (10, 20, 100, 50));
        assert_eq!(clamp_rect((150, 80, 100, 50), 200, 100), (150, 80, 50, 20));
        assert_eq!(clamp_rect((500, 0, 10, 10), 200, 100), (0, 0, 200, 100));
        // Slivers (here 1 pixel wide at the right edge) fall back to the whole sensor.
        assert_eq!(clamp_rect((199, 0, 10, 100), 200, 100), (0, 0, 200, 100));
        assert_eq!(clamp_rect((0, 90, 200, 50), 200, 100), (0, 0, 200, 100));
        assert_eq!(
            clamp_rect((0, 0, usize::MAX, usize::MAX), 200, 100),
            (0, 0, 200, 100)
        );
    }

    /// A flat RGGB sensor `w × h` with the given crop.
    fn bayer(w: usize, h: usize, crop: (usize, usize, usize, usize)) -> RawImage {
        use rawler::decoders::Camera;
        use rawler::imgop::sensor::SensorType;
        use rawler::imgop::{Dim2, Point, Rect};
        use rawler::pixarray::PixU16;
        use rawler::rawimage::CFAConfig;
        use rawler::CFA;

        let cfa = CFA::new("RGGB");
        let cam = Camera {
            cfa: cfa.clone(),
            ..Default::default()
        };
        let mut raw = RawImage::new(
            cam,
            PixU16::new_with(vec![0; w * h], w, h),
            1,
            [1.0; 4],
            RawPhotometricInterpretation::Cfa(CFAConfig {
                colors: rawler::cfa::PlaneColor::new("RGB"),
                cfa,
                sensor: SensorType::Bayer,
            }),
            None,
            Some(rawler::rawimage::WhiteLevel::new(vec![1])),
            false,
        );
        raw.data = RawImageData::Float(vec![0.5; w * h]);
        raw.crop_area = Some(Rect::new(
            Point::new(crop.0, crop.1),
            Dim2::new(crop.2, crop.3),
        ));
        raw
    }

    /// The value of sensor pixel `(x, y)` of a `w`-wide CFA image.
    fn sensor(raw: &RawImage, x: usize, y: usize) -> f32 {
        raw.data.as_f32()[y * raw.width + x]
    }

    #[test]
    fn scaling_uses_each_cfa_positions_own_black_level_and_keeps_values_below_it() {
        use rawler::rawimage::{BlackLevel, WhiteLevel};
        let mut raw = bayer(4, 4, (0, 0, 4, 4));
        // One black level per position of the 2 × 2 block, in the order (0,0), (1,0), (0,1),
        // (1,1), and one white level for all.
        raw.blacklevel = BlackLevel::new(&[64u16, 80, 96, 112], 2, 2, 1);
        raw.whitelevel = WhiteLevel::new(vec![1088u32]);
        // Each pixel at its own black level plus a quarter of its range, but for two: one at
        // white, and one below black (noise around black goes both ways).
        let black = |x: usize, y: usize| [64.0f32, 80.0, 96.0, 112.0][(y % 2) * 2 + x % 2];
        let mut data: Vec<f32> = (0..16)
            .map(|i| {
                let b = black(i % 4, i / 4);
                b + (1088.0 - b) / 4.0
            })
            .collect();
        data[4 + 3] = 1088.0; // (3, 1): position (1, 1), at white
        data[2 * 4] = 64.0 - 16.0; // (0, 2): position (0, 0), 16 below its black
        raw.data = RawImageData::Float(data);

        scale(&mut raw);

        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1), (2, 3), (3, 2)] {
            let v = sensor(&raw, x, y);
            assert!((v - 0.25).abs() < 1e-6, "({x}, {y}) is {v}");
        }
        assert!((sensor(&raw, 3, 1) - 1.0).abs() < 1e-6, "white is 1");
        let below = sensor(&raw, 0, 2);
        assert!(
            (below - (-16.0 / 1024.0)).abs() < 1e-6,
            "below black stays below zero: {below}"
        );
        // The levels now say what the data is: black 0, white 1.
        assert!(raw.blacklevel.levels.iter().all(|l| l.as_f32() == 0.0));
        assert_eq!(raw.whitelevel.0, [1]);
    }

    #[test]
    fn scaling_handles_images_that_are_not_a_cfa() {
        use rawler::rawimage::{BlackLevel, WhiteLevel};
        // Linear RGB (a DNG from a scanner or a phone): three samples per pixel, a level each.
        let mut raw = bayer(2, 1, (0, 0, 2, 1));
        raw.photometric = RawPhotometricInterpretation::LinearRaw;
        raw.cpp = 3;
        raw.blacklevel = BlackLevel::new(&[100u16, 200, 300], 1, 1, 3);
        raw.whitelevel = WhiteLevel::new(vec![1100u32, 1200, 1300]);
        raw.data = RawImageData::Float(vec![600.0, 700.0, 800.0, 1100.0, 200.0, 250.0]);
        scale(&mut raw);
        let px = raw.data.as_f32();
        for (got, want) in px.iter().zip([0.5, 0.5, 0.5, 1.0, 0.0, -0.05]) {
            assert!((got - want).abs() < 1e-6, "{px:?}");
        }
    }

    #[test]
    fn every_orientation_puts_each_pixel_where_exif_says() {
        // A 3 × 2 picture whose pixels are numbered row by row:  0 1 2
        //                                                         3 4 5
        let picture =
            || ImageBuffer::<image::Luma<u8>, Vec<u8>>::from_raw(3, 2, (0..6).collect()).unwrap();
        // What each EXIF orientation shows, row by row, once the picture is turned upright.
        let cases: [(Orientation, (u32, u32), &[u8]); 9] = [
            (Orientation::Normal, (3, 2), &[0, 1, 2, 3, 4, 5]),
            (Orientation::Unknown, (3, 2), &[0, 1, 2, 3, 4, 5]),
            (Orientation::HorizontalFlip, (3, 2), &[2, 1, 0, 5, 4, 3]),
            (Orientation::Rotate180, (3, 2), &[5, 4, 3, 2, 1, 0]),
            (Orientation::VerticalFlip, (3, 2), &[3, 4, 5, 0, 1, 2]),
            // Mirrored across the diagonal from top-left: rows become columns.
            (Orientation::Transpose, (2, 3), &[0, 3, 1, 4, 2, 5]),
            // Turned a quarter clockwise: the left column becomes the top row, bottom first.
            (Orientation::Rotate90, (2, 3), &[3, 0, 4, 1, 5, 2]),
            // Mirrored across the other diagonal.
            (Orientation::Transverse, (2, 3), &[5, 2, 4, 1, 3, 0]),
            // Turned a quarter anticlockwise.
            (Orientation::Rotate270, (2, 3), &[2, 5, 1, 4, 0, 3]),
        ];
        for (o, size, pixels) in cases {
            let out = orient(picture(), o);
            assert_eq!(out.dimensions(), size, "{o:?}");
            assert_eq!(out.as_raw().as_slice(), pixels, "{o:?}");
            // `transposes` is what sizes the full image before it is decoded: it has to say
            // "sideways" for exactly the orientations that come out sideways.
            assert_eq!(transposes(o), size == (2, 3), "{o:?}");
        }
    }

    #[test]
    fn camera_colour_prefers_a_daylight_matrix_and_survives_a_missing_white_balance() {
        let d65 = vec![0.9f32, 0.1, 0.0, 0.1, 0.8, 0.1, 0.0, 0.2, 0.7];
        let tungsten = vec![0.5f32; 9];
        let as_rows = |m: &[f32]| [[m[0], m[1], m[2]], [m[3], m[4], m[5]], [m[6], m[7], m[8]]];

        let mut raw = bayer(2, 2, (0, 0, 2, 2));
        raw.wb_coeffs = [2.1, 1.0, 1.6, f32::NAN];
        raw.color_matrix.insert(Illuminant::A, tungsten.clone());
        raw.color_matrix.insert(Illuminant::D65, d65.clone());
        let c = camera_color(&raw);
        assert_eq!(c.xyz_to_cam, as_rows(&d65), "D65 before tungsten");
        assert_eq!(
            c.as_shot_wb,
            [2.1, 1.0, 1.6],
            "the fourth (emerald) isn't needed"
        );

        // Only a tungsten matrix: better than none.
        raw.color_matrix.remove(&Illuminant::D65);
        assert_eq!(camera_color(&raw).xyz_to_cam, as_rows(&tungsten));
        // No matrix for any illuminant: the decoder's own.
        raw.color_matrix.clear();
        raw.xyz_to_cam = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]];
        assert_eq!(
            camera_color(&raw).xyz_to_cam,
            tonality_core::color::IDENTITY
        );

        // A white balance the camera didn't record (zero, NaN, negative) can't be multiplied
        // by: fall back to the one that makes the matrix's white neutral.
        let neutral = raw.neutralwb();
        for bad in [
            [0.0, 1.0, 1.5, 0.0],
            [f32::NAN, 1.0, 1.5, 0.0],
            [2.0, 1.0, -1.0, 0.0],
        ] {
            raw.wb_coeffs = bad;
            let wb = camera_color(&raw).as_shot_wb;
            assert_eq!(wb, [neutral[0], neutral[1], neutral[2]], "{bad:?}");
            assert!(wb.iter().all(|v| v.is_finite() && *v > 0.0), "{wb:?}");
        }
    }

    #[test]
    fn gain_maps_start_at_the_active_area_not_the_sensors_corner() {
        use rawler::imgop::{Dim2, Point, Rect};
        // A map that doubles its top-left 2 × 2 pixels. DNG opcodes count from the active
        // area; rawler keeps the whole sensor, masked border included.
        let double = crate::gainmap::GainMap {
            top: 0,
            left: 0,
            bottom: 2,
            right: 2,
            plane: 0,
            planes: 1,
            row_pitch: 1,
            col_pitch: 1,
            points_v: 1,
            points_h: 1,
            spacing_v: 1.0,
            spacing_h: 1.0,
            origin_v: 0.0,
            origin_h: 0.0,
            map_planes: 1,
            gains: vec![2.0],
        };
        let doubled = |raw: &RawImage| -> Vec<(usize, usize)> {
            (0..raw.height)
                .flat_map(|y| (0..raw.width).map(move |x| (x, y)))
                .filter(|&(x, y)| sensor(raw, x, y) == 1.0)
                .collect()
        };
        // The active area starts 4 across and 2 down.
        let mut raw = bayer(32, 24, (0, 0, 32, 24));
        raw.active_area = Some(Rect::new(Point::new(4, 2), Dim2::new(24, 20)));
        apply_gain_maps(&mut raw, std::slice::from_ref(&double));
        assert_eq!(doubled(&raw), [(4, 2), (5, 2), (4, 3), (5, 3)]);
        // With none recorded, the whole sensor is the active area.
        let mut raw = bayer(32, 24, (0, 0, 32, 24));
        apply_gain_maps(&mut raw, std::slice::from_ref(&double));
        assert_eq!(doubled(&raw), [(0, 0), (1, 0), (0, 1), (1, 1)]);
        // One too small to be believed (damaged metadata) is treated the same way.
        let mut raw = bayer(32, 24, (0, 0, 32, 24));
        raw.active_area = Some(Rect::new(Point::new(4, 2), Dim2::new(1, 1)));
        apply_gain_maps(&mut raw, std::slice::from_ref(&double));
        assert_eq!(doubled(&raw), [(0, 0), (1, 0), (0, 1), (1, 1)]);
        // No maps: nothing changes.
        let mut raw = bayer(32, 24, (0, 0, 32, 24));
        apply_gain_maps(&mut raw, &[]);
        assert!(doubled(&raw).is_empty());
    }

    #[test]
    fn a_one_pixel_crop_develops_the_whole_sensor() {
        let raw = bayer(32, 24, (31, 0, 1, 24));
        let RawPhotometricInterpretation::Cfa(cfg) = &raw.photometric else {
            unreachable!()
        };
        let img = superpixel(&raw, &cfg.cfa).unwrap();
        assert_eq!(img.dimensions(), (16, 12));
        assert!(img.as_raw().iter().all(|&v| v == 0.5));
    }

    #[test]
    fn a_tiny_sensor_is_an_error_not_a_panic() {
        let raw = bayer(1, 8, (0, 0, 1, 8));
        let RawPhotometricInterpretation::Cfa(cfg) = &raw.photometric else {
            unreachable!()
        };
        assert!(matches!(
            superpixel(&raw, &cfg.cfa),
            Err(RawError::Decode(_))
        ));
        assert!(crop_rect(&raw).is_err());
    }

    #[test]
    fn a_diagonal_fuji_sensor_is_refused_with_a_reason() {
        let mut raw = bayer(8, 8, (0, 0, 8, 8));
        assert!(fuji_diagonal_unsupported(&raw).is_ok());
        // As rawler's RAF decoder marks an S5 Pro file.
        raw.fuji_rotation_width = Some(6);
        raw.clean_make = "Fujifilm".into();
        raw.clean_model = "FinePix S5 Pro".into();
        match fuji_diagonal_unsupported(&raw) {
            Err(RawError::Unsupported(why)) => {
                assert!(
                    why.contains("FinePix S5 Pro") && why.contains("Super CCD"),
                    "{why}"
                )
            }
            other => panic!("{other:?}"),
        }
    }

    fn image(w: u32, h: u32) -> FullImage {
        // Each pixel's red channel encodes x, green encodes y.
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                pixels.extend([
                    f16::from_f32(x as f32).to_bits(),
                    f16::from_f32(y as f32).to_bits(),
                    0,
                ]);
            }
        }
        FullImage {
            width: w,
            height: h,
            pixels,
            color: CameraColor {
                xyz_to_cam: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                as_shot_wb: [1.0; 3],
            },
            lens: None,
        }
    }

    #[test]
    fn region_is_clamped_and_exact_at_level_0() {
        let img = image(40, 30);
        let r = img.region(30, 20, 50, 50, 0);
        assert_eq!(
            (r.x, r.y, r.w, r.h, r.width, r.height),
            (30, 20, 10, 10, 10, 10)
        );
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
    fn the_edge_of_a_blown_highlight_stays_neutral_when_scaled_down() {
        // Daylight multipliers: red and blue are raised, so a sensor-clipped (1, 1, 1) becomes
        // (2, 1, 1.5), which the shader clips back to white.
        let wb = [2.0f32, 1.0, 1.5];
        let developed = |p: &[u16]| -> [f32; 3] {
            std::array::from_fn(|c| (f16::from_bits(p[c]).to_f32() * wb[c]).min(1.0))
        };
        // A blown sky on the left; on the right a dark branch, neutral once balanced.
        let (w, h) = (240u32, 60u32);
        let sky = [1.0f32, 1.0, 1.0];
        let branch = [0.02f32, 0.04, 0.04 / 1.5];
        let mut img = image(w, h);
        img.pixels = (0..w * h)
            .flat_map(|i| if i % w < 121 { sky } else { branch })
            .map(|v| f16::from_f32(v).to_bits())
            .collect();
        let clip = wb.map(|m| 1.0 / m);

        // Both a plain reduction (a third) and one that box-averages first (a sixth).
        for edge in [80, 40] {
            let tint = |px: &[u16]| -> f32 {
                px.as_chunks::<3>()
                    .0
                    .iter()
                    .map(|p| {
                        let [r, g, b] = developed(p);
                        (r - g).abs().max((b - g).abs())
                    })
                    .fold(0.0, f32::max)
            };
            let (_, _, plain) = img.scaled(edge);
            let (_, _, clipped) = img.scaled_clipped(edge, Some(clip));
            assert!(
                tint(&plain) > 0.2,
                "at {edge}: averaged as it is, the edge pixel is tinted ({})",
                tint(&plain)
            );
            assert!(
                tint(&clipped) < 0.02,
                "at {edge}: clipped first, it is neutral ({})",
                tint(&clipped)
            );
        }
        // Away from any highlight nothing changes: the clip only touches what's above it.
        let mut dark = image(w, h);
        dark.pixels = vec![f16::from_f32(0.1).to_bits(); (w * h * 3) as usize];
        assert_eq!(dark.scaled(80), dark.scaled_clipped(80, Some(clip)));
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
