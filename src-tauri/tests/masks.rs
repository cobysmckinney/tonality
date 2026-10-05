//! Masks drawn by the GPU: each part covers what it should, and the sliders
//! of a mask apply only there. Skips itself on a machine without a GPU.

use tonality_lib::develop::LinearImage;
use tonality_lib::edit::{Adjustments, LocalAdjustments, Mask, MaskPart, Mode, Shape, Stroke};
use tonality_lib::gpu::{self, Gpu, Guides, Region, Session};

fn gpu() -> Option<&'static Gpu> {
    gpu::shared().inspect_err(|error| eprintln!("skipping: {error:#}")).ok()
}

/// A flat grey photo, 120 x 80, or one that darkens from left to right.
fn photo(gpu: &Gpu, ramp: bool) -> Session {
    let (width, height) = (120u32, 80u32);
    let pixels = (0..width * height)
        .map(|i| {
            let value = if ramp { 1.0 - (i % width) as f32 / width as f32 } else { 0.2 };
            [value; 3]
        })
        .collect();
    gpu.open(LinearImage { width, height, pixels, scene_referred: false }).unwrap()
}

fn brighter(parts: Vec<MaskPart>) -> Mask {
    Mask {
        id: 1,
        name: "Test".into(),
        parts,
        adjustments: LocalAdjustments { exposure: 1.0, ..Default::default() },
        ..Default::default()
    }
}

fn add(shape: Shape) -> MaskPart {
    MaskPart { mode: Mode::Add, shape }
}

/// The picture with these masks, and a way to read its brightness at a spot (0..1 across and down).
fn draw(gpu: &Gpu, session: &Session, recipe: &Adjustments) -> impl Fn(f32, f32) -> u8 {
    let picture = gpu.render_image(session, recipe, u32::MAX).unwrap();
    move |x, y| {
        let px = ((x * picture.width() as f32) as u32).min(picture.width() - 1);
        let py = ((y * picture.height() as f32) as u32).min(picture.height() - 1);
        picture.get_pixel(px, py).0[1]
    }
}

fn with(masks: Vec<Mask>) -> Adjustments {
    Adjustments { masks, ..Default::default() }
}

#[test]
fn a_linear_gradient_fades_from_full_to_nothing() {
    let Some(gpu) = gpu() else { return };
    let session = photo(gpu, false);
    let plain = draw(gpu, &session, &Adjustments::default())(0.5, 0.5);
    let shade = draw(gpu, &session, &with(vec![brighter(vec![add(Shape::Linear { from: [0.1, 0.5], to: [0.6, 0.5] })])]));
    assert!(shade(0.05, 0.5) > plain + 30, "full effect before the start: {} vs {plain}", shade(0.05, 0.5));
    let middle = shade(0.35, 0.5);
    assert!(middle > plain + 5 && middle < shade(0.05, 0.5) - 5, "part of the way in between: {middle}");
    assert_eq!(shade(0.8, 0.5), plain, "none past the end");
    // The bands are parallel: the same all the way down.
    assert_eq!(shade(0.35, 0.1), middle);
}

#[test]
fn a_radial_gradient_stays_round_and_can_be_inverted() {
    let Some(gpu) = gpu() else { return };
    let session = photo(gpu, false);
    let plain = draw(gpu, &session, &Adjustments::default())(0.5, 0.5);
    // A radius of 0.2 of the long side is 24 pixels both ways on a 120 x 80 photo.
    let circle = Shape::Radial { center: [0.5, 0.5], radius: [0.2, 0.2], angle: 0.0, feather: 0.1 };
    let shade = draw(gpu, &session, &with(vec![brighter(vec![add(circle.clone())])]));
    assert!(shade(0.5, 0.5) > plain + 30);
    assert!(shade(0.5 + 20.0 / 120.0, 0.5) > plain + 30, "20 pixels right is inside");
    assert!(shade(0.5, 0.5 + 20.0 / 80.0) > plain + 30, "and so is 20 pixels down");
    assert_eq!(shade(0.5 + 28.0 / 120.0, 0.5), plain);
    assert_eq!(shade(0.05, 0.05), plain);

    let inverted = Mask { invert: true, ..brighter(vec![add(circle)]) };
    let shade = draw(gpu, &session, &with(vec![inverted]));
    assert_eq!(shade(0.5, 0.5), plain);
    assert!(shade(0.05, 0.05) > plain + 30);
}

