//! Compare a photo's embedded lens data with lensfun's: `cargo run --example lens -- FILE…`
use tonality_core::lens::sample;

fn main() {
    for path in std::env::args().skip(1) {
        let p = std::path::Path::new(&path);
        let meta = tonality_raw::read_meta(p).expect("metadata");
        println!("{path}: {} {} · {:?} @ {:?} mm f/{:?}", meta.make, meta.model, meta.lens, meta.focal_length, meta.f_number);
        let (e, l) = tonality_raw::lens_profiles(p, &meta);
        println!("   r    | embedded: dist   red/g    blue/g   vig   | lensfun: dist   red/g    blue/g   vig");
        for i in 0..=10 {
            let r = i as f32 / 10.0;
            let row = |p: &Option<tonality_core::lens::LensProfile>| match p {
                Some(p) => {
                    let (d, rr, b) = match &p.distortion {
                        Some([rd, g, bd]) => (sample(g, r), sample(rd, r) / sample(g, r), sample(bd, r) / sample(g, r)),
                        None => (1.0, 1.0, 1.0),
                    };
                    let v = p.vignetting.as_ref().map_or(1.0, |v| sample(v, r));
                    format!("{d:.5} {rr:.6} {b:.6} {v:.3}")
                }
                None => "—".into(),
            };
            println!("   {r:.1}  | {} | {}", row(&e), row(&l));
        }
        if let Some(l) = &l {
            println!("   lensfun match: {}", l.name);
        }
    }
}
