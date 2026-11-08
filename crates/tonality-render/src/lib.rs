//! tonality-render: the WGSL develop pipeline.
//!
//! - [`DevelopParams::new`] turns an [`EditState`] + per-image colour info into the uniform block.
//! - [`preview_glsl`] transpiles the same WGSL to GLSL ES 3.00 for the WebGL2 preview, so the
//!   preview and the (wgpu) export can never drift apart.

mod glsl;
mod params;

pub use glsl::{preview_glsl, GlslProgram};
pub use params::{curve_lut_rgba16f, DevelopParams};

/// The develop shader source.
pub const DEVELOP_WGSL: &str = include_str!("../shaders/develop.wgsl");
