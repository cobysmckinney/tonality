//! Finding parts of a photo with small segmentation models: its main
//! subject, its sky, and the object inside a circle drawn on it.
//!
//! Each model (`models/README.md`) sees the photo squeezed to a square and
//! says how likely each pixel is to belong to what it looks for: IS-Net for
//! the subject, U²-Netp checked by a scene parser for the sky, EfficientSAM
//! for the object in a circle's box. That rough map is then fitted to the
//! photo's own edges by a guided filter, so hair and branches are followed
//! far more finely than the model's view allows.
//!
//! A matte is worked out from the photo file, the right way up but before
//! any edits, and kept on disk next to the thumbnails. Once the photo is
//! cropped, the subject and sky models are shown just the crop, the way it
//! is seen (`Framing`), so what was cropped away (a film scan's holder and
//! rebate, say) can't be taken for either. The matte is still laid over the
//! whole photo file, so it follows turns and flips like every other mask part.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, bail, Context, Result};
use image::{GrayImage, RgbImage};
use rayon::prelude::*;
use tract_linalg::multithread::{multithread_tract_scope, Executor};
use tract_onnx::prelude::*;

use crate::develop::LinearImage;
use crate::edit::{Adjustments, Crop, Shape};
use crate::geometry::{self, Affine};
use crate::gpu::fit_within;
use crate::masks::COVERAGE_EDGE;
use crate::media;

/// ImageNet's colour normalisation, which most of the models were trained with.
const IMAGENET: Normalise = Normalise { mean: [0.485, 0.456, 0.406], deviation: [0.229, 0.224, 0.225] };

/// The long edge at which the guided filter works out how the matte follows
/// the photo; the answer is then applied at the guide's full size.
const FILTER_EDGE: u32 = 1024;
/// The guided filter's window radius at `FILTER_EDGE`, in pixels.
const FILTER_RADIUS: usize = 8;
/// How much the filter trusts the model over the photo's edges: smaller
/// follows the photo's edges more closely, and its noise too.
const FILTER_EPSILON: f32 = 1e-3;
/// The filtered matte is stretched so that this much or less is none of the
/// part and this much or more is all of it. A model's answer fades out over
/// a few of its pixels, which at full size is a wide faint halo; below the
/// low end it is that halo, and the high end fills the part in.
const MATTE_LEVELS: (f32, f32) = (0.15, 0.85);

/// How far past the circle drawn an object may still reach, as a share of
/// the circle's longer side; circles are drawn roughly.
const CIRCLE_REACH: f32 = 0.04;
/// EfficientSAM sees the photo squeezed to this many pixels square.
const SAM_INPUT: usize = 1024;

/// How a model wants its input's colours: each channel, 0..1, less `mean` and over `deviation`.
struct Normalise {
    mean: [f32; 3],
    deviation: [f32; 3],
}

/// One of the bundled models, and how its answer is read.
struct Model {
    name: &'static str,
    onnx: &'static [u8],
    /// Its input, a square this many pixels a side.
    input: u32,
    normalise: Normalise,
    /// Whether its answer is stretched to fill 0..1. The subject model's is,
    /// as its authors do: there is always a most likely subject. The sky
    /// model's is not: a photo may have no sky.
    stretch: bool,
    loaded: Loaded,
}

type Loaded = OnceLock<Result<Arc<TypedRunnableModel>, String>>;

/// Loads a bundled model once, with its inputs fixed to `inputs`.
fn load(slot: &'static Loaded, name: &str, onnx: &[u8], inputs: Vec<InferenceFact>) -> Result<&'static Arc<TypedRunnableModel>> {
    slot.get_or_init(|| {
        let load = || -> TractResult<Arc<TypedRunnableModel>> {
            let mut model = tract_onnx::onnx().model_for_read(&mut &onnx[..])?;
            for (index, fact) in inputs.into_iter().enumerate() {
                model = model.with_input_fact(index, fact)?;
            }
            model.into_optimized()?.into_runnable()
        };
        load().map_err(|error| format!("{error:#}"))
    })
    .as_ref()
    .map_err(|error| anyhow!("The {name} model could not be loaded: {error}"))
}

/// Runs a model on the app's thread pool.
fn run(model: &Arc<TypedRunnableModel>, name: &str, inputs: TVec<TValue>) -> Result<TVec<TValue>> {
    multithread_tract_scope(Executor::RayonGlobal, || model.run(inputs)).map_err(|error| anyhow!("The {name} model failed: {error:#}"))
}

/// Values over a whole picture, `width` x `height` of them.
struct Grid {
    values: Vec<f32>,
    width: usize,
    height: usize,
}

impl Grid {
    /// The value at `u, v` in 0..1 across the picture, bilinearly.
    fn at(&self, u: f32, v: f32) -> f32 {
        sample(&self.values, self.width, self.height, u, v)
    }
}

static SUBJECT: Model = Model {
    name: "subject",
    onnx: include_bytes!("../models/isnet_general_use.f16.onnx"),
    input: 1024,
    normalise: Normalise { mean: [0.5; 3], deviation: [1.0; 3] },
    stretch: true,
    loaded: OnceLock::new(),
};
static SKY: Model = Model {
    name: "sky",
    onnx: include_bytes!("../models/u2netp_sky.onnx"),
    input: 384,
    normalise: IMAGENET,
    stretch: false,
    loaded: OnceLock::new(),
};
/// The scene parser: how likely each part of the photo is to be each of
/// ADE20K's 150 kinds of thing, on a grid an eighth of its input's size.
static SCENE: Model = Model {
    name: "sky",
    onnx: include_bytes!("../models/ade20k_mobilenetv2.onnx"),
    input: 512,
    normalise: IMAGENET,
    stretch: false,
    loaded: OnceLock::new(),
};
/// The sky, among the scene parser's classes.
const SCENE_SKY: usize = 2;

/// Names the subject model and the way its answer is refined. Part of each
/// cached matte's file name, so changing either finds every matte again.
/// Bump `gpu::LOOK_VERSION` too, so thumbnails drawn with the old mattes go.
/// A matte found in a crop also carries the crop's name (`Framing::name`).
const SUBJECT_TAG: &str = "isnet-1";
const SKY_TAG: &str = "skyseg-ade-1";

impl Model {
    /// The model's answer for its square view of `picture`: its first output, and that output's shape.
    fn answer(&'static self, picture: &RgbImage) -> Result<(Vec<f32>, Vec<usize>)> {
        let side = self.input as usize;
        let model = load(&self.loaded, self.name, self.onnx, vec![f32::fact([1, 3, side, side]).into()])?;
        let small = resize(picture, self.input, self.input)?;
        let Normalise { mean, deviation } = self.normalise;
        let input: Tensor = tract_ndarray::Array4::from_shape_fn((1, 3, side, side), |(_, c, y, x)| {
            (small.get_pixel(x as u32, y as u32)[c] as f32 / 255.0 - mean[c]) / deviation[c]
        })
        .into();
        let outputs = run(model, self.name, tvec!(input.into()))?;
        let view = outputs[0].to_plain_array_view::<f32>().map_err(|error| anyhow!("{error:#}"))?;
        Ok((view.iter().copied().collect(), view.shape().to_vec()))
    }

    /// How likely each pixel of the model's square view of `picture` is to be what it finds, 0..1.
    fn run(&'static self, picture: &RgbImage) -> Result<Grid> {
        let side = self.input as usize;
        let (mut values, _) = self.answer(picture)?;
        if self.stretch {
            let (low, high) = values.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
            let span = (high - low).max(1e-6);
            values.iter_mut().for_each(|v| *v = (*v - low) / span);
        }
        Ok(Grid { values, width: side, height: side })
    }

    /// The model's answer for `picture`, fitted to its edges, the same size.
    fn matte(&'static self, picture: &RgbImage) -> Result<GrayImage> {
        Ok(refine(picture, &self.run(picture)?))
    }
}

static SAM_ENCODER: Loaded = OnceLock::new();
static SAM_DECODER: Loaded = OnceLock::new();
const SAM_ENCODER_ONNX: &[u8] = include_bytes!("../models/efficient_sam_vitt_encoder.onnx");
const SAM_DECODER_ONNX: &[u8] = include_bytes!("../models/efficient_sam_vitt_decoder.onnx");
const OBJECT_TAG: &str = "esam-2";

/// A photo as the models see it: small and in sRGB (`guide`), with what
/// EfficientSAM makes of it kept once worked out, as that takes a few
/// seconds and every circle drawn on the photo uses it.
pub struct Picture {
    pub guide: RgbImage,
    embedding: Mutex<Option<TValue>>,
}

impl Picture {
    pub fn new(guide: RgbImage) -> Self {
        Self { guide, embedding: Mutex::new(None) }
    }

    /// EfficientSAM's view of the photo, worked out the first time it is wanted.
    /// Worth asking for early, before a circle is finished.
    pub fn prepare_circles(&self) -> Result<()> {
        self.embedding().map(|_| ())
    }

    fn embedding(&self) -> Result<TValue> {
        // Held while the encoder runs, so a circle drawn meanwhile waits for it rather than running it again.
        let mut kept = self.embedding.lock().unwrap();
        if let Some(embedding) = kept.as_ref() {
            return Ok(embedding.clone());
        }
        let encoder = load(&SAM_ENCODER, "object", SAM_ENCODER_ONNX, vec![f32::fact([1, 3, SAM_INPUT, SAM_INPUT]).into()])?;
        let small = resize(&self.guide, SAM_INPUT as u32, SAM_INPUT as u32)?;
        let input: Tensor = tract_ndarray::Array4::from_shape_fn((1, 3, SAM_INPUT, SAM_INPUT), |(_, c, y, x)| {
            small.get_pixel(x as u32, y as u32)[c] as f32 / 255.0
        })
        .into();
        let embedding = run(encoder, "object", tvec!(input.into()))?.remove(0);
        *kept = Some(embedding.clone());
        Ok(embedding)
    }
}

/// A part of the photo a model finds, rather than one drawn.
#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    Subject,
    Sky,
    /// The object inside a circle: its points on the photo file, 0..1.
    Object(Vec<[f32; 2]>),
}

