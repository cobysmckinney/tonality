//! Checks of the GPU develop pipeline against images whose right answer is
//! known. Skipped (with a note) on machines without a usable GPU.

use image::{Rgb, RgbImage};
use tempfile::TempDir;
use tonality_lib::develop;
use tonality_lib::edit::Adjustments;
use tonality_lib::gpu::{self, Gpu, Region, Session};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 64;

/// A test card: a grey ramp across the top half, saturated colour bars below.
fn test_card() -> RgbImage {
    const BARS: [[u8; 3]; 4] = [[200, 40, 40], [40, 180, 60], [40, 40, 220], [128, 128, 128]];
    RgbImage::from_fn(WIDTH, HEIGHT, |x, y| {
        if y < HEIGHT / 2 {
            Rgb([x as u8; 3])
        } else {
            Rgb(BARS[(x / (WIDTH / 4)) as usize])
        }
    })
}

struct Fixture {
    gpu: &'static Gpu,
    session: Session,
    card: RgbImage,
}

fn fixture() -> Option<Fixture> {
    // One device for every test, as in the app: graphics drivers are not
    // reliably happy about many devices being created and torn down at once.
    let gpu = match gpu::shared() {
        Ok(gpu) => gpu,
        Err(error) => {
            eprintln!("skipping: {error:#}");
            return None;
        }
    };
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("card.png");
    let card = test_card();
    card.save(&path).unwrap();
    let session = gpu.open(develop::load(&path, false).unwrap()).unwrap();
    Some(Fixture { gpu, session, card })
}

impl Fixture {
    fn render(&self, adjustments: &Adjustments) -> RgbImage {
        self.gpu.render_image(&self.session, adjustments, WIDTH).unwrap()
    }
}

fn recipe(json: &str) -> Adjustments {
    serde_json::from_str(json).unwrap()
}

fn largest_difference(a: &RgbImage, b: &RgbImage) -> u8 {
    a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| x.abs_diff(*y)).max().unwrap()
}

#[test]
fn an_unedited_picture_comes_out_as_it_went_in() {
    let Some(f) = fixture() else { return };
    let out = f.render(&Adjustments::default());
    assert_eq!(out.dimensions(), f.card.dimensions());
    assert!(largest_difference(&out, &f.card) <= 1, "off by {}", largest_difference(&out, &f.card));
}

