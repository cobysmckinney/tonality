//! Benchmark candidate mask models on RAW files and write contact sheets.
//!
//! `cargo run -p tonality-ml --release --example bench -- <models dir> <out dir> <raw>...`
//! Each sheet: the neutral input, then each model's mask as a red tint, labelled by order.

use std::path::{Path, PathBuf};
use std::time::Instant;

use image::{imageops, Rgb, RgbImage};
use tonality_core::mask::mask_size;
use tonality_ml::{input, refine, sam, scene, subject};

fn tint(img: &RgbImage, mask: &[u8]) -> RgbImage {
    let mut out = img.clone();
    for (p, &m) in out.pixels_mut().zip(mask) {
        let a = m as f32 / 255.0 * 0.6;
        let Rgb([r, g, b]) = *p;
        *p = Rgb([
            (r as f32 * (1.0 - a) + 255.0 * a) as u8,
            (g as f32 * (1.0 - a) + 40.0 * a) as u8,
            (b as f32 * (1.0 - a) + 40.0 * a) as u8,
        ]);
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let models = PathBuf::from(&args[0]);
    let out_dir = PathBuf::from(&args[1]);
    std::fs::create_dir_all(&out_dir).unwrap();
    let m = |n: &str| models.join(n);

    for raw in &args[2..] {
        let t = Instant::now();
        let p = tonality_raw::decode_preview(Path::new(raw), 1024).expect("decode");
        let (w, h) = mask_size(p.full_width, p.full_height);
        let img = input::neutral_srgb_sized(&p.pixels, p.width, p.height, &p.color, w, h);
        let guide = input::guide(&img, w, h);
        let name = Path::new(raw).file_stem().unwrap().to_string_lossy().into_owned();
        println!("\n{name}: {w}×{h} (decode + neutral {:.0} ms)", t.elapsed().as_secs_f32() * 1e3);

        let mut panels: Vec<(String, Vec<u8>)> = Vec::new();
        let mut time = |label: &str, f: &mut dyn FnMut() -> (Vec<f32>, u32, u32)| {
            let t = Instant::now();
            let (prob, pw, ph) = f();
            let dt = t.elapsed().as_secs_f32();
            let t2 = Instant::now();
            let r = refine::to_raster(&prob, pw, ph, &guide, w, h);
            println!("  {label:<22} {:>6.0} ms  (+ refine {:.0} ms)", dt * 1e3, t2.elapsed().as_secs_f32() * 1e3);
            panels.push((label.to_string(), r));
        };

        for (file, model, label) in [
            ("silueta.onnx", subject::SubjectModel::U2NET, "subject: silueta"),
            ("isnet.onnx", subject::SubjectModel::ISNET, "subject: isnet"),
            ("birefnet-lite.onnx", subject::SubjectModel::BIREFNET_LITE, "subject: birefnet-lite"),
        ] {
            // Warm-up run excluded (session creation).
            let _ = subject::detect(&m(file), model, &img);
            time(label, &mut || subject::detect(&m(file), model, &img).expect(label));
        }
        for (file, label) in [("segformer-b0.onnx", "segformer-b0"), ("upernet.onnx", "upernet")] {
            let cls = [scene::ade::SKY, scene::ade::PERSON];
            let _ = scene::detect(&m(file), scene::SceneModel::ADE_512, &img, &cls);
            let mut maps = None;
            time(&format!("sky: {label}"), &mut || {
                let (mm, pw, ph) = scene::detect(&m(file), scene::SceneModel::ADE_512, &img, &cls).expect(label);
                let sky = mm[0].clone();
                maps = Some((mm, pw, ph));
                (sky, pw, ph)
            });
            let (mm, pw, ph) = maps.unwrap();
            time(&format!("people: {label}"), &mut || (mm[1].clone(), pw, ph));
        }
        // Select object: a loop around the middle third.
        let lasso = vec![[0.3, 0.25], [0.7, 0.25], [0.75, 0.75], [0.3, 0.8]];
        let t = Instant::now();
        let emb = sam::encode(&m("mobilesam-enc.onnx"), &img).expect("sam encode");
        let _ = sam::encode(&m("mobilesam-enc.onnx"), &img).unwrap();
        println!("  sam encoder (2 runs)   {:>6.0} ms", t.elapsed().as_secs_f32() * 1e3);
        let _ = sam::decode(&m("mobilesam-dec.onnx"), &emb, &lasso, w, h);
        time("object: mobilesam", &mut || (sam::decode(&m("mobilesam-dec.onnx"), &emb, &lasso, w, h).unwrap(), w, h));

        // Contact sheet: input + panels, 4 per row, at half size.
        let (tw, th) = (w / 2, h / 2);
        let tiles: Vec<RgbImage> = std::iter::once(img.clone())
            .chain(panels.iter().map(|(_, r)| tint(&img, r)))
            .map(|t| imageops::resize(&t, tw, th, imageops::FilterType::Triangle))
            .collect();
        let cols = 4u32;
        let rows = (tiles.len() as u32).div_ceil(cols);
        let mut sheet = RgbImage::new(cols * tw, rows * th);
        for (i, t) in tiles.iter().enumerate() {
            imageops::replace(&mut sheet, t, ((i as u32 % cols) * tw) as i64, ((i as u32 / cols) * th) as i64);
        }
        sheet.save(out_dir.join(format!("{name}.jpg"))).unwrap();
        println!("  panels: input, {}", panels.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join(", "));
    }
}