impl Found {
    /// What a mask part asks to be found, if anything.
    pub fn of(shape: &Shape) -> Option<Found> {
        match shape {
            Shape::Subject => Some(Found::Subject),
            Shape::Sky => Some(Found::Sky),
            Shape::Object { points } => Some(Found::Object(points.clone())),
            _ => None,
        }
    }

    /// What this part is, in words: "the subject".
    pub fn name(&self) -> &'static str {
        match self {
            Found::Subject => "the subject",
            Found::Sky => "the sky",
            Found::Object(_) => "the object",
        }
    }

    /// Names this matte among the photo's, when found in `framing`: its
    /// cache file is named after it. A circled object is always found in the
    /// whole photo, its circle being enough to keep it to its place.
    pub fn key(&self, framing: &Framing) -> String {
        match self {
            Found::Subject => format!("subject-{SUBJECT_TAG}{}", framing.name()),
            Found::Sky => format!("sky-{SKY_TAG}{}", framing.name()),
            Found::Object(points) => {
                let bytes: Vec<u8> = points.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
                let hash = blake3::hash(&bytes).to_hex();
                format!("object-{}-{OBJECT_TAG}", &hash[..16])
            }
        }
    }

    /// The matte for `picture` when the models are shown `framing` of it, the
    /// size of its guide: white where this part is.
    pub fn find(&self, picture: &Picture, framing: &Framing) -> Result<GrayImage> {
        match self {
            Found::Subject => framing.find_in(&picture.guide, |view| SUBJECT.matte(view)),
            Found::Sky => framing.find_in(&picture.guide, find_sky),
            Found::Object(points) => find_object(picture, points),
        }
    }

    /// `find`, but a matte found before is read from `cache`, and a new one is written there.
    pub fn find_cached(&self, picture: &Picture, framing: &Framing, cache: Option<&Path>) -> Result<GrayImage> {
        if let Some(cached) = cache.and_then(|path| image::open(path).ok()) {
            return Ok(cached.into_luma8());
        }
        let matte = self.find(picture, framing)?;
        if let Some(path) = cache {
            // A matte that can't be kept is found again next time; not worth failing over.
            let _ = write_png(&matte, path);
        }
        Ok(matte)
    }
}

/// What the subject and sky models are shown of a photo: all of it, as the
/// file has it, until it is cropped; then just the crop, the way it is seen
/// (turned, flipped and straightened).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Framing {
    #[default]
    Whole,
    Cropped(Cropped),
}

/// A crop of a `width` x `height` photo, with the turns, flips and
/// straightening it is seen with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cropped {
    width: u32,
    height: u32,
    crop: Crop,
    straighten: f32,
    rotation: u8,
    flip_horizontal: bool,
    flip_vertical: bool,
}

impl Framing {
    /// What the models are shown of a `width` x `height` photo edited with
    /// `adjustments`. Turns and flips alone leave it whole: the matte follows
    /// them anyway, and a photo only turned is found as it always was.
    pub fn of(width: u32, height: u32, adjustments: &Adjustments) -> Framing {
        let crop = adjustments.crop;
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let whole = near(crop.x, 0.5)
            && near(crop.y, 0.5)
            && crop.width > 0.999
            && crop.height > 0.999
            && near(adjustments.straighten, 0.0);
        if whole {
            return Framing::Whole;
        }
        Framing::Cropped(Cropped {
            width,
            height,
            crop,
            straighten: adjustments.straighten,
            rotation: adjustments.rotation % 4,
            flip_horizontal: adjustments.flip_horizontal,
            flip_vertical: adjustments.flip_vertical,
        })
    }

