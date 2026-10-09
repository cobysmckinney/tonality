//! Film negatives, through the GPU: a scene made in code is put through a
//! simple model of film, scanned, and turned back into a positive by the
//! pipeline. Skips itself on a machine without a GPU.

use image::RgbImage;
use tonality_lib::develop::LinearImage;
use tonality_lib::edit::{Adjustments, Crop};
use tonality_lib::film::{Film, Kind};
use tonality_lib::gpu::{self, Gpu, Session};
use tonality_lib::segment::Found;

fn gpu() -> Option<&'static Gpu> {
    gpu::shared().inspect_err(|error| eprintln!("skipping: {error:#}")).ok()
}

/// The scene: a row of grey steps from deep shadow to bright highlight over
/// a row of colours, in scene light as a RAW holds it.
const STEPS: [f32; 8] = [0.004, 0.01, 0.025, 0.05, 0.1, 0.2, 0.4, 0.8];
const COLOURS: [[f32; 3]; 4] = [[0.3, 0.04, 0.03], [0.04, 0.2, 0.04], [0.03, 0.05, 0.3], [0.25, 0.2, 0.03]];
const PATCH: u32 = 16;
const SCENE: (u32, u32) = (8 * PATCH, 2 * PATCH);
/// Clear film around the frame, then the light source above and below it
/// and the black of a holder at the sides.
const REBATE: u32 = 10;
const OUTSIDE: u32 = 6;
const BASE: [f32; 3] = [0.7, 0.38, 0.2];

fn scene(x: u32, y: u32) -> [f32; 3] {
    let column = (x / PATCH) as usize;
    if y < PATCH { [STEPS[column]; 3] } else { COLOURS[column / 2] }
}

/// The scan of a negative of `scene`: each channel's density climbs with
/// log light at its own steepness, on top of the film's base.
fn scan(gammas: [f32; 3], base: [f32; 3]) -> LinearImage {
    scan_of(scene, gammas, base)
}

/// `scan`, of another scene the same size.
fn scan_of(scene: fn(u32, u32) -> [f32; 3], gammas: [f32; 3], base: [f32; 3]) -> LinearImage {
    let border = REBATE + OUTSIDE;
    let (width, height) = (SCENE.0 + 2 * border, SCENE.1 + 2 * border);
    let pixels = (0..width * height)
        .map(|i| {
            let (x, y) = (i % width, i / width);
            if y < OUTSIDE || y >= height - OUTSIDE {
                return [1.0; 3];
            }
            if x < OUTSIDE || x >= width - OUTSIDE {
                return [0.0005; 3];
            }
            if x < border || y < border || x >= width - border || y >= height - border {
                return base;
            }
            let light = scene(x - border, y - border);
            // Clear film below 0.001: anything dimmer leaves no mark.
            std::array::from_fn(|c| base[c] * 10f32.powf(-gammas[c] * (light[c] / 0.001).max(1.0).log10()))
        })
        .collect();
    LinearImage { width, height, pixels, scene_referred: true }
}

/// The crop that holds just the frame.
fn frame_crop(image: &LinearImage) -> Crop {
    Crop { x: 0.5, y: 0.5, width: SCENE.0 as f32 / image.width as f32, height: SCENE.1 as f32 / image.height as f32 }
}

fn negative(kind: Kind, crop: Crop) -> Adjustments {
    Adjustments { film: Film { kind, ..Default::default() }, crop, ..Default::default() }
}

/// A patch's colour in the drawn frame, averaged over its middle.
fn patch(picture: &RgbImage, column: u32, row: u32) -> [i32; 3] {
    let (sx, sy) = (picture.width() as f32 / SCENE.0 as f32, picture.height() as f32 / SCENE.1 as f32);
    let mut sum = [0i32; 3];
    let mut count = 0;
    for y in (row * PATCH + 5)..(row * PATCH + PATCH - 5) {
        for x in (column * PATCH + 5)..(column * PATCH + PATCH - 5) {
            let p = picture.get_pixel((x as f32 * sx) as u32, (y as f32 * sy) as u32).0;
            sum = std::array::from_fn(|c| sum[c] + p[c] as i32);
            count += 1;
        }
    }
    sum.map(|v| v / count)
}

