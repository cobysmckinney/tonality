//! Where each output pixel comes from in the original photo, given the
//! photo's rotation, flips, straightening and crop.
//!
//! The chain, from the picture you see back to the file:
//!
//! 1. A *frame* is the upright rectangle being drawn. Normally it is the
//!    crop; while cropping it is a box big enough to show the whole tilted
//!    photo.
//! 2. The frame sits somewhere on the *turned* photo (the photo after its
//!    quarter-turns and flips), tilted by the straighten angle.
//! 3. Undoing the flips and quarter-turns gives a position in the file.
//!
//! The crop tool in the interface does the same arithmetic (src/crop.ts).

use crate::edit::Adjustments;

/// An affine map of the plane: `x' = a*x + b*y + c`, `y' = d*x + e*y + f`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine(pub [[f64; 3]; 2]);

impl Affine {
    pub fn apply(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        let [[a, b, c], [d, e, f]] = self.0;
        [a * x + b * y + c, d * x + e * y + f]
    }

    /// The map that does `self` first and `next` second.
    pub fn then(&self, next: &Affine) -> Affine {
        let [[a, b, c], [d, e, f]] = self.0;
        let [[g, h, i], [j, k, l]] = next.0;
        Affine([
            [g * a + h * d, g * b + h * e, g * c + h * f + i],
            [j * a + k * d, j * b + k * e, j * c + k * f + l],
        ])
    }

    fn scale(sx: f64, sy: f64) -> Affine {
        Affine([[sx, 0.0, 0.0], [0.0, sy, 0.0]])
    }

    fn translate(dx: f64, dy: f64) -> Affine {
        Affine([[1.0, 0.0, dx], [0.0, 1.0, dy]])
    }
}

/// The upright rectangle being drawn, measured in pixels of the turned photo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub center: [f64; 2],
    pub size: [f64; 2],
    /// Radians; positive makes the photo appear turned clockwise.
    pub angle: f64,
}

/// The photo's size once its quarter-turns are applied.
pub fn turned_size(width: u32, height: u32, adjustments: &Adjustments) -> [f64; 2] {
    if adjustments.rotation % 2 == 1 {
        [height as f64, width as f64]
    } else {
        [width as f64, height as f64]
    }
}

/// The frame to draw: the crop, or with `uncropped` the whole tilted photo
/// (what the crop tool shows underneath its frame).
pub fn frame(width: u32, height: u32, adjustments: &Adjustments, uncropped: bool) -> Frame {
    let [w, h] = turned_size(width, height, adjustments);
    let angle = (adjustments.straighten.clamp(-45.0, 45.0) as f64).to_radians();
    if uncropped {
        let (sin, cos) = (angle.sin().abs(), angle.cos().abs());
        return Frame { center: [w / 2.0, h / 2.0], size: [w * cos + h * sin, w * sin + h * cos], angle };
    }
    let crop = &adjustments.crop;
    let part = |value: f32| (value as f64).clamp(0.0, 1.0);
    Frame {
        center: [part(crop.x) * w, part(crop.y) * h],
        size: [part(crop.width).max(0.01) * w, part(crop.height).max(0.01) * h],
        angle,
    }
}

