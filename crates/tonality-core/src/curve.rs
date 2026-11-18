//! Tone curve evaluation: monotone cubic Hermite interpolation (Fritsch–Carlson), so a curve
//! through increasing points never overshoots or wiggles between them.

pub type Point = [f32; 2];

/// Control points sorted by x, de-duplicated, clamped to 0..1. Fewer than two points is the
/// identity curve.
fn normalise(points: &[Point]) -> Vec<Point> {
    let mut p: Vec<Point> = points
        .iter()
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)])
        .collect();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]));
    p.dedup_by(|b, a| (b[0] - a[0]).abs() < 1e-4);
    if p.len() < 2 {
        return vec![[0.0, 0.0], [1.0, 1.0]];
    }
    p
}

/// Tangents for monotone cubic interpolation.
fn tangents(p: &[Point]) -> Vec<f32> {
    let n = p.len();
    let d: Vec<f32> = (0..n - 1)
        .map(|i| (p[i + 1][1] - p[i][1]) / (p[i + 1][0] - p[i][0]))
        .collect();
    let mut m = vec![0.0; n];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for i in 1..n - 1 {
        m[i] = if d[i - 1] * d[i] <= 0.0 {
            0.0
        } else {
            (d[i - 1] + d[i]) / 2.0
        };
    }
    // Fritsch–Carlson: limit tangents so each segment stays monotone.
    for i in 0..n - 1 {
        if d[i] == 0.0 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
            continue;
        }
        let (a, b) = (m[i] / d[i], m[i + 1] / d[i]);
        let s = a * a + b * b;
        if s > 9.0 {
            let t = 3.0 / s.sqrt();
            m[i] = t * a * d[i];
            m[i + 1] = t * b * d[i];
        }
    }
    m
}

/// A curve prepared for repeated evaluation.
pub struct Curve {
    p: Vec<Point>,
    m: Vec<f32>,
}

impl Curve {
    pub fn new(points: &[Point]) -> Self {
        let p = normalise(points);
        let m = tangents(&p);
        Curve { p, m }
    }

    pub fn is_identity(points: &[Point]) -> bool {
        let p = normalise(points);
        p.len() == 2 && p[0] == [0.0, 0.0] && p[1] == [1.0, 1.0]
    }

    pub fn eval(&self, x: f32) -> f32 {
        let p = &self.p;
        let x = x.clamp(0.0, 1.0);
        // Flat beyond the first/last point.
        if x <= p[0][0] {
            return p[0][1];
        }
        if x >= p[p.len() - 1][0] {
            return p[p.len() - 1][1];
        }
        let i = p.partition_point(|q| q[0] <= x) - 1;
        let h = p[i + 1][0] - p[i][0];
        let t = (x - p[i][0]) / h;
        let (t2, t3) = (t * t, t * t * t);
        let y = (2.0 * t3 - 3.0 * t2 + 1.0) * p[i][1]
            + (t3 - 2.0 * t2 + t) * h * self.m[i]
            + (-2.0 * t3 + 3.0 * t2) * p[i + 1][1]
            + (t3 - t2) * h * self.m[i + 1];
        y.clamp(0.0, 1.0)
    }
}

/// Size of the curve lookup table sampled by the shader.
pub const LUT_SIZE: usize = 1024;

/// `LUT_SIZE` RGB entries: each channel's curve applied after the master curve.
pub fn curve_lut(c: &crate::edit::ToneCurve) -> Vec<[f32; 3]> {
    let master = Curve::new(&c.rgb);
    let chans = [
        Curve::new(&c.red),
        Curve::new(&c.green),
        Curve::new(&c.blue),
    ];
    (0..LUT_SIZE)
        .map(|i| {
            let m = master.eval(i as f32 / (LUT_SIZE - 1) as f32);
            [chans[0].eval(m), chans[1].eval(m), chans[2].eval(m)]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn identity_and_degenerate() {
        for pts in [vec![], vec![[0.3, 0.7]], vec![[0.0, 0.0], [1.0, 1.0]]] {
            let c = Curve::new(&pts);
            for x in [0.0, 0.25, 0.5, 1.0] {
                assert!((c.eval(x) - x).abs() < 1e-6);
            }
        }
        assert!(Curve::is_identity(&[]));
        assert!(!Curve::is_identity(&[[0.0, 0.1], [1.0, 1.0]]));
    }

    #[test]
    fn passes_through_points() {
        let pts = [[0.0, 0.0], [0.25, 0.15], [0.75, 0.85], [1.0, 1.0]];
        let c = Curve::new(&pts);
        for p in pts {
            assert!((c.eval(p[0]) - p[1]).abs() < 1e-5, "{p:?}");
        }
    }

    proptest! {
        /// Increasing control points give a non-decreasing curve (no overshoot).
        #[test]
        fn monotone_points_give_monotone_curve(mut ys in proptest::collection::vec(0.0f32..1.0, 2..7)) {
            ys.sort_by(f32::total_cmp);
            let n = ys.len();
            let pts: Vec<Point> = ys.iter().enumerate().map(|(i, &y)| [i as f32 / (n - 1) as f32, y]).collect();
            let c = Curve::new(&pts);
            let mut last = -1.0;
            for i in 0..=200 {
                let y = c.eval(i as f32 / 200.0);
                prop_assert!(y >= last - 1e-5, "dip at {i}: {y} < {last}");
                last = y;
            }
        }
    }
}
