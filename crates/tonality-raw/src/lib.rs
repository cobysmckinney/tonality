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

use std::path::Path;

pub use lens::{lens_profile, lens_profiles};
pub use meta::{read_capture, read_meta};
pub use preview::{decode_full, decode_preview, FullImage, Preview, Region};
pub use thumbnail::thumbnail;

/// File extensions treated as RAW images (lower case).
pub const RAW_EXTENSIONS: &[&str] = &[
    "arw", "cr2", "cr3", "crw", "nef", "nrw", "raf", "orf", "rw2", "pef", "srw", "dng", "3fr",
    "iiq", "erf", "mef", "mos", "mrw", "kdc", "dcr", "x3f",
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