#[test]
fn parts_add_subtract_and_intersect() {
    let Some(gpu) = gpu() else { return };
    let session = photo(gpu, true);
    let base = draw(gpu, &session, &Adjustments::default());
    let left = Shape::Linear { from: [0.4, 0.5], to: [0.6, 0.5] };
    let hole = Shape::Radial { center: [0.2, 0.5], radius: [0.08, 0.08], angle: 0.0, feather: 0.05 };
    let shade = draw(
        gpu,
        &session,
        &with(vec![brighter(vec![add(left.clone()), MaskPart { mode: Mode::Subtract, shape: hole }])]),
    );
    assert!(shade(0.3, 0.2) > base(0.3, 0.2) + 10, "the gradient covers the left");
    assert_eq!(shade(0.2, 0.5), base(0.2, 0.5), "but not the hole taken out of it");

    // Only the darker half of the ramp (the right) is within this brightness range.
    let dark = Shape::Luminance { low: 0.0, high: 0.45, smoothness: 0.05 };
    let shade = draw(gpu, &session, &with(vec![brighter(vec![add(dark.clone())])]));
    assert!(shade(0.9, 0.5) > base(0.9, 0.5) + 5);
    assert_eq!(shade(0.1, 0.5), base(0.1, 0.5));
    // Intersected with the left-hand gradient, nothing is both dark and on the left.
    let both = brighter(vec![add(left), MaskPart { mode: Mode::Intersect, shape: dark }]);
    let shade = draw(gpu, &session, &with(vec![both]));
    assert_eq!(shade(0.9, 0.5), base(0.9, 0.5));
    assert_eq!(shade(0.1, 0.5), base(0.1, 0.5));
}

#[test]
fn brush_strokes_paint_and_erase() {
    let Some(gpu) = gpu() else { return };
    let session = photo(gpu, false);
    let plain = draw(gpu, &session, &Adjustments::default())(0.5, 0.5);
    let stroke = Stroke { points: vec![[0.1, 0.25], [0.9, 0.25]], radius: 0.06, feather: 0.2, strength: 1.0, erase: false };
    let eraser = Stroke { points: vec![[0.5, 0.0], [0.5, 1.0]], erase: true, ..stroke.clone() };

    let painted = draw(gpu, &session, &with(vec![brighter(vec![add(Shape::Brush { strokes: vec![stroke.clone()] })])]));
    assert!(painted(0.5, 0.25) > plain + 30);
    assert!(painted(0.2, 0.25) > plain + 30);
    assert_eq!(painted(0.5, 0.75), plain);

    // The same session again, as while painting: the eraser is added to what is there.
    let erased =
        draw(gpu, &session, &with(vec![brighter(vec![add(Shape::Brush { strokes: vec![stroke.clone(), eraser] })])]));
    assert_eq!(erased(0.5, 0.25), plain);
    assert!(erased(0.2, 0.25) > plain + 30);

    // And undoing the eraser brings the stroke back whole.
    let again = draw(gpu, &session, &with(vec![brighter(vec![add(Shape::Brush { strokes: vec![stroke] })])]));
    assert!(again(0.5, 0.25) > plain + 30);
}

#[test]
fn masks_stay_on_the_photo_when_it_is_turned() {
    let Some(gpu) = gpu() else { return };
    let session = photo(gpu, false);
    let spot = brighter(vec![add(Shape::Radial { center: [0.25, 0.5], radius: [0.1, 0.1], angle: 0.0, feather: 0.1 })]);
    let plain = draw(gpu, &session, &Adjustments::default())(0.5, 0.5);
    // Turned a quarter clockwise, the left of the photo is at the top.
    let turned = draw(gpu, &session, &Adjustments { rotation: 1, masks: vec![spot], ..Default::default() });
    assert!(turned(0.5, 0.25) > plain + 30);
    assert_eq!(turned(0.25, 0.5), plain);
}

#[test]
fn a_hidden_mask_does_nothing_but_can_still_be_shown() {
    let Some(gpu) = gpu() else { return };
    let session = photo(gpu, false);
    let spot = Shape::Radial { center: [0.5, 0.5], radius: [0.2, 0.2], angle: 0.0, feather: 0.1 };
    let hidden = Mask { visible: false, ..brighter(vec![add(spot)]) };
    let recipe = with(vec![hidden]);
    let plain = draw(gpu, &session, &Adjustments::default())(0.5, 0.5);
    assert_eq!(draw(gpu, &session, &recipe)(0.5, 0.5), plain);

    let guides = Guides { mask_overlay: Some(1), ..Default::default() };
    let frame = gpu.render(&session, &recipe, Region::FULL, (120, 80), guides).unwrap();
    let pixel = |x: usize, y: usize| &frame[(y * 120 + x) * 4..(y * 120 + x) * 4 + 3];
    let (centre, corner) = (pixel(60, 40), pixel(2, 2));
    assert!(centre[0] > centre[1] + 60, "tinted red where the mask covers: {centre:?}");
    assert_eq!(corner[0], corner[1], "and grey elsewhere: {corner:?}");

    // Its thumbnail: white where it covers, black where it doesn't, hidden or not.
    let matte = gpu.render(&session, &recipe, Region::FULL, (120, 80), Guides { matte: true, ..guides }).unwrap();
    let level = |x: usize, y: usize| matte[(y * 120 + x) * 4];
    assert!(level(60, 40) > 240, "{}", level(60, 40));
    assert_eq!(level(2, 2), 0);
}