    /// Added to the names of mattes found in this framing; nothing for the
    /// whole photo, so its mattes keep the names they always had.
    fn name(&self) -> String {
        match self {
            Framing::Whole => String::new(),
            Framing::Cropped(cropped) => {
                let Crop { x, y, width, height } = cropped.crop;
                let text = format!(
                    "{x:.4},{y:.4},{width:.4},{height:.4},{:.2},{},{},{}",
                    cropped.straighten, cropped.rotation, cropped.flip_horizontal, cropped.flip_vertical
                );
                format!("-in-{}", &blake3::hash(text.as_bytes()).to_hex()[..10])
            }
        }
    }

    /// `find`'s matte for `guide` (the whole photo file), found in this
    /// framing: the crop is cut out and turned, `find` is shown it, and its
    /// answer is laid back over the whole photo, empty outside the crop.
    fn find_in(&self, guide: &RgbImage, find: impl FnOnce(&RgbImage) -> Result<GrayImage>) -> Result<GrayImage> {
        match self {
            Framing::Whole => find(guide),
            Framing::Cropped(cropped) => {
                let to_source = cropped.on_source();
                let view = show(guide, &to_source, cropped.size_in(guide.dimensions()));
                Ok(place(&find(&view)?, &to_source, guide.dimensions()))
            }
        }
    }
}

impl Cropped {
    fn adjustments(&self) -> Adjustments {
        Adjustments {
            crop: self.crop,
            straighten: self.straighten,
            rotation: self.rotation,
            flip_horizontal: self.flip_horizontal,
            flip_vertical: self.flip_vertical,
            ..Default::default()
        }
    }

    /// Where each point of the crop as seen (0..1 across and down) is on the photo file (0..1).
    fn on_source(&self) -> Affine {
        let adjustments = self.adjustments();
        let frame = geometry::frame(self.width, self.height, &adjustments, false);
        geometry::frame_to_source(self.width, self.height, &adjustments, &frame)
    }

    /// The crop's size as seen, in pixels of a copy of the photo `width` x `height` big.
    fn size_in(&self, (width, height): (u32, u32)) -> (u32, u32) {
        let [w, h] = geometry::frame(self.width, self.height, &self.adjustments(), false).size;
        let scale = width.max(height) as f64 / self.width.max(self.height) as f64;
        // Never so small that the models and the filter have nothing to work on.
        let side = |v: f64| ((v * scale).round() as u32).max(MIN_VIEW);
        (side(w), side(h))
    }
}

/// The fewest pixels a side of a crop is shown to the models with.
const MIN_VIEW: u32 = 32;

/// The part of `guide` that `to_source` maps 0..1 onto, as a `width` x
/// `height` picture.
fn show(guide: &RgbImage, to_source: &Affine, (width, height): (u32, u32)) -> RgbImage {
    let (gw, gh) = guide.dimensions();
    let mut view = RgbImage::new(width, height);
    view.par_chunks_mut(width as usize * 3).enumerate().for_each(|(y, row)| {
        let v = (y as f64 + 0.5) / height as f64;
        for (x, out) in row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let [u, v] = to_source.apply([(x as f64 + 0.5) / width as f64, v]);
            *out = std::array::from_fn(|c| {
                bilinear(gw, gh, u as f32, v as f32, |x, y| guide.get_pixel(x, y)[c] as f32).round() as u8
            });
        }
    });
    view
}

/// `matte`, found in the part of the photo that `to_source` maps 0..1 onto,
/// laid back over the whole photo at `width` x `height`: empty outside that
/// part, except that its edge reaches a pixel or two past it, so the crop's
/// own edge is covered as fully as the matte says.
fn place(matte: &GrayImage, to_source: &Affine, (width, height): (u32, u32)) -> GrayImage {
    let to_view = to_source.inverse();
    let (mw, mh) = matte.dimensions();
    let reach = [1.5 / mw as f64, 1.5 / mh as f64];
    let outside = |p: f64, reach: f64| p < -reach || p > 1.0 + reach;
    let mut out = GrayImage::new(width, height);
    out.par_chunks_mut(width as usize).enumerate().for_each(|(y, row)| {
        let v = (y as f64 + 0.5) / height as f64;
        for (x, out) in row.iter_mut().enumerate() {
            let [s, t] = to_view.apply([(x as f64 + 0.5) / width as f64, v]);
            if !outside(s, reach[0]) && !outside(t, reach[1]) {
                *out = bilinear(mw, mh, s as f32, t as f32, |x, y| matte.get_pixel(x, y)[0] as f32).round() as u8;
            }
        }
    });
    out
}

/// A `width` x `height` picture's value at `u, v` in 0..1, bilinearly from
/// `at(x, y)`, its edges carried on past it.
fn bilinear(width: u32, height: u32, u: f32, v: f32, at: impl Fn(u32, u32) -> f32) -> f32 {
    let x = (u * width as f32 - 0.5).clamp(0.0, (width - 1) as f32);
    let y = (v * height as f32 - 0.5).clamp(0.0, (height - 1) as f32);
    let (x0, y0) = (x as u32, y as u32);
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
    top + (bottom - top) * fy
}

/// The picture the subject is found in: the photo in sRGB, at most
/// `COVERAGE_EDGE` on its long side, averaged down from the working image.
/// RAW files are given the editor's own look first: the models were trained
/// on finished photos, and see too little in a RAW's dark, flat linear values.
pub fn guide(image: &LinearImage) -> RgbImage {
    let (width, height) = fit_within(image.width, image.height, COVERAGE_EDGE.min(image.width.max(image.height)));
    let (sx, sy) = (image.width as f32 / width as f32, image.height as f32 / height as f32);
    // The working-image pixels under guide pixel `i`, along one side.
    let span = |i: usize, scale: f32, limit: u32| {
        let start = ((i as f32 * scale) as usize).min(limit as usize - 1);
        start..(((i + 1) as f32 * scale) as usize).clamp(start + 1, limit as usize)
    };
    let stride = image.width as usize;
    let mut bytes = vec![0u8; (width * height * 3) as usize];
    bytes.par_chunks_mut(width as usize * 3).enumerate().for_each(|(y, row)| {
        let lines = span(y, sy, image.height);
        for (x, out) in row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let columns = span(x, sx, image.width);
            let mut sum = [0f32; 3];
            for line in lines.clone() {
                for pixel in &image.pixels[line * stride + columns.start..line * stride + columns.end] {
                    sum = std::array::from_fn(|c| sum[c] + pixel[c]);
                }
            }
            let count = (lines.len() * columns.len()) as f32;
            let average = sum.map(|v| v / count);
            *out = if image.scene_referred { base_look(average, true) } else { average }.map(encode);
        }
    });
    RgbImage::from_raw(width, height, bytes).expect("sized to fit")
}

