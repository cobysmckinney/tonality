use tonality_core::color::CameraColor;
use tonality_core::EditState;

/// Uniform block matching `struct Params` in `develop.wgsl` (std140-compatible: all vec4).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DevelopParams {
    pub wb: [f32; 4],
    pub cam: [[f32; 4]; 3],
    pub light0: [f32; 4],
    pub light1: [f32; 4],
}

impl DevelopParams {
    pub const FLOATS: usize = 24;

    pub fn new(edit: &EditState, color: &CameraColor) -> Self {
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
        Self {
            wb: [wb[0], wb[1], wb[2], 1.0],
            cam: [
                [m[0][0], m[0][1], m[0][2], 0.0],
                [m[1][0], m[1][1], m[1][2], 0.0],
                [m[2][0], m[2][1], m[2][2], 0.0],
            ],
            light0: [l.exposure.clamp(-10.0, 10.0).exp2(), n(l.contrast), n(l.highlights), n(l.shadows)],
            light1: [n(l.whites), n(l.blacks), 0.0, 0.0],
        }
    }

    pub fn to_vec(&self) -> Vec<f32> {
        let mut v = Vec::with_capacity(Self::FLOATS);
        v.extend_from_slice(&self.wb);
        for row in &self.cam {
            v.extend_from_slice(row);
        }
        v.extend_from_slice(&self.light0);
        v.extend_from_slice(&self.light1);
        v
    }
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
    fn layout_is_24_floats() {
        assert_eq!(DevelopParams::new(&EditState::default(), &color()).to_vec().len(), DevelopParams::FLOATS);
        assert_eq!(std::mem::size_of::<DevelopParams>(), DevelopParams::FLOATS * 4);
    }

    #[test]
    fn default_is_as_shot_and_neutral() {
        let p = DevelopParams::new(&EditState::default(), &color());
        let m = color().as_shot_multipliers();
        assert_eq!(&p.wb[..3], &m[..]);
        assert_eq!(p.light0, [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn exposure_is_in_stops() {
        let mut e = EditState::default();
        e.light.exposure = 1.0;
        assert!((DevelopParams::new(&e, &color()).light0[0] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn warmer_temperature_raises_red_relative_to_blue() {
        let c = color();
        let (t, tint) = c.as_shot_temperature_tint();
        let mut e = EditState::default();
        e.white_balance.temperature = Some(t + 1500.0);
        e.white_balance.tint = Some(tint);
        let warm = DevelopParams::new(&e, &c).wb;
        let shot = DevelopParams::new(&EditState::default(), &c).wb;
        assert!(warm[0] / warm[2] > shot[0] / shot[2], "warm {warm:?} vs shot {shot:?}");
    }
}
