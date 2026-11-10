//! On-device models for AI masks (subject, sky, people, select-object).
//!
//! Everything runs locally on the CPU; photos never leave the computer. Model files are
//! downloaded on first use (see [`registry`]).

pub mod download;
pub mod input;
pub mod refine;
pub mod registry;
pub mod sam;
pub mod scene;
pub mod session;
pub mod subject;

#[derive(Debug, thiserror::Error)]
pub enum MlError {
    #[error("model download failed: {0}")]
    Download(String),
    #[error("inference failed: {0}")]
    Inference(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, MlError>;
