//! Masks drawn by the GPU: each part covers what it should, and the sliders
//! of a mask apply only there. Skips itself on a machine without a GPU.

use tonality_lib::develop::LinearImage;
use tonality_lib::edit::{Adjustments, Crop, LocalAdjustments, Mask, MaskPart, Mode, Shape, Stroke};
use tonality_lib::gpu::{self, Gpu, Guides, Region, Session};
use tonality_lib::segment::{Found, Framing};

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

/// A matte whose left half is the subject, as if the model had found it.
fn left_half() -> image::GrayImage {
    image::GrayImage::from_fn(60, 40, |x, _| image::Luma([if x < 30 { 255 } else { 0 }]))
}

#[test]
fn the_subject_and_the_background_cover_what_the_matte_says() {
    let Some(gpu) = gpu() else { return };
    let session = photo(gpu, false);
    session.set_matte(&Found::Subject, left_half());
    let plain = draw(gpu, &session, &Adjustments::default())(0.5, 0.5);

    let subject = draw(gpu, &session, &with(vec![brighter(vec![add(Shape::Subject)])]));
    assert!(subject(0.2, 0.5) > plain + 30);
    assert_eq!(subject(0.8, 0.5), plain);

    // The background is the subject, inverted.
    let background = Mask { invert: true, ..brighter(vec![add(Shape::Subject)]) };
    let background = draw(gpu, &session, &with(vec![background]));
    assert_eq!(background(0.2, 0.5), plain);
    assert!(background(0.8, 0.5) > plain + 30);

    // A circle across the middle, kept only where it is on the subject.
    let circle = Shape::Radial { center: [0.5, 0.5], radius: [0.3, 0.3], angle: 0.0, feather: 0.05 };
    let both = brighter(vec![add(circle), MaskPart { mode: Mode::Intersect, shape: Shape::Subject }]);
    let both = draw(gpu, &session, &with(vec![both]));
    assert!(both(0.4, 0.5) > plain + 30);
    assert_eq!(both(0.6, 0.5), plain);
    assert_eq!(both(0.05, 0.5), plain, "on the subject but outside the circle");
}

#[test]
fn the_subject_stays_on_the_photo_when_it_is_turned() {
    let Some(gpu) = gpu() else { return };
    let session = photo(gpu, false);
    session.set_matte(&Found::Subject, left_half());
    let plain = draw(gpu, &session, &Adjustments::default())(0.5, 0.5);
    // Turned a quarter clockwise, the left of the photo is at the top.
    let turned = Adjustments { rotation: 1, masks: vec![brighter(vec![add(Shape::Subject)])], ..Default::default() };
    let turned = draw(gpu, &session, &turned);
    assert!(turned(0.5, 0.2) > plain + 30);
    assert_eq!(turned(0.5, 0.8), plain);
}

#[test]
fn a_cropped_photo_draws_the_subject_found_in_its_crop() {
    let Some(gpu) = gpu() else { return };
    let crop = Crop { x: 0.75, y: 0.5, width: 0.5, height: 1.0 };
    let recipe = Adjustments { crop, ..with(vec![brighter(vec![add(Shape::Subject)])]) };
    let half = |top: bool| image::GrayImage::from_fn(60, 40, move |_, y| image::Luma([if (y < 20) == top { 255 } else { 0 }]));

    // A thumbnail or an export draws each recipe with what was found in its own crop.
    let session = photo(gpu, false);
    let plain = draw(gpu, &session, &Adjustments { crop, ..Default::default() })(0.5, 0.5);
    let framing = session.framing_of(&recipe);
    assert_ne!(framing, Framing::Whole);
    session.set_matte(&Found::Subject, half(false));
    session.set_matte_named(session.key_in(&Found::Subject, &framing), half(true));
    let lit = draw(gpu, &session, &recipe);
    assert!(lit(0.5, 0.2) > plain + 30, "the subject found in the crop");
    assert_eq!(lit(0.5, 0.8), plain, "not the one found in the whole photo");
    assert_eq!(session.framing(), framing);

    // The editor's photo keeps what it found in the crop before until told
    // otherwise, so dragging a crop doesn't run the model at every step.
    let held = photo(gpu, false);
    held.hold_framing.store(true, std::sync::atomic::Ordering::Relaxed);
    held.set_matte(&Found::Subject, half(false));
    held.set_matte_named(held.key_in(&Found::Subject, &framing), half(true));
    let shown = draw(gpu, &held, &recipe);
    assert!(shown(0.5, 0.8) > plain + 30 && shown(0.5, 0.2) == plain, "drawn with the whole photo's subject");
    assert_eq!(held.framing(), Framing::Whole);
    held.set_framing(framing);
    let shown = draw(gpu, &held, &recipe);
    assert!(shown(0.5, 0.2) > plain + 30 && shown(0.5, 0.8) == plain, "then with the crop's");
}

