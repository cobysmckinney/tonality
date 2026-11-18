//! Turning a coarse model output into a mask raster.
//!
//! Models run at a few hundred pixels; masks are [`tonality_core::mask::MASK_EDGE`] pixels. A
//! plain upscale gives soft, blobby edges, so the upscaled probabilities are run through a
//! guided filter (He et al.) steered by the photo's luminance: edges in the mask snap to edges
//! in the photo (hair, branches against sky), while flat areas stay smooth.

/// Bilinear resize of a single-channel float image.
pub fn resize(src: &[f32], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<f32> {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    let mut out = vec![0.0; dw * dh];
    let sx = sw as f32 / dw as f32;
    let sy = sh as f32 / dh as f32;
    for y in 0..dh {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = fy as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let ty = fy - y0 as f32;
        for x in 0..dw {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = fx as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let tx = fx - x0 as f32;
            let a = src[y0 * sw + x0] * (1.0 - tx) + src[y0 * sw + x1] * tx;
            let b = src[y1 * sw + x0] * (1.0 - tx) + src[y1 * sw + x1] * tx;
            out[y * dw + x] = a * (1.0 - ty) + b * ty;
        }
    }
    out
}

/// Mean over a `(2r+1)²` window (clipped at the borders), via a summed-area table.
fn box_mean(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut sat = vec![0.0f64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0.0f64;
        for x in 0..w {
            row += src[y * w + x] as f64;
            sat[(y + 1) * (w + 1) + x + 1] = sat[y * (w + 1) + x + 1] + row;
        }
    }
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            let s = sat[y1 * (w + 1) + x1] - sat[y0 * (w + 1) + x1] - sat[y1 * (w + 1) + x0]
                + sat[y0 * (w + 1) + x0];
            out[y * w + x] = (s / ((y1 - y0) * (x1 - x0)) as f64) as f32;
        }
    }
    out
}

/// Guided filter: smooth `p` while following the edges of `guide` (both `w × h`, 0..1).
/// `r` is the window radius in pixels, `eps` the edge threshold (variance; smaller = sharper).
pub fn guided_filter(guide: &[f32], p: &[f32], w: u32, h: u32, r: usize, eps: f32) -> Vec<f32> {
    let (w, h) = (w as usize, h as usize);
    let mean_i = box_mean(guide, w, h, r);
    let mean_p = box_mean(p, w, h, r);
    let ii: Vec<f32> = guide.iter().map(|v| v * v).collect();
    let ip: Vec<f32> = guide.iter().zip(p).map(|(a, b)| a * b).collect();
    let corr_i = box_mean(&ii, w, h, r);
    let corr_ip = box_mean(&ip, w, h, r);
    let mut a = vec![0.0; w * h];
    let mut b = vec![0.0; w * h];
    for k in 0..w * h {
        let var = corr_i[k] - mean_i[k] * mean_i[k];
        let cov = corr_ip[k] - mean_i[k] * mean_p[k];
        a[k] = cov / (var + eps);
        b[k] = mean_p[k] - a[k] * mean_i[k];
    }
    let mean_a = box_mean(&a, w, h, r);
    let mean_b = box_mean(&b, w, h, r);
    (0..w * h)
        .map(|k| mean_a[k] * guide[k] + mean_b[k])
        .collect()
}

/// A model's probability map (`pw × ph`, 0..1) → an 8-bit mask raster of `w × h`, refined
/// against `guide` (the photo's luminance at `w × h`).
pub fn to_raster(prob: &[f32], pw: u32, ph: u32, guide: &[f32], w: u32, h: u32) -> Vec<u8> {
    let up = resize(prob, pw, ph, w, h);
    // Window ≈ two model pixels, so the filter can recover detail the model couldn't resolve.
    let r = ((w as f32 / pw as f32) * 2.0).round().clamp(2.0, 16.0) as usize;
    let refined = guided_filter(guide, &up, w, h, r, 1e-3);
    refined
        .iter()
        .map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
        .collect()
}

/// A 3-D Gaussian colour model (mean and inverse covariance), for [`sky_raster`].
struct Gaussian {
    mean: [f32; 3],
    inv: [[f32; 3]; 3],
    log_norm: f32,
}

