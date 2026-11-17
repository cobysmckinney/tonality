use tonality_core::color::CameraColor;
use tonality_core::geometry::Geometry;
use tonality_core::lens::{lens_table, LensProfile, LENS_KNOTS};
use tonality_core::mask::{Mask, MaskShape};
use tonality_core::EditState;

/// Masks the shader evaluates (more are ignored).
pub const MAX_MASKS: usize = 16;
/// vec4s per mask in the uniform block.
const MASK_VEC4S: usize = 8;

/// Uniform block matching `struct Params` in `develop.wgsl` (std140-compatible: all vec4).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DevelopParams {
    pub wb: [f32; 4],
    pub cam: [[f32; 4]; 3],
    pub light0: [f32; 4],
    pub light1: [f32; 4],
    pub color0: [f32; 4],
    pub mix_hue: [[f32; 4]; 2],
    pub mix_sat: [[f32; 4]; 2],
    pub mix_lum: [[f32; 4]; 2],
    pub vignette: [f32; 4],
    pub grain: [f32; 4],
    pub detail0: [f32; 4],
    pub detail1: [f32; 4],
    pub geo0: [f32; 4],
    pub geo1: [f32; 4],
    pub masks_head: [f32; 4],
    pub masks: [[[f32; 4]; MASK_VEC4S]; MAX_MASKS],
    /// x: active, y: tile margin (source uv), zw: source size / its diagonal.
    pub lens_head: [f32; 4],
    /// Per radius knot: red, green, blue source/output radius ratio; vignetting gain.
    pub lens: [[f32; 4]; LENS_KNOTS],
}

impl DevelopParams {
    pub const FLOATS: usize = 80 + MAX_MASKS * MASK_VEC4S * 4 + 4 + LENS_KNOTS * 4;

    /// `src_w × src_h`: the full-resolution source size (framing is computed in its pixels).
    /// `lens`: the photo's lens profile, if known.
    pub fn new(edit: &EditState, color: &CameraColor, lens: Option<&LensProfile>, src_w: u32, src_h: u32) -> Self {
        let (w, h) = (src_w.max(1) as f32, src_h.max(1) as f32);
        let lt = lens_table(lens, &edit.lens, w, h);
        let diag = w.hypot(h);
        let geo = Geometry::new(&edit.crop, src_w as f32, src_h as f32);
        let (shot_t, shot_tint) = color.as_shot_temperature_tint();
        let wbs = &edit.white_balance;
        let wb = if wbs.temperature.is_none() && wbs.tint.is_none() {
            color.as_shot_multipliers()
        } else {
            color.wb_for(wbs.temperature.unwrap_or(shot_t), wbs.tint.unwrap_or(shot_tint))
        };
        let m = color.cam_to_working();
        let l = &edit.light;
        let n = |v: f32| (v / 100.0).clamp(-1.0, 1.0);
        let u = |v: f32| (v / 100.0).clamp(0.0, 1.0);
        let c = &edit.color;
        let bands = c.mixer.bands();
        let (v, g) = (&edit.effects.vignette, &edit.effects.grain);
        let d = &edit.detail;
        Self {
            wb: [wb[0], wb[1], wb[2], 1.0],
            cam: [
                [m[0][0], m[0][1], m[0][2], 0.0],
                [m[1][0], m[1][1], m[1][2], 0.0],
                [m[2][0], m[2][1], m[2][2], 0.0],
            ],
            light0: [l.exposure.clamp(-10.0, 10.0).exp2(), n(l.contrast), n(l.highlights), n(l.shadows)],
            light1: [n(l.whites), n(l.blacks), 0.0, 0.0],
            color0: [n(c.saturation), n(c.vibrance), 0.0, 0.0],
            mix_hue: split(bands.map(|b| n(b.hue))),
            mix_sat: split(bands.map(|b| n(b.saturation))),
            mix_lum: split(bands.map(|b| n(b.luminance))),
            vignette: [n(v.amount), u(v.midpoint), u(v.feather), 0.0],
            grain: [u(g.amount), u(g.size), 0.0, 0.0],
            detail0: [
                (d.sharpen_amount / 100.0).clamp(0.0, 1.5) * 1.6,
                d.sharpen_radius.clamp(0.5, 3.0),
                u(d.sharpen_masking),
                u(d.luminance_noise),
            ],
            detail1: [u(d.color_noise), 0.0, 0.0, 0.0],
            geo0: [geo.m[0][0], geo.m[0][1], geo.m[0][2], geo.width],
            geo1: [geo.m[1][0], geo.m[1][1], geo.m[1][2], geo.height],
            masks_head: [0.0; 4],
            masks: [[[0.0; 4]; MASK_VEC4S]; MAX_MASKS],
            lens_head: [if lt.active { 1.0 } else { 0.0 }, lt.margin, w / diag, h / diag],
            lens: lt.knots,
        }
        .with_masks(&edit.masks, src_w, src_h)
    }