#[test]
fn the_subject_is_found_when_first_drawn_and_then_kept() {
    let Some(gpu) = gpu() else { return };
    // A bright disc on a dim background.
    let disc = || {
        let (width, height) = (240u32, 160u32);
        let pixels = (0..width * height)
            .map(|i| {
                let (x, y) = ((i % width) as f32, (i / width) as f32);
                if (x - 120.0).hypot(y - 80.0) < 40.0 { [0.7, 0.15, 0.1] } else { [0.08, 0.1, 0.12] }
            })
            .collect();
        gpu.open(LinearImage { width, height, pixels, scene_referred: false }).unwrap()
    };
    let folder = tempfile::tempdir().unwrap();
    let start = folder.path().join("mattes/1-");
    let recipe = with(vec![brighter(vec![add(Shape::Subject)])]);

    let mut session = disc();
    session.matte_files = Some(start.clone());
    let cache = session.matte_path(&Found::Subject).unwrap();
    let plain = draw(gpu, &session, &Adjustments::default());
    let lit = draw(gpu, &session, &recipe);
    assert!(lit(0.5, 0.5) > plain(0.5, 0.5) + 20, "the disc is the subject");
    assert!(lit(0.03, 0.05) <= plain(0.03, 0.05) + 2, "the background is not");
    assert!(cache.exists(), "and the matte was kept");

    // Another session reads the kept matte instead of finding it again.
    image::GrayImage::new(8, 8).save(&cache).unwrap();
    let mut session = disc();
    session.matte_files = Some(start);
    assert_eq!(draw(gpu, &session, &recipe)(0.5, 0.5), plain(0.5, 0.5));
}

#[test]
fn the_sky_and_circled_objects_each_read_their_own_matte() {
    let Some(gpu) = gpu() else { return };
    let session = photo(gpu, false);
    let top_half = image::GrayImage::from_fn(60, 40, |_, y| image::Luma([if y < 20 { 255 } else { 0 }]));
    let corner = vec![[0.6, 0.6], [0.95, 0.6], [0.95, 0.95], [0.6, 0.95]];
    let bottom_right = image::GrayImage::from_fn(60, 40, |x, y| image::Luma([if x >= 40 && y >= 28 { 255 } else { 0 }]));
    session.set_matte(&Found::Subject, left_half());
    session.set_matte(&Found::Sky, top_half);
    session.set_matte(&Found::Object(corner.clone()), bottom_right);
    let plain = draw(gpu, &session, &Adjustments::default())(0.5, 0.5);

    let sky = draw(gpu, &session, &with(vec![brighter(vec![add(Shape::Sky)])]));
    assert!(sky(0.8, 0.2) > plain + 30);
    assert_eq!(sky(0.8, 0.8), plain);

    // The sky, less the subject, in one mask; the circled object in another.
    let sky_only = brighter(vec![add(Shape::Sky), MaskPart { mode: Mode::Subtract, shape: Shape::Subject }]);
    let object = Mask { id: 2, ..brighter(vec![add(Shape::Object { points: corner })]) };
    let both = draw(gpu, &session, &with(vec![sky_only, object]));
    assert!(both(0.8, 0.2) > plain + 30, "sky off the subject");
    assert_eq!(both(0.2, 0.2), plain, "sky on the subject");
    assert!(both(0.85, 0.85) > plain + 30, "the object");
    assert_eq!(both(0.5, 0.85), plain);
}

/// A loop of a few pixels: too small to find anything in, so finding it fails.
fn tiny_loop() -> Vec<[f32; 2]> {
    vec![[0.5, 0.5], [0.52, 0.5], [0.52, 0.52]]
}

#[test]
fn a_part_that_cant_be_found_draws_as_empty() {
    let Some(gpu) = gpu() else { return };
    let folder = tempfile::tempdir().unwrap();
    let mut session = photo(gpu, false);
    session.matte_files = Some(folder.path().join("1-"));
    let object = Found::Object(tiny_loop());
    let plain = draw(gpu, &session, &Adjustments::default());
    let recipe = with(vec![brighter(vec![add(Shape::Object { points: tiny_loop() })])]);

    let shade = draw(gpu, &session, &recipe);
    assert_eq!(shade(0.51, 0.51), plain(0.51, 0.51));
    assert_eq!(shade(0.1, 0.1), plain(0.1, 0.1));
    assert!(!session.matte_path(&object).unwrap().exists(), "an empty part isn't kept, so it's tried again next time");
    // It's tried once, not on every frame.
    assert!(gpu.ensure_found(&session, std::slice::from_ref(&object)).is_empty());

    // Found after all (circled again from the editor), it draws.
    session.set_matte(&object, image::GrayImage::from_pixel(60, 40, image::Luma([255])));
    assert!(draw(gpu, &session, &recipe)(0.1, 0.1) > plain(0.1, 0.1) + 30);
}

