//! Sky mask refinement check: `cargo run -p tonality-ml --release --example sky -- <segformer.onnx> <out dir> <raw>...`
//! Writes `<name>-input.png` and `<name>-sky.png` (the raster as tonality stores it).
use std::path::Path;
use std::time::Instant;

use image::GrayImage;
use tonality_core::mask::mask_size;
use tonality_ml::{input, refine, scene};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let model = Path::new(&args[0]);
    let out = Path::new(&args[1]);
    std::fs::create_dir_all(out).unwrap();
    for raw in &args[2..] {
        let p = tonality_raw::decode_preview(Path::new(raw), 1024).unwrap();
        let (w, h) = mask_size(p.full_width, p.full_height);
        let img = input::neutral_srgb_sized(&p.pixels, p.width, p.height, &p.color, w, h);
        let name = Path::new(raw)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (maps, pw, ph) =
            scene::detect(model, scene::SceneModel::ADE_512, &img, &[scene::ade::SKY]).unwrap();
        let t = Instant::now();
        let r = refine::sky_raster(&maps[0], pw, ph, &img);
        println!(
            "{name}: sky refine {:.0} ms",
            t.elapsed().as_secs_f32() * 1e3
        );
        img.save(out.join(format!("{name}-input.png"))).unwrap();
        GrayImage::from_raw(w, h, r)
            .unwrap()
            .save(out.join(format!("{name}-sky.png")))
            .unwrap();
    }
}
