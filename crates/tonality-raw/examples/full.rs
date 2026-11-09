//! Time the full-resolution decode: `cargo run --release -p tonality-raw --example full -- FILE [out.png]`
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = std::path::Path::new(&args[0]);
    let t = Instant::now();
    let full = tonality_raw::decode_full(path).expect("decode");
    println!("{}×{} in {:?}", full.width, full.height, t.elapsed());
    let t = Instant::now();
    let r = full.region(1000, 1000, 1200, 800, 0);
    println!("region {}×{} in {:?}", r.width, r.height, t.elapsed());
    if let Some(out) = args.get(1) {
        // Crude gamma-only dump of a 1:1 crop, to eyeball demosaic quality.
        let mut img = image::RgbImage::new(r.width, r.height);
        for (i, p) in img.pixels_mut().enumerate() {
            for c in 0..3 {
                let v = half::f16::from_bits(r.pixels[i * 3 + c]).to_f32() * [2.0, 1.0, 1.6][c] * 3.0;
                p.0[c] = (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
            }
        }
        img.save(out).unwrap();
    }
}
