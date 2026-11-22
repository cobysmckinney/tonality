//! tonality-raw: RAW decoding into camera-space linear pixels.
//!
//! The decoder outputs **camera RGB, black-subtracted and normalised to 0..1, without white
//! balance**. White balance and the camera→working-space matrix are applied on the GPU so
//! the temperature/tint sliders are free to move (see `tonality_core::color`).

mod gainmap;
mod lens;
mod meta;
mod preview;
mod thumbnail;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use tonality_core::exif::Capture;
use tonality_core::lens::LensProfile;
use tonality_core::library::ImageMeta;

pub use preview::{FullImage, Preview, Region};

/// File extensions treated as RAW images (lower case).
pub const RAW_EXTENSIONS: &[&str] = &[
    "arw", "cr2", "cr3", "crw", "nef", "nrw", "raf", "orf", "rw2", "pef", "srw", "dng", "3fr",
    "iiq", "erf", "mef", "mos", "mrw", "kdc", "dcr",
];

pub fn is_raw_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| RAW_EXTENSIONS.iter().any(|r| r.eq_ignore_ascii_case(e)))
}

#[derive(Debug, thiserror::Error)]
pub enum RawError {
    #[error("could not decode RAW: {0}")]
    Decode(String),
    #[error("unsupported sensor layout: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, RawError>;

/// Run a decoder call, turning a panic inside it into an error. The decoder reads files the
/// user hands it, and a file it wasn't written for (or a damaged one) shouldn't be able to take
/// the app down with it.
fn guard<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|payload| {
        let what = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        Err(RawError::Decode(format!(
            "the decoder failed on this file ({what})"
        )))
    })
}

/// Camera, lens and exposure details from the file's metadata.
pub fn read_meta(path: &Path) -> Result<ImageMeta> {
    guard(|| meta::read_meta(path))
}

/// What's needed to write the photo's metadata into an export.
pub fn read_capture(path: &Path) -> Result<Capture> {
    guard(|| meta::read_capture(path))
}

/// The photo at preview size, for editing: no demosaic, long edge at most `max_edge`.
pub fn decode_preview(path: &Path, max_edge: usize) -> Result<Preview> {
    guard(|| preview::decode_preview(path, max_edge))
}

/// The photo at full resolution, demosaiced, for 1:1 zoom and export.
pub fn decode_full(path: &Path) -> Result<FullImage> {
    guard(|| preview::decode_full(path))
}

/// A small sRGB picture of the photo for the Library.
pub fn thumbnail(path: &Path, max_edge: u32) -> Result<image::RgbImage> {
    guard(|| thumbnail::thumbnail(path, max_edge))
}

/// The lens profile for this photo: the one embedded in the file, else lensfun's.
pub fn lens_profile(path: &Path, meta: &ImageMeta) -> Option<LensProfile> {
    guard(|| Ok(lens::lens_profile(path, meta))).ok().flatten()
}

/// The embedded and lensfun profiles, separately.
pub fn lens_profiles(path: &Path, meta: &ImageMeta) -> (Option<LensProfile>, Option<LensProfile>) {
    guard(|| Ok(lens::lens_profiles(path, meta))).unwrap_or((None, None))
}

/// Abstraction over RAW decoders so LibRaw can be added later without touching callers.
pub trait RawDecoder {
    fn decode_preview(&self, path: &Path, max_edge: usize) -> Result<Preview>;
}

/// Default decoder backed by `rawler` (pure Rust).
pub struct Rawler;

impl RawDecoder for Rawler {
    fn decode_preview(&self, path: &Path, max_edge: usize) -> Result<Preview> {
        decode_preview(path, max_edge)
    }
}

/// Basic metadata shown in the UI.
#[derive(Debug, Clone)]
pub struct Metadata {
    pub make: String,
    pub model: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigma_x3f_is_not_accepted() {
        // rawler's X3F decoder is unfinished (its metadata call is `todo!()`).
        assert!(!is_raw_path(Path::new("photo.X3F")));
        assert!(is_raw_path(Path::new("photo.ARW")));
    }

    #[test]
    fn a_decoder_panic_becomes_an_error() {
        let r: Result<()> = guard(|| panic!("boom"));
        match r {
            Err(RawError::Decode(m)) => assert!(m.contains("boom"), "{m}"),
            other => panic!("expected a decode error, got {other:?}"),
        }
    }

    #[test]
    fn a_file_that_is_not_a_raw_is_an_error_from_every_entry_point() {
        let path =
            std::env::temp_dir().join(format!("tonality-raw-junk-{}.dng", std::process::id()));
        std::fs::write(&path, b"not a raw file at all").unwrap();
        assert!(read_meta(&path).is_err());
        assert!(read_capture(&path).is_err());
        assert!(decode_preview(&path, 256).is_err());
        assert!(decode_full(&path).is_err());
        assert!(thumbnail(&path, 256).is_err());
        std::fs::remove_file(&path).unwrap();
    }
}