/// The picture the models are shown of a film negative: its positive
/// (`width` x `height` pixels in scene light, already the guide's size),
/// with the editor's look. Nothing in it is a clipped sensor channel.
pub fn guide_of_positive(width: u32, height: u32, pixels: &[[f32; 3]]) -> RgbImage {
    let bytes = pixels.par_iter().flat_map_iter(|&pixel| base_look(pixel, false).map(encode)).collect();
    RgbImage::from_raw(width, height, bytes).expect("sized to fit")
}

/// A display-linear value as an 8-bit sRGB level.
pub(crate) fn encode(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let v = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (v * 255.0).round() as u8
}

/// The editor's built-in look for RAW files (`base_look` in develop.wgsl,
/// with the fade of clipped colours to white before it): scene values to
/// display-linear, the sensor's clipping point on white.
pub(crate) fn base_look(c: [f32; 3], fade_clipped: bool) -> [f32; 3] {
    const EXPOSURE: f32 = 1.4;
    const SATURATION: f32 = 1.05;
    let curve = |x: f32| (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14);
    let peak = c[0].max(c[1]).max(c[2]);
    let fade = if fade_clipped { smoothstep(0.82, 1.0, peak) } else { 0.0 };
    let toned = c.map(|v| (curve((v + (peak - v) * fade).max(0.0) * EXPOSURE) / curve(EXPOSURE)).clamp(0.0, 1.0));
    let luma = 0.2126 * toned[0] + 0.7152 * toned[1] + 0.0722 * toned[2];
    toned.map(|v| luma + (v - luma) * SATURATION)
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The sky in `guide`. The sky model follows its edges finely, but takes
/// snow and white buildings against the sky for more of it, and misses the
/// sky through leaves when it is blown out. The scene parser sees each
/// part's place in the scene, but only on a coarse grid. So the sky model's
/// answer is kept where the parser allows it, and where the sky model saw
/// none, the parser's is taken, narrowed to what is as bright as the sky
/// that is sure.
fn find_sky(guide: &RgbImage) -> Result<GrayImage> {
    let fine = SKY.run(guide)?;
    let (classes, shape) = SCENE.answer(guide)?;
    let cells = shape[3];
    let plane = cells * cells;
    // Closed, so a small cloud the parser wasn't sure of stays part of the sky around it.
    let parsed = Grid { values: closing(&classes[SCENE_SKY * plane..(SCENE_SKY + 1) * plane], cells, 1), width: cells, height: cells };

    let (width, height) = (fine.width, fine.height);
    let at = |i: usize| (((i % width) as f32 + 0.5) / width as f32, ((i / width) as f32 + 0.5) / height as f32);
    // Where the answer is the parser's alone: it sees sky and the sky model doesn't.
    let alone: Vec<f32> = (0..width * height)
        .map(|i| {
            let (u, v) = at(i);
            parsed.at(u, v) * (1.0 - fine.values[i] * 2.0).clamp(0.0, 1.0)
        })
        .collect();
    let values = (0..width * height)
        .map(|i| {
            let (u, v) = at(i);
            (fine.values[i] * smoothstep(SKY_ALLOWED.0, SKY_ALLOWED.1, parsed.at(u, v))).max(alone[i])
        })
        .collect();
    let mut matte = refine(guide, &Grid { values, width, height });

    let luma = |p: &image::Rgb<u8>| (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0;
    let mut sure: Vec<f32> = guide.pixels().zip(matte.pixels()).filter(|(_, m)| m[0] > 230).map(|(p, _)| luma(p)).collect();
    if sure.len() < 64 {
        return Ok(matte);
    }
    let middle = sure.len() / 2;
    let reference = *sure.select_nth_unstable_by(middle, f32::total_cmp).1;
    let (low, high) = (reference * SKY_BRIGHTNESS.0, (reference * SKY_BRIGHTNESS.1).max(reference * SKY_BRIGHTNESS.0 + 1e-3));
    let alone = Grid { values: alone, width, height };
    let (gw, gh) = guide.dimensions();
    matte.par_chunks_mut(gw as usize).enumerate().for_each(|(y, row)| {
        let v = (y as f32 + 0.5) / gh as f32;
        for (x, m) in row.iter_mut().enumerate() {
            let share = alone.at((x as f32 + 0.5) / gw as f32, v);
            if share > 0.0 && *m > 0 {
                let bright = smoothstep(low, high, luma(guide.get_pixel(x as u32, y as u32)));
                *m = (*m as f32 * (1.0 - share + share * bright)).round() as u8;
            }
        }
    });
    Ok(matte)
}

/// The sky model's answer counts where the scene parser gives the sky at
/// least the first of these, fully from the second. Snow it takes for sky
/// is around 0.01..0.04, a cloud it isn't sure of 0.05 or more.
const SKY_ALLOWED: (f32, f32) = (0.05, 0.3);
/// Where only the scene parser sees sky, a pixel is none of it below the
/// first of these shares of the sure sky's brightness, and all of it from
/// the second.
const SKY_BRIGHTNESS: (f32, f32) = (0.6, 0.85);

/// `values` (a `size` x `size` grid) closed by a `(2r+1)²` square: holes and
/// notches narrower than that are filled, and larger shapes are left alone.
fn closing(values: &[f32], size: usize, radius: usize) -> Vec<f32> {
    let rank = |values: &[f32], pick: fn(f32, f32) -> f32| -> Vec<f32> {
        (0..size * size)
            .map(|i| {
                let (x, y) = (i % size, i / size);
                let mut v = values[i];
                for yy in y.saturating_sub(radius)..(y + radius + 1).min(size) {
                    for xx in x.saturating_sub(radius)..(x + radius + 1).min(size) {
                        v = pick(v, values[yy * size + xx]);
                    }
                }
                v
            })
            .collect()
    };
    rank(&rank(values, f32::max), f32::min)
}

/// The object inside the circle `points` (0..1 on the photo): what
/// EfficientSAM finds in the circle's box, fitted to the photo's edges and
/// kept to the circle and a little past it.
fn find_object(picture: &Picture, points: &[[f32; 2]]) -> Result<GrayImage> {
    let guide = &picture.guide;
    let (width, height) = guide.dimensions();
    let at: Vec<[f32; 2]> = points.iter().map(|[x, y]| [x * width as f32, y * height as f32]).collect();
    let (left, top, right, bottom) = at.iter().fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |(l, t, r, b), [x, y]| {
        (l.min(*x), t.min(*y), r.max(*x), b.max(*y))
    });
    let long = (right - left).max(bottom - top);
    if at.len() < 3 || long < 8.0 {
        bail!("Circle a larger part of the photo.");
    }

    // The circle's box, as a prompt: its corners are labelled 2 and 3.
    let embedding = picture.embedding()?;
    let side = SAM_INPUT as f32;
    let corners = [left / width as f32, top / height as f32, right / width as f32, bottom / height as f32].map(|v| v * side);
    let decoder = load(
        &SAM_DECODER,
        "object",
        SAM_DECODER_ONNX,
        vec![
            f32::fact([1, 256, 64, 64]).into(),
            f32::fact([1, 1, 2, 2]).into(),
            f32::fact([1, 1, 2]).into(),
            InferenceFact::from(tensor1(&[SAM_INPUT as i64, SAM_INPUT as i64])),
        ],
    )?;
    let box_prompt = Tensor::from_shape(&[1, 1, 2, 2], &corners)?;
    let labels = Tensor::from_shape(&[1, 1, 2], &[2f32, 3.0])?;
    let size = tensor1(&[SAM_INPUT as i64, SAM_INPUT as i64]);
    let outputs = run(decoder, "object", tvec!(embedding, box_prompt.into(), labels.into(), size.into()))?;
    let logits = outputs[0].to_plain_array_view::<f32>().map_err(|error| anyhow!("{error:#}"))?;
    let scores = outputs[1].to_plain_array_view::<f32>().map_err(|error| anyhow!("{error:#}"))?;
    // Of its three guesses, the one it is surest of.
    let best = (0..scores.len()).max_by(|&a, &b| scores[[0, 0, a]].total_cmp(&scores[[0, 0, b]])).unwrap_or(0);
    let likely = Grid {
        values: logits.slice(tract_ndarray::s![0, 0, best, .., ..]).iter().map(|v| 1.0 / (1.0 + (-v).exp())).collect(),
        width: SAM_INPUT,
        height: SAM_INPUT,
    };

    // Fitted to the photo's edges within the box, with room around it.
    let margin = long * 0.15;
    let x0 = (left - margin).max(0.0) as u32;
    let y0 = (top - margin).max(0.0) as u32;
    let x1 = ((right + margin).ceil() as u32).min(width).max(x0 + 1);
    let y1 = ((bottom + margin).ceil() as u32).min(height).max(y0 + 1);
    let (bw, bh) = (x1 - x0, y1 - y0);
    let crop = image::imageops::crop_imm(guide, x0, y0, bw, bh).to_image();
    let (gw, gh) = fit_within(bw, bh, FILTER_EDGE.min(bw.max(bh)));
    let in_crop = Grid {
        values: (0..gw * gh)
            .map(|i| {
                let u = (x0 as f32 + ((i % gw) as f32 + 0.5) / gw as f32 * bw as f32) / width as f32;
                let v = (y0 as f32 + ((i / gw) as f32 + 0.5) / gh as f32 * bh as f32) / height as f32;
                likely.at(u, v)
            })
            .collect(),
        width: gw as usize,
        height: gh as usize,
    };
    let found = refine(&crop, &in_crop);

    // The circle, filled, then blurred and lifted, so it reaches a little
    // past where it was drawn and fades out from there.
    let inside: Vec<f32> = (0..bw * bh)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % bw) as f32 + 0.5 + x0 as f32, (i / bw) as f32 + 0.5 + y0 as f32);
            if encloses(&at, x, y) { 1.0 } else { 0.0 }
        })
        .collect();
    let reach = ((long * CIRCLE_REACH) as usize).max(1);
    let near = box_filter(&inside, bw as usize, bh as usize, reach);

    let mut matte = GrayImage::new(width, height);
    for y in 0..bh {
        for x in 0..bw {
            let keep = (near[(y * bw + x) as usize] * 2.0).min(1.0);
            let value = found.get_pixel(x, y)[0] as f32 * keep;
            matte.put_pixel(x0 + x, y0 + y, image::Luma([value.round() as u8]));
        }
    }
    Ok(matte)
}

