//! Healing through the GPU: specks planted on pictures made in code are
//! covered by spots and found as dust. Skips itself on a machine without a GPU.

use image::RgbImage;
use tonality_lib::develop::LinearImage;
use tonality_lib::edit::{Adjustments, Crop, Spot};
use tonality_lib::film::{Film, Kind, Range};
use tonality_lib::gpu::{self, Gpu};
use tonality_lib::heal;

fn gpu() -> Option<&'static Gpu> {
    gpu::shared().inspect_err(|error| eprintln!("skipping: {error:#}")).ok()
}

const SIZE: (u32, u32) = (160, 120);
/// The speck: where it is and how far it reaches, in pixels.
const SPECK: (f32, f32, f32) = (60.0, 50.0, 4.0);

/// Repeatable noise in -1..1.
fn noise(x: u32, y: u32) -> f32 {
    let mut n = x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263);
    n = (n ^ (n >> 13)).wrapping_mul(1274126177);
    ((n ^ (n >> 16)) & 0xffff) as f32 / 32767.5 - 1.0
}

/// A photo that brightens left to right and warms top to bottom, with fine
/// grain, and a speck of `speck` (added light) if that isn't zero.
fn photo(speck: f32) -> LinearImage {
    let (width, height) = SIZE;
    let pixels = (0..width * height)
        .map(|i| {
            let (x, y) = (i % width, i / width);
            let d = ((x as f32 + 0.5 - SPECK.0).powi(2) + (y as f32 + 0.5 - SPECK.1).powi(2)).sqrt();
            let dust = speck * (1.0 - ((d - SPECK.2 * 0.6) / (SPECK.2 * 0.4)).clamp(0.0, 1.0));
            let base = 0.08 + 0.3 * x as f32 / width as f32 + 0.004 * noise(x, y);
            [base + 0.05 * y as f32 / height as f32 + dust, base + dust, base - 0.03 * y as f32 / height as f32 + dust]
        })
        .collect();
    LinearImage { width, height, pixels, scene_referred: false }
}

/// A spot over the speck, taking its texture from `away` pixels to the right.
fn spot(away: f32) -> Spot {
    let (width, height) = SIZE;
    Spot {
        points: vec![[SPECK.0 / width as f32, SPECK.1 / height as f32]],
        radius: 6.5 / width as f32,
        source: [(SPECK.0 + away) / width as f32, SPECK.1 / height as f32],
    }
}

/// The mean of each channel over a box of `reach` around (`x`, `y`).
fn mean(picture: &RgbImage, (x, y): (f32, f32), reach: f32) -> [f32; 3] {
    let (mut sum, mut count) = ([0.0; 3], 0.0);
    for py in (y - reach) as u32..=(y + reach) as u32 {
        for px in (x - reach) as u32..=(x + reach) as u32 {
            let p = picture.get_pixel(px, py).0;
            sum = std::array::from_fn(|c| sum[c] + p[c] as f32);
            count += 1.0;
        }
    }
    sum.map(|v| v / count)
}

/// The largest difference between two pictures, in 8-bit steps.
fn largest_difference(a: &RgbImage, b: &RgbImage) -> u8 {
    assert_eq!(a.dimensions(), b.dimensions());
    a.as_raw().iter().zip(b.as_raw()).map(|(a, b)| a.abs_diff(*b)).max().unwrap()
}

#[test]
fn a_spot_covers_a_speck_with_the_brightness_and_colour_around_it() {
    let Some(gpu) = gpu() else { return };
    let dusty = gpu.open(photo(0.4)).unwrap();
    let clean = gpu.render_image(&gpu.open(photo(0.0)).unwrap(), &Adjustments::default(), u32::MAX).unwrap();
    let before = gpu.render_image(&dusty, &Adjustments::default(), u32::MAX).unwrap();
    let at = (SPECK.0, SPECK.1);
    assert!(mean(&before, at, 2.0)[1] > mean(&clean, at, 2.0)[1] + 60.0, "the speck shows");

    // Its patch is 25 pixels to the right, where the photo is a good deal brighter.
    let healed = gpu.render_image(&dusty, &Adjustments { spots: vec![spot(25.0)], ..Default::default() }, u32::MAX).unwrap();
    let (fixed, wanted) = (mean(&healed, at, 4.0), mean(&clean, at, 4.0));
    for c in 0..3 {
        assert!((fixed[c] - wanted[c]).abs() <= 2.0, "healed {fixed:?}, around it {wanted:?}");
    }
    let patch = mean(&clean, (SPECK.0 + 25.0, SPECK.1), 4.0);
    assert!(patch[1] > wanted[1] + 8.0, "a plain copy of the patch would have shown");
    // No pixel stands out: the texture is the patch's grain, nothing more.
    assert!(largest_difference(&healed, &clean) <= 6, "{} steps off", largest_difference(&healed, &clean));
    // And the rest of the photo is untouched.
    for (x, y, pixel) in healed.enumerate_pixels() {
        let far = (x as f32 + 0.5 - SPECK.0).hypot(y as f32 + 0.5 - SPECK.1) > 7.5;
        let was = before.get_pixel(x, y);
        if far {
            assert!(pixel.0.iter().zip(was.0).all(|(a, b)| a.abs_diff(b) <= 1), "({x}, {y}) changed from {was:?} to {pixel:?}");
        }
    }

    // Taking the spot away brings the speck back.
    let again = gpu.render_image(&dusty, &Adjustments::default(), u32::MAX).unwrap();
    assert!(again == before);
}

