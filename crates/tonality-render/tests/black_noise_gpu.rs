//! Noise around black must average out, not lift the shadows: noise reduction sees the values
//! below black (sensor noise is symmetric), and black is clipped only afterwards.
//! Skipped (passes with a note) on machines without a usable adapter, unless
//! `TONALITY_REQUIRE_GPU` is set.

mod common;

use half::f16;
use tonality_core::color::CameraColor;
use tonality_core::mask::brush_layers;
use tonality_core::EditState;
use tonality_render::{curve_lut_rgba16f, DevelopParams, Gpu, Rendered, Source};

const W: u32 = 96;
const H: u32 = 96;
const LEVEL: f32 = 0.002;
const SIGMA: f32 = 0.004;

/// Gaussian noise from a hash: deterministic, independent per pixel and channel.
fn gauss(i: u32) -> f32 {
    let h = |mut x: u32| {
        x ^= x >> 16;
        x = x.wrapping_mul(0x7feb_352d);
        x ^= x >> 15;
        x = x.wrapping_mul(0x846c_a68b);
        x ^= x >> 16;
        (x as f32 + 0.5) / 4_294_967_296.0
    };
    let (u, v) = (h(2 * i), h(2 * i + 1));
    (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
}

/// `clip`: clip at black before the pipeline sees the data (what the decoder used to do).
fn render(gpu: &Gpu, sigma: f32, clip: bool) -> Rendered {
    let mut edit = EditState::default();
    edit.light.exposure = 5.0; // shadows pushed hard, as a lifted-shadow edit would
    edit.detail.luminance_noise = 100.0;
    edit.detail.color_noise = 100.0;
    let color = CameraColor {
        xyz_to_cam: tonality_core::color::IDENTITY,
        as_shot_wb: [1.0; 3],
    };
    let mut params = DevelopParams::new(&edit, &color, None, W, H);
    params.wb = [1.0, 1.0, 1.0, 1.0];
    params.cam = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    let pixels: Vec<u16> = (0..W * H * 3)
        .map(|i| {
            let v = LEVEL + sigma * gauss(i);
            f16::from_f32(if clip { v.max(0.0) } else { v }).to_bits()
        })
        .collect();
    let src = Source {
        width: W,
        height: H,
        pixels: &pixels,
        frame: [W, H],
    };
    gpu.render(
        &src,
        &params,
        &curve_lut_rgba16f(&edit),
        &brush_layers(&[], W, H),
        None,
        |_| {},
    )
    .unwrap()
}

/// Mean display-linear value of the interior (away from the clamped borders), per channel.
fn mean(r: &Rendered) -> [f64; 3] {
    let lin = |v: u16| {
        let c = v as f64 / 65535.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let mut acc = [0.0; 3];
    let mut n = 0.0;
    for y in 8..H - 8 {
        for x in 8..W - 8 {
            let i = ((y * W + x) * 3) as usize;
            for (k, a) in acc.iter_mut().enumerate() {
                *a += lin(r.pixels[i + k]);
            }
            n += 1.0;
        }
    }
    acc.map(|a| a / n)
}

#[test]
fn noise_around_black_averages_out() {
    let Some(gpu) = common::gpu() else { return };
    let clean = mean(&render(&gpu, 0.0, false));
    let noisy = mean(&render(&gpu, SIGMA, false));
    let clipped = mean(&render(&gpu, SIGMA, true));
    // σ is twice the level, so a quarter of the samples sit below black. Some lift remains either
    // way (the noise left after noise reduction is clipped at black for display, and goes through
    // a nonlinear tone curve): measured about +5–16% kept, against +45–49% clipped first.
    for k in 0..3 {
        let (e, c) = (noisy[k] / clean[k] - 1.0, clipped[k] / clean[k] - 1.0);
        eprintln!(
            "channel {k}: clean {:.5}  kept {:+.1}%  clipped first {:+.1}%",
            clean[k],
            100.0 * e,
            100.0 * c
        );
        assert!(e.abs() < 0.25, "channel {k} off by {:.1}%", 100.0 * e);
        assert!(c > e + 0.2, "clipping first should lift more: {c} vs {e}");
    }
}