#[test]
fn exposure_works_in_stops_of_light() {
    let Some(f) = fixture() else { return };
    let out = f.render(&recipe(r#"{"exposure": 1.0}"#));
    // sRGB 100 is 12.7% linear; one stop up is 25.4%, which encodes to 137.
    assert!(out.get_pixel(100, 4).0.iter().all(|v| v.abs_diff(137) <= 1), "{:?}", out.get_pixel(100, 4));
    assert_eq!(out.get_pixel(0, 4).0, [0, 0, 0], "black stays black");
    assert_eq!(out.get_pixel(255, 4).0, [255, 255, 255], "white clips at white");
}

#[test]
fn an_inverted_curve_gives_a_negative() {
    let Some(f) = fixture() else { return };
    let out = f.render(&recipe(r#"{"curves": {"master": [[0, 1], [1, 0]]}}"#));
    for x in [0, 60, 128, 200, 255] {
        let expected = 255 - x as u8;
        assert!(out.get_pixel(x, 4).0.iter().all(|v| v.abs_diff(expected) <= 1), "at {x}: {:?}", out.get_pixel(x, 4));
    }

    let red_only = f.render(&recipe(r#"{"curves": {"red": [[0, 1], [1, 0]]}}"#));
    let pixel = red_only.get_pixel(60, 4).0;
    assert!(pixel[0].abs_diff(195) <= 1 && pixel[1].abs_diff(60) <= 1 && pixel[2].abs_diff(60) <= 1, "{pixel:?}");
}

#[test]
fn removing_saturation_leaves_only_greys() {
    let Some(f) = fixture() else { return };
    let out = f.render(&recipe(r#"{"saturation": -100}"#));
    for x in [30, 90, 150, 220] {
        let [r, g, b] = out.get_pixel(x, 48).0;
        assert!(r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1, "bar at {x} still has colour: {:?}", [r, g, b]);
    }
    // Greys were grey already.
    assert!(out.get_pixel(128, 4).0.iter().all(|v| v.abs_diff(128) <= 1));
}

#[test]
fn the_colour_mixer_only_touches_its_own_hues() {
    let Some(f) = fixture() else { return };
    let mut edit = Adjustments::default();
    edit.mixer[5].saturation = -100.0; // blue
    let out = f.render(&edit);
    let [r, g, b] = out.get_pixel(160, 48).0;
    assert!(r.abs_diff(g) <= 2 && g.abs_diff(b) <= 2, "the blue bar should lose its colour: {:?}", [r, g, b]);
    for x in [30, 90] {
        let (before, after) = (f.card.get_pixel(x, 48).0, out.get_pixel(x, 48).0);
        assert!(before.iter().zip(after).all(|(a, b)| a.abs_diff(b) <= 1), "red and green bars are untouched");
    }
}

#[test]
fn a_region_renders_just_that_part_of_the_picture() {
    let Some(f) = fixture() else { return };
    let right_half = Region { x: 0.5, y: 0.0, width: 0.5, height: 1.0 };
    let rgba = f.gpu.render(&f.session, &Adjustments::default(), right_half, WIDTH / 2, HEIGHT, false, false).unwrap();
    assert_eq!(rgba.len() as u32, WIDTH / 2 * HEIGHT * 4);
    // Left edge of the frame is the middle of the ramp; right edge is its end.
    assert!(rgba[0].abs_diff(128) <= 1 && rgba[(WIDTH as usize / 2 - 1) * 4].abs_diff(255) <= 1);
    assert_eq!(rgba[3], 255, "frames are opaque");
}

#[test]
fn clipping_warnings_mark_pure_white_and_black() {
    let Some(f) = fixture() else { return };
    let rgba = f.gpu.render(&f.session, &Adjustments::default(), Region::FULL, WIDTH, HEIGHT, true, false).unwrap();
    let pixel = |x: usize| &rgba[x * 4..x * 4 + 3];
    assert!(pixel(255)[0] > 200 && pixel(255)[2] < 60, "white is flagged red: {:?}", pixel(255));
    assert!(pixel(0)[2] > 200 && pixel(0)[0] < 60, "black is flagged blue: {:?}", pixel(0));
    assert_eq!(pixel(128), [128, 128, 128], "midtones are left alone");
}

#[test]
fn a_crop_keeps_only_its_part_at_its_own_size() {
    let Some(f) = fixture() else { return };
    // The right half of the top (grey ramp) half.
    let out = f.render(&recipe(r#"{"crop": {"x": 0.75, "y": 0.25, "width": 0.5, "height": 0.5}}"#));
    assert_eq!(out.dimensions(), (WIDTH / 2, HEIGHT / 2));
    assert!(out.get_pixel(0, 0).0[0].abs_diff(128) <= 1);
    assert!(out.get_pixel(WIDTH / 2 - 1, HEIGHT / 2 - 1).0[0].abs_diff(255) <= 1);
}

#[test]
fn a_quarter_turn_swaps_the_sides_and_goes_clockwise() {
    let Some(f) = fixture() else { return };
    let out = f.gpu.render_image(&f.session, &recipe(r#"{"rotation": 1}"#), WIDTH).unwrap();
    assert_eq!(out.dimensions(), (HEIGHT, WIDTH));
    // The card's top edge (the ramp) is now the right-hand edge, running
    // dark at the top to light at the bottom.
    assert!(out.get_pixel(HEIGHT - 2, 2).0[0] <= 3);
    assert!(out.get_pixel(HEIGHT - 2, WIDTH - 3).0[0] >= 252);
    // And the red bar, bottom left on the card, is now top left.
    let [r, g, b] = out.get_pixel(4, 20).0;
    assert!(r > 180 && g < 60 && b < 60, "{:?}", [r, g, b]);
}

#[test]
fn a_flip_mirrors_the_picture() {
    let Some(f) = fixture() else { return };
    let out = f.render(&recipe(r#"{"flipHorizontal": true}"#));
    for x in [0u32, 60, 200, 255] {
        assert!(out.get_pixel(x, 4).0[0].abs_diff(255 - x as u8) <= 1, "at {x}");
    }
}

#[test]
fn the_crop_tool_view_shows_the_whole_tilted_photo_with_empty_corners() {
    let Some(f) = fixture() else { return };
    let edit = recipe(r#"{"straighten": 20, "crop": {"x": 0.5, "y": 0.5, "width": 0.2, "height": 0.2}}"#);
    let (width, height) = f.gpu.frame_size(&f.session, &edit, true);
    assert!(width > WIDTH && height > HEIGHT, "the tilted card needs a bigger box: {width}x{height}");
    let rgba = f.gpu.render(&f.session, &edit, Region::FULL, width, height, false, true).unwrap();
    let alpha = |x: u32, y: u32| rgba[((y * width + x) * 4 + 3) as usize];
    assert_eq!(alpha(1, 1), 0, "corners lie outside the photo");
    assert_eq!(alpha(width / 2, height / 2), 255, "the middle is the photo");
}