/// Whether the outline `points` encloses `x, y` (even-odd).
fn encloses(points: &[[f32; 2]], x: f32, y: f32) -> bool {
    let mut inside = false;
    let mut previous = points[points.len() - 1];
    for &point in points {
        let ([xa, ya], [xb, yb]) = (previous, point);
        if (ya > y) != (yb > y) && x < xa + (y - ya) / (yb - ya) * (xb - xa) {
            inside = !inside;
        }
        previous = point;
    }
    inside
}

/// Fits the model's rough map to the guide's edges: a guided filter (He,
/// Sun and Tang) whose coefficients are worked out at `FILTER_EDGE` and
/// applied at the guide's full size.
fn refine(guide: &RgbImage, likely: &Grid) -> GrayImage {
    let (width, height) = guide.dimensions();
    let (fw, fh) = fit_within(width, height, FILTER_EDGE.min(width.max(height)));
    let small = resize(guide, fw, fh).expect("resizing to a non-empty size");
    let (fw, fh) = (fw as usize, fh as usize);
    let pixels = fw * fh;

    // The guide's colours and the model's answer at the filter's size.
    let colour: [Vec<f32>; 3] =
        std::array::from_fn(|c| small.as_raw().iter().skip(c).step_by(3).map(|&v| v as f32 / 255.0).collect());
    let p: Vec<f32> = (0..pixels)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % fw, i / fw);
            likely.at((x as f32 + 0.5) / fw as f32, (y as f32 + 0.5) / fh as f32)
        })
        .collect();

    let mean = |values: &[f32]| box_filter(values, fw, fh, FILTER_RADIUS);
    let product = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(a, b)| a * b).collect::<Vec<f32>>();
    let mean_i: [Vec<f32>; 3] = std::array::from_fn(|c| mean(&colour[c]));
    let mean_p = mean(&p);
    let mean_ip: [Vec<f32>; 3] = std::array::from_fn(|c| mean(&product(&colour[c], &p)));
    // The colour covariance, as its six distinct entries: rr rg rb gg gb bb.
    const PAIRS: [(usize, usize); 6] = [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)];
    let mean_ii: Vec<Vec<f32>> = PAIRS.iter().map(|&(a, b)| mean(&product(&colour[a], &colour[b]))).collect();

    // Per pixel, the linear fit of the answer to the colours around it.
    let fits: Vec<[f32; 4]> = (0..pixels)
        .into_par_iter()
        .map(|i| {
            let m = [mean_i[0][i], mean_i[1][i], mean_i[2][i]];
            let cov = |k: usize| {
                let (a, b) = PAIRS[k];
                mean_ii[k][i] - m[a] * m[b] + if a == b { FILTER_EPSILON } else { 0.0 }
            };
            let s = [[cov(0), cov(1), cov(2)], [cov(1), cov(3), cov(4)], [cov(2), cov(4), cov(5)]];
            let cip: [f32; 3] = std::array::from_fn(|c| mean_ip[c][i] - m[c] * mean_p[i]);
            let a = solve3(s, cip);
            let b = mean_p[i] - a[0] * m[0] - a[1] * m[1] - a[2] * m[2];
            [a[0], a[1], a[2], b]
        })
        .collect();
    let smooth: [Vec<f32>; 4] = std::array::from_fn(|k| mean(&fits.iter().map(|fit| fit[k]).collect::<Vec<_>>()));

    // Applied at full size, against the guide's own colours.
    let mut out = vec![0u8; (width * height) as usize];
    out.par_chunks_mut(width as usize).enumerate().for_each(|(y, row)| {
        let v = (y as f32 + 0.5) / height as f32;
        for (x, out) in row.iter_mut().enumerate() {
            let u = (x as f32 + 0.5) / width as f32;
            let [r, g, b] = guide.get_pixel(x as u32, y as u32).0.map(|v| v as f32 / 255.0);
            let at = |k: usize| sample(&smooth[k], fw, fh, u, v);
            let q = at(0) * r + at(1) * g + at(2) * b + at(3);
            *out = (levels(q) * 255.0).round() as u8;
        }
    });
    GrayImage::from_raw(width, height, out).expect("sized to the guide")
}

