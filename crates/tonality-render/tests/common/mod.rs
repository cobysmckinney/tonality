//! Shared by the GPU tests.

use tonality_render::{Gpu, GpuError};

/// The GPU to test on, or `None` to skip the test where there isn't one (a machine or CI runner
/// with no usable driver). With `TONALITY_REQUIRE_GPU` set, having none is a failure instead:
/// CI's GPU job sets it, so that these tests can't pass there by quietly not running.
///
/// Only "nothing to render on" is a skip. A device that won't open, or a develop pipeline that
/// won't build, on a machine that has a GPU is the very thing these tests are for, and fails
/// them wherever they run.
#[allow(dead_code)] // (each test file is its own crate; not all of them use every helper)
pub fn gpu() -> Option<Gpu> {
    match Gpu::new() {
        Ok(gpu) => {
            eprintln!("adapter: {}", gpu.adapter);
            Some(gpu)
        }
        Err(e @ GpuError::NoAdapter(_)) => {
            if std::env::var_os("TONALITY_REQUIRE_GPU").is_some() {
                panic!("TONALITY_REQUIRE_GPU is set, but there is no GPU to test on: {e}")
            }
            eprintln!("skipped: {e}");
            None
        }
        Err(e) => panic!("this machine has a GPU, but the renderer can't start on it: {e}"),
    }
}