    fn with_masks(mut self, masks: &[Mask], src_w: u32, src_h: u32) -> Self {
        let long = src_w.max(src_h).max(1) as f32;
        let norm = [src_w as f32 / long, src_h as f32 / long];
        let q = |p: [f32; 2]| [p[0] * norm[0], p[1] * norm[1]];
        // Clamped like the global sliders: a sidecar from elsewhere can hold anything, and an
        // out-of-range value overflows the shader's exp2 into NaN.
        let n = |v: f32| if v.is_finite() { (v / 100.0).clamp(-1.0, 1.0) } else { 0.0 };
        let mut count = 0;
        let mut layer = 0.0;
        let mut local_detail = false;
        for m in masks {
            let brush_layer = layer;
            if m.shape.is_raster() {
                layer += 1.0;
            }
            if !m.enabled || count == MAX_MASKS {
                continue;
            }
            let (kind, shape0, shape1) = match &m.shape {
                MaskShape::Linear { from, to } => {
                    let (a, b) = (q(*from), q(*to));
                    (1.0, [a[0], a[1], b[0], b[1]], [0.0; 4])
                }
                MaskShape::Radial { center, radius, angle, feather } => {
                    let c = q(*center);
                    let (s, co) = angle.to_radians().sin_cos();
                    (2.0, [c[0], c[1], radius[0].max(1e-4), radius[1].max(1e-4)], [co, s, feather.clamp(0.0, 1.0), 0.0])
                }
                MaskShape::Brush { .. } | MaskShape::Ai { .. } => (3.0, [0.0; 4], [0.0; 4]),
            };
            let r = &m.range;
            let a = &m.adjust;
            self.masks[count] = [
                shape0,
                shape1,
                [kind, if m.invert { 1.0 } else { 0.0 }, (m.amount / 100.0).clamp(0.0, 1.0), brush_layer],
                [n(r.low).clamp(0.0, 1.0), n(r.high).clamp(0.0, 1.0), n(r.smoothness).clamp(0.0, 1.0), if r.is_full() { 0.0 } else { 1.0 }],
                [if a.exposure.is_finite() { a.exposure.clamp(-5.0, 5.0) } else { 0.0 }, n(a.contrast), n(a.highlights), n(a.shadows)],
                [n(a.whites), n(a.blacks), n(a.temperature), n(a.tint)],
                [n(a.saturation), n(a.vibrance), n(a.hue), n(a.sharpness)],
                [
                    n(a.noise),
                    if a.color_hue.is_finite() { a.color_hue.rem_euclid(360.0).to_radians() } else { 0.0 },
                    n(a.color_amount).max(0.0),
                    0.0,
                ],
            ];
            local_detail |= n(a.sharpness) != 0.0 || n(a.noise) != 0.0;
            count += 1;
        }
        // w: some mask sharpens or denoises, so the detail pass must look at the masks.
        self.masks_head = [count as f32, norm[0], norm[1], if local_detail { 1.0 } else { 0.0 }];
        self
    }

    pub fn to_vec(&self) -> Vec<f32> {
        let mut v = Vec::with_capacity(Self::FLOATS);
        v.extend_from_slice(&self.wb);
        for row in &self.cam {
            v.extend_from_slice(row);
        }
        v.extend_from_slice(&self.light0);
        v.extend_from_slice(&self.light1);
        v.extend_from_slice(&self.color0);
        for block in [&self.mix_hue, &self.mix_sat, &self.mix_lum] {
            v.extend_from_slice(&block[0]);
            v.extend_from_slice(&block[1]);
        }
        v.extend_from_slice(&self.vignette);
        v.extend_from_slice(&self.grain);
        v.extend_from_slice(&self.detail0);
        v.extend_from_slice(&self.detail1);
        v.extend_from_slice(&self.geo0);
        v.extend_from_slice(&self.geo1);
        v.extend_from_slice(&self.masks_head);
        for m in &self.masks {
            for row in m {
                v.extend_from_slice(row);
            }
        }
        v.extend_from_slice(&self.lens_head);
        for k in &self.lens {
            v.extend_from_slice(k);
        }
        v
    }
}

fn split(v: [f32; 8]) -> [[f32; 4]; 2] {
    [[v[0], v[1], v[2], v[3]], [v[4], v[5], v[6], v[7]]]
}

/// Width of the tone-curve lookup texture.
pub const CURVE_LUT_SIZE: usize = tonality_core::curve::LUT_SIZE;