fn chroma([r, g, b]: [i32; 3]) -> i32 {
    r.max(g).max(b) - r.min(g).min(b)
}

fn grey([r, g, b]: [i32; 3]) -> i32 {
    (r + g + b) / 3
}

/// The scene as an ordinary RAW of it would open.
fn as_shot(gpu: &Gpu) -> RgbImage {
    as_shot_of(gpu, scene)
}

/// `scene` as an ordinary RAW of it would open, exposed as a camera's meter
/// would: its average (the mean of its light in stops) on `film::KEY`.
fn as_shot_of(gpu: &Gpu, scene: fn(u32, u32) -> [f32; 3]) -> RgbImage {
    let (width, height) = SCENE;
    let pixels: Vec<[f32; 3]> = (0..width * height).map(|i| scene(i % width, i / width)).collect();
    let average = pixels.iter().map(|p| p.iter().map(|v| v.log2()).sum::<f32>() / 3.0).sum::<f32>() / pixels.len() as f32;
    let exposure = tonality_lib::film::KEY / average.exp2();
    let pixels = pixels.into_iter().map(|p| p.map(|v| v * exposure)).collect();
    let session = gpu.open(LinearImage { width, height, pixels, scene_referred: true }).unwrap();
    gpu.render_image(&session, &Adjustments::default(), u32::MAX).unwrap()
}

#[test]
fn a_colour_negative_comes_back_neutral_and_in_order() {
    let Some(gpu) = gpu() else { return };
    let image = scan([0.55, 0.62, 0.7], BASE);
    let crop = frame_crop(&image);
    let session = gpu.open(image).unwrap();
    let picture = gpu.render_image(&session, &negative(Kind::Colour, crop), u32::MAX).unwrap();
    let shot = as_shot(gpu);

    let mut last = -1;
    for column in 0..8 {
        let step = patch(&picture, column, 0);
        assert!(chroma(step) <= 4, "grey step {column} came out {step:?}");
        assert!(grey(step) > last, "grey step {column} ({step:?}) is no brighter than the one before");
        last = grey(step);
        // About as bright as a RAW of the scene opens.
        let shot = grey(patch(&shot, column, 0));
        assert!((grey(step) - shot).abs() <= 24, "grey step {column}: {} against {shot} as shot", grey(step));
    }
    // The ends of the range land near black and near white.
    assert!(grey(patch(&picture, 0, 0)) < 40);
    assert!(grey(patch(&picture, 7, 0)) > 215);

    // Colours keep their hue: red stays reddest, and so on.
    let [red, green, blue, yellow] = [0, 2, 4, 6].map(|column| patch(&picture, column, 1));
    assert!(red[0] > red[1] + 40 && red[0] > red[2] + 40, "red came out {red:?}");
    assert!(green[1] > green[0] + 30 && green[1] > green[2] + 30, "green came out {green:?}");
    assert!(blue[2] > blue[0] + 40 && blue[2] > blue[1] + 30, "blue came out {blue:?}");
    assert!(yellow[0] > yellow[2] + 40 && yellow[1] > yellow[2] + 40, "yellow came out {yellow:?}");
}

#[test]
fn a_black_and_white_negative_comes_out_grey() {
    let Some(gpu) = gpu() else { return };
    // A faint pink tint in the base and light.
    let image = scan([0.62, 0.6, 0.63], [0.72, 0.64, 0.7]);
    let crop = frame_crop(&image);
    let session = gpu.open(image).unwrap();
    let picture = gpu.render_image(&session, &negative(Kind::BlackAndWhite, crop), u32::MAX).unwrap();
    for pixel in picture.pixels() {
        assert!(chroma(pixel.0.map(i32::from)) <= 1, "{:?} isn't grey", pixel.0);
    }
    let mut last = -1;
    for column in 0..8 {
        let step = grey(patch(&picture, column, 0));
        assert!(step > last, "grey step {column} is no brighter than the one before");
        last = step;
    }
}

