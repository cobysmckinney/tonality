//! Shared by the GPU tests.

use tonality_render::Gpu;

/// The GPU to test on, or `None` to skip the test where there isn't one (a machine or CI runner
/// with no usable driver). With `TONALITY_REQUIRE_GPU` set, having none is a failure instead:
/// CI's GPU job sets it, so that these tests can't pass there by quietly not running.
pub fn gpu() -> Option<Gpu> {
    match Gpu::new() {
        Ok(gpu) => {
            eprintln!("adapter: {}", gpu.adapter);
            Some(gpu)
        }
        Err(e) if std::env::var_os("TONALITY_REQUIRE_GPU").is_some() => {
            panic!("TONALITY_REQUIRE_GPU is set, but there is no GPU to test on: {e}")
        }
        Err(e) => {
            eprintln!("skipped: {e}");
            None
        }
    }
}