/// The tone-curve lookup table as RGBA half floats (`LUT_SIZE` × 1), for a texture upload.
pub fn curve_lut_rgba16f(edit: &EditState) -> Vec<u8> {
    let lut = tonality_core::curve::curve_lut(&edit.curve);
    let mut out = Vec::with_capacity(lut.len() * 8);
    for [r, g, b] in lut {
        for v in [r, g, b, 1.0] {
            out.extend_from_slice(&half::f16::from_f32(v).to_bits().to_le_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color() -> CameraColor {
        CameraColor {
            xyz_to_cam: [[0.7657, -0.2847, -0.0607], [-0.4083, 1.1966, 0.2389], [-0.0684, 0.1418, 0.5844]],
            as_shot_wb: [1.6836, 1.0, 3.2539],
        }
    }

    #[test]
    fn mask_adjustments_are_clamped_and_finite() {
        let mut m = tonality_core::mask::Mask { shape: tonality_core::mask::MaskShape::Brush { strokes: vec![] }, ..Default::default() };
        m.adjust.contrast = 1e5;
        m.adjust.shadows = f32::NAN;
        m.adjust.exposure = f32::INFINITY;
        let e = EditState { masks: vec![m], ..Default::default() };
        let p = DevelopParams::new(&e, &color(), None, 600, 400);
        assert_eq!(p.masks[0][4], [0.0, 1.0, 0.0, 0.0]);
        assert!(p.to_vec().iter().all(|v| v.is_finite()));
    }

    #[test]
    fn local_color_and_detail_are_packed() {
        let mut m = Mask::default();
        m.adjust.vibrance = 50.0;
        m.adjust.hue = -100.0;
        m.adjust.sharpness = 40.0;
        m.adjust.noise = -20.0;
        m.adjust.color_hue = 450.0; // wraps to 90°
        m.adjust.color_amount = -10.0; // tint can't go negative
        let e = EditState { masks: vec![m], ..Default::default() };
        let p = DevelopParams::new(&e, &color(), None, 600, 400);
        assert_eq!(p.masks[0][6], [0.0, 0.5, -1.0, 0.4]);
        assert_eq!(p.masks[0][7], [-0.2, 90f32.to_radians(), 0.0, 0.0]);
        assert_eq!(p.masks_head[3], 1.0, "the detail pass must read the masks");

        let p = DevelopParams::new(&EditState { masks: vec![Mask::default()], ..Default::default() }, &color(), None, 600, 400);
        assert_eq!(p.masks_head[3], 0.0);
    }

    #[test]
    fn layout_matches_floats() {
        assert_eq!(DevelopParams::new(&EditState::default(), &color(), None, 600, 400).to_vec().len(), DevelopParams::FLOATS);
        assert_eq!(std::mem::size_of::<DevelopParams>(), DevelopParams::FLOATS * 4);
    }

    #[test]
    fn default_is_as_shot_and_neutral() {
        let p = DevelopParams::new(&EditState::default(), &color(), None, 600, 400);
        let m = color().as_shot_multipliers();
        assert_eq!(&p.wb[..3], &m[..]);
        assert_eq!(p.light0, [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn masks_skip_hidden_ones_but_keep_brush_layers() {
        let mut e = EditState::default();
        let brush = || Mask { shape: MaskShape::Brush { strokes: vec![] }, ..Mask::default() };
        e.masks = vec![Mask { enabled: false, ..brush() }, Mask::default(), brush()];
        e.masks[1].adjust.exposure = 1.0;
        let p = DevelopParams::new(&e, &color(), None, 600, 400);
        assert_eq!(p.masks_head, [2.0, 1.0, 400.0 / 600.0, 0.0]);
        assert_eq!(p.masks[0][2][0], 2.0); // radial
        assert_eq!(p.masks[0][4][0], 1.0);
        assert_eq!(p.masks[1][2], [3.0, 0.0, 1.0, 1.0]); // second brush mask: layer 1

        // AI masks are raster layers too, counted in order with brush masks.
        let ai = Mask {
            shape: MaskShape::Ai { target: tonality_core::mask::AiTarget::Sky, model: "scene@1".into(), refine: vec![] },
            ..Mask::default()
        };
        e.masks = vec![ai.clone(), brush(), Mask { invert: true, ..ai }];
        let p = DevelopParams::new(&e, &color(), None, 600, 400);
        assert_eq!(p.masks[0][2], [3.0, 0.0, 1.0, 0.0]);
        assert_eq!(p.masks[2][2], [3.0, 1.0, 1.0, 2.0]);
    }

    #[test]
    fn exposure_is_in_stops() {
        let mut e = EditState::default();
        e.light.exposure = 1.0;
        assert!((DevelopParams::new(&e, &color(), None, 600, 400).light0[0] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn warmer_temperature_raises_red_relative_to_blue() {
        let c = color();
        let (t, tint) = c.as_shot_temperature_tint();
        let mut e = EditState::default();
        e.white_balance.temperature = Some(t + 1500.0);
        e.white_balance.tint = Some(tint);
        let warm = DevelopParams::new(&e, &c, None, 600, 400).wb;
        let shot = DevelopParams::new(&EditState::default(), &c, None, 600, 400).wb;
        assert!(warm[0] / warm[2] > shot[0] / shot[2], "warm {warm:?} vs shot {shot:?}");
    }
}
