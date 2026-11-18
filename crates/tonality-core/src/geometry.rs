//! Framing geometry: how a [`Crop`] maps output pixels to the source image.
//!
//! Steps, in order: turn the source by quarter turns, mirror it, rotate it by `angle` about the
//! frame centre, then crop. All of these are affine, so the whole mapping from the output frame
//! (`f` in 0..1²) to source coordinates (`uv` in 0..1²) is a single 2×3 matrix. The shader
//! applies that matrix, so preview, zoomed tiles and export can't disagree about framing.

use crate::edit::Crop;

/// The framed output: its size in source pixels and the output → source mapping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// Output size in (full-resolution) source pixels.
    pub width: f32,
    pub height: f32,
    /// `uv = m · [f.x, f.y, 1]`, rows for u and v.
    pub m: [[f32; 3]; 2],
}

impl Geometry {
    pub fn new(crop: &Crop, src_w: f32, src_h: f32) -> Self {
        let (tw, th) = turned_size(crop, src_w, src_h);
        let map = |fx: f32, fy: f32| source_uv(crop, src_w, src_h, fx, fy);
        let o = map(0.0, 0.0);
        let x = map(1.0, 0.0);
        let y = map(0.0, 1.0);
        let r = rect(crop);
        Self {
            width: (r[2] * tw).max(1.0),
            height: (r[3] * th).max(1.0),
            m: [
                [x[0] - o[0], y[0] - o[0], o[0]],
                [x[1] - o[1], y[1] - o[1], o[1]],
            ],
        }
    }

    pub fn apply(&self, f: [f32; 2]) -> [f32; 2] {
        let [a, b] = self.m;
        [
            a[0] * f[0] + a[1] * f[1] + a[2],
            b[0] * f[0] + b[1] * f[1] + b[2],
        ]
    }

    /// Bounding box `[u0, v0, u1, v1]` of the source area that output region `f0..f1` reads.
    pub fn source_bounds(&self, f0: [f32; 2], f1: [f32; 2]) -> [f32; 4] {
        let pts = [f0, [f1[0], f0[1]], [f0[0], f1[1]], f1].map(|p| self.apply(p));
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for [u, v] in pts {
            b = [b[0].min(u), b[1].min(v), b[2].max(u), b[3].max(v)];
        }
        b
    }
}

/// Size of the frame after quarter turns (pixels).
pub fn turned_size(crop: &Crop, src_w: f32, src_h: f32) -> (f32, f32) {
    if crop.turns % 2 == 1 {
        (src_h, src_w)
    } else {
        (src_w, src_h)
    }
}

/// The crop rect, sanitised to lie within the frame.
fn rect(crop: &Crop) -> [f32; 4] {
    let [x, y, w, h] = crop.rect;
    let w = w.clamp(1e-4, 1.0);
    let h = h.clamp(1e-4, 1.0);
    [x.clamp(0.0, 1.0 - w), y.clamp(0.0, 1.0 - h), w, h]
}

/// Source uv for output-frame position `f`.
fn source_uv(crop: &Crop, src_w: f32, src_h: f32, fx: f32, fy: f32) -> [f32; 2] {
    let (tw, th) = turned_size(crop, src_w, src_h);
    let r = rect(crop);
    // Output → position in the straightened, turned frame (pixels).
    let qx = (r[0] + fx * r[2]) * tw;
    let qy = (r[1] + fy * r[3]) * th;
    // Undo the straighten rotation about the frame centre (y points down, so a positive angle
    // turns the image clockwise on screen).
    let (s, c) = (-crop.angle.to_radians()).sin_cos();
    let (dx, dy) = (qx - tw / 2.0, qy - th / 2.0);
    let mut x = (tw / 2.0 + c * dx - s * dy) / tw;
    let y = (th / 2.0 + s * dx + c * dy) / th;
    if crop.flip {
        x = 1.0 - x;
    }
    // Undo the quarter turns (clockwise).
    match crop.turns % 4 {
        1 => [y, 1.0 - x],
        2 => [1.0 - x, 1.0 - y],
        3 => [1.0 - y, x],
        _ => [x, y],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    #[test]
    fn default_is_identity() {
        let g = Geometry::new(&Crop::default(), 6000.0, 4000.0);
        assert_eq!((g.width, g.height), (6000.0, 4000.0));
        assert!(close(g.apply([0.0, 0.0]), [0.0, 0.0]));
        assert!(close(g.apply([1.0, 1.0]), [1.0, 1.0]));
        assert!(close(g.apply([0.25, 0.75]), [0.25, 0.75]));
    }

    #[test]
    fn quarter_turn_clockwise() {
        let crop = Crop {
            turns: 1,
            ..Crop::default()
        };
        let g = Geometry::new(&crop, 6000.0, 4000.0);
        assert_eq!((g.width, g.height), (4000.0, 6000.0));
        // After turning clockwise, the output's top-left shows the source's bottom-left.
        assert!(close(g.apply([0.0, 0.0]), [0.0, 1.0]));
        assert!(close(g.apply([1.0, 0.0]), [0.0, 0.0]));
        assert!(close(g.apply([1.0, 1.0]), [1.0, 0.0]));
    }

    #[test]
    fn flip_mirrors() {
        let crop = Crop {
            flip: true,
            ..Crop::default()
        };
        let g = Geometry::new(&crop, 100.0, 50.0);
        assert!(close(g.apply([0.0, 0.3]), [1.0, 0.3]));
    }

    #[test]
    fn crop_selects_a_region() {
        let crop = Crop {
            rect: [0.5, 0.25, 0.5, 0.5],
            ..Crop::default()
        };
        let g = Geometry::new(&crop, 6000.0, 4000.0);
        assert_eq!((g.width, g.height), (3000.0, 2000.0));
        assert!(close(g.apply([0.0, 0.0]), [0.5, 0.25]));
        assert!(close(g.apply([1.0, 1.0]), [1.0, 0.75]));
    }

    #[test]
    fn rotation_keeps_the_centre_and_preserves_pixel_distances() {
        let crop = Crop {
            angle: 10.0,
            rect: [0.2, 0.2, 0.6, 0.6],
            ..Crop::default()
        };
        let (w, h) = (6000.0, 4000.0);
        let g = Geometry::new(&crop, w, h);
        assert!(close(g.apply([0.5, 0.5]), [0.5, 0.5]));
        // Moving 1 output pixel moves 1 source pixel (rotation, no scaling).
        let a = g.apply([0.5, 0.5]);
        let b = g.apply([0.5 + 1.0 / g.width, 0.5]);
        let d = (((b[0] - a[0]) * w).powi(2) + ((b[1] - a[1]) * h).powi(2)).sqrt();
        assert!((d - 1.0).abs() < 1e-2, "{d}");
        // A clockwise image rotation means moving right on screen moves "up" in the source.
        assert!(b[1] < a[1]);
    }

    #[test]
    fn bounds_cover_the_footprint() {
        let crop = Crop {
            angle: 30.0,
            rect: [0.3, 0.3, 0.4, 0.4],
            ..Crop::default()
        };
        let g = Geometry::new(&crop, 1000.0, 1000.0);
        let b = g.source_bounds([0.0, 0.0], [1.0, 1.0]);
        for f in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [0.5, 0.5]] {
            let [u, v] = g.apply(f);
            assert!(u >= b[0] - 1e-6 && u <= b[2] + 1e-6 && v >= b[1] - 1e-6 && v <= b[3] + 1e-6);
        }
    }
}