/// Maps a position in the frame (0..1 across and down) to a position in the
/// photo file (0..1 across and down).
pub fn frame_to_source(width: u32, height: u32, adjustments: &Adjustments, frame: &Frame) -> Affine {
    let [w, h] = turned_size(width, height, adjustments);
    let (sin, cos) = frame.angle.sin_cos();

    // Frame position to pixels on the turned photo: offset from the frame's
    // centre, tilted, then placed at the frame's centre.
    let offset = Affine::translate(-0.5, -0.5).then(&Affine::scale(frame.size[0], frame.size[1]));
    let tilt = Affine([[cos, sin, 0.0], [-sin, cos, 0.0]]);
    let on_turned = offset.then(&tilt).then(&Affine::translate(frame.center[0], frame.center[1]));

    // Pixels to 0..1, then undo the flips (which were applied last).
    let flip = Affine([
        [if adjustments.flip_horizontal { -1.0 } else { 1.0 }, 0.0, if adjustments.flip_horizontal { 1.0 } else { 0.0 }],
        [0.0, if adjustments.flip_vertical { -1.0 } else { 1.0 }, if adjustments.flip_vertical { 1.0 } else { 0.0 }],
    ]);
    // Undo the clockwise quarter-turns: where each turned position sat in the file.
    let unturn = match adjustments.rotation % 4 {
        0 => Affine([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
        1 => Affine([[0.0, 1.0, 0.0], [-1.0, 0.0, 1.0]]),
        2 => Affine([[-1.0, 0.0, 1.0], [0.0, -1.0, 1.0]]),
        _ => Affine([[0.0, -1.0, 1.0], [1.0, 0.0, 0.0]]),
    };
    on_turned.then(&Affine::scale(1.0 / w, 1.0 / h)).then(&flip).then(&unturn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Crop;

    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
    }

    fn map(adjustments: &Adjustments, uncropped: bool) -> Affine {
        frame_to_source(300, 200, adjustments, &frame(300, 200, adjustments, uncropped))
    }

    #[test]
    fn an_untouched_photo_maps_straight_through() {
        let m = map(&Adjustments::default(), false);
        for point in [[0.0, 0.0], [1.0, 0.0], [0.25, 0.75], [1.0, 1.0]] {
            assert!(close(m.apply(point), point));
        }
        assert_eq!(frame(300, 200, &Adjustments::default(), false).size, [300.0, 200.0]);
    }

    #[test]
    fn a_crop_shows_just_its_part() {
        let cropped = Adjustments { crop: Crop { x: 0.75, y: 0.5, width: 0.5, height: 0.5 }, ..Default::default() };
        let m = map(&cropped, false);
        assert!(close(m.apply([0.0, 0.0]), [0.5, 0.25]));
        assert!(close(m.apply([1.0, 1.0]), [1.0, 0.75]));
        assert_eq!(frame(300, 200, &cropped, false).size, [150.0, 100.0]);
    }

    #[test]
    fn quarter_turns_go_clockwise() {
        let once = Adjustments { rotation: 1, ..Default::default() };
        assert_eq!(turned_size(300, 200, &once), [200.0, 300.0]);
        // After one turn clockwise the file's bottom-left corner is at the top left.
        let m = map(&once, false);
        assert!(close(m.apply([0.0, 0.0]), [0.0, 1.0]));
        assert!(close(m.apply([1.0, 0.0]), [0.0, 0.0]));
        assert!(close(m.apply([1.0, 1.0]), [1.0, 0.0]));

        let thrice = Adjustments { rotation: 3, ..Default::default() };
        assert!(close(map(&thrice, false).apply([0.0, 0.0]), [1.0, 0.0]));
        let twice = Adjustments { rotation: 2, ..Default::default() };
        assert!(close(map(&twice, false).apply([0.0, 0.0]), [1.0, 1.0]));
    }

    #[test]
    fn flips_mirror_what_you_see() {
        let mirrored = Adjustments { flip_horizontal: true, ..Default::default() };
        assert!(close(map(&mirrored, false).apply([0.0, 0.25]), [1.0, 0.25]));
        // Flips act on the turned picture: after a quarter-turn, mirroring
        // left-right swaps the file's top and bottom.
        let both = Adjustments { rotation: 1, flip_horizontal: true, ..Default::default() };
        assert!(close(map(&both, false).apply([0.0, 0.0]), [0.0, 0.0]));
        assert!(close(map(&both, false).apply([1.0, 0.0]), [0.0, 1.0]));
    }

    #[test]
    fn a_positive_angle_turns_the_picture_clockwise() {
        // A square photo keeps the arithmetic readable.
        let tilted = Adjustments { straighten: 45.0, ..Default::default() };
        let f = frame(200, 200, &tilted, true);
        let m = frame_to_source(200, 200, &tilted, &f);
        // The whole tilted square needs a frame as wide as its diagonal…
        assert!((f.size[0] - 200.0 * 2f64.sqrt()).abs() < 1e-9);
        // …its centre stays put…
        assert!(close(m.apply([0.5, 0.5]), [0.5, 0.5]));
        // …and what was the top-left corner is now at the top, in the middle.
        assert!(close(m.apply([0.5, 0.0]), [0.0, 0.0]));
        // The right-hand point of the frame shows the old top-right corner.
        assert!(close(m.apply([1.0, 0.5]), [1.0, 0.0]));
    }
}
