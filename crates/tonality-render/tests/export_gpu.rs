//! What every real export goes through and the other GPU tests don't: more than one tile, a
//! source smaller than the photo it stands for, a crop, quarter turns and a straighten angle,
//! lens correction, masks and the presence maps.
//!
//! The develop of one pixel is checked elsewhere (AgX against its reference, the tone ramp).
//! These tests check *where* pixels come from and go to, by rendering the same picture two ways
//! and comparing: in tiles and in one piece, cropped and whole, turned and upright.
//! Skipped (passes with a note) on machines without a usable adapter, unless
//! `TONALITY_REQUIRE_GPU` is set.

mod common;

use half::f16;
use tonality_core::color::CameraColor;
use tonality_core::edit::Crop;
use tonality_core::geometry::Geometry;
use tonality_core::lens::{
    lens_table, radial, LensCorrection, LensProfile, LensSource, UvToSource,
};
use tonality_core::mask::{brush_layers, Mask, MaskShape};
use tonality_core::EditState;
use tonality_render::presence::presence_maps;
use tonality_render::{curve_lut_rgba16f, DevelopParams, Gpu, Rendered, Source};

const COLOR: CameraColor = CameraColor {
    xyz_to_cam: tonality_core::color::IDENTITY,
    as_shot_wb: [1.0; 3],
};

/// A picture as camera-space pixels, `w × h`.
struct Picture {
    w: u32,
    h: u32,
    pixels: Vec<u16>,
}

impl Picture {
    fn new(w: u32, h: u32, f: impl Fn(u32, u32) -> [f32; 3]) -> Self {
        let pixels = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .flat_map(|(x, y)| f(x, y))
            .map(|v| f16::from_f32(v).to_bits())
            .collect();
        Picture { w, h, pixels }
    }

    /// Smooth in both directions, different in each channel, with every pixel its own value:
    /// a picture in which a shifted, mirrored or swapped tile can't hide.
    fn gradient(w: u32, h: u32) -> Self {
        Self::new(w, h, |x, y| {
            let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
            [0.04 + 0.5 * u, 0.04 + 0.5 * v, 0.3 - 0.2 * u * v]
        })
    }

    /// The gradient with fine detail on it, for the filters that read a pixel's neighbours.
    fn textured(w: u32, h: u32) -> Self {
        let g = Self::gradient(w, h);
        Self::new(w, h, |x, y| {
            let i = ((y * w + x) * 3) as usize;
            let ripple = 0.03 * ((x as f32 * 0.9).sin() * (y as f32 * 0.7).cos());
            [0, 1, 2].map(|c| f16::from_bits(g.pixels[i + c]).to_f32() + ripple)
        })
    }
}

/// How `picture` is rendered: the edit, the lens profile, and the size of the photo it stands
/// for (its own size, unless it is a downscaled source).
struct Job<'a> {
    edit: &'a EditState,
    lens: Option<&'a LensProfile>,
    frame: Option<[u32; 2]>,
    presence: bool,
}

impl<'a> Job<'a> {
    fn of(edit: &'a EditState) -> Self {
        Job {
            edit,
            lens: None,
            frame: None,
            presence: false,
        }
    }
}

fn render(gpu: &Gpu, p: &Picture, job: Job) -> Rendered {
    let frame = job.frame.unwrap_or([p.w, p.h]);
    let mut params = DevelopParams::new(job.edit, &COLOR, job.lens, frame[0], frame[1]);
    let maps = job
        .presence
        .then(|| presence_maps(&p.pixels, p.w, p.h, &COLOR));
    if let Some(m) = &maps {
        params = params.with_presence(m);
    }
    let src = Source {
        width: p.w,
        height: p.h,
        pixels: &p.pixels,
        frame,
    };
    gpu.render(
        &src,
        &params,
        &curve_lut_rgba16f(job.edit),
        &brush_layers(&job.edit.masks, frame[0], frame[1]),
        maps.as_ref(),
        |_| {},
    )
    .expect("render")
}

/// Pixel `(x, y)` of a render, 0–1 per channel.
fn at(r: &Rendered, x: u32, y: u32) -> [f64; 3] {
    let i = ((y * r.width + x) * 3) as usize;
    [0, 1, 2].map(|c| r.pixels[i + c] as f64 / 65535.0)
}

