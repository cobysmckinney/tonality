//! Colour grading tints the range it is set on and leaves the others alone.
//! Skipped (passes with a note) on machines without a usable adapter, unless
//! `TONALITY_REQUIRE_GPU` is set.

mod common;

use half::f16;
use tonality_core::color::CameraColor;
use tonality_core::grading::{ColorGrading, GradeRange};
use tonality_core::mask::brush_layers;
use tonality_core::EditState;
use tonality_render::{curve_lut_rgba16f, DevelopParams, Gpu, Source};

/// Grey ramp from −6 to +5 EV around middle grey, one pixel per step.
const STEPS: usize = 64;

fn ramp() -> Vec<f32> {
    (0..STEPS)
        .map(|i| 0.18 * (-6.0 + 11.0 * i as f32 / (STEPS - 1) as f32).exp2())
        .collect()
}

/// The ramp developed with `edit`: red, green, blue (0–1) per step.
fn develop(gpu: &Gpu, edit: &EditState) -> Vec<[f64; 3]> {
    let grey = ramp();
    let (w, h) = (grey.len() as u32, 1u32);
    let color = CameraColor {
        xyz_to_cam: tonality_core::color::IDENTITY,
        as_shot_wb: [1.0; 3],
    };
    let mut params = DevelopParams::new(edit, &color, None, w, h);
    params.wb = [1.0, 1.0, 1.0, 1e6];
    params.cam = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    // Nothing that looks at neighbours or position: each pixel is just its input, graded.
    params.detail0 = [0.0; 4];
    params.detail1 = [0.0; 4];
    params.grain = [0.0; 4];
    params.vignette = [0.0; 4];
    let pixels: Vec<u16> = grey
        .iter()
        .flat_map(|&v| [f16::from_f32(v).to_bits(); 3])
        .collect();
    let src = Source {
        width: w,
        height: h,
        pixels: &pixels,
        frame: [w, h],
    };
    let out = gpu
        .render(
            &src,
            &params,
            &curve_lut_rgba16f(edit),
            &brush_layers(&edit.masks, w, h),
            None,
            |_| {},
        )
        .unwrap();
    (0..grey.len())
        .map(|i| [0, 1, 2].map(|k| out.pixels[i * 3 + k] as f64 / 65535.0))
        .collect()
}

fn graded(g: ColorGrading) -> EditState {
    EditState {
        grading: g,
        ..EditState::default()
    }
}

fn tint(hue: f32) -> GradeRange {
    GradeRange {
        hue,
        saturation: 100.0,
        luminance: 0.0,
    }
}

/// Red minus blue: positive is warm, negative is cool.
fn warmth(p: [f64; 3]) -> f64 {
    p[0] - p[2]
}

fn luma(p: [f64; 3]) -> f64 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// The steps that come out this bright (0–1) without grading.
fn steps_between(plain: &[[f64; 3]], lo: f64, hi: f64) -> Vec<usize> {
    (0..plain.len())
        .filter(|&i| (lo..hi).contains(&luma(plain[i])))
        .collect()
}

const ORANGE: f32 = 60.0;
const BLUE: f32 = 250.0;
/// Beyond 16-bit rounding.
const SEEN: f64 = 0.01;

#[test]
fn each_range_tints_its_own_tones() {
    let Some(gpu) = common::gpu() else { return };
    let plain = develop(&gpu, &EditState::default());
    assert!(
        plain.iter().all(|&p| warmth(p).abs() < 1e-3),
        "the ramp starts out grey"
    );
    let dark = steps_between(&plain, 0.02, 0.15);
    let mid = steps_between(&plain, 0.42, 0.58);
    let bright = steps_between(&plain, 0.85, 0.98);
    assert!(!dark.is_empty() && !mid.is_empty() && !bright.is_empty());

    // Cool shadows, warm highlights, and the midtones left alone.
    let out = develop(
        &gpu,
        &graded(ColorGrading {
            shadows: tint(BLUE),
            highlights: tint(ORANGE),
            blending: 0.0,
            ..ColorGrading::default()
        }),
    );
    for &i in &dark {
        assert!(
            warmth(out[i]) < -SEEN,
            "step {i}: shadows cool, {:?}",
            out[i]
        );
    }
    for &i in &bright {
        assert!(
            warmth(out[i]) > SEEN,
            "step {i}: highlights warm, {:?}",
            out[i]
        );
    }
    for &i in &mid {
        assert!(
            warmth(out[i]).abs() < 2e-3,
            "step {i}: midtones grey, {:?}",
            out[i]
        );
    }
    // A tint changes the colour, not how bright the tone looks.
    for i in 0..STEPS {
        assert!(
            (luma(out[i]) - luma(plain[i])).abs() < 0.03,
            "step {i}: {:?} from {:?}",
            out[i],
            plain[i]
        );
    }

    // Midtones alone: the middle is tinted, the ends fade back to grey.
    let out = develop(
        &gpu,
        &graded(ColorGrading {
            midtones: tint(ORANGE),
            ..ColorGrading::default()
        }),
    );
    for &i in &mid {
        assert!(warmth(out[i]) > SEEN, "step {i}: {:?}", out[i]);
    }
    let least = mid.iter().map(|&i| warmth(out[i])).fold(f64::MAX, f64::min);
    for i in [0, STEPS - 1] {
        assert!(
            warmth(out[i]).abs() < 0.2 * least,
            "step {i}: {:?} against {least} in the middle",
            out[i]
        );
    }
}

#[test]
fn balance_moves_the_split_and_luminance_moves_brightness() {
    let Some(gpu) = common::gpu() else { return };
    let plain = develop(&gpu, &EditState::default());
    let mid = steps_between(&plain, 0.42, 0.58);
    let dark = steps_between(&plain, 0.02, 0.15);
    let bright = steps_between(&plain, 0.85, 0.98);

    // With the balance towards highlights, the highlight tint reaches further down.
    let with_balance = |balance: f32| {
        develop(
            &gpu,
            &graded(ColorGrading {
                highlights: tint(ORANGE),
                balance,
                ..ColorGrading::default()
            }),
        )
    };
    let (low, high) = (with_balance(-100.0), with_balance(100.0));
    for &i in &mid {
        assert!(
            warmth(high[i]) > warmth(low[i]) + SEEN,
            "step {i}: {:?} against {:?}",
            high[i],
            low[i]
        );
    }

    // Luminance: brighter shadows, and the highlights where they were.
    let out = develop(
        &gpu,
        &graded(ColorGrading {
            shadows: GradeRange {
                luminance: 100.0,
                ..GradeRange::default()
            },
            ..ColorGrading::default()
        }),
    );
    for &i in &dark {
        assert!(luma(out[i]) > luma(plain[i]) + SEEN, "step {i}");
    }
    for &i in &bright {
        assert!((luma(out[i]) - luma(plain[i])).abs() < 2e-3, "step {i}");
    }
    // A brighter input still comes out brighter.
    assert!(out
        .windows(2)
        .all(|w| luma(w[1]) >= luma(w[0]) - 2.0 / 65535.0));
}
