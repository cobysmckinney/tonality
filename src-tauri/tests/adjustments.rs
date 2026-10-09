//! Every slider, drawn by the GPU: each does what it says, where it says,
//! and leaves alone what it should. The pictures are made in code so the
//! right answer is known. Skips itself on a machine without a GPU.

use image::RgbImage;
use tonality_lib::develop::LinearImage;
use tonality_lib::edit::{Adjustments, LocalAdjustments, Mask, MaskPart, Mode, Shape};
use tonality_lib::gpu::{self, Gpu, Session};

fn gpu() -> Option<&'static Gpu> {
    gpu::shared().inspect_err(|error| eprintln!("skipping: {error:#}")).ok()
}

/// sRGB level 0..255 to linear light.
fn linear(level: f32) -> f32 {
    let v = level / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// A picture `width` x `height`, each pixel's linear colour from `colour(x, y)`.
fn picture(gpu: &Gpu, (width, height): (u32, u32), scene_referred: bool, colour: impl Fn(u32, u32) -> [f32; 3]) -> Session {
    let pixels = (0..width * height).map(|i| colour(i % width, i / width)).collect();
    gpu.open(LinearImage { width, height, pixels, scene_referred }).unwrap()
}

/// A grey ramp, 256 x 32, that shows as level `x` at column `x`.
fn ramp(gpu: &Gpu) -> Session {
    picture(gpu, (256, 32), false, |x, _| [linear(x as f32); 3])
}

/// A flat picture of one colour, given as sRGB levels.
fn flat(gpu: &Gpu, colour: [f32; 3]) -> Session {
    picture(gpu, (64, 48), false, |_, _| colour.map(linear))
}

fn draw(gpu: &Gpu, session: &Session, recipe: &Adjustments) -> RgbImage {
    gpu.render_image(session, recipe, u32::MAX).unwrap()
}

fn at(picture: &RgbImage, x: u32, y: u32) -> [i32; 3] {
    picture.get_pixel(x, y).0.map(i32::from)
}

/// The middle pixel of a picture.
fn middle(picture: &RgbImage) -> [i32; 3] {
    at(picture, picture.width() / 2, picture.height() / 2)
}

/// How far apart a colour's strongest and weakest channels are.
fn chroma([r, g, b]: [i32; 3]) -> i32 {
    r.max(g).max(b) - r.min(g).min(b)
}

fn grey([r, g, b]: [i32; 3]) -> i32 {
    (r + g + b) / 3
}

/// How much neighbouring pixels differ, summed over the picture.
fn roughness(picture: &RgbImage) -> i64 {
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

fn recipe(edit: impl FnOnce(&mut Adjustments)) -> Adjustments {
    let mut adjustments = Adjustments::default();
    edit(&mut adjustments);
    adjustments
}

#[test]
fn contrast_spreads_tones_around_mid_grey() {
    let Some(gpu) = gpu() else { return };
    let session = ramp(gpu);
    let plain = draw(gpu, &session, &Adjustments::default());
    // Mid grey is 18% of the light, which shows as level 118.
    for contrast in [100.0, -100.0] {
        let out = draw(gpu, &session, &recipe(|a| a.contrast = contrast));
        assert!((at(&out, 118, 4)[1] - at(&plain, 118, 4)[1]).abs() <= 1, "mid grey moved with {contrast}");
        let (dark, light) = (at(&out, 50, 4)[1] - at(&plain, 50, 4)[1], at(&out, 200, 4)[1] - at(&plain, 200, 4)[1]);
        if contrast > 0.0 {
            assert!(dark < -10 && light > 10, "more contrast: darks {dark}, lights {light}");
        } else {
            assert!(dark > 10 && light < -10, "less contrast: darks {dark}, lights {light}");
        }
    }
}

/// A dark half and a bright half, each big enough that the middle of one
/// is far from the other.
fn halves(gpu: &Gpu) -> Session {
    picture(gpu, (256, 64), false, |x, _| [if x < 128 { 0.02 } else { 0.6 }; 3])
}

#[test]
fn shadows_and_highlights_each_move_their_own_end() {
    let Some(gpu) = gpu() else { return };
    let session = halves(gpu);
    let plain = draw(gpu, &session, &Adjustments::default());
    let (dark, bright) = (|p: &RgbImage| at(p, 32, 32)[1], |p: &RgbImage| at(p, 224, 32)[1]);

    let lifted = draw(gpu, &session, &recipe(|a| a.shadows = 100.0));
    assert!(dark(&lifted) > dark(&plain) + 30, "shadows lift the dark half: {} from {}", dark(&lifted), dark(&plain));
    assert!((bright(&lifted) - bright(&plain)).abs() <= 1, "and leave the bright half");
    let deepened = draw(gpu, &session, &recipe(|a| a.shadows = -100.0));
    assert!(dark(&deepened) < dark(&plain) - 10);

    let recovered = draw(gpu, &session, &recipe(|a| a.highlights = -100.0));
    assert!(bright(&recovered) < bright(&plain) - 30, "highlights bring the bright half down: {} from {}", bright(&recovered), bright(&plain));
    assert!((dark(&recovered) - dark(&plain)).abs() <= 1, "and leave the dark half");
    let brightened = draw(gpu, &session, &recipe(|a| a.highlights = 100.0));
    assert!(bright(&brightened) > bright(&plain) + 10);
}

#[test]
fn whites_and_blacks_move_the_ends_of_the_range() {
    let Some(gpu) = gpu() else { return };
    let session = ramp(gpu);
    let plain = draw(gpu, &session, &Adjustments::default());
    let moved = |p: &RgbImage, x| at(p, x, 4)[1] - at(&plain, x, 4)[1];

    let whites = draw(gpu, &session, &recipe(|a| a.whites = 100.0));
    assert!(moved(&whites, 200) > 20 && moved(&whites, 30).abs() <= 1, "{} {}", moved(&whites, 200), moved(&whites, 30));
    let whites_down = draw(gpu, &session, &recipe(|a| a.whites = -100.0));
    assert!(moved(&whites_down, 230) < -20 && moved(&whites_down, 30).abs() <= 1);

    let blacks = draw(gpu, &session, &recipe(|a| a.blacks = -100.0));
    assert!(moved(&blacks, 50) < -20 && moved(&blacks, 230).abs() <= 1, "{} {}", moved(&blacks, 50), moved(&blacks, 230));
    let blacks_up = draw(gpu, &session, &recipe(|a| a.blacks = 100.0));
    assert!(moved(&blacks_up, 20) > 20 && moved(&blacks_up, 230).abs() <= 1);
    // Black and white stay where they are unless pushed past them.
    assert_eq!(at(&whites, 0, 4), [0, 0, 0]);
    assert_eq!(at(&blacks, 255, 4), [255, 255, 255]);
}

#[test]
fn temperature_and_tint_shift_white_balance() {
    let Some(gpu) = gpu() else { return };
    let session = flat(gpu, [128.0; 3]);
    let balance = |edit: fn(&mut Adjustments)| middle(&draw(gpu, &session, &recipe(edit)));

    let [r, g, b] = balance(|a| a.temperature = 100.0);
    assert!(r > b + 40, "warmer is redder: {:?}", [r, g, b]);
    let [r, g, b] = balance(|a| a.temperature = -100.0);
    assert!(b > r + 40, "cooler is bluer: {:?}", [r, g, b]);
    let [r, g, b] = balance(|a| a.tint = 100.0);
    assert!(r > g + 10 && b > g + 10, "tint up is magenta: {:?}", [r, g, b]);
    let [r, g, b] = balance(|a| a.tint = -100.0);
    assert!(g > r + 10 && g > b + 10, "tint down is green: {:?}", [r, g, b]);
}

#[test]
fn a_picked_white_balance_makes_its_colour_grey_however_far_it_goes() {
    let Some(gpu) = gpu() else { return };
    // A grey card as a photo might hold it under lamplight, shade and
    // fluorescent light: the first two need more than 100 to fix.
    for colour in [[0.30, 0.18, 0.06], [0.08, 0.14, 0.25], [0.15, 0.22, 0.12]] {
        let session = picture(gpu, (64, 48), false, |_, _| colour);
        let (temperature, tint) = tonality_lib::white::neutralising(colour, false).unwrap();
        let picked = recipe(|a| (a.temperature, a.tint) = (temperature, tint));
        let shown = middle(&draw(gpu, &session, &picked));
        assert!(chroma(shown) <= 2, "{colour:?} with {temperature}, {tint} came out {shown:?}");
    }
}

#[test]
fn vibrance_favours_muted_colours_and_saturation_treats_all_alike() {
    let Some(gpu) = gpu() else { return };
    // A muted colour on the left, a vivid one on the right.
    let session = picture(gpu, (128, 32), false, |x, _| if x < 64 { [140.0, 120.0, 110.0] } else { [220.0, 40.0, 40.0] }.map(linear));
    let plain = draw(gpu, &session, &Adjustments::default());
    let (muted, vivid) = (|p: &RgbImage| chroma(at(p, 16, 16)), |p: &RgbImage| chroma(at(p, 112, 16)));
    let gain = |p: &RgbImage| (muted(p) as f32 / muted(&plain) as f32, vivid(p) as f32 / vivid(&plain) as f32);

    let (muted_gain, vivid_gain) = gain(&draw(gpu, &session, &recipe(|a| a.vibrance = 50.0)));
    assert!(muted_gain > 1.2, "vibrance livens muted colours: {muted_gain}");
    assert!(muted_gain > vivid_gain + 0.2, "more than vivid ones: {muted_gain} vs {vivid_gain}");

    let (muted_gain, vivid_gain) = gain(&draw(gpu, &session, &recipe(|a| a.saturation = 50.0)));
    assert!(muted_gain > 1.2 && vivid_gain > 1.05, "saturation livens both: {muted_gain}, {vivid_gain}");

    // Turned down, it takes most of the colour out of muted colours, and less out of vivid ones.
    let (muted_gain, vivid_gain) = gain(&draw(gpu, &session, &recipe(|a| a.vibrance = -100.0)));
    assert!(muted_gain < 0.4, "vibrance down nearly greys muted colours: {muted_gain}");
    assert!(vivid_gain > muted_gain + 0.2, "but vivid ones keep more: {vivid_gain} vs {muted_gain}");
}

/// Stripes four pixels wide, light and dark around mid grey, beside a flat patch of the same grey.
fn stripes(gpu: &Gpu) -> Session {
    picture(gpu, (512, 64), false, |x, _| {
        let level = if x >= 256 { 118.0 } else if (x / 4) % 2 == 0 { 100.0 } else { 136.0 };
        [linear(level); 3]
    })
}

/// How far apart the light and dark stripes are, on average.
fn stripe_gap(picture: &RgbImage) -> f32 {
    let (mut light, mut dark, mut n) = (0, 0, 0);
    for x in (64..192).step_by(8) {
        dark += at(picture, x + 1, 32)[1];
        light += at(picture, x + 5, 32)[1];
        n += 1;
    }
    (light - dark) as f32 / n as f32
}

#[test]
fn clarity_strengthens_or_softens_local_contrast_only() {
    let Some(gpu) = gpu() else { return };
    let session = stripes(gpu);
    let plain = draw(gpu, &session, &Adjustments::default());
    let more = draw(gpu, &session, &recipe(|a| a.clarity = 100.0));
    let less = draw(gpu, &session, &recipe(|a| a.clarity = -100.0));
    assert!(stripe_gap(&more) > stripe_gap(&plain) * 1.2, "{} vs {}", stripe_gap(&more), stripe_gap(&plain));
    assert!(stripe_gap(&less) < stripe_gap(&plain) * 0.8, "{} vs {}", stripe_gap(&less), stripe_gap(&plain));
    // A flat area has no detail to bring out.
    for out in [&more, &less] {
        assert!((at(out, 400, 32)[1] - at(&plain, 400, 32)[1]).abs() <= 1);
    }
}

#[test]
fn sharpening_brings_out_fine_detail_and_leaves_flat_areas() {
    let Some(gpu) = gpu() else { return };
    let session = stripes(gpu);
    let plain = draw(gpu, &session, &Adjustments::default());
    let sharp = draw(gpu, &session, &recipe(|a| a.sharpening = 100.0));
    assert!(roughness(&sharp) > roughness(&plain) * 11 / 10, "{} vs {}", roughness(&sharp), roughness(&plain));
    assert_eq!(at(&sharp, 400, 32), at(&plain, 400, 32));
}

#[test]
fn noise_reduction_smooths_noise_but_keeps_edges() {
    let Some(gpu) = gpu() else { return };
    // Fine noise on two greys with a hard edge between them.
    let session = picture(gpu, (128, 64), false, |x, y| {
        let wobble = ((x * 7919 + y * 104_729).wrapping_mul(2_654_435_761) >> 16) % 100;
        let base = if x < 64 { 0.05 } else { 0.4 };
        [base * (0.85 + wobble as f32 / 330.0); 3]
    });
    let plain = draw(gpu, &session, &Adjustments::default());
    let smooth = draw(gpu, &session, &recipe(|a| a.noise_reduction = 100.0));
    let side = |p: &RgbImage, from: u32| {
        let mut total = 0;
        for y in 8..56 {
            for x in from..from + 40 {
                total += (at(p, x, y)[1] - at(p, x + 1, y)[1]).abs() as i64;
            }
        }
        total
    };
    assert!(side(&smooth, 8) < side(&plain, 8) / 2, "dark side: {} vs {}", side(&smooth, 8), side(&plain, 8));
    assert!(side(&smooth, 78) < side(&plain, 78) / 2, "light side: {} vs {}", side(&smooth, 78), side(&plain, 78));
    // The edge stays an edge: two pixels either side are as far apart as before.
    let across = |p: &RgbImage| (8..56).map(|y| at(p, 66, y)[1] - at(p, 61, y)[1]).sum::<i32>() / 48;
    assert!(across(&smooth) > across(&plain) * 9 / 10, "{} vs {}", across(&smooth), across(&plain));
}

#[test]
fn dehaze_lifts_a_veil_or_lays_one_on() {
    let Some(gpu) = gpu() else { return };
    // A hazy scene: two pale, washed-out patches.
    let session = picture(gpu, (256, 64), false, |x, _| if x < 128 { [0.55, 0.5, 0.45] } else { [0.35, 0.35, 0.4] });
    let plain = draw(gpu, &session, &Adjustments::default());
    let (left, right) = (|p: &RgbImage| at(p, 40, 32), |p: &RgbImage| at(p, 216, 32));
    let spread = |p: &RgbImage| grey(left(p)) - grey(right(p));

    let clear = draw(gpu, &session, &recipe(|a| a.dehaze = 100.0));
    assert!(grey(left(&clear)) < grey(left(&plain)) - 10, "the veil comes off: {:?} from {:?}", left(&clear), left(&plain));
    assert!(spread(&clear) > spread(&plain), "patches stand further apart: {} vs {}", spread(&clear), spread(&plain));
    assert!(chroma(left(&clear)) > chroma(left(&plain)), "and colour comes back: {:?}", left(&clear));

    let hazier = draw(gpu, &session, &recipe(|a| a.dehaze = -100.0));
    assert!(grey(left(&hazier)) > grey(left(&plain)) + 5, "{:?} from {:?}", left(&hazier), left(&plain));
    assert!(spread(&hazier) < spread(&plain));
}

#[test]
fn a_vignette_darkens_or_lightens_the_corners_and_not_the_middle() {
    let Some(gpu) = gpu() else { return };
    let session = flat(gpu, [128.0; 3]);
    let plain = draw(gpu, &session, &Adjustments::default());
    let corner = |p: &RgbImage| at(p, 0, 0)[1];
    let dark = draw(gpu, &session, &recipe(|a| a.vignette = -100.0));
    assert!(corner(&dark) < corner(&plain) - 40, "{} from {}", corner(&dark), corner(&plain));
    let light = draw(gpu, &session, &recipe(|a| a.vignette = 100.0));
    assert!(corner(&light) > corner(&plain) + 40, "{} from {}", corner(&light), corner(&plain));
    for out in [&dark, &light] {
        assert_eq!(middle(out), middle(&plain));
        // Round, so the four corners match.
        let (w, h) = (out.width() - 1, out.height() - 1);
        assert_eq!([at(out, 0, 0), at(out, w, 0), at(out, 0, h)], [at(out, w, h); 3]);
    }
}

#[test]
fn grain_is_noise_that_keeps_brightness_and_stays_put() {
    let Some(gpu) = gpu() else { return };
    let session = flat(gpu, [128.0; 3]);
    let plain = draw(gpu, &session, &Adjustments::default());
    assert_eq!(roughness(&plain), 0);
    let grainy = draw(gpu, &session, &recipe(|a| a.grain = 100.0));
    assert!(roughness(&grainy) > 1000, "{}", roughness(&grainy));
    let mean = |p: &RgbImage| p.pixels().map(|pixel| pixel.0[1] as f64).sum::<f64>() / (p.width() * p.height()) as f64;
    assert!((mean(&grainy) - mean(&plain)).abs() < 3.0, "{} vs {}", mean(&grainy), mean(&plain));
    assert!(grainy == draw(gpu, &session, &recipe(|a| a.grain = 100.0)), "the same grain every time");
}

#[test]
fn raw_files_get_a_camera_like_look() {
    let Some(gpu) = gpu() else { return };
    // Sensor values from nothing to where the sensor clips.
    let session = picture(gpu, (256, 32), true, |x, _| [x as f32 / 255.0; 3]);
    let out = draw(gpu, &session, &Adjustments::default());
    let level = |x| at(&out, x, 16)[1];
    assert_eq!(at(&out, 0, 16), [0, 0, 0], "no light is black");
    assert_eq!(at(&out, 255, 16), [255, 255, 255], "the sensor's clipping point is white");
    for x in 1..256 {
        assert!(level(x) >= level(x - 1), "brighter all the way: {} then {} at {x}", level(x - 1), level(x));
    }
    // Mid grey (18% of clipping) is brightened as a camera's JPEG would be,
    // well past the 118 it would be without the look.
    assert!((level(46) - 176).abs() <= 3, "{}", level(46));

    // A highlight that clipped in one channel comes out white, not tinted.
    let blown = picture(gpu, (32, 32), true, |_, _| [1.0, 0.75, 0.7]);
    let [r, g, b] = middle(&draw(gpu, &blown, &Adjustments::default()));
    assert!(r >= 250 && g >= 250 && b >= 250, "{:?}", [r, g, b]);
}

#[test]
fn straightening_turns_the_picture_clockwise_about_the_crop() {
    let Some(gpu) = gpu() else { return };
    // A bright line across the middle of a dark picture.
    let session = picture(gpu, (400, 200), false, |_, y| [if (98..102).contains(&y) { 0.8 } else { 0.02 }; 3]);
    let tilted = recipe(|a| {
        a.straighten = 10.0;
        // Small enough to stay inside the tilted photo.
        a.crop.width = 0.6;
        a.crop.height = 0.6;
    });
    let out = draw(gpu, &session, &tilted);
    assert_eq!(out.dimensions(), (240, 120));
    // Where the line crosses a column: the brightest row in it.
    let line_at = |x| (0..out.height()).max_by_key(|&y| at(&out, x, y)[1]).unwrap() as f32;
    let (left, right) = (line_at(20), line_at(220));
    // Clockwise, the line falls from left to right, by tan 10° of the distance.
    let fall = 200.0 * 10f32.to_radians().tan();
    assert!((right - left - fall).abs() <= 3.0, "the line falls {} over 200 px, not {fall}", right - left);
    assert!((line_at(120) - 60.0).abs() <= 2.0, "and still crosses the middle");
    // The corners are photo, not empty.
    assert!(at(&out, 0, 0)[1] < 40 && at(&out, 239, 119)[1] < 40);
}

/// A mask covering the left quarter of the picture, faded out by a third of the way across.
fn left_side(adjustments: LocalAdjustments) -> Mask {
    Mask {
        id: 1,
        name: "Left".into(),
        parts: vec![MaskPart { mode: Mode::Add, shape: Shape::Linear { from: [0.25, 0.5], to: [0.33, 0.5] } }],
        adjustments,
        ..Default::default()
    }
}

/// A slider a mask can carry, by name, and how to set it.
type Slider = (&'static str, fn(&mut LocalAdjustments));

#[test]
fn every_slider_a_mask_carries_works_only_where_it_covers() {
    let Some(gpu) = gpu() else { return };
    // Something for every slider to act on: colour, tone and fine detail.
    let session = picture(gpu, (300, 120), false, |x, y| {
        let wobble = ((x * 7919 + y * 104_729).wrapping_mul(2_654_435_761) >> 16) % 100;
        let stripe = if (x / 3 + y / 3) % 2 == 0 { 0.8 } else { 1.2 };
        let tone = (0.03 + 0.5 * y as f32 / 120.0) * stripe * (0.9 + wobble as f32 / 500.0);
        [tone * 1.2, tone, tone * 0.7]
    });
    let plain = draw(gpu, &session, &Adjustments::default());
    let sliders: [Slider; 14] = [
        ("exposure", |l| l.exposure = 1.0),
        ("contrast", |l| l.contrast = 100.0),
        ("highlights", |l| l.highlights = -100.0),
        ("shadows", |l| l.shadows = 100.0),
        ("whites", |l| l.whites = 100.0),
        ("blacks", |l| l.blacks = -100.0),
        ("temperature", |l| l.temperature = 100.0),
        ("tint", |l| l.tint = 100.0),
        ("vibrance", |l| l.vibrance = 100.0),
        ("saturation", |l| l.saturation = -100.0),
        ("clarity", |l| l.clarity = 100.0),
        ("dehaze", |l| l.dehaze = 100.0),
        ("sharpening", |l| l.sharpening = 100.0),
        ("noise reduction", |l| l.noise_reduction = 100.0),
    ];
    let differs = |out: &RgbImage, columns: std::ops::Range<u32>| {
        columns.flat_map(|x| (0..120).map(move |y| (x, y))).filter(|&(x, y)| at(out, x, y) != at(&plain, x, y)).count()
    };
    for (name, set) in sliders {
        let mut local = LocalAdjustments::default();
        set(&mut local);
        let out = draw(gpu, &session, &Adjustments { masks: vec![left_side(local)], ..Default::default() });
        assert!(differs(&out, 0..60) > 60 * 120 / 4, "{name} changes the covered side: {} pixels", differs(&out, 0..60));
        assert_eq!(differs(&out, 110..300), 0, "{name} leaves the rest alone");
    }
}

#[test]
fn a_mask_stays_on_its_part_of_the_photo_when_it_is_straightened() {
    let Some(gpu) = gpu() else { return };
    // A red photo with a white dot, and a round mask over the dot that takes the colour out.
    let dot = [0.35f32, 0.4];
    let session = picture(gpu, (400, 300), false, |x, y| {
        let near = (x as f32 - dot[0] * 400.0).hypot(y as f32 - dot[1] * 300.0) < 3.0;
        if near { [1.0; 3] } else { [0.5, 0.05, 0.05] }
    });
    let grey = Mask {
        id: 1,
        name: "Dot".into(),
        parts: vec![MaskPart {
            mode: Mode::Add,
            shape: Shape::Radial { center: dot, radius: [0.05, 0.05], angle: 0.0, feather: 0.1 },
        }],
        adjustments: LocalAdjustments { saturation: -100.0, ..Default::default() },
        ..Default::default()
    };
    for straighten in [0.0, 12.0, -25.0] {
        let edit = recipe(|a| {
            a.straighten = straighten;
            a.crop.width = 0.5;
            a.crop.height = 0.5;
            a.masks = vec![grey.clone()];
        });
        let out = draw(gpu, &session, &edit);
        // Where the dot is now, and the middle of the grey disc.
        let (width, height) = out.dimensions();
        let pixels = || (0..height).flat_map(move |y| (0..width).map(move |x| (x, y)));
        let middle_of = |points: Vec<(u32, u32)>| {
            let sum = points.iter().fold((0.0, 0.0), |(sx, sy), &(x, y)| (sx + x as f32, sy + y as f32));
            (points.len(), (sum.0 / points.len() as f32, sum.1 / points.len() as f32))
        };
        let (dot_size, found) = middle_of(pixels().filter(|&(x, y)| at(&out, x, y)[1] > 200).collect());
        let (disc_size, middle) = middle_of(pixels().filter(|&(x, y)| chroma(at(&out, x, y)) < 20).collect());
        assert!(dot_size > 5 && disc_size > 100, "at {straighten}°: the dot and the disc are on the picture");
        assert!(
            (middle.0 - found.0).abs() <= 1.0 && (middle.1 - found.1).abs() <= 1.0,
            "at {straighten}°, the disc is at {middle:?} but the dot at {found:?}",
        );
    }
}
