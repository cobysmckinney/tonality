//! Turning a photo file into the editor's working image: linear-light RGB
//! at full resolution, the right way up.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use image::metadata::Orientation;
use rawler::decoders::RawDecodeParams;
use rawler::imgop::develop::{Intermediate, ProcessingStep, RawDevelop};

use crate::media;

pub struct LinearImage {
    pub width: u32,
    pub height: u32,
    /// Interleaved linear RGB with sRGB primaries.
    pub pixels: Vec<[f32; 3]>,
    /// True for RAW files: values are proportional to light hitting the
    /// sensor (1.0 is where it clips) and still need a tone curve. False for
    /// already-rendered images such as JPEGs.
    pub scene_referred: bool,
}

pub fn load(path: &Path, is_raw: bool) -> Result<LinearImage> {
    let image = if is_raw { media::decoder_guard(|| load_raw(path)) } else { load_rendered(path) };
    image.with_context(|| format!("opening {} for editing", path.display()))
}

fn load_raw(path: &Path) -> Result<LinearImage> {
    let source = media::open_raw(path)?;
    let decoder = rawler::get_decoder(&source).map_err(|e| anyhow!("{e}"))?;
    let params = RawDecodeParams::default();
    let raw = media::raw_image(decoder.as_ref(), &source, &params)?;
    let orientation = decoder
        .raw_metadata(&source, &params)
        .ok()
        .and_then(|md| md.exif.orientation)
        .unwrap_or_else(|| raw.orientation.to_u16());

    // Everything up to linear sRGB with the camera's white balance; the tone
    // curve is the editor's job.
    let develop = RawDevelop::new_with(&[
        ProcessingStep::Rescale,
        ProcessingStep::Demosaic,
        ProcessingStep::FujiRotate,
        ProcessingStep::CropActiveArea,
        ProcessingStep::WhiteBalance,
        ProcessingStep::Calibrate,
        ProcessingStep::CropDefault,
    ]);
    let (width, height, pixels) = match develop.develop_intermediate(&raw).map_err(|e| anyhow!("{e}"))? {
        Intermediate::ThreeColor(rgb) => (rgb.width, rgb.height, rgb.data),
        Intermediate::Monochrome(gray) => {
            (gray.width, gray.height, gray.data.iter().map(|&v| [v, v, v]).collect())
        }
        Intermediate::FourColor(_) => bail!("four-colour sensors are not supported yet"),
    };
    let (width, height, pixels) = orient(width, height, pixels, orientation);
    Ok(LinearImage { width: width as u32, height: height as u32, pixels, scene_referred: true })
}

fn load_rendered(path: &Path) -> Result<LinearImage> {
    let (mut image, orientation) = media::decode_image(path)?;
    image.apply_orientation(orientation);
    let image = image.into_rgb16();
    // sRGB decoding through a table: one entry per 16-bit code value.
    let table: Vec<f32> = (0..=u16::MAX)
        .map(|code| {
            let v = code as f32 / 65535.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        })
        .collect();
    let pixels = image.pixels().map(|p| p.0.map(|code| table[code as usize])).collect();
    Ok(LinearImage { width: image.width(), height: image.height(), pixels, scene_referred: false })
}

/// Applies an EXIF orientation to a pixel grid.
fn orient<T: Copy>(width: usize, height: usize, pixels: Vec<T>, orientation: u16) -> (usize, usize, Vec<T>) {
    let Some(orientation) = Orientation::from_exif(orientation as u8) else {
        return (width, height, pixels);
    };
    // For each output pixel (x, y), where it comes from in the source.
    let (out_width, out_height) = match orientation {
        Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH => {
            (height, width)
        }
        _ => (width, height),
    };
    let source = |x: usize, y: usize| -> (usize, usize) {
        match orientation {
            Orientation::NoTransforms => (x, y),
            Orientation::FlipHorizontal => (width - 1 - x, y),
            Orientation::Rotate180 => (width - 1 - x, height - 1 - y),
            Orientation::FlipVertical => (x, height - 1 - y),
            Orientation::Rotate90 => (y, height - 1 - x),
            Orientation::Rotate270 => (width - 1 - y, x),
            Orientation::Rotate90FlipH => (y, x),
            Orientation::Rotate270FlipH => (width - 1 - y, height - 1 - x),
        }
    };
    if orientation == Orientation::NoTransforms {
        return (width, height, pixels);
    }
    let mut out = Vec::with_capacity(pixels.len());
    for y in 0..out_height {
        for x in 0..out_width {
            let (sx, sy) = source(x, y);
            out.push(pixels[sy * width + sx]);
        }
    }
    (out_width, out_height, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decoder_that_panics_gives_an_error() {
        let result: Result<()> = media::decoder_guard(|| panic!("index out of bounds"));
        assert_eq!(format!("{:#}", result.unwrap_err()), "the decoder crashed");
        assert_eq!(media::decoder_guard(|| Ok(7)).unwrap(), 7);
    }

    #[test]
    fn orientation_moves_pixels_where_exif_says() {
        // A 3x2 grid numbered row by row:  0 1 2
        //                                  3 4 5
        let grid: Vec<u8> = (0..6).collect();
        assert_eq!(orient(3, 2, grid.clone(), 1), (3, 2, vec![0, 1, 2, 3, 4, 5]));
        assert_eq!(orient(3, 2, grid.clone(), 3), (3, 2, vec![5, 4, 3, 2, 1, 0]));
        // 6 = rotate 90 degrees clockwise to display: the left column becomes the top row.
        assert_eq!(orient(3, 2, grid.clone(), 6), (2, 3, vec![3, 0, 4, 1, 5, 2]));
        // 8 = rotate 90 degrees counter-clockwise.
        assert_eq!(orient(3, 2, grid, 8), (2, 3, vec![2, 5, 1, 4, 0, 3]));
    }
}