#[test]
fn the_light_around_the_film_is_black_not_a_blown_highlight() {
    let Some(gpu) = gpu() else { return };
    let image = scan([0.6; 3], BASE);
    let (width, height) = (image.width, image.height);
    let crop = frame_crop(&image);
    let session = gpu.open(image).unwrap();
    // The whole scan, with the balance measured inside the frame.
    let recipe = negative(Kind::Colour, crop);
    let whole = gpu.render(&session, &recipe, gpu::Region::FULL, (width, height), gpu::Guides { uncropped: true, ..Default::default() });
    let whole = whole.unwrap();
    let at = |x: u32, y: u32| {
        let i = ((y * width + x) * 4) as usize;
        [whole[i], whole[i + 1], whole[i + 2]].map(i32::from)
    };
    // The light source (clipped in the scan) and the clear rebate are the deepest shadows.
    assert!(grey(at(width / 2, 2)) < 10, "the light source came out {:?}", at(width / 2, 2));
    assert!(grey(at(width / 2, OUTSIDE + 3)) < 30, "the rebate came out {:?}", at(width / 2, OUTSIDE + 3));
    // The holder's black, the densest film there could be, is white.
    assert!(grey(at(2, height / 2)) > 245);
    // A bright colour keeps its colour rather than fading to white.
    let picture = gpu.render_image(&session, &recipe, u32::MAX).unwrap();
    assert!(chroma(patch(&picture, 0, 1)) > 60);
}

#[test]
fn changing_the_film_redraws_the_working_image() {
    let Some(gpu) = gpu() else { return };
    let image = scan([0.6; 3], BASE);
    let crop = frame_crop(&image);
    let session: Session = gpu.open(image).unwrap();
    let plain = Adjustments { crop, ..Default::default() };
    let draw = |recipe: &Adjustments| gpu.render_image(&session, recipe, u32::MAX).unwrap();
    let before = draw(&plain);
    // The scan itself is orange and inverted: its brightest step is the densest film.
    assert!(grey(patch(&before, 0, 0)) > grey(patch(&before, 7, 0)));

    let colour = draw(&negative(Kind::Colour, crop));
    let subject = session.key(&Found::Subject);
    let bw = draw(&negative(Kind::BlackAndWhite, crop));
    assert_ne!(colour, bw);
    assert_ne!(session.key(&Found::Subject), subject, "mattes found in one positive aren't used for another");

    // A base picked by hand is used instead of the guess.
    let picked = Adjustments { film: Film { kind: Kind::Colour, base: Some([0.7, 0.3, 0.2]), range: None }, crop, ..Default::default() };
    assert_ne!(draw(&picked), colour);
    assert!(draw(&negative(Kind::Colour, crop)) == colour, "the same settings draw the same picture again");

    // Back to the scan as it was.
    assert!(draw(&plain) == before, "turning film off brings back the scan as opened");
    assert_eq!(session.key(&Found::Subject), Found::Subject.key());

    // The blurs behind shadows and highlights are of the positive too:
    // lifting the shadows lifts its dark steps, not the negative's.
    let positive = negative(Kind::Colour, crop);
    let lifted = draw(&Adjustments { shadows: 100.0, ..positive.clone() });
    let gain = |column| grey(patch(&lifted, column, 0)) - grey(patch(&colour, column, 0));
    assert!(gain(1) > gain(7) + 20, "shadows lifted step 1 by {} and step 7 by {}", gain(1), gain(7));
    // And so are the mipmaps a small picture is drawn from.
    let small = gpu.render_image(&session, &positive, SCENE.0 / 4).unwrap();
    for column in 0..8 {
        let (small, full) = (grey(patch(&small, column, 0)), grey(patch(&colour, column, 0)));
        assert!((small - full).abs() <= 6, "step {column}: {small} small, {full} at full size");
    }
}

