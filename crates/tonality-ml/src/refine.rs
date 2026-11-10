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
            let s = sat[y1 * (w + 1) + x1] - sat[y0 * (w + 1) + x1] - sat[y1 * (w + 1) + x0] + sat[y0 * (w + 1) + x0];
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
    (0..w * h).map(|k| mean_a[k] * guide[k] + mean_b[k]).collect()
}

/// A model's probability map (`pw × ph`, 0..1) → an 8-bit mask raster of `w × h`, refined
/// against `guide` (the photo's luminance at `w × h`).
pub fn to_raster(prob: &[f32], pw: u32, ph: u32, guide: &[f32], w: u32, h: u32) -> Vec<u8> {
    let up = resize(prob, pw, ph, w, h);
    // Window ≈ two model pixels, so the filter can recover detail the model couldn't resolve.
    let r = ((w as f32 / pw as f32) * 2.0).round().clamp(2.0, 16.0) as usize;
    let refined = guided_filter(guide, &up, w, h, r, 1e-3);
    refined.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect()
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
        assert!(resize(&[0.5; 9], 3, 3, 7, 5).iter().all(|v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn guided_filter_snaps_a_blurry_mask_to_the_photo_edge() {
        // Photo: a hard vertical edge at x = 32. Mask: the same edge, badly blurred.
        let (w, h) = (64u32, 16u32);
        let guide: Vec<f32> = (0..w * h).map(|i| if i % w < 32 { 0.1 } else { 0.9 }).collect();
        let blurry: Vec<f32> = (0..w * h).map(|i| ((i % w) as f32 - 20.0).clamp(0.0, 24.0) / 24.0).collect();
        let out = guided_filter(&guide, &blurry, w, h, 8, 1e-4);
        let at = |x: u32| out[(8 * w + x) as usize];
        // Inside each side the mask flattens out; across the photo's edge it jumps.
        assert!(at(31) < 0.45 && at(33) > 0.55, "edge {} → {}", at(31), at(33));
        assert!((at(33) - at(31)) > (blurry[33] - blurry[31]) * 4.0, "sharper than the input");
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