/// `MATTE_LEVELS` applied to one value, with a smooth start and end.
fn levels(q: f32) -> f32 {
    smoothstep(MATTE_LEVELS.0, MATTE_LEVELS.1, q)
}

/// `s · a = b` for a symmetric 3 x 3 `s`, by Cramer's rule.
fn solve3(s: [[f32; 3]; 3], b: [f32; 3]) -> [f32; 3] {
    let det = |m: [[f32; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(s);
    if d.abs() < 1e-12 {
        return [0.0; 3];
    }
    std::array::from_fn(|column| {
        let mut m = s;
        for row in 0..3 {
            m[row][column] = b[row];
        }
        det(m) / d
    })
}

/// The mean of each pixel's `(2r+1)²` neighbourhood, the window cut short at the edges.
fn box_filter(values: &[f32], width: usize, height: usize, radius: usize) -> Vec<f32> {
    // Across: a running sum along each row.
    let mut across = vec![0f32; values.len()];
    across.par_chunks_mut(width).zip(values.par_chunks(width)).for_each(|(out, row)| {
        let mut prefix = vec![0f64; width + 1];
        for x in 0..width {
            prefix[x + 1] = prefix[x] + row[x] as f64;
        }
        for (x, out) in out.iter_mut().enumerate() {
            let (lo, hi) = (x.saturating_sub(radius), (x + radius + 1).min(width));
            *out = ((prefix[hi] - prefix[lo]) / (hi - lo) as f64) as f32;
        }
    });
    // Down: running sums of whole rows.
    let mut prefix = vec![0f64; (height + 1) * width];
    for y in 0..height {
        for x in 0..width {
            prefix[(y + 1) * width + x] = prefix[y * width + x] + across[y * width + x] as f64;
        }
    }
    let mut out = vec![0f32; values.len()];
    out.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        let (lo, hi) = (y.saturating_sub(radius), (y + radius + 1).min(height));
        for (x, out) in row.iter_mut().enumerate() {
            *out = ((prefix[hi * width + x] - prefix[lo * width + x]) / (hi - lo) as f64) as f32;
        }
    });
    out
}

/// `values` (a `width` x `height` grid) at `u, v` in 0..1, bilinearly.
fn sample(values: &[f32], width: usize, height: usize, u: f32, v: f32) -> f32 {
    let x = (u * width as f32 - 0.5).clamp(0.0, (width - 1) as f32);
    let y = (v * height as f32 - 0.5).clamp(0.0, (height - 1) as f32);
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |x: usize, y: usize| values[y * width + x];
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
    top + (bottom - top) * fy
}

/// `image` scaled to exactly `width` x `height`.
fn resize(image: &RgbImage, width: u32, height: u32) -> Result<RgbImage> {
    use fast_image_resize::images::Image;
    use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};

    let source = Image::from_vec_u8(image.width(), image.height(), image.as_raw().clone(), PixelType::U8x3)?;
    let mut target = Image::new(width, height, PixelType::U8x3);
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));
    Resizer::new().resize(&source, &mut target, &options)?;
    RgbImage::from_raw(width, height, target.into_vec()).context("resized image has the wrong size")
}