/// A render sampled between pixels: `(u, v)` in 0–1 across it, as a texture would be.
fn sample(r: &Rendered, u: f64, v: f64) -> [f64; 3] {
    let (x, y) = (u * r.width as f64 - 0.5, v * r.height as f64 - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let px = |dx: f64, dy: f64| {
        at(
            r,
            (x0 + dx).clamp(0.0, r.width as f64 - 1.0) as u32,
            (y0 + dy).clamp(0.0, r.height as f64 - 1.0) as u32,
        )
    };
    let (a, b, c, d) = (px(0.0, 0.0), px(1.0, 0.0), px(0.0, 1.0), px(1.0, 1.0));
    [0, 1, 2].map(|k| {
        (a[k] * (1.0 - fx) + b[k] * fx) * (1.0 - fy) + (c[k] * (1.0 - fx) + d[k] * fx) * fy
    })
}

fn far(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0, f64::max)
}

/// The largest difference between two renders of one size, and where.
fn worst(a: &Rendered, b: &Rendered) -> (f64, u32, u32) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let mut worst = (0.0, 0, 0);
    for y in 0..a.height {
        for x in 0..a.width {
            let d = far(at(a, x, y), at(b, x, y));
            if d > worst.0 {
                worst = (d, x, y);
            }
        }
    }
    worst
}

/// One 16-bit step, with room for a GPU's own rounding.
const SAME: f64 = 3.0 / 65535.0;

#[test]
fn tiles_join_without_a_seam() {
    let Some(whole) = common::gpu() else { return };
    let Some(tiled) = common::gpu().map(|g| g.with_tile_size(96)) else {
        return;
    };
    // Sharpening and noise reduction read a pixel's neighbours, which near a tile's edge come
    // from the apron round it; a crop makes the tiles' source rectangles start off the grid.
    let mut edit = EditState::default();
    edit.detail.sharpen_amount = 120.0;
    edit.detail.sharpen_radius = 2.0;
    edit.detail.luminance_noise = 60.0;
    edit.detail.color_noise = 60.0;
    edit.crop.rect = [0.05, 0.1, 0.9, 0.8];
    let p = Picture::textured(500, 380);

    let one = render(&whole, &p, Job::of(&edit));
    let many = render(&tiled, &p, Job::of(&edit));
    assert_eq!((one.width, one.height), (450, 304));
    // 5 × 4 tiles, the last column and row partial.
    let (d, x, y) = worst(&one, &many);
    assert!(
        d <= SAME,
        "in tiles, pixel ({x}, {y}) differs by {d} from the render in one piece"
    );
}

#[test]
fn tiles_join_under_lens_correction_and_a_straighten_angle() {
    let Some(whole) = common::gpu() else { return };
    let Some(tiled) = common::gpu().map(|g| g.with_tile_size(96)) else {
        return;
    };
    // Each tile's source rectangle is the bounding box of a rotated tile, grown by the lens
    // table's margin: a pixel that reads outside it is lost.
    let lens = barrel();
    let mut edit = corrected();
    edit.crop.angle = 7.5;
    edit.crop.rect = [0.15, 0.15, 0.7, 0.7];
    let p = Picture::textured(480, 320);
    let job = || Job {
        lens: Some(&lens),
        ..Job::of(&edit)
    };
    let (one, many) = (render(&whole, &p, job()), render(&tiled, &p, job()));
    // Not bit for bit this time. Turned or bent, a pixel is read between source pixels, and a
    // GPU weighs those with a few bits of fraction, counted from wherever the texture starts:
    // a tile's texture starts elsewhere than the whole picture's, so here and there a pixel
    // rounds the other way (measured: under 1% of them, by at most 0.15%). A tile that is
    // shifted, or reads past its source rectangle, is wrong along a whole edge or all over,
    // and by far more.
    let (d, x, y) = worst(&one, &many);
    assert!(d < 0.004, "pixel ({x}, {y}) differs by {d}");
    let over = (0..one.height)
        .flat_map(|y| (0..one.width).map(move |x| (x, y)))
        .filter(|&(x, y)| far(at(&one, x, y), at(&many, x, y)) > SAME)
        .count();
    let all = (one.width * one.height) as usize;
    assert!(
        over * 50 < all,
        "{over} of {all} pixels differ: more than rounding"
    );
}

