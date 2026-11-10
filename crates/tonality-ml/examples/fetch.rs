//! Download a registered model: `cargo run -p tonality-ml --example fetch -- <dir> <model id>`.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = std::path::PathBuf::from(&args[0]);
    let m = tonality_ml::registry::by_id(&args[1]).expect("unknown model id");
    for f in m.files {
        let mut last = 0;
        let p = tonality_ml::download::ensure(&dir, f, &mut |d, t| {
            let pct = d * 100 / t.max(1);
            if pct >= last + 25 {
                last = pct;
                println!("  {} {pct}%", f.name);
            }
        })
        .expect("download");
        println!("ok {}", p.display());
    }
}