#[test]
fn overlapping_spots_each_see_the_ones_before_them_healed() {
    let Some(gpu) = gpu() else { return };
    // A second speck just below the first, close enough that their spots overlap.
    let mut image = photo(0.4);
    let below = (SPECK.0 + 2.0, SPECK.1 + 9.0);
    for (i, pixel) in image.pixels.iter_mut().enumerate() {
        let (x, y) = ((i as u32 % SIZE.0) as f32 + 0.5, (i as u32 / SIZE.0) as f32 + 0.5);
        if (x - below.0).hypot(y - below.1) < 2.5 {
            *pixel = pixel.map(|v| v + 0.4);
        }
    }
    let session = gpu.open(image).unwrap();
    let clean = gpu.render_image(&gpu.open(photo(0.0)).unwrap(), &Adjustments::default(), u32::MAX).unwrap();
    let second = Spot {
        points: vec![[below.0 / SIZE.0 as f32, below.1 / SIZE.1 as f32]],
        radius: 6.5 / SIZE.0 as f32,
        source: [(below.0 - 24.0) / SIZE.0 as f32, below.1 / SIZE.1 as f32],
    };
    let off = [vec![spot(25.0), second.clone()], vec![second, spot(25.0)]].map(|spots| {
        let healed = gpu.render_image(&session, &Adjustments { spots, ..Default::default() }, u32::MAX).unwrap();
        largest_difference(&healed, &clean)
    });
    assert!(off.iter().all(|off| *off <= 6), "{off:?} steps off");
}

#[test]
fn a_spot_stays_on_its_speck_through_crops_turns_and_flips() {
    let Some(gpu) = gpu() else { return };
    let dusty = gpu.open(photo(0.4)).unwrap();
    let clean = gpu.open(photo(0.0)).unwrap();
    let crop = Crop { x: 0.45, y: 0.5, width: 0.7, height: 0.8 };
    for (rotation, flip_horizontal, straighten) in [(1, false, 0.0), (2, true, 0.0), (3, false, 8.0), (0, true, -5.0)] {
        let framed = Adjustments { crop, rotation, flip_horizontal, straighten, ..Default::default() };
        let healed = Adjustments { spots: vec![spot(-22.0)], ..framed.clone() };
        let want = gpu.render_image(&clean, &framed, u32::MAX).unwrap();
        let speck = largest_difference(&gpu.render_image(&dusty, &framed, u32::MAX).unwrap(), &want);
        let left = largest_difference(&gpu.render_image(&dusty, &healed, u32::MAX).unwrap(), &want);
        assert!(speck > 60, "the speck shows in the frame (turned {rotation})");
        assert!(left <= 8, "turned {rotation}, flipped {flip_horizontal}, straightened {straighten}: {left} steps off");
    }
}

#[test]
fn a_line_covers_a_scratch() {
    let Some(gpu) = gpu() else { return };
    let (width, height) = SIZE;
    // A thin bright scratch across the middle, a little slanted.
    let mut image = photo(0.0);
    let clean = gpu.render_image(&gpu.open(photo(0.0)).unwrap(), &Adjustments::default(), u32::MAX).unwrap();
    let on_scratch = |x: f32, y: f32| (y - (55.0 + 0.1 * (x - 30.0))).abs() < 1.2 && (30.0..=130.0).contains(&x);
    for (i, pixel) in image.pixels.iter_mut().enumerate() {
        let (x, y) = ((i as u32 % width) as f32 + 0.5, (i as u32 / width) as f32 + 0.5);
        if on_scratch(x, y) {
            *pixel = pixel.map(|v| v + 0.5);
        }
    }
    let session = gpu.open(image).unwrap();
    let line = Spot {
        points: vec![[28.0 / width as f32, 54.8 / height as f32], [132.0 / width as f32, 65.2 / height as f32]],
        radius: 4.0 / width as f32,
        source: [28.0 / width as f32, 40.0 / height as f32],
    };
    let healed = gpu.render_image(&session, &Adjustments { spots: vec![line], ..Default::default() }, u32::MAX).unwrap();
    let off = largest_difference(&healed, &clean);
    assert!(off <= 8, "{off} steps off");
}