#[test]
fn a_crop_is_that_part_of_the_whole_picture() {
    let Some(gpu) = common::gpu() else { return };
    let p = Picture::gradient(400, 300);
    let whole = render(&gpu, &p, Job::of(&EditState::default()));
    assert_eq!((whole.width, whole.height), (400, 300));

    // On whole pixels: 100..300 across, 60..210 down.
    let mut edit = EditState::default();
    edit.crop.rect = [0.25, 0.2, 0.5, 0.5];
    let crop = render(&gpu, &p, Job::of(&edit));
    assert_eq!((crop.width, crop.height), (200, 150));
    for (x, y) in [(0, 0), (199, 0), (0, 149), (199, 149), (77, 31)] {
        let d = far(at(&crop, x, y), at(&whole, x + 100, y + 60));
        assert!(
            d <= SAME,
            "cropped pixel ({x}, {y}) is {d} from the whole picture's"
        );
    }
}

#[test]
fn quarter_turns_and_flips_move_every_pixel_to_its_place() {
    let Some(gpu) = common::gpu() else { return };
    let p = Picture::gradient(60, 40);
    let upright = render(&gpu, &p, Job::of(&EditState::default()));
    let (w, h) = (upright.width, upright.height);
    for turns in 0..4u8 {
        for flip in [false, true] {
            let edit = EditState {
                crop: Crop {
                    turns,
                    flip,
                    ..Crop::default()
                },
                ..EditState::default()
            };
            let turned = render(&gpu, &p, Job::of(&edit));
            let expect = if turns % 2 == 1 { (h, w) } else { (w, h) };
            assert_eq!((turned.width, turned.height), expect, "turns {turns}");
            // Where the framing says each output pixel comes from, it does.
            let g = Geometry::new(&edit.crop, w as f32, h as f32);
            let mut worst = 0.0f64;
            for y in 0..turned.height {
                for x in 0..turned.width {
                    let f = [
                        (x as f32 + 0.5) / turned.width as f32,
                        (y as f32 + 0.5) / turned.height as f32,
                    ];
                    let [u, v] = g.apply(f);
                    let from = at(
                        &upright,
                        ((u * w as f32) as u32).min(w - 1),
                        ((v * h as f32) as u32).min(h - 1),
                    );
                    worst = worst.max(far(at(&turned, x, y), from));
                }
            }
            assert!(worst <= SAME, "turns {turns}, flip {flip}: off by {worst}");
        }
    }
    // And a turn is a turn: clockwise, the top-left corner shows the photo's bottom-left.
    let mut edit = EditState::default();
    edit.crop.turns = 1;
    let turned = render(&gpu, &p, Job::of(&edit));
    assert!(far(at(&turned, 0, 0), at(&upright, 0, h - 1)) <= SAME);
}

#[test]
fn a_straightened_crop_reads_where_the_framing_says() {
    let Some(gpu) = common::gpu() else { return };
    let p = Picture::gradient(400, 300);
    let upright = render(&gpu, &p, Job::of(&EditState::default()));
    let mut edit = EditState::default();
    edit.crop.angle = 12.0;
    edit.crop.rect = [0.2, 0.2, 0.6, 0.6];
    let turned = render(&gpu, &p, Job::of(&edit));
    let g = Geometry::new(&edit.crop, 400.0, 300.0);
    // The picture is smooth, so a pixel of the turned render is the upright render at the
    // place it reads, to within the interpolation.
    for (x, y) in [(0, 0), (239, 0), (0, 179), (239, 179), (120, 90), (33, 150)] {
        let f = [
            (x as f32 + 0.5) / turned.width as f32,
            (y as f32 + 0.5) / turned.height as f32,
        ];
        let [u, v] = g.apply(f);
        let d = far(at(&turned, x, y), sample(&upright, u as f64, v as f64));
        assert!(d < 0.004, "pixel ({x}, {y}) reads ({u}, {v}): off by {d}");
    }
}