#[test]
fn a_part_that_cant_be_found_doesnt_show_another_parts_matte() {
    let Some(gpu) = gpu() else { return };
    let session = photo(gpu, false);
    session.set_matte(&Found::Subject, left_half());
    let plain = draw(gpu, &session, &Adjustments::default())(0.5, 0.5);
    // The subject is drawn first, into the layer the object will use next.
    let subject = draw(gpu, &session, &with(vec![brighter(vec![add(Shape::Subject)])]));
    assert!(subject(0.2, 0.5) > plain + 30);

    let object = draw(gpu, &session, &with(vec![brighter(vec![add(Shape::Object { points: tiny_loop() })])]));
    assert_eq!(object(0.2, 0.5), plain);
    assert_eq!(object(0.8, 0.5), plain);
}

/// A mask over the whole photo with only its sharpening set.
fn sharpened(amount: f32) -> Adjustments {
    with(vec![Mask {
        id: 1,
        name: "Test".into(),
        parts: vec![add(Shape::Luminance { low: 0.0, high: 1.0, smoothness: 0.0 })],
        adjustments: LocalAdjustments { sharpening: amount, ..Default::default() },
        ..Default::default()
    }])
}

/// How much neighbouring pixels differ, summed over the picture.
fn roughness(picture: &image::RgbImage) -> i64 {
    let (width, height) = picture.dimensions();
    let level = |x, y| picture.get_pixel(x, y).0[1] as i64;
    let mut total = 0;
    for y in 0..height - 1 {
        for x in 0..width - 1 {
            total += (level(x, y) - level(x + 1, y)).abs() + (level(x, y) - level(x, y + 1)).abs();
        }
    }
    total
}

#[test]
fn a_masks_negative_sharpening_softens_a_photo_with_no_sharpening() {
    let Some(gpu) = gpu() else { return };
    // A one-pixel checkerboard: the finest detail there is.
    let (width, height) = (40u32, 30u32);
    let light = |x: u32, y: u32| (x + y).is_multiple_of(2);
    let pixels = (0..width * height).map(|i| [if light(i % width, i / width) { 0.3 } else { 0.2 }; 3]).collect();
    let session = gpu.open(LinearImage { width, height, pixels, scene_referred: false }).unwrap();

    let plain = gpu.render_image(&session, &Adjustments::default(), u32::MAX).unwrap();
    let half = gpu.render_image(&session, &sharpened(-50.0), u32::MAX).unwrap();
    let soft = gpu.render_image(&session, &sharpened(-100.0), u32::MAX).unwrap();
    assert!(roughness(&half) < roughness(&plain), "{} vs {}", roughness(&half), roughness(&plain));
    assert!(roughness(&soft) < roughness(&half), "{} vs {}", roughness(&soft), roughness(&half));

    // At the end of the slider the checkerboard goes about flat, and doesn't turn inside out.
    let gap = |picture: &image::RgbImage| {
        let (mut lights, mut darks) = (0i64, 0i64);
        for (x, y, pixel) in picture.enumerate_pixels() {
            if light(x, y) { lights += pixel.0[1] as i64 } else { darks += pixel.0[1] as i64 }
        }
        (lights - darks) as f32 / (width * height / 2) as f32
    };
    assert!(gap(&plain) > 20.0, "{}", gap(&plain));
    assert!(gap(&soft).abs() < 2.0, "{} levels apart, from {}", gap(&soft), gap(&plain));
    assert!(gap(&soft) > -0.5, "turned inside out: {}", gap(&soft));
}

#[test]
fn a_masks_negative_sharpening_softens_a_raw_past_its_built_in_sharpening() {
    let Some(gpu) = gpu() else { return };
    // Fine noise on a dark grey, as a RAW file would have it.
    let (width, height) = (64u32, 48u32);
    let pixels = (0..width * height)
        .map(|i| {
            let wobble = (i.wrapping_mul(2_654_435_761) >> 16) % 100;
            [0.08 + wobble as f32 / 2000.0; 3]
        })
        .collect();
    let session = gpu.open(LinearImage { width, height, pixels, scene_referred: true }).unwrap();

    let roughness_at = |amount| roughness(&gpu.render_image(&session, &sharpened(amount), u32::MAX).unwrap());
    let (plain, a_little, most) = (roughness_at(0.0), roughness_at(-30.0), roughness_at(-100.0));
    assert!(a_little < plain, "{a_little} vs {plain}");
    // Before, everything past about -16 looked the same.
    assert!(most < a_little * 3 / 4, "{most} vs {a_little}");
}

/// A mask over the whole photo, with these sliders.
fn everywhere(adjustments: LocalAdjustments) -> Mask {
    let all = Shape::Luminance { low: 0.0, high: 1.0, smoothness: 0.0 };
    Mask { adjustments, ..brighter(vec![add(all)]) }
}