#[test]
fn a_negative_is_healed_in_its_positive() {
    let Some(gpu) = gpu() else { return };
    // Dust on a negative blocks the light: dark in the scan, a white speck once turned.
    let negative = |speck: bool| {
        let mut image = photo(0.0);
        image.scene_referred = true;
        for (i, pixel) in image.pixels.iter_mut().enumerate() {
            let (x, y) = ((i as u32 % SIZE.0) as f32 + 0.5, (i as u32 / SIZE.0) as f32 + 0.5);
            let dust = speck && (x - SPECK.0).hypot(y - SPECK.1) < SPECK.2 * 0.8;
            *pixel = pixel.map(|v| if dust { 0.002 } else { 0.7 * 10f32.powf(-0.6 * (v / 0.01).max(1.0).log10()) });
        }
        image
    };
    // A set range, so the speck can't change how the rest of the frame is stretched.
    let range = Range { low: [0.5; 3], high: [1.0; 3] };
    let film = Film { kind: Kind::BlackAndWhite, base: Some([0.7; 3]), range: Some(range) };
    let recipe = Adjustments { film, ..Default::default() };
    let dusty = gpu.open(negative(true)).unwrap();
    let want = gpu.render_image(&gpu.open(negative(false)).unwrap(), &recipe, u32::MAX).unwrap();
    let before = gpu.render_image(&dusty, &recipe, u32::MAX).unwrap();
    assert!(mean(&before, (SPECK.0, SPECK.1), 1.0)[0] > mean(&want, (SPECK.0, SPECK.1), 1.0)[0] + 60.0, "the speck is white");

    let healed = gpu.render_image(&dusty, &Adjustments { spots: vec![spot(20.0)], ..recipe.clone() }, u32::MAX).unwrap();
    let off = largest_difference(&healed, &want);
    assert!(off <= 8, "{off} steps off");

    // And found as dust there, light on its surroundings, with a patch to cover it.
    let area = gpu.working_area(&dusty, &recipe, heal::DUST_EDGE).unwrap();
    let found = heal::find_dust(&area, SIZE, true, &[], |_| true);
    assert_eq!(found.len(), 1, "{found:?}");
    let speck = &found[0];
    let at = [speck.center[0] * SIZE.0 as f32, speck.center[1] * SIZE.1 as f32];
    assert!((at[0] - SPECK.0).hypot(at[1] - SPECK.1) < 1.5, "found at {at:?}");
    let covered = Spot { points: vec![speck.center], radius: speck.radius, source: speck.source };
    let healed = gpu.render_image(&dusty, &Adjustments { spots: vec![covered], ..recipe }, u32::MAX).unwrap();
    let off = largest_difference(&healed, &want);
    assert!(off <= 10, "healed as found, {off} steps off");
}

#[test]
fn a_patch_is_chosen_from_the_photo_as_it_is_drawn() {
    let Some(gpu) = gpu() else { return };
    let session = gpu.open(photo(0.4)).unwrap();
    let points = [[SPECK.0 / SIZE.0 as f32, SPECK.1 / SIZE.1 as f32]];
    let radius = 6.5 / SIZE.0 as f32;
    let area = gpu.area_around(&session, &Adjustments::default(), &points, radius).unwrap();
    let spot = Spot { points: points.to_vec(), radius, source: [0.0; 2] };
    let source = heal::find_source(&area, &spot, SIZE, &[]);
    let (sx, sy) = (source[0] * SIZE.0 as f32, source[1] * SIZE.1 as f32);
    let away = (sx - SPECK.0).hypot(sy - SPECK.1);
    assert!((13.0..=45.0).contains(&away), "the patch is {away} pixels away");
    // The photo brightens to the right: a patch straight up or down matches it best.
    assert!((sx - SPECK.0).abs() < (sy - SPECK.1).abs() + 4.0, "the patch at ({sx}, {sy}) is off to the side");
}

