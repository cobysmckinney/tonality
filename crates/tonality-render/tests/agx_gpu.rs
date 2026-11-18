//! The export shader's AgX against the CPU reference (`tonality_core::agx`), on the GPU, and
//! colours outside Rec.2020 on their way to it. Skipped (passes with a note) on machines without
//! a usable adapter.

use half::f16;
use tonality_core::agx::{Agx, Display, GAIN_LOG2, GAIN_OFFSET};
use tonality_core::color::{invert, CameraColor, Mat3, REC2020_TO_XYZ};
use tonality_core::mask::brush_layers;
use tonality_core::EditState;
use tonality_render::{curve_lut_rgba16f, DevelopParams, Gpu, Source};

#[derive(serde::Deserialize)]
struct Reference {
    rows: Vec<Row>,
}
#[derive(serde::Deserialize)]
struct Row {
    #[serde(rename = "in")]
    input: [f64; 3],
}

// Camera RGB can't be negative (black clips after noise reduction), but Rec.2020 can: colours
// outside Rec.2020 are made here from camera values ≥ 0 through a matrix with negative entries,
// as a real camera's would be. Camera = CIE XYZ, which holds every colour, so camera → Rec.2020
// is XYZ→2020 and each colour's camera value is 2020→XYZ · colour.

fn to_f64(m: Mat3) -> [[f64; 3]; 3] {
    m.map(|r| r.map(f64::from))
}

fn mv(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    m.map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
}

/// Camera values (f16, as uploaded) for Rec.2020 colours, given camera → Rec.2020 `m`; `None`
/// when one would be negative.
fn camera(m: &Mat3, rgb: [f64; 3]) -> Option<[f32; 3]> {
    let c = mv(&to_f64(invert(m).unwrap()), rgb).map(|v| f16::from_f64(v).to_f32());
    c.iter().all(|&v| v >= 0.0).then_some(c)
}

/// The Rec.2020 colour the shader receives for camera value `c`.
fn rec2020(m: &Mat3, c: [f32; 3]) -> [f64; 3] {
    mv(&to_f64(*m), c.map(f64::from))
}

/// Develops a row of pixels, one per camera value, through camera → Rec.2020 `m`, with every
/// adjustment off except what `tweak` sets. Display code values, 0–1.
fn develop(
    gpu: &Gpu,
    m: &Mat3,
    cam: &[[f32; 3]],
    tweak: impl FnOnce(&mut DevelopParams),
) -> Vec<[f64; 3]> {
    let (w, h) = (cam.len() as u32, 1u32);
    let edit = EditState::default();
    let color = CameraColor {
        xyz_to_cam: tonality_core::color::IDENTITY,
        as_shot_wb: [1.0; 3],
    };
    let mut params = DevelopParams::new(&edit, &color, None, w, h);
    params.wb = [1.0, 1.0, 1.0, 1e6]; // no sensor clip: scene values above 1 go straight to AgX
    params.cam = m.map(|r| [r[0], r[1], r[2], 0.0]);
    params.light0[0] = 1.0;
    params.detail0 = [0.0; 4];
    params.detail1 = [0.0; 4];
    params.grain = [0.0; 4];
    params.vignette = [0.0; 4];
    tweak(&mut params);
    let pixels: Vec<u16> = cam
        .iter()
        .flat_map(|c| c.map(|v| f16::from_f32(v).to_bits()))
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
            &curve_lut_rgba16f(&edit),
            &brush_layers(&[], w, h),
            None,
            |_| {},
        )
        .unwrap();
    assert_eq!((out.width, out.height), (w, h));
    (0..cam.len())
        .map(|i| [0, 1, 2].map(|k| out.pixels[i * 3 + k] as f64 / 65535.0))
        .collect()
}

fn gpu() -> Option<Gpu> {
    Gpu::new().map_err(|e| eprintln!("skipped: {e}")).ok()
}

/// The reference colours, as camera values and as the Rec.2020 colours the shader receives.
fn reference_colours() -> (Mat3, Vec<[f32; 3]>, Vec<[f64; 3]>) {
    let reference: Reference = serde_json::from_str(include_str!(
        "../../tonality-core/tests/data/agx_reference.json"
    ))
    .unwrap();
    let m = invert(&REC2020_TO_XYZ).unwrap();
    let cam: Vec<[f32; 3]> = reference
        .rows
        .iter()
        .filter_map(|r| camera(&m, r.input))
        .collect();
    let inputs: Vec<[f64; 3]> = cam.iter().map(|&c| rec2020(&m, c)).collect();
    let negative = inputs
        .iter()
        .filter(|c| c.iter().any(|&v| v < -1e-4))
        .count();
    assert!(
        negative >= 10,
        "only {negative} colours outside Rec.2020 reach the test"
    );
    (m, cam, inputs)
}

/// Largest difference between the shader's output and `want`, and the input it was at.
fn worst(
    inputs: &[[f64; 3]],
    out: &[[f64; 3]],
    want: impl Fn([f64; 3]) -> [f64; 3],
) -> (f64, [f64; 3]) {
    let mut worst = (0.0f64, [0.0f64; 3]);
    for (c, got) in inputs.iter().zip(out) {
        let want = want(*c);
        for k in 0..3 {
            let d = (got[k] - want[k]).abs();
            if d > worst.0 {
                worst = (d, *c);
            }
        }
    }
    worst
}