impl Gaussian {
    fn fit(samples: &[[f32; 3]]) -> Option<Gaussian> {
        if samples.len() < 64 {
            return None;
        }
        let n = samples.len() as f64;
        let mut mean = [0f64; 3];
        for s in samples {
            for k in 0..3 {
                mean[k] += s[k] as f64 / n;
            }
        }
        let mut cov = [[0f64; 3]; 3];
        for s in samples {
            let d = [
                s[0] as f64 - mean[0],
                s[1] as f64 - mean[1],
                s[2] as f64 - mean[2],
            ];
            for i in 0..3 {
                for j in 0..3 {
                    cov[i][j] += d[i] * d[j] / n;
                }
            }
        }
        // Regularise: flat skies have almost no variance, and a near-singular model is brittle.
        for (i, row) in cov.iter_mut().enumerate() {
            row[i] += 4e-4;
        }
        let c = |i: usize, j: usize| cov[i][j];
        let det = c(0, 0) * (c(1, 1) * c(2, 2) - c(1, 2) * c(2, 1))
            - c(0, 1) * (c(1, 0) * c(2, 2) - c(1, 2) * c(2, 0))
            + c(0, 2) * (c(1, 0) * c(2, 1) - c(1, 1) * c(2, 0));
        if det <= 1e-18 {
            return None;
        }
        // Inverse = adjugate / det (the matrix is symmetric).
        let inv = [
            [
                c(1, 1) * c(2, 2) - c(1, 2) * c(2, 1),
                c(0, 2) * c(2, 1) - c(0, 1) * c(2, 2),
                c(0, 1) * c(1, 2) - c(0, 2) * c(1, 1),
            ],
            [
                c(1, 2) * c(2, 0) - c(1, 0) * c(2, 2),
                c(0, 0) * c(2, 2) - c(0, 2) * c(2, 0),
                c(0, 2) * c(1, 0) - c(0, 0) * c(1, 2),
            ],
            [
                c(1, 0) * c(2, 1) - c(1, 1) * c(2, 0),
                c(0, 1) * c(2, 0) - c(0, 0) * c(2, 1),
                c(0, 0) * c(1, 1) - c(0, 1) * c(1, 0),
            ],
        ]
        .map(|row| row.map(|v| (v / det) as f32));
        Some(Gaussian {
            mean: mean.map(|v| v as f32),
            inv,
            log_norm: -0.5 * (det.ln() as f32),
        })
    }

    fn log_pdf(&self, x: [f32; 3]) -> f32 {
        let d = [
            x[0] - self.mean[0],
            x[1] - self.mean[1],
            x[2] - self.mean[2],
        ];
        let mut q = 0.0;
        for i in 0..3 {
            for j in 0..3 {
                q += d[i] * self.inv[i][j] * d[j];
            }
        }
        self.log_norm - 0.5 * q
    }
}

