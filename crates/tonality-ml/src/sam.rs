//! Select-object (MobileSAM): circle something, get its mask.
//!
//! The encoder runs once per photo (its embedding is cached by the caller); the decoder is fast
//! and runs per prompt. Prompts are the circled loop's bounding box plus a point inside it, and
//! the result is limited to a little beyond the loop, so nothing outside the circle is selected.

use std::path::Path;

use image::{imageops, RgbImage};

use crate::session::{run, Tensor32};
use crate::Result;

/// Encoder input edge: images are resized so their long side is this.
pub const SAM_EDGE: u32 = 1024;

/// The image embedding (`[1, 256, 64, 64]`) of `img`, which must already have its long side
/// at [`SAM_EDGE`] (the mask raster size).
pub fn encode(encoder: &Path, img: &RgbImage) -> Result<Tensor32> {
    let img = if img.width().max(img.height()) == SAM_EDGE {
        img.clone()
    } else {
        let s = SAM_EDGE as f32 / img.width().max(img.height()) as f32;
        let (w, h) = (
            (img.width() as f32 * s).round() as u32,
            (img.height() as f32 * s).round() as u32,
        );
        imageops::resize(img, w, h, imageops::FilterType::Triangle)
    };
    // HWC, RGB 0..255: the graph normalises and pads itself.
    let data: Vec<f32> = img.as_raw().iter().map(|&v| v as f32).collect();
    let x = Tensor32::new([img.height() as i64, img.width() as i64, 3], data);
    Ok(run(encoder, vec![("input_image", x)], &["image_embeddings"])?.remove(0))
}

/// The object inside `lasso` (closed loop, 0..1 across the image), as a `w × h` 0..1 mask.
/// `w × h` is the encoded image's size (long side [`SAM_EDGE`]).
pub fn decode(
    decoder: &Path,
    embedding: &Tensor32,
    lasso: &[[f32; 2]],
    w: u32,
    h: u32,
) -> Result<Vec<f32>> {
    let (x0, y0, x1, y1) = lasso
        .iter()
        .fold((1f32, 1f32, 0f32, 0f32), |(a, b, c, d), p| {
            (a.min(p[0]), b.min(p[1]), c.max(p[0]), d.max(p[1]))
        });
    let (fw, fh) = (w as f32, h as f32);
    let (cx, cy) = interior_point(lasso);
    // Box corners (labels 2, 3) and a positive point (1).
    let coords = vec![x0 * fw, y0 * fh, x1 * fw, y1 * fh, cx * fw, cy * fh];
    let inputs = vec![
        ("image_embeddings", embedding.clone()),
        ("point_coords", Tensor32::new([1, 3, 2], coords)),
        ("point_labels", Tensor32::new([1, 3], vec![2.0, 3.0, 1.0])),
        (
            "mask_input",
            Tensor32::new([1, 1, 256, 256], vec![0.0; 256 * 256]),
        ),
        ("has_mask_input", Tensor32::new([1], vec![0.0])),
        ("orig_im_size", Tensor32::new([2], vec![fh, fw])),
    ];
    let masks = run(decoder, inputs, &["masks"])?.remove(0);
    let limit = polygon_mask(lasso, w, h, 0.03);
    Ok(masks
        .data
        .iter()
        .zip(&limit)
        .map(|(&l, &k)| k / (1.0 + (-l).exp()))
        .collect())
}

/// A point well inside the loop: its centroid if that's inside, else the inside sample nearest it.
fn interior_point(lasso: &[[f32; 2]]) -> (f32, f32) {
    let n = lasso.len().max(1) as f32;
    let c = lasso
        .iter()
        .fold([0.0, 0.0], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n]);
    if inside(lasso, c[0], c[1]) {
        return (c[0], c[1]);
    }
    let mut best = (c[0], c[1], f32::MAX);
    for i in 0..32 {
        for j in 0..32 {
            let (x, y) = ((i as f32 + 0.5) / 32.0, (j as f32 + 0.5) / 32.0);
            let d = (x - c[0]).powi(2) + (y - c[1]).powi(2);
            if d < best.2 && inside(lasso, x, y) {
                best = (x, y, d);
            }
        }
    }
    (best.0, best.1)
}

fn inside(poly: &[[f32; 2]], x: f32, y: f32) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
    }
    c
}

/// 1 inside `poly` and within `grow` (fraction of the long edge) of it, fading to 0 over another
/// `grow`.
pub fn polygon_mask(poly: &[[f32; 2]], w: u32, h: u32, grow: f32) -> Vec<f32> {
    let long = w.max(h) as f32;
    let g = grow * long;
    let pts: Vec<[f32; 2]> = poly
        .iter()
        .map(|p| [p[0] * w as f32, p[1] * h as f32])
        .collect();
    let seg_dist = |x: f32, y: f32| -> f32 {
        let n = pts.len();
        (0..n)
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let t = (((x - a[0]) * dx + (y - a[1]) * dy) / (dx * dx + dy * dy).max(1e-9))
                    .clamp(0.0, 1.0);
                ((x - a[0] - t * dx).powi(2) + (y - a[1] - t * dy).powi(2)).sqrt()
            })
            .fold(f32::MAX, f32::min)
    };
    let mut out = vec![0.0; (w * h) as usize];
    if pts.len() < 3 {
        return out;
    }
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            out[(y * w + x) as usize] = if inside(&pts, px, py) {
                1.0
            } else {
                let d = seg_dist(px, py);
                if d <= g {
                    1.0
                } else {
                    (1.0 - (d - g) / g).max(0.0)
                }
            };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: [[f32; 2]; 4] = [[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]];

    #[test]
    fn polygon_mask_covers_the_loop_and_a_margin() {
        let m = polygon_mask(&SQUARE, 100, 100, 0.05);
        assert_eq!(m[50 * 100 + 50], 1.0, "inside");
        assert_eq!(m[50 * 100 + 22], 1.0, "just outside, within the margin");
        assert!(m[50 * 100 + 17] > 0.0 && m[50 * 100 + 17] < 1.0, "fading");
        assert_eq!(m[50 * 100 + 5], 0.0, "far outside");
    }

    #[test]
    fn interior_point_handles_concave_loops() {
        assert_eq!(interior_point(&SQUARE), (0.5, 0.5));
        // A "C" whose centroid falls in the gap.
        let c = [
            [0.1, 0.1],
            [0.9, 0.1],
            [0.9, 0.3],
            [0.3, 0.3],
            [0.3, 0.7],
            [0.9, 0.7],
            [0.9, 0.9],
            [0.1, 0.9],
        ];
        let (x, y) = interior_point(&c);
        assert!(inside(&c, x, y));
    }
}