#[test]
fn a_downscaled_source_renders_at_its_own_size() {
    let Some(gpu) = common::gpu() else { return };
    // A 6000 × 4000 photo exported at 300 px: the source handed to the renderer is 300 × 200,
    // and the framing, lens and Detail sizes are all still in the photo's pixels.
    let small = Picture::gradient(300, 200);
    let mut edit = EditState::default();
    let job = |edit: &EditState| {
        render(
            &gpu,
            &small,
            Job {
                frame: Some([6000, 4000]),
                ..Job::of(edit)
            },
        )
    };
    let out = job(&edit);
    assert_eq!((out.width, out.height), (300, 200));
    // It is the same picture as rendering the small source as a photo of its own size.
    let own = render(&gpu, &small, Job::of(&edit));
    let (d, x, y) = worst(&out, &own);
    assert!(d <= SAME, "pixel ({x}, {y}) differs by {d}");
    // A crop of the photo is that crop of the small source.
    edit.crop.rect = [0.5, 0.25, 0.5, 0.5];
    let crop = job(&edit);
    assert_eq!((crop.width, crop.height), (150, 100));
    assert!(far(at(&crop, 10, 10), at(&own, 160, 60)) <= SAME);
}

/// An edit with the lens profile switched on.
fn corrected() -> EditState {
    EditState {
        lens: LensCorrection {
            profile: true,
            ..Default::default()
        },
        ..EditState::default()
    }
}

/// A lens that bows lines outwards and darkens the corners.
fn barrel() -> LensProfile {
    LensProfile {
        source: LensSource::Embedded,
        name: "test".into(),
        distortion: Some([radial(|r| 1.0 - 0.06 * r * r); 3]),
        chromatic: false,
        vignetting: Some(radial(|r| 1.0 + 0.5 * r * r)),
    }
}

#[test]
fn lens_correction_reads_each_pixel_where_the_cpu_says_and_with_its_gain() {
    let Some(gpu) = common::gpu() else { return };
    let (w, h) = (360u32, 240u32);
    let edit = corrected();
    let table = |profile: &LensProfile| lens_table(Some(profile), &edit.lens, w as f32, h as f32);

    // Distortion alone, on a smooth picture: a corrected pixel is the uncorrected render at the
    // place `UvToSource` (which Auto and the AI masks use) maps it to.
    let bend = LensProfile {
        vignetting: None,
        ..barrel()
    };
    let p = Picture::gradient(w, h);
    let flat = render(&gpu, &p, Job::of(&EditState::default()));
    let bent = render(
        &gpu,
        &p,
        Job {
            lens: Some(&bend),
            ..Job::of(&edit)
        },
    );
    let map = UvToSource::new(&table(&bend), w, h);
    for (x, y) in [
        (2, 2),
        (357, 2),
        (2, 237),
        (357, 237),
        (180, 120),
        (90, 200),
    ] {
        let uv = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
        let [u, v] = map.map(uv);
        let d = far(at(&bent, x, y), sample(&flat, u as f64, v as f64));
        assert!(d < 0.004, "pixel ({x}, {y}) reads ({u}, {v}): off by {d}");
    }

    // Vignetting alone, on a flat grey: a pixel is the grey times the gain the CPU gives for
    // it, which is to say, what a brighter flat grey develops to with no lens at all.
    let shade = LensProfile {
        distortion: None,
        ..barrel()
    };
    let grey = 0.12f32;
    let lit = render(
        &gpu,
        &Picture::new(w, h, |_, _| [grey; 3]),
        Job {
            lens: Some(&shade),
            ..Job::of(&edit)
        },
    );
    let map = UvToSource::new(&table(&shade), w, h);
    for (x, y) in [(180, 120), (40, 30), (350, 230), (5, 120)] {
        let uv = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
        let (_, gain) = map.map_with_gain(uv);
        let brighter = render(
            &gpu,
            &Picture::new(4, 4, |_, _| [grey * gain; 3]),
            Job::of(&EditState::default()),
        );
        let d = far(at(&lit, x, y), at(&brighter, 1, 1));
        assert!(d < 0.003, "pixel ({x}, {y}), gain {gain}: off by {d}");
    }
    // The corners did get brighter than the middle.
    assert!(at(&lit, 2, 2)[1] > at(&lit, 180, 120)[1] + 0.03);
}