/// Looks for dust in real scans and saves each as found (specks ringed) and
/// healed, at full size, for a look:
/// `TONALITY_SAMPLES=… TONALITY_OUT=… cargo test --test integration dust_in_real -- --ignored --nocapture`.
/// The recipe is a black and white negative unless `TONALITY_RECIPE` gives one as JSON.
#[test]
#[ignore = "needs TONALITY_SAMPLES, TONALITY_OUT and a GPU"]
fn dust_in_real_photos() {
    use std::path::PathBuf;
    use std::time::Instant;
    let samples = PathBuf::from(std::env::var_os("TONALITY_SAMPLES").expect("set TONALITY_SAMPLES"));
    let out = PathBuf::from(std::env::var_os("TONALITY_OUT").expect("set TONALITY_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let recipe: Adjustments = std::env::var("TONALITY_RECIPE")
        .map(|json| serde_json::from_str(&json).unwrap())
        .unwrap_or(Adjustments { film: Film { kind: Kind::BlackAndWhite, ..Default::default() }, ..Default::default() });
    let Some(gpu) = gpu() else { return };
    for entry in walkdir::WalkDir::new(samples).into_iter().flatten().filter(|e| e.file_type().is_file()) {
        let Some(kind) = tonality_lib::media::kind_of(entry.path()) else { continue };
        let name = entry.path().file_stem().unwrap().to_string_lossy().into_owned();
        let image = tonality_lib::develop::load(entry.path(), kind == tonality_lib::media::Kind::Raw).unwrap();
        let session = gpu.open(image).unwrap();
        let size = (session.width, session.height);
        gpu.render_image(&session, &recipe, 64).unwrap();

        let started = Instant::now();
        let area = gpu.working_area(&session, &recipe, heal::DUST_EDGE).unwrap();
        let read = started.elapsed();
        let inside = heal::inside_crop(size.0, size.1, &recipe);
        let found = heal::find_dust(&area, size, recipe.film.is_negative(), &recipe.spots, inside);
        println!("{name:<12} {} specks; read in {read:?}, looked through in {:?}", found.len(), started.elapsed() - read);

        let mut marked = gpu.render_image(&session, &recipe, u32::MAX).unwrap();
        let long = size.0.max(size.1) as f32;
        let frame = tonality_lib::geometry::frame(size.0, size.1, &recipe, false);
        let to_frame = tonality_lib::geometry::frame_to_source(size.0, size.1, &recipe, &frame).inverse();
        for speck in &found {
            let [u, v] = to_frame.apply(speck.center.map(f64::from));
            let (cx, cy) = (u as f32 * marked.width() as f32, v as f32 * marked.height() as f32);
            let r = speck.radius * long + 3.0;
            for step in 0..720 {
                let angle = step as f32 / 720.0 * std::f32::consts::TAU;
                for (reach, level) in [(r, 255u8), (r + 1.5, 0)] {
                    let (x, y) = ((cx + reach * angle.cos()) as i64, (cy + reach * angle.sin()) as i64);
                    if x >= 0 && y >= 0 && (x as u32) < marked.width() && (y as u32) < marked.height() {
                        marked.put_pixel(x as u32, y as u32, image::Rgb([level; 3]));
                    }
                }
            }
        }
        marked.save(out.join(format!("{name}-found.jpg"))).unwrap();

        let spots = found.iter().map(|s| Spot { points: vec![s.center], radius: s.radius, source: s.source }).collect();
        let mut healed = Adjustments { spots, ..recipe.clone() };
        let started = Instant::now();
        gpu.render_image(&session, &healed, 1024).unwrap();
        println!("{name:<12} healed and drawn small in {:?}", started.elapsed());
        // As when a spot or its patch is dragged: the spots change, frame after frame.
        if let Some(last) = healed.spots.pop() {
            let started = Instant::now();
            for step in 0..10 {
                let mut moved = last.clone();
                moved.source[0] += step as f32 * 0.001;
                let spots = healed.spots.iter().cloned().chain([moved]).collect();
                gpu.render_image(&session, &Adjustments { spots, ..healed.clone() }, 1024).unwrap();
            }
            println!("{name:<12} a spot moved, drawn again in {:?} a frame", started.elapsed() / 10);
            healed.spots.push(last);
        }
        gpu.render_image(&session, &healed, u32::MAX).unwrap().save(out.join(format!("{name}-healed.jpg"))).unwrap();
    }
}
