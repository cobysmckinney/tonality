//! The export shader's AgX against the CPU reference (`tonality_core::agx`), on the GPU.
//! Skipped (passes with a note) on machines without a usable adapter.

use half::f16;
use tonality_core::agx::Agx;
use tonality_core::color::CameraColor;
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

#[test]
fn shader_agx_matches_the_cpu_reference() {
    let gpu = match Gpu::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipped: {e}");
            return;
        }
    };
    let reference: Reference =
        serde_json::from_str(include_str!("../../tonality-core/tests/data/agx_reference.json")).unwrap();
    // The develop pass clamps negatives before AgX, so only in-gamut colours reach it here.
    let inputs: Vec<[f32; 3]> = reference
        .rows
        .iter()
        .filter(|r| r.input.iter().all(|&v| v >= 0.0))
        .map(|r| r.input.map(|v| f16::from_f64(v).to_f32()))
        .collect();

    // A row of pixels, one per colour; every adjustment off and camera RGB = Rec.2020.
    let (w, h) = (inputs.len() as u32, 1u32);
    let edit = EditState::default();
    let color = CameraColor { xyz_to_cam: tonality_core::color::IDENTITY, as_shot_wb: [1.0; 3] };
    let mut params = DevelopParams::new(&edit, &color, None, w, h);
    params.wb = [1.0, 1.0, 1.0, 1e6]; // no sensor clip: scene values above 1 go straight to AgX
    params.cam = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
    params.light0[0] = 1.0;
    params.detail0 = [0.0; 4];
    params.detail1 = [0.0; 4];
    params.grain = [0.0; 4];
    params.vignette = [0.0; 4];
    let pixels: Vec<u16> = inputs.iter().flat_map(|c| c.map(|v| f16::from_f32(v).to_bits())).collect();
    let src = Source { width: w, height: h, pixels: &pixels, frame: [w, h] };
    let out = gpu.render(&src, &params, &curve_lut_rgba16f(&edit), &brush_layers(&[], w, h), |_| {}).unwrap();
    assert_eq!((out.width, out.height), (w, h));

    let agx = Agx::get();
    let mut worst = (0.0f64, [0.0f32; 3]);
    for (i, c) in inputs.iter().enumerate() {
        let want = agx.display(c.map(f64::from));
        for (k, want) in want.iter().enumerate() {
            let got = out.pixels[i * 3 + k] as f64 / 65535.0;
            let d = (got - want).abs();
            if d > worst.0 {
                worst = (d, *c);
            }
        }
    }
    eprintln!("GPU ({}) vs CPU: worst {:.6} at {:?}", gpu.adapter, worst.0, worst.1);
    assert!(worst.0 < 1e-3, "worst {} at {:?}", worst.0, worst.1);
}