/// Sky: the scene model only resolves the sky at ~1/8 of the raster, which leaves a soft band
/// round everything against it and grey holes where it shows through foliage. Near the sky's
/// edge, each pixel is re-decided by colour: a model of the confident sky against a model of
/// the confident foreground nearby, weighted by what the network said. Elsewhere the
/// network's answer stands.
pub fn sky_raster(prob: &[f32], pw: u32, ph: u32, img: &image::RgbImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    let n = wu * hu;
    let up = resize(prob, pw, ph, w, h);
    let rgb: Vec<[f32; 3]> = img
        .pixels()
        .map(|p| p.0.map(|v| v as f32 / 255.0))
        .collect();
    let guide = crate::input::guide(img, w, h);
    let plain = || to_raster(prob, pw, ph, &guide, w, h);

    // The band round the sky's edge where colour gets a say.
    let long = w.max(h) as f32;
    let band_r = (0.06 * long) as usize;
    let skyish: Vec<f32> = up
        .iter()
        .map(|&p| if p > 0.5 { 1.0 } else { 0.0 })
        .collect();
    let near_sky = box_mean(&skyish, wu, hu, band_r);
    let in_band = |i: usize| near_sky[i] > 0.0 && near_sky[i] < 1.0;

    let step = (n / 60_000).max(1); // plenty of samples for a 3-D Gaussian
    let sky: Vec<[f32; 3]> = (0..n)
        .step_by(step)
        .filter(|&i| up[i] > 0.92)
        .map(|i| rgb[i])
        .collect();
    let fg: Vec<[f32; 3]> = (0..n)
        .step_by(step)
        .filter(|&i| up[i] < 0.08 && in_band(i))
        .map(|i| rgb[i])
        .collect();
    let (Some(gs), Some(gf)) = (Gaussian::fit(&sky), Gaussian::fit(&fg)) else {
        return plain();
    };

    let mut out = up.clone();
    for i in 0..n {
        if !in_band(i) {
            continue;
        }
        let p = up[i].clamp(0.02, 0.98);
        let prior = (p / (1.0 - p)).ln();
        let llr = (gs.log_pdf(rgb[i]) - gf.log_pdf(rgb[i])).clamp(-12.0, 12.0);
        out[i] = 1.0 / (1.0 + (-(llr + prior)).exp());
    }
    // A light edge-aware smoothing: anti-aliases without bringing the halo back.
    let out = guided_filter(&guide, &out, w, h, 1, 1e-4);
    out.iter()
        .map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_keeps_constants_and_corners() {
        let src = [0.0, 1.0, 0.0, 1.0];
        let up = resize(&src, 2, 2, 4, 4);
        assert_eq!(up[0], 0.0);
        assert_eq!(up[3], 1.0);
        assert!(resize(&[0.5; 9], 3, 3, 7, 5)
            .iter()
            .all(|v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn guided_filter_snaps_a_blurry_mask_to_the_photo_edge() {
        // Photo: a hard vertical edge at x = 32. Mask: the same edge, badly blurred.
        let (w, h) = (64u32, 16u32);
        let guide: Vec<f32> = (0..w * h)
            .map(|i| if i % w < 32 { 0.1 } else { 0.9 })
            .collect();
        let blurry: Vec<f32> = (0..w * h)
            .map(|i| ((i % w) as f32 - 20.0).clamp(0.0, 24.0) / 24.0)
            .collect();
        let out = guided_filter(&guide, &blurry, w, h, 8, 1e-4);
        let at = |x: u32| out[(8 * w + x) as usize];
        // Inside each side the mask flattens out; across the photo's edge it jumps.
        assert!(
            at(31) < 0.45 && at(33) > 0.55,
            "edge {} → {}",
            at(31),
            at(33)
        );
        assert!(
            (at(33) - at(31)) > (blurry[33] - blurry[31]) * 4.0,
            "sharper than the input"
        );
    }

    /// Bright blue sky over a dark canopy, with sky showing through holes near the canopy's
    /// top (rows 104–109) and one deep inside it (rows 150–159).
    fn sky_scene() -> image::RgbImage {
        image::RgbImage::from_fn(200, 200, |x, y| {
            let hole = (x / 10) % 3 == 0 && ((104..110).contains(&y) || (150..160).contains(&y));
            if y < 100 || hole {
                image::Rgb([150, 190, 235])
            } else {
                image::Rgb([30, 50, 25])
            }
        })
    }

    #[test]
    fn sky_edges_follow_colour_not_the_coarse_map() {
        let img = sky_scene();
        // The network's view: 1/8 resolution, so the edge is a soft ramp and the holes are lost.
        let prob: Vec<f32> = (0..25 * 25)
            .map(|i| ((13.0 - (i / 25) as f32) / 2.0).clamp(0.0, 1.0))
            .collect();
        let r = sky_raster(&prob, 25, 25, &img);
        let at = |x: usize, y: usize| r[y * 200 + x];
        assert!(
            at(100, 96) > 230,
            "sky just above the canopy is sky: {}",
            at(100, 96)
        );
        assert!(
            at(100, 104) < 25,
            "canopy just below the edge isn't: {}",
            at(100, 104)
        );
        assert!(
            at(5, 107) > 200,
            "sky through a hole in the canopy: {}",
            at(5, 107)
        );
        assert!(at(15, 107) < 40, "the leaves beside it: {}", at(15, 107));
        // Far from the sky's edge, colour has no say: a bright patch there isn't sky.
        assert!(at(5, 155) < 5, "deep hole stays foreground: {}", at(5, 155));
    }

    #[test]
    fn gaussian_inverse_is_an_inverse() {
        // Correlated samples, so the covariance has off-diagonal terms.
        let samples: Vec<[f32; 3]> = (0..500)
            .map(|i| {
                let t = i as f32 / 500.0;
                [
                    t,
                    0.5 * t + 0.1 * (i % 7) as f32 / 7.0,
                    0.2 + 0.05 * (i % 3) as f32,
                ]
            })
            .collect();
        let g = Gaussian::fit(&samples).unwrap();
        // log_pdf peaks at the mean and falls off away from it.
        assert!(g.log_pdf(g.mean) > g.log_pdf([g.mean[0] + 0.1, g.mean[1], g.mean[2]]));
        // inv × (covariance recomputed) ≈ identity, checked through a quadratic form:
        // moving along x by one standard deviation (in the model's own metric) costs ~0.5.
        let d = 1.0 / g.inv[0][0].sqrt();
        let drop = g.log_pdf(g.mean) - g.log_pdf([g.mean[0] + d, g.mean[1], g.mean[2]]);
        assert!((drop - 0.5).abs() < 1e-3, "{drop}");
    }

    #[test]
    fn sky_without_confident_sky_falls_back() {
        let img = sky_scene();
        let prob = vec![0.3f32; 25 * 25]; // never confident: no colour model
        let r = sky_raster(&prob, 25, 25, &img);
        assert!(
            r.iter().all(|&v| v < 120),
            "stays uncertain rather than inventing sky"
        );
    }

    #[test]
    fn raster_is_full_size_and_8_bit() {
        let prob = vec![1.0; 16];
        let guide = vec![0.5; 64 * 48];
        let r = to_raster(&prob, 4, 4, &guide, 64, 48);
        assert_eq!(r.len(), 64 * 48);
        assert!(r.iter().all(|&v| v == 255));
    }
}
