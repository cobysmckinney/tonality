//! tonality-raw: RAW decoding into camera-space linear pixels.
//!
//! The decoder outputs **camera RGB, black-subtracted and normalised to 0..1, without white
//! balance**. White balance and the camera→working-space matrix are applied on the GPU so
//! the temperature/tint sliders are free to move (see `tonality_core::color`).

mod preview;

use std::path::Path;

pub use preview::{decode_preview, Preview};


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

