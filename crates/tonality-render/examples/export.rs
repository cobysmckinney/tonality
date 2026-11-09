//! Headless full-resolution render: `cargo run --release -p tonality-render --example export -- FILE OUT.png [sharpen] [nr]`
use std::time::Instant;
use tonality_core::EditState;
use tonality_render::{curve_lut_rgba16f, DevelopParams, Gpu, Source};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let t = Instant::now();
    let gpu = Gpu::new().expect("gpu");
    println!("gpu {} in {:?}", gpu.adapter, t.elapsed());
    let t = Instant::now();
    let full = tonality_raw::decode_full(args[0].as_ref()).expect("decode");
    println!("decode {}×{} in {:?}", full.width, full.height, t.elapsed());
    let mut edit = EditState::default();
    edit.light.exposure = 1.0;
    edit.detail.sharpen_amount = args.get(2).map_or(0.0, |v| v.parse().unwrap());
    edit.detail.luminance_noise = args.get(3).map_or(0.0, |v| v.parse().unwrap());
    edit.detail.color_noise = args.get(3).map_or(0.0, |v| v.parse().unwrap());
    // CROP="angle,x,y,w,h,turns" frames the render.
    if let Ok(c) = std::env::var("CROP") {
        let v: Vec<f32> = c.split(',').map(|x| x.parse().unwrap()).collect();
        edit.crop.angle = v[0];
        edit.crop.rect = [v[1], v[2], v[3], v[4]];
        edit.crop.turns = v[5] as u8;
    }
    // MASKS=masks.json (a JSON array of masks) adds local adjustments.
    if let Ok(path) = std::env::var("MASKS") {
        edit.masks = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    }
    let params = DevelopParams::new(&edit, &full.color, full.width, full.height);
    let brush = tonality_core::mask::brush_layers(&edit.masks, full.width, full.height);
    let src = Source { width: full.width, height: full.height, pixels: &full.pixels, frame: [full.width, full.height] };
    let t = Instant::now();
    let out = gpu.render(&src, &params, &curve_lut_rgba16f(&edit), &brush, |_| {}).expect("render");
    println!("render in {:?}", t.elapsed());
    println!("output {}×{}", out.width, out.height);
    if std::env::var("CROP").is_ok() || std::env::var("MASKS").is_ok() {
        let rgb: Vec<u8> = out.pixels.iter().map(|v| (v >> 8) as u8).collect();
        let img = image::RgbImage::from_raw(out.width, out.height, rgb).unwrap();
        image::imageops::thumbnail(&img, out.width / 6, out.height / 6).save(&args[1]).unwrap();
        return;
    }
    // Mean |laplacian| of luma in the 1:1 crop (detail energy), and the seam check: the mean
    // vertical step across a tile boundary vs. across ordinary rows.
    let w = out.width as usize;
    let luma = |x: usize, y: usize| {
        let i = (y * w + x) * 3;
        (out.pixels[i] as f64 + 2.0 * out.pixels[i + 1] as f64 + out.pixels[i + 2] as f64) / 4.0 / 257.0
    };
    let mut lap = 0.0;
    for y in 1001..1799 {
        for x in 1001..2199 {
            lap += (4.0 * luma(x, y) - luma(x - 1, y) - luma(x + 1, y) - luma(x, y - 1) - luma(x, y + 1)).abs();
        }
    }
    let step = |y: usize| (0..w).map(|x| (luma(x, y) - luma(x, y - 1)).abs()).sum::<f64>() / w as f64;
    println!(
        "detail energy {:.3}  seam step {:.3} vs rows {:.3} {:.3}",
        lap / (798.0 * 1198.0),
        step(2048),
        step(2047),
        step(2049)
    );
    let rgb: Vec<u8> = out.pixels.iter().map(|v| (v >> 8) as u8).collect();
    let img = image::RgbImage::from_raw(out.width, out.height, rgb).unwrap();
    // Save a 1:1 crop plus a small whole-frame view.
    image::imageops::crop_imm(&img, 1000, 1000, 1200, 800).to_image().save(&args[1]).unwrap();
    image::imageops::thumbnail(&img, 600, 900).save(args[1].replace(".png", "-small.png")).unwrap();
}