#[test]
fn shader_agx_matches_the_cpu_reference() {
    let Some(gpu) = gpu() else { return };
    let (m, cam, inputs) = reference_colours();
    for display in [Display::Srgb, Display::DisplayP3] {
        let out = develop(&gpu, &m, &cam, |p| *p = p.for_display(display));
        let agx = Agx::sdr(display);
        let (d, at) = worst(&inputs, &out, |c| agx.display(c));
        eprintln!(
            "GPU ({}) vs CPU, {display:?}, {} colours: worst {d:.6} at {at:?}",
            gpu.adapter,
            inputs.len()
        );
        assert!(d < 1e-3, "{display:?}: worst {d} at {at:?}");
    }
}

/// The gain map pass: per channel, log2 of AgX HDR over AgX Base (linear, in the display's
/// primaries), over the stored range.
#[test]
fn shader_gain_map_matches_the_cpu_reference() {
    let Some(gpu) = gpu() else { return };
    let (m, cam, inputs) = reference_colours();
    let linear = |v: f64| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    for display in [Display::Srgb, Display::DisplayP3] {
        let out = develop(&gpu, &m, &cam, |p| *p = p.for_display(display).gain_map());
        let (sdr, hdr) = (Agx::sdr(display), Agx::hdr(display));
        let (d, at) = worst(&inputs, &out, |c| {
            let (s, h) = (sdr.display(c).map(linear), hdr.hdr_relative(c));
            [0, 1, 2].map(|i| {
                let g = ((h[i] + GAIN_OFFSET) / (s[i] + GAIN_OFFSET)).log2();
                ((g - GAIN_LOG2[0]) / (GAIN_LOG2[1] - GAIN_LOG2[0])).clamp(0.0, 1.0)
            })
        });
        // 1/255 is one step of the 8-bit gain map.
        eprintln!("GPU gain map vs CPU, {display:?}: worst {d:.6} at {at:?}");
        assert!(d < 1.0 / 255.0, "{display:?}: worst {d} at {at:?}");
    }
}

/// A dark, saturated blue just outside Rec.2020 has a luminance at or below zero. Tone scales
/// colours by a luminance ratio; with the plain luminance (floored at 1e-7) lower contrast lifts
/// that "black" by thousands of times and the pixel goes white. It must stay about as bright as
/// its neighbour just inside the gamut (its hue may move a little: AgX's guard rail lifts the
/// negative green, which brings some red with it).
#[test]
fn colours_outside_rec2020_survive_tone() {
    let Some(gpu) = gpu() else { return };
    // XYZ can't hold a negative luminance, but a real camera's matrix can produce one: this
    // camera's green takes away a fifth of its blue.
    let m: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, -0.2], [0.0, 0.0, 1.0]];
    let colours = [[0.0, -0.01, 0.1], [0.0, -0.0115, 0.13], [0.0, 0.0, 0.1]];
    let cam: Vec<[f32; 3]> = colours.iter().map(|&c| camera(&m, c).unwrap()).collect();
    let y = |c: [f64; 3]| 0.2627 * c[0] + 0.6780 * c[1] + 0.0593 * c[2];
    assert!(
        y(rec2020(&m, cam[0])) < 0.0 && y(rec2020(&m, cam[1])) < 0.0,
        "test colours have negative luminance"
    );
    // Display luminance, in code values (Rec.709 weights): how bright the pixel looks.
    let shown = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    for (contrast, shadows) in [(0.0, 0.0), (-1.0, 0.0), (-1.0, 1.0)] {
        let out = develop(&gpu, &m, &cam, |p| {
            p.light0[1] = contrast;
            p.light0[3] = shadows;
        });
        let inside = shown(out[2]);
        for (c, o) in colours.iter().zip(&out).take(2) {
            assert!(
                (shown(*o) - inside).abs() < 0.05,
                "contrast {contrast}, shadows {shadows}: {c:?} → {o:?} ({:.3}), inside the gamut {:?} ({inside:.3})",
                shown(*o),
                out[2]
            );
        }
    }
}

/// Auto's model of the screen (`tonality_core::auto`: `tone()` ported, then AgX on grey) against
/// the shader, for greys across the range and a few Light settings, so the two can't drift.
#[test]
fn auto_tone_model_matches_the_shader() {
    use tonality_core::auto::{display_of_ev, tone_ev};
    use tonality_core::edit::Light;
    let Some(gpu) = gpu() else { return };
    let m = invert(&REC2020_TO_XYZ).unwrap();
    let evs: Vec<f32> = (0..=56).map(|i| -10.0 + i as f32 * 0.25).collect();
    let cam: Vec<[f32; 3]> = evs
        .iter()
        .map(|&ev| camera(&m, [0.18 * (ev as f64).exp2(); 3]).unwrap())
        .collect();
    let lights = [
        Light::default(),
        Light {
            exposure: 1.3,
            contrast: 40.0,
            highlights: -60.0,
            shadows: 50.0,
            whites: 20.0,
            blacks: -30.0,
        },
        Light {
            exposure: -0.7,
            contrast: -50.0,
            highlights: 30.0,
            shadows: -20.0,
            whites: -40.0,
            blacks: 45.0,
        },
    ];
    for l in lights {
        let n = |v: f32| v / 100.0;
        let out = develop(&gpu, &m, &cam, |p| {
            p.light0 = [
                l.exposure.exp2(),
                n(l.contrast),
                n(l.highlights),
                n(l.shadows),
            ];
            p.light1 = [n(l.whites), n(l.blacks), 0.0, 0.0];
        });
        for (k, c) in cam.iter().enumerate() {
            // The grey the shader received (f16 camera values), in stops.
            let y = rec2020(&m, *c)[1];
            let ev = ((y / 0.18).log2()) as f32;
            let want = display_of_ev(tone_ev(ev + l.exposure, &l)) as f64;
            assert!(
                (out[k][1] - want).abs() < 2e-3,
                "{l:?} at {ev}: shader {} vs model {want}",
                out[k][1]
            );
        }
    }
}