/// A bright day: a sky across the top half, getting brighter to one side,
/// over ground from deep shadow up through a grey card (column 4). Exposed
/// for the whole frame, as a camera's meter would, the sky's bright end is
/// near where a RAW clips.
fn bright_day(x: u32, y: u32) -> [f32; 3] {
    const SKY: [f32; 8] = [0.4, 0.45, 0.5, 0.56, 0.63, 0.71, 0.8, 1.0];
    const GROUND: [f32; 8] = [0.002, 0.004, 0.008, 0.015, 0.03, 0.03, 0.05, 0.08];
    let column = (x / PATCH) as usize;
    [if y < PATCH { SKY[column] } else { GROUND[column] }; 3]
}

#[test]
fn a_bright_sky_keeps_its_detail_and_the_rest_its_brightness() {
    let Some(gpu) = gpu() else { return };
    let shot = as_shot_of(gpu, bright_day);
    let grey_card = grey(patch(&shot, 4, 1));
    for (kind, gammas) in [(Kind::Colour, [0.55, 0.62, 0.7]), (Kind::BlackAndWhite, [0.62; 3])] {
        let image = scan_of(bright_day, gammas, BASE);
        let crop = frame_crop(&image);
        let session = gpu.open(image).unwrap();
        let picture = gpu.render_image(&session, &negative(kind, crop), u32::MAX).unwrap();
        let sky: Vec<i32> = (0..8).map(|column| grey(patch(&picture, column, 0))).collect();
        // The sky isn't pressed against white: its brightest part stays off
        // it, and each of its steps is brighter than the last.
        assert!(sky[7] < 242, "{kind:?}: the sky's brightest part came out {}", sky[7]);
        assert!(sky.windows(2).all(|pair| pair[1] >= pair[0] + 2), "{kind:?}: the sky came out {sky:?}");
        // The grey card is about as bright as an ordinary RAW of the scene opens.
        let card = grey(patch(&picture, 4, 1));
        assert!((card - grey_card).abs() <= 12, "{kind:?}: the grey card came out {card}, against {grey_card} as shot");
    }
}

/// Converts camera scans of black and white negatives as they open, uncropped,
/// and checks the frame's highlights aren't blown:
/// `TONALITY_SAMPLES=/path/to/scans [TONALITY_OUT=/path] cargo test --test integration real_black_and_white_scans -- --ignored --nocapture`
#[test]
#[ignore = "needs TONALITY_SAMPLES pointing at camera scans of black and white negatives, and a GPU"]
fn real_black_and_white_scans() {
    let samples = std::path::PathBuf::from(std::env::var_os("TONALITY_SAMPLES").expect("set TONALITY_SAMPLES"));
    let out = std::env::var_os("TONALITY_OUT").map(std::path::PathBuf::from);
    let gpu = gpu().expect("a GPU");
    for entry in walkdir::WalkDir::new(samples).into_iter().flatten().filter(|e| e.file_type().is_file()) {
        let Some(kind) = tonality_lib::media::kind_of(entry.path()) else { continue };
        let name = entry.path().file_stem().unwrap().to_string_lossy().into_owned();
        let image = tonality_lib::develop::load(entry.path(), kind == tonality_lib::media::Kind::Raw).unwrap();
        let session = gpu.open(image).unwrap();
        let picture = gpu.render_image(&session, &negative(Kind::BlackAndWhite, Crop::default()), 1200).unwrap();
        if let Some(out) = &out {
            picture.save(out.join(format!("{name}-positive.jpg"))).unwrap();
        }
        // The middle of the scan, where the frame is.
        let (w, h) = picture.dimensions();
        let middle = image::imageops::crop_imm(&picture, w / 5, h / 5, w * 3 / 5, h * 3 / 5).to_image();
        let mut tones: Vec<u8> = middle.pixels().map(|p| p[1]).collect();
        tones.sort_unstable();
        let at = |share: f64| tones[((tones.len() - 1) as f64 * share) as usize];
        let white = tones.iter().filter(|v| **v >= 250).count() as f64 / tones.len() as f64;
        println!("{name}: median {}, 99th percentile {}, {:.1}% white", at(0.5), at(0.99), white * 100.0);
        assert!(white < 0.01, "{name}: {:.1}% of the frame is white", white * 100.0);
        assert!(at(0.99) < 250, "{name}: the frame's highlights are blown");
    }
}