#[test]
fn exposure_stops_at_its_limit_however_many_masks_add_up() {
    let Some(gpu) = gpu() else { return };
    // Dim enough that five and six stops up still come out different.
    let (width, height) = (32u32, 24u32);
    let pixels = (0..width * height).map(|i| [0.0015 + 0.0015 * (i % width) as f32 / width as f32; 3]).collect();
    let session = gpu.open(LinearImage { width, height, pixels, scene_referred: false }).unwrap();
    let render = |recipe: &Adjustments| gpu.render_image(&session, recipe, u32::MAX).unwrap().into_raw();
    let up = |exposure| everywhere(LocalAdjustments { exposure, ..Default::default() });

    let top = Adjustments { exposure: 5.0, ..Default::default() };
    let full = render(&top);
    assert_ne!(full, render(&Adjustments { exposure: 4.0, ..Default::default() }), "the photo is dim enough to tell");
    assert!(full.chunks(4).all(|pixel| pixel[1] < 250), "and doesn't clip at the limit");

    assert!(full == render(&Adjustments { masks: vec![up(5.0)], ..top.clone() }), "a mask can't go past the limit");
    assert!(full == render(&with(vec![up(3.0), Mask { id: 2, ..up(3.0) }])), "nor can two added together");
    // Darkening from the top still works.
    let lower = Adjustments { exposure: 3.0, ..Default::default() };
    assert!(render(&lower) == render(&Adjustments { masks: vec![up(-2.0)], ..top }), "a mask still darkens");
}

#[test]
fn a_photo_that_isnt_open_exports_with_its_subject_and_sky() {
    use std::sync::atomic::AtomicBool;
    use tonality_lib::export::{self, Format, Job, Settings};
    use tonality_lib::library::{Library, View};
    use tonality_lib::{import, thumbs};

    let Some(gpu) = gpu() else { return };
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(&dir.path().join("Tonality")).unwrap();
    let card = dir.path().join("card");
    std::fs::create_dir_all(&card).unwrap();
    image::RgbImage::from_pixel(60, 40, image::Rgb([90, 90, 90])).save(card.join("IMG_0001.png")).unwrap();
    let session = import::scan(&library, 1, &[card], None, &|_, _| {}).unwrap();
    import::run(&library, &session, &[0], &AtomicBool::new(false), &|_, _| {}).unwrap();
    let id = library.list_photos(View::Library).unwrap()[0].id;

    // The subject (left half) brightened and the sky (top half) darkened,
    // their mattes found while the photo was open and kept in the library.
    let darker = |parts| Mask { id: 2, adjustments: LocalAdjustments { exposure: -1.0, ..Default::default() }, ..brighter(parts) };
    let recipe = with(vec![brighter(vec![add(Shape::Subject)]), darker(vec![add(Shape::Sky)])]);
    library.history_commit(id, &recipe, "Masks").unwrap();
    let top_half = image::GrayImage::from_fn(60, 40, |_, y| image::Luma([if y < 20 { 255 } else { 0 }]));
    let mut kept = gpu.open(LinearImage { width: 1, height: 1, pixels: vec![[0.0; 3]], scene_referred: false }).unwrap();
    kept.matte_files = Some(library.matte_files(id));
    for (found, matte) in [(Found::Subject, left_half()), (Found::Sky, top_half)] {
        let path = kept.matte_path(&found).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        matte.save(path).unwrap();
    }

    // Exported the way the app exports a photo the editor doesn't have open: loaded from the library.
    let draw = |id: i64, recipe: &Adjustments, long_edge: u32, _deep: bool| -> anyhow::Result<image::DynamicImage> {
        let session = thumbs::open_session(&library, id)?;
        Ok(gpu.render_image(&session, recipe, long_edge)?.into())
    };
    let settings = Settings { format: Format::Png, ..Default::default() };
    let summary = export::run(&library, &Job::of(&[id]), &settings, &draw, &AtomicBool::new(false), &|_, _| {}).unwrap();
    assert!(summary.failed.is_empty(), "{:?}", summary.failed.iter().map(|f| &f.reason).collect::<Vec<_>>());
    let picture = image::open(&summary.exported[0].path).unwrap().into_rgb8();
    let level = |x: u32, y: u32| picture.get_pixel(x, y).0[1];

    let plain = level(45, 30);
    assert!(level(15, 30) > plain + 30, "the subject is brighter: {} vs {plain}", level(15, 30));
    assert!(level(45, 10) + 20 < plain, "the sky is darker: {} vs {plain}", level(45, 10));
    assert!(level(15, 10).abs_diff(plain) <= 2, "where both apply they cancel out: {} vs {plain}", level(15, 10));
}
