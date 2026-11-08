//! Time metadata + thumbnail extraction: `cargo run -p tonality-raw --release --example thumbs -- DIR`
use std::time::Instant;

fn main() {
    let dir = std::env::args().nth(1).expect("usage: thumbs DIR");
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("arw")) { continue; }
        let t = Instant::now();
        let meta = tonality_raw::read_meta(&path);
        let tm = t.elapsed();
        let t = Instant::now();
        let thumb = tonality_raw::thumbnail(&path, 640);
        let tt = t.elapsed();
        match (meta, thumb) {
            (Ok(m), Ok(img)) => println!(
                "{:?}: meta {tm:.1?} thumb {tt:.1?} {}×{} | {} {} {:?} ISO {:?} {:?}s f/{:?} {:?}mm {:?}",
                path.file_name().unwrap(), img.width(), img.height(), m.make, m.model, m.lens, m.iso,
                m.exposure_time, m.f_number, m.focal_length, m.captured_at
            ),
            (m, t) => println!("{path:?}: {:?} {:?}", m.err(), t.err()),
        }
    }
}
