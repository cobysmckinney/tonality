//! Tone adjustments must keep a brighter input brighter, even when a mask adds its own on top of
//! the global ones (issue #12: the sum went past what `tone()` handles and inverted tones).
//! Skipped (passes with a note) on machines without a usable adapter, unless
//! `TONALITY_REQUIRE_GPU` is set.

mod common;

use half::f16;
use tonality_core::color::CameraColor;
use tonality_core::mask::{brush_layers, Mask, MaskShape};
use tonality_core::EditState;
use tonality_render::{curve_lut_rgba16f, DevelopParams, Gpu, Source};

/// Grey ramp from −3 to +5 EV around middle grey, one pixel per step.
const STEPS: usize = 64;

fn ramp() -> Vec<f32> {
    (0..STEPS)
        .map(|i| 0.18 * (-3.0 + 8.0 * i as f32 / (STEPS - 1) as f32).exp2())
        .collect()
}

/// Display luminance (mean code value, 0–1) of the ramp developed with `edit`.
fn develop(gpu: &Gpu, edit: &EditState) -> Vec<f64> {
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
    // Nothing that looks at neighbours or position: each pixel is just its input, toned.
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
        .map(|i| (0..3).map(|k| out.pixels[i * 3 + k] as f64).sum::<f64>() / (3.0 * 65535.0))
        .collect()
}

/// A mask selecting the whole frame (an empty brush, inverted).
fn everywhere() -> Mask {
    Mask {
        shape: MaskShape::Brush { strokes: vec![] },
        invert: true,
        ..Mask::default()
    }
}

/// Steps where a brighter input came out darker (beyond 16-bit rounding).
fn inversions(out: &[f64]) -> Vec<usize> {
    (1..out.len())
        .filter(|&i| out[i] < out[i - 1] - 2.0 / 65535.0)
        .collect()
}

#[test]
fn stacked_tone_adjustments_keep_the_ramp_in_order() {
    let Some(gpu) = common::gpu() else { return };

    type Set = fn(&mut EditState, f32);
    let sliders: [(&str, Set); 5] = [
        ("contrast", |e, v| {
            e.light.contrast = v;
            e.masks[0].adjust.contrast = v;
        }),
        ("highlights", |e, v| {
            e.light.highlights = v;
            e.masks[0].adjust.highlights = v;
        }),
        ("shadows", |e, v| {
            e.light.shadows = v;
            e.masks[0].adjust.shadows = v;
        }),
        ("whites", |e, v| {
            e.light.whites = v;
            e.masks[0].adjust.whites = v;
        }),
        ("blacks", |e, v| {
            e.light.blacks = v;
            e.masks[0].adjust.blacks = v;
        }),
    ];
    let mut failures = Vec::new();
    for (name, set) in sliders {
        for v in [-100.0, 100.0] {
            let mut edit = EditState {
                masks: vec![everywhere()],
                ..EditState::default()
            };
            set(&mut edit, v);
            let out = develop(&gpu, &edit);
            let bad = inversions(&out);
            eprintln!("{name} {v:+} global + mask: {} inversions", bad.len());
            if !bad.is_empty() {
                failures.push(format!("{name} {v:+}: steps {bad:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "tones inverted: {failures:?}");

    // The clamp is what the sum is held to: a full-frame mask on top of a maxed slider changes
    // nothing, rather than pushing past it.
    let mut alone = EditState::default();
    alone.light.contrast = -100.0;
    let mut stacked = alone.clone();
    stacked.masks = vec![everywhere()];
    stacked.masks[0].adjust.contrast = -100.0;
    let (a, b) = (develop(&gpu, &alone), develop(&gpu, &stacked));
    let d = a
        .iter()
        .zip(&b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f64::max);
    assert!(
        d < 2.0 / 65535.0,
        "a mask past a maxed slider moved the ramp by {d}"
    );
}
