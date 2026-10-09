//! Every integration test, built as one binary. Run one file's tests with
//! `cargo test --test integration masks::`.

mod adjustments;
mod curves;
mod develop;
mod editor;
mod export;
mod film;
mod history;
mod library;
mod masks;
mod presets;

/// A file in tests/ without a `mod` line above is never compiled, so its
/// tests would be skipped without a word.
#[test]
fn every_test_file_is_built() {
    let this = include_str!("main.rs");
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            let name = path.file_stem().unwrap().to_str().unwrap();
            assert!(
                name == "main" || this.contains(&format!("\nmod {name};")),
                "tests/{name}.rs needs `mod {name};` in tests/main.rs",
            );
        }
    }
}
