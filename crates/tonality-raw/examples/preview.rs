//! Sanity check: decode a RAW and write a naive sRGB PNG (as-shot WB, matrix, gamma).
//! `cargo run -p tonality-raw --release --example preview -- in.ARW out.png`
use std::time::Instant;

use tonality_core::color::{mul_vec, REC2020_TO_XYZ};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let t = Instant::now();
    let p = tonality_raw::decode_preview(args[1].as_ref(), 2560).expect("decode");
    println!(
        "{} {} — {}x{} in {:?}; as-shot {:?}",
        p.metadata.make, p.metadata.model, p.width, p.height, t.elapsed(),
        p.color.as_shot_temperature_tint()
    );
    println!("xyz_to_cam {:?}\nwb {:?}\ncam_to_working {:?}", p.color.xyz_to_cam, p.color.as_shot_wb, p.color.cam_to_working());
    let wb = p.color.as_shot_multipliers();
    let m = p.color.cam_to_working();
    // Rec.2020 → XYZ → sRGB
    let xyz_to_srgb = [[3.2406, -1.5372, -0.4986], [-0.9689, 1.8758, 0.0415], [0.0557, -0.2040, 1.0570]];
    let px: Vec<u8> = p
        .pixels
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|c| {
            let cam: Vec<f32> = c.iter().map(|b| half::f16::from_bits(*b).to_f32()).collect();
            let cam = [(cam[0] * wb[0]).min(1.0), (cam[1] * wb[1]).min(1.0), (cam[2] * wb[2]).min(1.0)];
            let rgb = mul_vec(&xyz_to_srgb, mul_vec(&REC2020_TO_XYZ, mul_vec(&m, cam)));
            rgb.map(|v| ((v * 2.0).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8)
        })
        .collect();
    image::RgbImage::from_raw(p.width, p.height, px).unwrap().save(&args[2]).unwrap();
}
