//! Tone curves: the backend draws with the same curve the interface shows.

use serde::Deserialize;
use tonality_lib::edit::curve_table;

/// Curves and their values at 33 even steps, as the interface's `curveSampler`
/// gives them; src/adjustments.test.ts checks the interface against the same file.
const SAMPLES: &str = include_str!("../../src/curve-samples.json");

#[derive(Deserialize)]
struct Sample {
    name: String,
    points: Vec<[f32; 2]>,
    samples: Vec<f32>,
}

#[test]
fn curves_match_the_ones_the_interface_draws() {
    let samples: Vec<Sample> = serde_json::from_str(SAMPLES).unwrap();
    assert!(!samples.is_empty());
    for sample in samples {
        assert_eq!(sample.samples.len(), 33, "{}", sample.name);
        let table: [f32; 33] = curve_table(&sample.points);
        for (i, (got, want)) in table.iter().zip(&sample.samples).enumerate() {
            assert!((got - want).abs() < 1e-4, "{} at {i}/32: {got}, not {want}", sample.name);
        }
    }
}