#[test]
fn masks_change_their_own_area_and_nothing_else() {
    let Some(gpu) = common::gpu() else { return };
    let (w, h) = (240u32, 160u32);
    let p = Picture::new(w, h, |_, _| [0.1; 3]);
    let plain = render(&gpu, &p, Job::of(&EditState::default()));
    let brighter = |shape: MaskShape| {
        let mut mask = Mask {
            shape,
            ..Mask::default()
        };
        mask.adjust.exposure = 1.0;
        let edit = EditState {
            masks: vec![mask],
            ..EditState::default()
        };
        render(&gpu, &p, Job::of(&edit))
    };
    let lifted = |r: &Rendered, x: u32, y: u32| at(r, x, y)[1] - at(&plain, x, y)[1];

    // A circle in the middle, a quarter of the long edge across, with a soft edge.
    let radial = brighter(MaskShape::Radial {
        center: [0.5, 0.5],
        radius: [0.125, 0.125],
        angle: 0.0,
        feather: 0.5,
    });
    let full = lifted(&radial, 120, 80);
    assert!(full > 0.05, "the centre is lifted a stop: {full}");
    assert!(
        lifted(&radial, 5, 5).abs() <= SAME,
        "the corner is untouched"
    );
    assert!(lifted(&radial, 200, 80).abs() <= SAME, "outside the circle");
    // In the feather the lift is partial, and it falls off outwards.
    let (inner, outer) = (lifted(&radial, 120 + 18, 80), lifted(&radial, 120 + 26, 80));
    assert!(
        full > inner && inner > outer && outer > 0.0,
        "{full} > {inner} > {outer} > 0"
    );

    // A gradient from the left edge (full) to the middle (none).
    let linear = brighter(MaskShape::Linear {
        from: [0.0, 0.5],
        to: [0.5, 0.5],
    });
    let across: Vec<f64> = [2, 40, 80, 118, 180]
        .map(|x| lifted(&linear, x, 80))
        .to_vec();
    assert!(across.windows(2).all(|p| p[0] >= p[1] - SAME), "{across:?}");
    assert!(across[0] > 0.05 && across[4].abs() <= SAME, "{across:?}");

    // A hidden mask does nothing.
    let mut hidden = Mask {
        enabled: false,
        shape: MaskShape::Radial {
            center: [0.5, 0.5],
            radius: [0.4, 0.4],
            angle: 0.0,
            feather: 0.0,
        },
        ..Mask::default()
    };
    hidden.adjust.exposure = 2.0;
    let edit = EditState {
        masks: vec![hidden],
        ..EditState::default()
    };
    assert!(worst(&render(&gpu, &p, Job::of(&edit)), &plain).0 <= SAME);
}

#[test]
fn clarity_leaves_a_flat_picture_alone_and_strengthens_an_edge() {
    let Some(gpu) = common::gpu() else { return };
    let (w, h) = (256u32, 128u32);
    let mut edit = EditState::default();
    edit.effects.clarity = 80.0;
    let with_maps = |p: &Picture, edit: &EditState| {
        render(
            &gpu,
            p,
            Job {
                presence: true,
                ..Job::of(edit)
            },
        )
    };

    // Nothing to bring out in a flat picture: with its maps bound, Clarity changes nothing.
    // (Maps that weren't bound, or were read as something else, show up here as a shift.)
    let flat = Picture::new(w, h, |_, _| [0.15; 3]);
    let d = worst(
        &with_maps(&flat, &edit),
        &render(&gpu, &flat, Job::of(&EditState::default())),
    );
    assert!(d.0 < 0.004, "a flat picture moved by {}", d.0);

    // A step from dark to light: Clarity pulls the two sides apart near the step.
    let step = Picture::new(w, h, |x, _| if x < w / 2 { [0.08; 3] } else { [0.22; 3] });
    let plain = render(&gpu, &step, Job::of(&EditState::default()));
    let clear = with_maps(&step, &edit);
    let contrast = |r: &Rendered| at(r, w / 2 + 6, h / 2)[1] - at(r, w / 2 - 6, h / 2)[1];
    assert!(
        contrast(&clear) > contrast(&plain) + 0.004,
        "across the step: {} with Clarity, {} without",
        contrast(&clear),
        contrast(&plain)
    );
    // Sharpening does the same at the scale of a pixel or two, with no maps at all.
    let mut sharp = EditState::default();
    sharp.detail.sharpen_amount = 150.0;
    let sharpened = render(&gpu, &step, Job::of(&sharp));
    let edge = |r: &Rendered| at(r, w / 2, h / 2)[1] - at(r, w / 2 - 1, h / 2)[1];
    assert!(edge(&sharpened) > edge(&plain) + 0.005);
}