/// Writes `matte` as a PNG, atomically, so a half-written file is never read.
fn write_png(matte: &GrayImage, dest: &Path) -> Result<()> {
    use image::ImageEncoder;
    media::write_atomically(dest, |writer| {
        image::codecs::png::PngEncoder::new(writer).write_image(
            matte.as_raw(),
            matte.width(),
            matte.height(),
            image::ExtendedColorType::L8,
        )?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_matte_is_cached_as_a_png_and_a_failed_one_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let matte = GrayImage::from_fn(6, 3, |x, _| image::Luma([x as u8 * 40]));
        let dest = dir.path().join("ab/1.png");
        write_png(&matte, &dest).unwrap();
        assert_eq!(image::open(&dest).unwrap().into_luma8(), matte);

        // A file can't be renamed over a folder that has something in it.
        let blocked = dir.path().join("2.png");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("keep"), b"x").unwrap();
        assert!(write_png(&matte, &blocked).is_err());
        for folder in [dir.path(), dest.parent().unwrap()] {
            let names: Vec<_> = std::fs::read_dir(folder).unwrap().map(|e| e.unwrap().file_name()).collect();
            assert!(names.iter().all(|name| !name.to_string_lossy().ends_with(".part")), "{names:?}");
        }
    }

    #[test]
    fn box_filter_matches_counting_by_hand() {
        let (width, height, radius) = (7, 5, 2);
        let values: Vec<f32> = (0..width * height).map(|i| ((i * 37) % 11) as f32).collect();
        let filtered = box_filter(&values, width, height, radius);
        for y in 0..height {
            for x in 0..width {
                let (mut sum, mut count) = (0.0, 0.0);
                for yy in y.saturating_sub(radius)..(y + radius + 1).min(height) {
                    for xx in x.saturating_sub(radius)..(x + radius + 1).min(width) {
                        sum += values[yy * width + xx];
                        count += 1.0;
                    }
                }
                assert!((filtered[y * width + x] - sum / count).abs() < 1e-4, "at {x},{y}");
            }
        }
    }

    #[test]
    fn the_filter_snaps_a_soft_edge_to_the_photos_edge() {
        // The photo is dark left of x = 40 and light right of it; the
        // model's answer ramps softly across 30..50.
        let guide = RgbImage::from_fn(80, 60, |x, _| if x < 40 { image::Rgb([30, 40, 50]) } else { image::Rgb([220, 200, 180]) });
        let side = 320;
        let values: Vec<f32> =
            (0..side * side).map(|i| (((i % side) as f32 / side as f32 * 80.0 - 30.0) / 20.0).clamp(0.0, 1.0)).collect();
        let matte = refine(&guide, &Grid { values, width: side, height: side });
        let at = |x| matte.get_pixel(x, 30)[0] as i32;
        // The model's ramp climbs about 13 levels a pixel, 52 over four;
        // the filtered one climbs mostly in the step across the edge, and
        // less than half as much either side of it.
        assert!(at(40) - at(39) > 90, "across the edge {} to {}", at(39), at(40));
        assert!(at(39) - at(36) < 26 && at(44) - at(40) < 26, "{} {} {} {}", at(36), at(39), at(40), at(44));
        assert!(at(5) < 10 && at(75) > 245);
    }

    /// Red discs of radius 40 at these centres, on a plain grey-blue ground.
    fn discs(centres: &[(f32, f32)]) -> RgbImage {
        RgbImage::from_fn(300, 200, |x, y| {
            let on = centres.iter().any(|(cx, cy)| (x as f32 - cx).hypot(y as f32 - cy) < 40.0);
            if on { image::Rgb([230, 60, 40]) } else { image::Rgb([90, 120, 140]) }
        })
    }

    #[test]
    fn a_bright_disc_on_a_plain_background_is_the_subject() {
        let guide = discs(&[(150.0, 100.0)]);
        let matte = Found::Subject.find(&Picture::new(guide), &Framing::Whole).unwrap();
        assert_eq!(matte.dimensions(), (300, 200));
        assert!(matte.get_pixel(150, 100)[0] > 230, "centre");
        for (x, y) in [(5, 5), (294, 5), (5, 194), (294, 194)] {
            assert!(matte.get_pixel(x, y)[0] < 25, "corner {x},{y}");
        }
    }

    #[test]
    fn a_circle_finds_the_object_inside_it_and_nothing_outside() {
        let guide = discs(&[(70.0, 100.0), (220.0, 100.0)]);
        // A rough loop around the right-hand disc, as drawn by hand.
        let circle: Vec<[f32; 2]> = (0..24)
            .map(|i| {
                let turn = i as f32 / 24.0 * std::f32::consts::TAU;
                [(220.0 + 62.0 * turn.cos()) / 300.0, (100.0 + 58.0 * turn.sin()) / 200.0]
            })
            .collect();
        let matte = Found::Object(circle).find(&Picture::new(guide), &Framing::Whole).unwrap();
        assert!(matte.get_pixel(220, 100)[0] > 230, "the circled disc");
        assert!(matte.get_pixel(190, 100)[0] > 200, "all of it");
        assert_eq!(matte.get_pixel(70, 100)[0], 0, "not the other disc");
        assert!(matte.get_pixel(220, 160)[0] < 25, "nor the ground inside the circle");
    }

    #[test]
    fn a_circle_too_small_to_see_into_is_refused() {
        let guide = discs(&[(150.0, 100.0)]);
        assert!(Found::Object(vec![[0.5, 0.5], [0.501, 0.5], [0.5, 0.501]]).find(&Picture::new(guide), &Framing::Whole).is_err());
    }

    #[test]
    fn the_sky_is_found_above_the_land_and_not_in_a_photo_without_one() {
        // Blue sky over the top half, darkening upward; dark green land below.
        let landscape = RgbImage::from_fn(300, 200, |_, y| {
            if y < 100 { image::Rgb([90 + y as u8 / 2, 150 + y as u8 / 2, 230]) } else { image::Rgb([40, 70, 30]) }
        });
        let matte = Found::Sky.find(&Picture::new(landscape), &Framing::Whole).unwrap();
        assert!(matte.get_pixel(150, 30)[0] > 200, "sky {}", matte.get_pixel(150, 30)[0]);
        assert!(matte.get_pixel(150, 170)[0] < 30, "land {}", matte.get_pixel(150, 170)[0]);
        // Leaves and earth fill this one: there is no sky, and the answer is not stretched to find one.
        let foliage = RgbImage::from_fn(300, 200, |x, y| {
            let leaf = (x / 7 * 31 + y / 5 * 17) % 5;
            image::Rgb([30 + leaf as u8 * 12, 60 + leaf as u8 * 15, 20 + leaf as u8 * 4])
        });
        let matte = Found::Sky.find(&Picture::new(foliage), &Framing::Whole).unwrap();
        assert!(matte.pixels().all(|p| p[0] < 30), "{:?}", matte.pixels().map(|p| p[0]).max());
    }

    #[test]
    fn each_found_part_has_its_own_key() {
        let a = Found::Object(vec![[0.1, 0.1], [0.5, 0.1], [0.3, 0.5]]);
        let b = Found::Object(vec![[0.1, 0.1], [0.5, 0.1], [0.3, 0.6]]);
        assert_eq!(a.key(&Framing::Whole), a.clone().key(&Framing::Whole));
        assert_ne!(a.key(&Framing::Whole), b.key(&Framing::Whole));
        assert!(Found::Subject.key(&Framing::Whole).starts_with("subject-") && Found::Sky.key(&Framing::Whole).starts_with("sky-"));
    }

    fn framing(width: u32, height: u32, adjustments: Adjustments) -> Cropped {
        match Framing::of(width, height, &adjustments) {
            Framing::Cropped(cropped) => cropped,
            Framing::Whole => panic!("{adjustments:?} is cropped"),
        }
    }

    #[test]
    fn an_uncropped_photo_is_shown_whole_and_its_mattes_keep_their_names() {
        let turned = Adjustments { rotation: 1, flip_horizontal: true, ..Default::default() };
        for adjustments in [Adjustments::default(), turned] {
            assert_eq!(Framing::of(300, 200, &adjustments), Framing::Whole);
        }
        // The names mattes were kept under before crops counted.
        assert_eq!(Found::Subject.key(&Framing::Whole), "subject-isnet-1");
        assert_eq!(Found::Sky.key(&Framing::Whole), "sky-skyseg-ade-1");

        let crop = |x| Framing::of(300, 200, &Adjustments { crop: Crop { x, y: 0.5, width: 0.5, height: 1.0 }, ..Default::default() });
        let (left, right) = (crop(0.25), crop(0.75));
        assert_ne!(Found::Subject.key(&left), Found::Subject.key(&Framing::Whole));
        assert_ne!(Found::Subject.key(&left), Found::Subject.key(&right));
        assert_eq!(Found::Subject.key(&left), Found::Subject.key(&crop(0.25)));
        let straightened = Framing::of(300, 200, &Adjustments { straighten: 3.0, ..Default::default() });
        assert_ne!(straightened, Framing::Whole, "straightening crops the corners off");
        // A circled object is found in the whole photo whatever the crop.
        let object = Found::Object(vec![[0.1, 0.1], [0.5, 0.1], [0.3, 0.5]]);
        assert_eq!(object.key(&left), object.key(&Framing::Whole));
    }

    /// A photo whose red says how far across a pixel is and green how far down.
    fn positions(width: u32, height: u32) -> RgbImage {
        RgbImage::from_fn(width, height, |x, y| image::Rgb([x as u8, y as u8, 0]))
    }

    #[test]
    fn a_crop_is_shown_to_the_models_the_way_it_is_seen() {
        // Turned a quarter clockwise, the 200 x 100 photo stands 100 x 200;
        // the crop is the lower half of that, which is the file's right half.
        let cropped = framing(200, 100, Adjustments {
            rotation: 1,
            crop: Crop { x: 0.5, y: 0.75, width: 1.0, height: 0.5 },
            ..Default::default()
        });
        let guide = positions(200, 100);
        assert_eq!(cropped.size_in(guide.dimensions()), (100, 100));
        let view = show(&guide, &cropped.on_source(), (100, 100));
        // The view's top left is the file's bottom middle, and down the view is rightward in the file.
        for (x, y) in [(0, 0), (99, 0), (0, 99), (40, 70), (99, 99)] {
            let [r, g, _] = view.get_pixel(x, y).0;
            let (want_x, want_y) = (100 + y, 99 - x);
            assert!(r.abs_diff(want_x as u8) <= 1 && g.abs_diff(want_y as u8) <= 1, "at {x},{y}: {r},{g}");
        }
        // Half the size on a guide half the size.
        assert_eq!(cropped.size_in((100, 50)), (50, 50));
    }

    #[test]
    fn a_matte_found_in_a_crop_is_laid_back_where_the_crop_is() {
        let cropped = framing(200, 100, Adjustments {
            rotation: 1,
            crop: Crop { x: 0.5, y: 0.75, width: 1.0, height: 0.5 },
            ..Default::default()
        });
        // The subject is the left half of what the model saw: the bottom of the file's right half.
        let matte = GrayImage::from_fn(100, 100, |x, _| image::Luma([if x < 50 { 255 } else { 0 }]));
        let placed = place(&matte, &cropped.on_source(), (200, 100));
        assert_eq!(placed.dimensions(), (200, 100));
        let at = |x, y| placed.get_pixel(x, y)[0];
        assert_eq!(at(150, 75), 255);
        assert_eq!(at(150, 25), 0);
        assert_eq!(at(50, 75), 0, "nothing outside the crop");
        // The crop's edge is covered fully, and the matte stops just past it.
        assert_eq!(at(100, 75), 255);
        assert_eq!(at(99, 75), 255);
        assert_eq!(at(97, 75), 0);
    }

    #[test]
    fn showing_a_crop_and_laying_it_back_meet_for_every_turn_and_flip() {
        let guide = RgbImage::from_fn(300, 200, |x, y| image::Rgb([(x * 255 / 299) as u8, (y * 255 / 199) as u8, 128]));
        for rotation in 0..4 {
            for (flip_horizontal, flip_vertical) in [(false, false), (true, false), (false, true)] {
                for straighten in [0.0, 8.0] {
                    let cropped = framing(300, 200, Adjustments {
                        crop: Crop { x: 0.4, y: 0.55, width: 0.5, height: 0.6 },
                        straighten,
                        rotation,
                        flip_horizontal,
                        flip_vertical,
                        ..Default::default()
                    });
                    let to_source = cropped.on_source();
                    let size = cropped.size_in(guide.dimensions());
                    let view = show(&guide, &to_source, size);
                    // The model's answer is the view's red: laid back, it is the photo's red again.
                    let red = GrayImage::from_fn(size.0, size.1, |x, y| image::Luma([view.get_pixel(x, y)[0]]));
                    let placed = place(&red, &to_source, guide.dimensions());
                    let pixel = |p: [f64; 2]| ((p[0] * 300.0) as u32, (p[1] * 200.0) as u32);
                    for t in [[0.2, 0.2], [0.5, 0.5], [0.8, 0.3], [0.3, 0.8]] {
                        let (x, y) = pixel(to_source.apply(t));
                        let (got, want) = (placed.get_pixel(x, y)[0], guide.get_pixel(x, y)[0]);
                        assert!(got.abs_diff(want) <= 3, "{cropped:?} at {t:?}: {got} vs {want}");
                    }
                    let (x, y) = pixel(to_source.apply([-0.2, 0.5]));
                    assert_eq!(placed.get_pixel(x, y)[0], 0, "{cropped:?}: outside the crop");
                }
            }
        }
    }

    #[test]
    fn a_cropped_photo_has_its_subject_found_in_the_crop_alone() {
        // Two discs; the crop holds only the right-hand one.
        let guide = discs(&[(70.0, 100.0), (220.0, 100.0)]);
        let crop = Crop { x: 220.0 / 300.0, y: 0.5, width: 0.4, height: 0.8 };
        let framing = Framing::of(300, 200, &Adjustments { crop, ..Default::default() });
        let matte = Found::Subject.find(&Picture::new(guide), &framing).unwrap();
        assert_eq!(matte.dimensions(), (300, 200));
        assert!(matte.get_pixel(220, 100)[0] > 230, "the disc in the crop");
        assert_eq!(matte.get_pixel(70, 100)[0], 0, "not the one cropped away");
        assert!(matte.get_pixel(165, 25)[0] < 25, "nor the ground around it");
    }

    #[test]
    fn the_guide_averages_the_photo_down_and_encodes_it_as_srgb() {
        let (width, height) = (COVERAGE_EDGE * 2, 4);
        let pixels = (0..width * height).map(|i| if i % 2 == 0 { [0.0; 3] } else { [0.4; 3] }).collect();
        let guide = guide(&LinearImage { width, height, pixels, scene_referred: false });
        assert_eq!(guide.dimensions(), (COVERAGE_EDGE, 2));
        // Linear 0.2 is about 124 in sRGB.
        assert!(guide.pixels().all(|p| (p[0] as i32 - 124).abs() <= 1), "{:?}", guide.get_pixel(0, 0));
    }

    #[test]
    fn a_raw_guide_has_the_editors_look() {
        // Mid grey off the sensor is shown well above mid grey, as the editor shows it.
        let guide = guide(&LinearImage { width: 8, height: 8, pixels: vec![[0.18; 3]; 64], scene_referred: true });
        assert!(guide.pixels().all(|p| (p[0] as i32 - 176).abs() <= 2), "{:?}", guide.get_pixel(0, 0));
        // The sensor's clipping point is white, and a clipped colour fades to it.
        let guide = super::guide(&LinearImage { width: 8, height: 8, pixels: vec![[1.0, 0.7, 0.6]; 64], scene_referred: true });
        assert!(guide.get_pixel(0, 0).0.iter().all(|&v| v == 255), "{:?}", guide.get_pixel(0, 0));
    }

    #[test]
    fn closing_fills_a_small_hole_and_leaves_a_wide_shape() {
        // A 10 x 10 grid of sky with a one-cell hole near the top, and a
        // block four cells wide rising into it from the bottom.
        let size = 10;
        let values: Vec<f32> = (0..size * size)
            .map(|i| {
                let (x, y) = (i % size, i / size);
                if (x, y) == (5, 2) || (3..7).contains(&x) && y >= 6 { 0.0 } else { 1.0 }
            })
            .collect();
        let closed = closing(&values, size, 1);
        assert_eq!(closed[2 * size + 5], 1.0, "the hole");
        for y in 6..size {
            for x in 3..7 {
                assert_eq!(closed[y * size + x], 0.0, "the block at {x},{y}");
            }
        }
    }
}
