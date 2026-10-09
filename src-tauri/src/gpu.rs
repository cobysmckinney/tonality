//! The GPU side of the editor. A photo is uploaded once at full resolution
//! (`Session`); after that every slider change is one draw of the develop
//! shader, at whatever size and crop the screen needs.

use anyhow::{anyhow, Context, Result};
use bytemuck::{Pod, Zeroable};
use half::f16;
use rayon::prelude::*;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::develop::LinearImage;
use crate::edit::{curve_table, Adjustments, Stroke, MAX_EXPOSURE};
use crate::film::{self, Look, Sample};
use crate::geometry;
use crate::masks::{self, Coverage, COVERAGE_EDGE, MAX_BRUSHES, MAX_FOUND, MAX_MASKS, MAX_PARTS};
use crate::segment::{self, Found, Framing};

/// Which look the develop pipeline draws. **Bump it whenever a change makes
/// an edited photo come out differently**: in this file, in
/// `shaders/develop.wgsl`, in `develop.rs`, or in how a recipe becomes the
/// shader's input (`edit.rs`, `masks.rs`, `segment.rs`). On its next start a
/// library then redraws the thumbnails and previews of its edited photos, so
/// the grid keeps matching the editor (`thumbs::forget_old_looks`).
pub const LOOK_VERSION: u32 = 3;

const WORKING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// What 16-bit exports can be drawn into, best first: the same picture
/// before it is rounded to 256 levels. Not every graphics driver can draw
/// into the first two; half-float still gives far finer steps than 8 bits.
const DEEP_FORMATS: [wgpu::TextureFormat; 3] =
    [wgpu::TextureFormat::Rgba32Float, wgpu::TextureFormat::Rgba16Unorm, wgpu::TextureFormat::Rgba16Float];
/// What every texture drawn into is made for (`Gpu::texture`).
const DRAWN_USAGES: wgpu::TextureUsages = wgpu::TextureUsages::TEXTURE_BINDING
    .union(wgpu::TextureUsages::RENDER_ATTACHMENT)
    .union(wgpu::TextureUsages::COPY_DST)
    .union(wgpu::TextureUsages::COPY_SRC);
/// Brush and found coverage: 256 levels is far finer than any slider step it scales.
const COVERAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Long edge and blur width (in their own pixels) of the two blurred copies.
/// Fixed sizes make the blurs cover the same share of any photo.
const MEDIUM_BLUR: (u32, f32) = (768, 4.0);
const LARGE_BLUR: (u32, f32) = (192, 5.0);

/// The most pixels read back from the GPU in one go. A full-size export is
/// drawn in bands of this many, so no photo is too big for one buffer.
const BAND_PIXELS: u32 = 16 << 20;

/// A whole picture with 16 bits for each of red, green and blue.
pub type DeepImage = image::ImageBuffer<image::Rgb<u16>, Vec<u16>>;

/// The part of the frame to draw, in 0..1 coordinates. The frame is the
/// cropped picture (or the whole tilted photo while cropping).
#[derive(Debug, Clone, Copy)]
pub struct Region {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Region {
    pub const FULL: Region = Region { x: 0.0, y: 0.0, width: 1.0, height: 1.0 };
}

/// What the editor can draw over the picture while you work on it. None of
/// it is ever part of an export or a thumbnail.
#[derive(Debug, Clone, Copy, Default)]
pub struct Guides {
    /// Blown highlights in red, crushed shadows in blue.
    pub show_clipping: bool,
    /// The whole tilted photo with transparent corners, for the crop tool.
    pub uncropped: bool,
    /// The mask, by id, whose coverage is tinted red.
    pub mask_overlay: Option<u32>,
    /// Instead of the picture, that mask's coverage alone: white where it applies, black where not.
    pub matte: bool,
}

/// Mirrors `Params` in develop.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    view: [f32; 4],
    to_source: [[f32; 4]; 2],
    crop: [f32; 4],
    image: [f32; 4],
    light: [f32; 4],
    tone: [f32; 4],
    color: [f32; 4],
    detail: [f32; 4],
    flags: [f32; 4],
    mixer: [[f32; 4]; 8],
    mask_counts: [f32; 4],
    masks: [[[f32; 4]; 5]; MAX_MASKS],
    mask_parts: [[[f32; 4]; 3]; MAX_PARTS],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BlurParams {
    step: [f32; 2],
    sigma: f32,
    radius: f32,
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    develop: wgpu::RenderPipeline,
    /// The develop shader again, drawing into the best of `DEEP_FORMATS`
    /// this device can draw into, if any.
    develop_deep: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    copy: wgpu::RenderPipeline,
    gaussian: wgpu::RenderPipeline,
    /// Turns a scanned negative into the positive the develop shader works on.
    film: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    max_texture_size: u32,
}

/// A photo loaded for editing.
pub struct Session {
    pub width: u32,
    pub height: u32,
    scene_referred: bool,
    params: wgpu::Buffer,
    curves: wgpu::Texture,
    /// The working image with its mipmaps: the photo as opened, or the
    /// positive of a negative (`apply_film`).
    source: wgpu::Texture,
    /// The working image blurred, medium and large.
    blurs: [Blur; 2],
    views: Views,
    coverages: Mutex<Coverages>,
    /// The photo as the models see it, kept to find its subject, sky and
    /// circled objects in (`segment::guide`). None until it is next wanted,
    /// once a negative's positive has replaced it (`Gpu::picture`).
    picture: Mutex<Option<Arc<segment::Picture>>>,
    /// The photo as opened, as the models see it.
    original_picture: Arc<segment::Picture>,
    /// Added to found mattes' keys while the photo is a negative's positive,
    /// so mattes found in the negative or another positive aren't used.
    matte_suffix: Mutex<String>,
    /// What the models are shown of the photo: all of it, or its crop.
    framing: Mutex<Framing>,
    /// Whether `framing` changes only when asked (`set_framing`,
    /// `Gpu::prepare`), rather than following each recipe drawn. The
    /// editor's photo is held: its crop changes at every step of a drag, and
    /// the models take seconds.
    pub hold_framing: AtomicBool,
    /// A small copy of the photo as opened, to measure film in.
    sample: Arc<Sample>,
    /// The film look in the working image. Held while a frame is drawn, so
    /// no other frame changes the working image under it.
    film: Mutex<FilmState>,
    /// Where the photo's found mattes are kept between sessions, if anywhere:
    /// the start of their file names, which end in each matte's key.
    pub matte_files: Option<PathBuf>,
    /// The mattes found so far, by key.
    mattes: Mutex<HashMap<String, Arc<image::GrayImage>>>,
    /// Held while a matte is being found, so each is found only once.
    finding: Mutex<()>,
    /// Keys of parts that couldn't be found this session. They draw as
    /// empty and aren't tried again until asked for (`find_parts`) or the
    /// photo is opened again.
    missing: Mutex<HashSet<String>>,
    /// What recent frames were drawn into, least recently used first, so
    /// the next frame of the same size doesn't need new ones.
    targets: Mutex<Vec<Target>>,
    /// Counts the mattes set, which change the picture without changing the recipe.
    mattes_set: AtomicU64,
    /// The last histogram, with the recipe and `mattes_set` it was drawn with.
    histogram: Mutex<Option<(Adjustments, u64, Histogram)>>,
}

/// How the whole picture's tones are spread, as the histogram shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    /// 256 bins each for red, green, blue and brightness.
    pub bins: [[u32; 256]; 4],
    /// Enough of the picture is blown out, or crushed to black, to warn about.
    pub blown: bool,
    pub crushed: bool,
}

impl Histogram {
    /// The long edge of the small picture a histogram is counted from.
    const EDGE: u32 = 256;

    fn of(picture: &image::RgbImage) -> Self {
        let mut bins = [[0u32; 256]; 4];
        let (mut blown, mut crushed) = (0usize, 0usize);
        for pixel in picture.pixels() {
            let [r, g, b] = pixel.0;
            bins[0][r as usize] += 1;
            bins[1][g as usize] += 1;
            bins[2][b as usize] += 1;
            let brightness = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32).round() as usize;
            bins[3][brightness.min(255)] += 1;
            blown += (r.max(g).max(b) >= 254) as usize;
            crushed += (r.max(g).max(b) <= 1) as usize;
        }
        // A handful of specular highlights is not worth a warning.
        let noticeable = (picture.width() * picture.height()) as usize / 2000;
        Self { bins, blown: blown > noticeable, crushed: crushed > noticeable }
    }
}

/// How many frame targets a session keeps: the editor's frame and the
/// histogram's small picture, with one to spare.
const TARGETS_KEPT: usize = 3;

/// A texture to draw a frame into and the buffer it is read back through.
struct Target {
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

/// What the working image holds, and what it was made from.
#[derive(Default)]
struct FilmState {
    /// The photo as opened, kept here while the working image holds a
    /// negative's positive instead.
    scan: Option<wgpu::Texture>,
    /// The film look the working image has; None for the photo as opened.
    applied: Option<Look>,
    /// The last look worked out, and the film and framing settings it was
    /// worked out from, so each frame doesn't measure the film again.
    measured: Option<(Adjustments, Option<Look>)>,
}

/// A blurred copy of the working image, and a texture to blur it through.
struct Blur {
    view: wgpu::TextureView,
    scratch: wgpu::TextureView,
    width: u32,
    height: u32,
    sigma: f32,
}

impl Session {
    /// The name a found part's matte goes by in this session.
    pub fn key(&self, found: &Found) -> String {
        self.key_in(found, &self.framing())
    }

    /// The name a found part's matte goes by in this session, found in `framing`.
    pub fn key_in(&self, found: &Found, framing: &Framing) -> String {
        format!("{}{}", found.key(framing), self.matte_suffix.lock().unwrap())
    }

    /// What the models are shown of the photo now.
    pub fn framing(&self) -> Framing {
        *self.framing.lock().unwrap()
    }

    /// Shows the models `framing` of the photo from now on.
    pub fn set_framing(&self, framing: Framing) {
        let mut current = self.framing.lock().unwrap();
        if *current != framing {
            *current = framing;
            // Its mattes are other ones, which changes the picture without changing the recipe.
            self.mattes_set.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// What the models would be shown of the photo edited with `adjustments`.
    pub fn framing_of(&self, adjustments: &Adjustments) -> Framing {
        Framing::of(self.width, self.height, adjustments)
    }

    /// Whether this part has been found (or set) for this session.
    pub fn has_matte(&self, found: &Found) -> bool {
        self.has_matte_named(&self.key(found))
    }

    /// Whether the matte named `key` has been found (or set) for this session.
    pub fn has_matte_named(&self, key: &str) -> bool {
        self.mattes.lock().unwrap().contains_key(key)
    }

    /// Uses `matte` for this found part: white where it is. Any size; it is
    /// stretched over the whole photo.
    pub fn set_matte(&self, found: &Found, matte: image::GrayImage) {
        self.set_matte_named(self.key(found), matte);
    }

    /// `set_matte`, for the part whose key was `key` when it was looked for.
    pub fn set_matte_named(&self, key: String, matte: image::GrayImage) {
        self.missing.lock().unwrap().remove(&key);
        self.mattes.lock().unwrap().insert(key, Arc::new(matte));
        self.mattes_set.fetch_add(1, Ordering::Relaxed);
    }

    /// Where this part's matte is kept between sessions, if anywhere.
    pub fn matte_path(&self, found: &Found) -> Option<PathBuf> {
        self.matte_path_named(&self.key(found))
    }

    /// Where the matte named `key` is kept between sessions, if anywhere.
    pub fn matte_path_named(&self, key: &str) -> Option<PathBuf> {
        let start = self.matte_files.as_ref()?;
        let mut name = start.file_name()?.to_os_string();
        name.push(format!("{key}.png"));
        Some(start.with_file_name(name))
    }

    /// A small copy of the photo as opened, to measure film in.
    pub fn sample(&self) -> Arc<Sample> {
        self.sample.clone()
    }
}

/// What the develop shader reads, apart from the mask coverage.
struct Views {
    source: wgpu::TextureView,
    blur_medium: wgpu::TextureView,
    blur_large: wgpu::TextureView,
    curves: wgpu::TextureView,
}

/// The coverage maps of the photo's brush parts and found parts, and the
/// bindings that include them. Each stack is made when first needed (the
/// brushes on the first stroke); until then a blank stand-in is bound.
struct Coverages {
    brushes: Option<wgpu::Texture>,
    brush_view: wgpu::TextureView,
    maps: Vec<Coverage>,
    found: Option<wgpu::Texture>,
    found_view: wgpu::TextureView,
    /// The key of the matte in each layer of `found`.
    found_layers: Vec<Option<String>>,
    bind_group: wgpu::BindGroup,
    /// The same bindings for the deep pipeline; a bind group belongs to one pipeline's layout.
    deep_bind_group: Option<wgpu::BindGroup>,
}

/// The app's one GPU connection, opened on first use.
pub fn shared() -> Result<&'static Gpu> {
    static GPU: std::sync::OnceLock<std::result::Result<Gpu, String>> = std::sync::OnceLock::new();
    GPU.get_or_init(|| Gpu::new().map_err(|error| format!("{error:#}")))
        .as_ref()
        .map_err(|error| anyhow!("The editor needs a working graphics driver: {error}"))
}

/// The size in pixels of the picture `adjustments` make of a `width` x
/// `height` photo, scaled down to fit within `long_edge` if it is larger.
pub fn picture_size(width: u32, height: u32, adjustments: &Adjustments, long_edge: u32) -> (u32, u32) {
    let [w, h] = geometry::frame(width, height, adjustments, false).size;
    let (w, h) = ((w.round() as u32).max(1), (h.round() as u32).max(1));
    fit_within(w, h, long_edge.min(w.max(h)))
}

pub(crate) fn fit_within(width: u32, height: u32, long_edge: u32) -> (u32, u32) {
    let scale = long_edge as f32 / width.max(height) as f32;
    (((width as f32 * scale).round() as u32).max(1), ((height as f32 * scale).round() as u32).max(1))
}

impl Gpu {
    /// The longest side, in pixels, of a photo this device can hold at full
    /// size. Larger ones are scaled down to fit when opened.
    pub fn largest_picture(&self) -> u32 {
        self.max_texture_size
    }

    pub fn new() -> Result<Self> {
        Self::on(wgpu::InstanceDescriptor::new_without_display_handle())
    }

    /// Opens the best graphics adapter `instance` offers.
    fn on(instance: wgpu::InstanceDescriptor) -> Result<Self> {
        let instance = wgpu::Instance::new(instance);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| anyhow!("no usable graphics adapter: {e}"))?;
        let limits = adapter.limits();
        let deep_format = deep_format(|format| {
            adapter.features().contains(format.required_features())
                && adapter.get_texture_format_features(format).allowed_usages.contains(DRAWN_USAGES)
        });
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("tonality"),
            required_features: deep_format.map(|format| format.required_features()).unwrap_or_default(),
            // Ask for the adapter's real limits: photos are far larger than the default texture size.
            required_limits: limits.clone(),
            ..Default::default()
        }))
        .context("opening the graphics device")?;

        let develop_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/develop.wgsl"));
        let prepare_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/prepare.wgsl"));
        let pipeline = |label, shader: &wgpu::ShaderModule, fragment, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: None,
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        Ok(Self {
            develop: pipeline("develop", &develop_shader, "fragment", OUTPUT_FORMAT),
            develop_deep: deep_format.map(|format| (format, pipeline("develop deep", &develop_shader, "fragment", format))),
            copy: pipeline("copy", &prepare_shader, "copy", WORKING_FORMAT),
            gaussian: pipeline("gaussian", &prepare_shader, "gaussian", WORKING_FORMAT),
            film: pipeline("film", &prepare_shader, "film_positive", WORKING_FORMAT),
            sampler,
            max_texture_size: limits.max_texture_dimension_2d,
            device,
            queue,
        })
    }

    fn texture(&self, label: &str, width: u32, height: u32, mips: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: DRAWN_USAGES,
            view_formats: &[],
        })
    }

    /// Draws one full-target pass of `pipeline` into `target`.
    fn draw(&self, encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::RenderPipeline, bindings: &wgpu::BindGroup, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bindings, &[]);
        pass.draw(0..3, 0..1);
    }

    fn copy_bindings(&self, input: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.copy.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(input) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        })
    }

    /// Textures for a blurred, shrunken copy of a `width` x `height` picture,
    /// `long_edge` pixels along its longer side.
    fn blur(&self, width: u32, height: u32, (long_edge, sigma): (u32, f32)) -> Blur {
        let (w, h) = fit_within(width, height, long_edge.min(width.max(height)));
        let make = |label| self.texture(label, w, h, 1, WORKING_FORMAT).create_view(&Default::default());
        Blur { view: make("blur"), scratch: make("blur scratch"), width: w, height: h, sigma }
    }

    /// Draws `blur` from `source`.
    fn draw_blur(&self, encoder: &mut wgpu::CommandEncoder, source: &wgpu::TextureView, blur: &Blur) {
        use wgpu::util::DeviceExt;
        let Blur { view: first, scratch: second, width: w, height: h, sigma } = blur;
        let (w, h, sigma) = (*w, *h, *sigma);
        self.draw(encoder, &self.copy, &self.copy_bindings(source), first);

        // Horizontal into the scratch texture, then vertical back.
        for (input, target, step) in [(first, second, [1.0 / w as f32, 0.0]), (second, first, [0.0, 1.0 / h as f32])] {
            let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::bytes_of(&BlurParams { step, sigma, radius: (sigma * 2.5).ceil() }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.gaussian.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(input) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                ],
            });
            self.draw(encoder, &self.gaussian, &bindings, target);
        }
    }

    /// Draws the working image's mipmaps and blurs from its full-size level,
    /// each mip level averaged down from the one above, so a zoomed-out view
    /// is a properly filtered one.
    fn draw_derived(&self, encoder: &mut wgpu::CommandEncoder, source: &wgpu::Texture, blurs: &[Blur; 2]) {
        let level_view = |level| {
            source.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        for level in 1..source.mip_level_count() {
            self.draw(encoder, &self.copy, &self.copy_bindings(&level_view(level - 1)), &level_view(level));
        }
        let whole = source.create_view(&Default::default());
        for blur in blurs {
            self.draw_blur(encoder, &whole, blur);
        }
    }

    /// Uploads a photo and prepares everything the develop shader reads.
    pub fn open(&self, image: LinearImage) -> Result<Session> {
        let image = shrink_to_fit(image, self.max_texture_size);
        let (width, height) = (image.width, image.height);
        let picture = Arc::new(segment::Picture::new(segment::guide(&image)));
        let sample = Arc::new(Sample::of(&image));

        // Half-float is plenty for photographic range at half the memory of f32.
        let mut texels = vec![f16::ONE; image.pixels.len() * 4];
        texels.par_chunks_mut(4).zip(image.pixels.par_iter()).for_each(|(texel, pixel)| {
            for (out, value) in texel.iter_mut().zip(pixel) {
                *out = f16::from_f32(*value);
            }
        });
        let mips = width.max(height).ilog2() + 1;
        let source = self.texture("source", width, height, mips, WORKING_FORMAT);
        self.queue.write_texture(
            source.as_image_copy(),
            bytemuck::cast_slice(&texels),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 8), rows_per_image: Some(height) },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        drop(texels);

        let blurs = [self.blur(width, height, MEDIUM_BLUR), self.blur(width, height, LARGE_BLUR)];
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.draw_derived(&mut encoder, &source, &blurs);
        self.queue.submit([encoder.finish()]);

        let source_view = source.create_view(&Default::default());
        let (blur_medium, blur_large) = (blurs[0].view.clone(), blurs[1].view.clone());

        let curves = self.texture("curves", 256, 1, 1, WORKING_FORMAT);
        let params = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let views = Views { source: source_view, blur_medium, blur_large, curves: curves.create_view(&Default::default()) };
        let (_, brush_view) = self.coverage_maps(1, 1, 1);
        let (_, found_view) = self.coverage_maps(1, 1, 1);
        let coverages = Coverages {
            bind_group: self.develop_bindings(&self.develop, &views, &params, &brush_view, &found_view),
            deep_bind_group: self.deep_bindings(&views, &params, &brush_view, &found_view),
            brushes: None,
            brush_view,
            maps: Vec::new(),
            found: None,
            found_view,
            found_layers: vec![None; MAX_FOUND],
        };
        Ok(Session {
            width,
            height,
            scene_referred: image.scene_referred,
            params,
            curves,
            source,
            blurs,
            views,
            coverages: Mutex::new(coverages),
            picture: Mutex::new(Some(picture.clone())),
            original_picture: picture,
            matte_suffix: Mutex::new(String::new()),
            framing: Mutex::new(Framing::Whole),
            hold_framing: AtomicBool::new(false),
            sample,
            film: Mutex::new(FilmState::default()),
            matte_files: None,
            mattes: Mutex::new(HashMap::new()),
            finding: Mutex::new(()),
            missing: Mutex::new(HashSet::new()),
            targets: Mutex::new(Vec::new()),
            mattes_set: AtomicU64::new(0),
            histogram: Mutex::new(None),
        })
    }

    /// The histogram of the whole picture `adjustments` make, whatever part
    /// of it is on screen. It is kept until the recipe or a found part
    /// changes, so panning, zooming and the guides don't count it again.
    pub fn histogram(&self, session: &Session, adjustments: &Adjustments) -> Result<Histogram> {
        let mattes_set = session.mattes_set.load(Ordering::Relaxed);
        if let Some((recipe, set, histogram)) = &*session.histogram.lock().unwrap() {
            if *set == mattes_set && recipe == adjustments {
                return Ok(histogram.clone());
            }
        }
        let histogram = Histogram::of(&self.render_image(session, adjustments, Histogram::EDGE)?);
        *session.histogram.lock().unwrap() = Some((adjustments.clone(), mattes_set, histogram.clone()));
        Ok(histogram)
    }

    /// A target for a `width` x `height` frame in `format`: the session's
    /// last one of that size, or a new one. Hand it back with `keep_target`.
    fn take_target(&self, session: &Session, width: u32, height: u32, format: wgpu::TextureFormat) -> Result<Target> {
        let mut targets = session.targets.lock().unwrap();
        if let Some(i) = targets.iter().position(|t| (t.width, t.height, t.format) == (width, height, format)) {
            return Ok(targets.remove(i));
        }
        drop(targets);
        let pixel_bytes = format.block_copy_size(None).context("a frame format without a pixel size")?;
        let texture = self.texture("frame", width, height, 1, format);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: padded_row(width * pixel_bytes) as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Target { width, height, format, view: texture.create_view(&Default::default()), texture, readback })
    }

    /// Keeps `target` for the next frame of its size, letting go of the
    /// least recently used if there are too many.
    fn keep_target(&self, session: &Session, target: Target) {
        let mut targets = session.targets.lock().unwrap();
        targets.push(target);
        if targets.len() > TARGETS_KEPT {
            targets.remove(0);
        }
    }

    /// A stack of `layers` coverage maps, as the shader reads them.
    fn coverage_maps(&self, width: u32, height: u32, layers: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("brush coverage"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: layers },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COVERAGE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        (texture, view)
    }

    fn develop_bindings(
        &self,
        pipeline: &wgpu::RenderPipeline,
        views: &Views,
        params: &wgpu::Buffer,
        brushes: &wgpu::TextureView,
        found: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("develop"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&views.source) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views.blur_medium) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&views.blur_large) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&views.curves) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 5, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(brushes) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(found) },
            ],
        })
    }

    /// `develop_bindings` for the deep pipeline, if there is one.
    fn deep_bindings(
        &self,
        views: &Views,
        params: &wgpu::Buffer,
        brushes: &wgpu::TextureView,
        found: &wgpu::TextureView,
    ) -> Option<wgpu::BindGroup> {
        let (_, pipeline) = self.develop_deep.as_ref()?;
        Some(self.develop_bindings(pipeline, views, params, brushes, found))
    }

    /// Points both pipelines' bindings at the session's current coverage maps.
    fn rebind(&self, session: &Session, coverages: &mut Coverages) {
        let (brushes, found) = (&coverages.brush_view, &coverages.found_view);
        coverages.bind_group = self.develop_bindings(&self.develop, &session.views, &session.params, brushes, found);
        coverages.deep_bind_group = self.deep_bindings(&session.views, &session.params, brushes, found);
    }

    /// Makes sure these found parts' mattes are ready for the shader: read
    /// from the cache, or found by a model, which takes a second or two each.
    /// A part that can't be found draws as empty rather than stopping the
    /// photo drawing; those found missing by this call are returned.
    pub fn ensure_found(&self, session: &Session, found: &[Found]) -> Vec<(Found, anyhow::Error)> {
        let film = session.film.lock().unwrap();
        self.ensure_found_with(session, &film, found)
    }

    /// Gets a photo ready to draw with `adjustments`: its film look in the
    /// working image, and the found parts of its masks (`ensure_found`),
    /// found in its crop even if the session's framing is held.
    pub fn prepare(&self, session: &Session, adjustments: &Adjustments) -> Result<Vec<(Found, anyhow::Error)>> {
        let mut film = session.film.lock().unwrap();
        self.apply_film(session, &mut film, adjustments)?;
        session.set_framing(session.framing_of(adjustments));
        Ok(self.ensure_found_with(session, &film, &masks::found(&adjustments.masks)))
    }

    /// `ensure_found`, with the session's film lock held (`film`), so the
    /// working image the models may be shown can't change meanwhile.
    fn ensure_found_with(&self, session: &Session, _film: &FilmState, found: &[Found]) -> Vec<(Found, anyhow::Error)> {
        let mut missing = Vec::new();
        let framing = session.framing();
        for part in found {
            let key = session.key_in(part, &framing);
            let ready =
                || session.mattes.lock().unwrap().contains_key(&key) || session.missing.lock().unwrap().contains(&key);
            if ready() {
                continue;
            }
            let _finding = session.finding.lock().unwrap();
            // Another thread may have found it while this one waited.
            if ready() {
                continue;
            }
            let found = self
                .picture_with(session)
                .and_then(|picture| part.find_cached(&picture, &framing, session.matte_path_named(&key).as_deref()));
            match found {
                Ok(matte) => session.set_matte_named(key, matte),
                Err(error) => {
                    session.missing.lock().unwrap().insert(key);
                    missing.push((part.clone(), error));
                }
            }
        }
        missing
    }

    /// The photo as the models see it, to find parts in.
    pub fn picture(&self, session: &Session) -> Result<Arc<segment::Picture>> {
        let _film = session.film.lock().unwrap();
        self.picture_with(session)
    }

    /// `picture`, with the session's film lock held. Once a negative's
    /// positive has replaced the photo, the models are shown that, drawn
    /// from the working image the first time it is wanted.
    fn picture_with(&self, session: &Session) -> Result<Arc<segment::Picture>> {
        if let Some(picture) = &*session.picture.lock().unwrap() {
            return Ok(picture.clone());
        }
        let (width, height) =
            fit_within(session.width, session.height, masks::COVERAGE_EDGE.min(session.width.max(session.height)));
        let pixels = self.read_working(session, width, height)?;
        let picture = Arc::new(segment::Picture::new(segment::guide_of_positive(width, height, &pixels)));
        *session.picture.lock().unwrap() = Some(picture.clone());
        Ok(picture)
    }

    /// The working image averaged down to `width` x `height`, in linear light.
    fn read_working(&self, session: &Session, width: u32, height: u32) -> Result<Vec<[f32; 3]>> {
        let texture = self.texture("working copy", width, height, 1, WORKING_FORMAT);
        let view = texture.create_view(&Default::default());
        let row = width * 8;
        let padded = padded_row(row);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("working readback"),
            size: padded as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.draw(&mut encoder, &self.copy, &self.copy_bindings(&session.views.source), &view);
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).context("waiting for the graphics device")?;
        receiver.recv().context("the graphics device dropped the picture")??;
        let mapped = readback.slice(..).get_mapped_range().context("reading the picture back")?;
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for line in mapped.chunks_exact(padded as usize) {
            let texels: &[f16] = bytemuck::cast_slice(&line[..row as usize]);
            pixels.extend(texels.as_chunks::<4>().0.iter().map(|t| [t[0].to_f32(), t[1].to_f32(), t[2].to_f32()]));
        }
        Ok(pixels)
    }

    /// Makes the working image hold what `adjustments` ask of the film: a
    /// negative's positive, or the photo as opened. Only redraws it (and its
    /// mipmaps and blurs) when that changes. Returns the look applied.
    fn apply_film(&self, session: &Session, state: &mut FilmState, adjustments: &Adjustments) -> Result<Option<Look>> {
        use wgpu::util::DeviceExt;
        // The look depends only on the film settings and the framing (the
        // balance is measured inside the crop).
        let mut wanted = Adjustments { film: adjustments.film, ..Default::default() };
        wanted.keep_framing_of(adjustments);
        let look = match &state.measured {
            Some((measured, look)) if *measured == wanted => *look,
            _ => {
                let look = film::resolve(&session.sample, adjustments);
                state.measured = Some((wanted, look));
                look
            }
        };
        if look == state.applied {
            return Ok(look);
        }

        let size = wgpu::Extent3d { width: session.width, height: session.height, depth_or_array_layers: 1 };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        match look {
            Some(look) => {
                if state.scan.is_none() {
                    let scan = self.texture("scan", session.width, session.height, 1, WORKING_FORMAT);
                    encoder.copy_texture_to_texture(session.source.as_image_copy(), scan.as_image_copy(), size);
                    state.scan = Some(scan);
                }
                let scan = state.scan.as_ref().expect("just kept").create_view(&Default::default());
                let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("film"),
                    contents: bytemuck::cast_slice(&look.uniform()),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("film"),
                    layout: &self.film.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&scan) },
                        wgpu::BindGroupEntry { binding: 3, resource: uniform.as_entire_binding() },
                    ],
                });
                let full_size = session.source.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: 0,
                    mip_level_count: Some(1),
                    ..Default::default()
                });
                self.draw(&mut encoder, &self.film, &bindings, &full_size);
            }
            None => {
                // Back to the photo as opened; the copy kept of it isn't needed any more.
                if let Some(scan) = state.scan.take() {
                    encoder.copy_texture_to_texture(scan.as_image_copy(), session.source.as_image_copy(), size);
                }
            }
        }
        self.draw_derived(&mut encoder, &session.source, &session.blurs);
        self.queue.submit([encoder.finish()]);

        // The models are shown the new picture once the film or its base changes.
        let seen = |look: Option<Look>| look.map(|look| (look.kind, look.base));
        if seen(look) != seen(state.applied) {
            *session.matte_suffix.lock().unwrap() = look.map(|look| look.matte_name()).unwrap_or_default();
            *session.picture.lock().unwrap() = if look.is_none() { Some(session.original_picture.clone()) } else { None };
        }
        state.applied = look;
        Ok(look)
    }

    /// Puts the mattes of `found` into the layers the shader reads them from,
    /// in order, uploading only those that changed. They must be ready (`ensure_found`).
    fn upload_found(&self, session: &Session, coverages: &mut Coverages, found: &[Found]) {
        let (width, height) = session.original_picture.guide.dimensions();
        if coverages.found.is_none() {
            let (texture, view) = self.coverage_maps(width, height, MAX_FOUND as u32);
            coverages.found = Some(texture);
            coverages.found_view = view;
            self.rebind(session, coverages);
        }
        let texture = coverages.found.as_ref().expect("just made");
        let mattes = session.mattes.lock().unwrap();
        for (layer, part) in found.iter().enumerate() {
            let key = session.key(part);
            let matte = mattes.get(&key);
            // An empty layer is labelled apart from the part, so the matte replaces it once found.
            let label = if matte.is_some() { key } else { format!("missing {key}") };
            if coverages.found_layers[layer].as_ref() == Some(&label) {
                continue;
            }
            // A matte kept from a session with another texture limit may be another size.
            let resized;
            let matte = match matte {
                Some(matte) if matte.dimensions() == (width, height) => matte.as_ref(),
                Some(matte) => {
                    resized =
                        image::imageops::resize(matte.as_ref(), width, height, image::imageops::FilterType::Triangle);
                    &resized
                }
                None => {
                    resized = image::GrayImage::new(width, height);
                    &resized
                }
            };
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: layer as u32 },
                    aspect: wgpu::TextureAspect::All,
                },
                matte.as_raw(),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width), rows_per_image: Some(height) },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );
            coverages.found_layers[layer] = Some(label);
        }
    }

    /// Brings the coverage maps up to date with the brush parts' strokes,
    /// uploading only the rows that changed.
    fn paint_brushes(&self, session: &Session, coverages: &mut Coverages, strokes: &[&[Stroke]]) {
        let (width, height) =
            fit_within(session.width, session.height, COVERAGE_EDGE.min(session.width.max(session.height)));
        if coverages.brushes.is_none() {
            let (texture, view) = self.coverage_maps(width, height, MAX_BRUSHES as u32);
            coverages.brushes = Some(texture);
            coverages.brush_view = view;
            self.rebind(session, coverages);
        }
        let texture = coverages.brushes.as_ref().expect("just made");
        for (layer, strokes) in strokes.iter().enumerate() {
            if coverages.maps.len() <= layer {
                coverages.maps.push(Coverage::new(width, height));
            }
            let map = &mut coverages.maps[layer];
            let Some(rows) = map.update(strokes) else { continue };
            let span = rows.start as usize * width as usize..rows.end as usize * width as usize;
            let levels: Vec<u8> = map.values[span].iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: rows.start, z: layer as u32 },
                    aspect: wgpu::TextureAspect::All,
                },
                &levels,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width), rows_per_image: Some(rows.len() as u32) },
                wgpu::Extent3d { width, height: rows.len() as u32, depth_or_array_layers: 1 },
            );
        }
    }

    /// The size in photo pixels of the picture `adjustments` produce: the
    /// crop, or with `uncropped` the box that holds the whole tilted photo.
    pub fn frame_size(&self, session: &Session, adjustments: &Adjustments, uncropped: bool) -> (u32, u32) {
        let [w, h] = geometry::frame(session.width, session.height, adjustments, uncropped).size;
        ((w.round() as u32).max(1), (h.round() as u32).max(1))
    }

    /// Renders `region` of the picture with `adjustments` applied, as
    /// `width` x `height` RGBA bytes. With `uncropped` the crop is ignored
    /// and the whole photo is drawn, tilted by the straighten angle, with
    /// transparent corners: the view the crop tool works on.
    pub fn render(
        &self,
        session: &Session,
        adjustments: &Adjustments,
        region: Region,
        (width, height): (u32, u32),
        guides: Guides,
    ) -> Result<Vec<u8>> {
        self.draw_frame(session, adjustments, region, width, height, guides, false)
    }

    /// `render`, or with `deep` the same frame in the deep pipeline's format.
    #[allow(clippy::too_many_arguments)]
    fn draw_frame(
        &self,
        session: &Session,
        adjustments: &Adjustments,
        region: Region,
        width: u32,
        height: u32,
        guides: Guides,
        deep: bool,
    ) -> Result<Vec<u8>> {
        let Guides { show_clipping, uncropped, mask_overlay, matte } = guides;
        let (width, height) = (width.clamp(1, self.max_texture_size), height.clamp(1, self.max_texture_size));
        let a = adjustments;
        let unit = |value: f32| (value / 100.0).clamp(-1.0, 1.0);
        let frame = geometry::frame(session.width, session.height, adjustments, uncropped);
        let to_source = geometry::frame_to_source(session.width, session.height, adjustments, &frame).0;
        let crop = geometry::crop_in_frame(session.width, session.height, adjustments, &frame);
        let (packed, strokes, found) = masks::pack(&a.masks, session.width, session.height, mask_overlay);
        // Held to the end, so no other frame changes the working image under this one.
        let mut film = session.film.lock().unwrap();
        let look = self.apply_film(session, &mut film, adjustments)?;
        // A negative's positive is in scene light, like a RAW's.
        let scene_referred = session.scene_referred || look.is_some();
        if !session.hold_framing.load(Ordering::Relaxed) {
            session.set_framing(session.framing_of(adjustments));
        }
        for (part, error) in self.ensure_found_with(session, &film, &found) {
            eprintln!("couldn't find {}, drawing it as empty: {error:#}", part.name());
        }
        let mut coverages = session.coverages.lock().unwrap();
        if !strokes.is_empty() {
            self.paint_brushes(session, &mut coverages, &strokes);
        }
        if !found.is_empty() {
            self.upload_found(session, &mut coverages, &found);
        }
        let source_pixels_per_output_pixel = (region.width * frame.size[0] as f32 / width as f32)
            .max(region.height * frame.size[1] as f32 / height as f32);
        let params = Params {
            view: [region.x, region.y, region.width, region.height],
            to_source: to_source.map(|row| [row[0] as f32, row[1] as f32, row[2] as f32, 0.0]),
            crop: crop.map(|value| value as f32),
            image: [
                session.width as f32,
                session.height as f32,
                source_pixels_per_output_pixel,
                scene_referred as u8 as f32,
            ],
            light: [a.exposure.clamp(-MAX_EXPOSURE, MAX_EXPOSURE), unit(a.contrast), unit(a.highlights), unit(a.shadows)],
            tone: [unit(a.whites), unit(a.blacks), unit(a.temperature), unit(a.tint)],
            color: [unit(a.vibrance), unit(a.saturation), unit(a.clarity), unit(a.dehaze)],
            detail: [unit(a.sharpening).max(0.0), unit(a.noise_reduction).max(0.0), unit(a.vignette), unit(a.grain).max(0.0)],
            flags: [show_clipping as u8 as f32, uncropped as u8 as f32, matte as u8 as f32, look.is_some() as u8 as f32],
            mixer: a.mixer.map(|band| [unit(band.hue), unit(band.saturation), unit(band.luminance), 0.0]),
            mask_counts: packed.counts,
            masks: packed.masks,
            mask_parts: packed.parts,
        };
        self.queue.write_buffer(&session.params, 0, bytemuck::bytes_of(&params));

        let master: [f32; 256] = curve_table(&a.curves.master);
        let (red, green, blue): ([f32; 256], [f32; 256], [f32; 256]) =
            (curve_table(&a.curves.red), curve_table(&a.curves.green), curve_table(&a.curves.blue));
        let table: Vec<f16> =
            (0..256).flat_map(|i| [red[i], green[i], blue[i], master[i]]).map(f16::from_f32).collect();
        self.queue.write_texture(
            session.curves.as_image_copy(),
            bytemuck::cast_slice(&table),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 8), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
        );

        let (format, pipeline, bindings) = match deep {
            false => (OUTPUT_FORMAT, &self.develop, &coverages.bind_group),
            true => match (&self.develop_deep, &coverages.deep_bind_group) {
                (Some((format, pipeline)), Some(bindings)) => (*format, pipeline, bindings),
                _ => return Err(anyhow!(NO_DEEP)),
            },
        };
        let row = width * format.block_copy_size(None).context("a frame format without a pixel size")?;
        let padded_row = padded_row(row);
        // The coverage lock is held to the end, so no other frame of this session uses the target meanwhile.
        let target = self.take_target(session, width, height, format)?;
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.draw(&mut encoder, pipeline, bindings, &target.view);
        encoder.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &target.readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded_row), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);

        let (sender, receiver) = std::sync::mpsc::channel();
        target.readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).context("waiting for the graphics device")?;
        receiver.recv().context("the graphics device dropped the frame")??;

        let mut pixels = Vec::with_capacity((row * height) as usize);
        {
            let mapped = target.readback.slice(..).get_mapped_range().context("reading the frame back")?;
            for line in mapped.chunks_exact(padded_row as usize) {
                pixels.extend_from_slice(&line[..row as usize]);
            }
        }
        target.readback.unmap();
        self.keep_target(session, target);
        drop(film);
        Ok(pixels)
    }

    /// Renders the whole photo to fit within `long_edge`, as an RGB image.
    /// A `long_edge` larger than the picture gives it at full size.
    pub fn render_image(&self, session: &Session, adjustments: &Adjustments, long_edge: u32) -> Result<image::RgbImage> {
        let mut rgb = Vec::new();
        let (width, height) = self.render_in_bands(session, adjustments, long_edge, BAND_PIXELS, false, |rgba| {
            rgb.extend(rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]));
        })?;
        image::RgbImage::from_raw(width, height, rgb).context("frame has the wrong size")
    }

    /// `render_image` with 16 bits a channel: the picture before it is
    /// rounded to 256 levels, so smooth tones stay smooth through more editing.
    pub fn render_deep_image(&self, session: &Session, adjustments: &Adjustments, long_edge: u32) -> Result<DeepImage> {
        let Some((format, _)) = self.develop_deep else { return Err(anyhow!(NO_DEEP)) };
        let channel_bytes = format.block_copy_size(None).unwrap_or(16) as usize / 4;
        // Bands of the same number of bytes as 8-bit ones.
        let band_pixels = BAND_PIXELS / channel_bytes as u32;
        let mut rgb = Vec::new();
        let (width, height) = self.render_in_bands(session, adjustments, long_edge, band_pixels, true, |rgba| {
            let level = |bytes: &[u8]| deep_level(format, bytes);
            rgb.extend(rgba.chunks_exact(channel_bytes * 4).flat_map(|pixel| {
                let channel = |i: usize| level(&pixel[i * channel_bytes..(i + 1) * channel_bytes]);
                [channel(0), channel(1), channel(2)]
            }));
        })?;
        DeepImage::from_raw(width, height, rgb).context("frame has the wrong size")
    }

    /// Draws the whole picture as horizontal bands of about `band_pixels`
    /// pixels each, handing each band's pixels to `take` from the top down.
    /// Returns the picture's size.
    fn render_in_bands(
        &self,
        session: &Session,
        adjustments: &Adjustments,
        long_edge: u32,
        band_pixels: u32,
        deep: bool,
        mut take: impl FnMut(Vec<u8>),
    ) -> Result<(u32, u32)> {
        let (width, height) = picture_size(session.width, session.height, adjustments, long_edge);
        let band = (band_pixels / width).max(1);
        for top in (0..height).step_by(band as usize) {
            let rows = band.min(height - top);
            let region = Region { x: 0.0, y: top as f32 / height as f32, width: 1.0, height: rows as f32 / height as f32 };
            take(self.draw_frame(session, adjustments, region, width, rows, Guides::default(), deep)?);
        }
        Ok((width, height))
    }
}

/// The bytes a row of `row` bytes takes in a readback buffer, whose rows
/// must start at multiples of 256 bytes.
fn padded_row(row: u32) -> u32 {
    row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
}

/// Why there is no 16-bit picture on a device that can't draw one.
const NO_DEEP: &str = "This graphics driver can't draw 16-bit pictures, so it can't export TIFF. Export as JPEG or PNG instead.";

/// The first of `DEEP_FORMATS` that the device can draw into and read back,
/// by `drawable`.
fn deep_format(drawable: impl Fn(wgpu::TextureFormat) -> bool) -> Option<wgpu::TextureFormat> {
    DEEP_FORMATS.into_iter().find(|&format| drawable(format))
}

/// One channel of a deep frame, read back in `format`, as a 16-bit level.
fn deep_level(format: wgpu::TextureFormat, bytes: &[u8]) -> u16 {
    let value = match format {
        wgpu::TextureFormat::Rgba16Unorm => return u16::from_le_bytes([bytes[0], bytes[1]]),
        wgpu::TextureFormat::Rgba16Float => f16::from_le_bytes([bytes[0], bytes[1]]).to_f32(),
        _ => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
    };
    (value.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// The size a `width` x `height` photo is held at on a device whose largest
/// texture is `largest` pixels a side: its own, or scaled down to fit.
pub fn held_size(width: u32, height: u32, largest: u32) -> (u32, u32) {
    match width.max(height) > largest {
        true => fit_within(width, height, largest),
        false => (width, height),
    }
}

/// Scales an image down to fit the largest texture the device supports,
/// each new pixel the average of the old ones it covers.
fn shrink_to_fit(image: LinearImage, max_size: u32) -> LinearImage {
    let (width, height) = held_size(image.width, image.height, max_size);
    if (width, height) == (image.width, image.height) {
        return image;
    }
    let (columns, rows) = (spans(image.width, width), spans(image.height, height));
    let average = |pixels: &mut dyn Iterator<Item = ([f32; 3], f32)>| {
        pixels.fold([0.0; 3], |sum, (pixel, weight)| std::array::from_fn(|i| sum[i] + pixel[i] * weight))
    };
    // Across each row first, then down each column of that.
    let mut across = vec![[0.0; 3]; width as usize * image.height as usize];
    across.par_chunks_mut(width as usize).zip(image.pixels.par_chunks(image.width as usize)).for_each(|(out, row)| {
        for (pixel, span) in out.iter_mut().zip(&columns) {
            *pixel = average(&mut span.iter().map(|&(x, weight)| (row[x], weight)));
        }
    });
    let mut pixels = vec![[0.0; 3]; width as usize * height as usize];
    pixels.par_chunks_mut(width as usize).zip(rows.par_iter()).for_each(|(out, span)| {
        for (x, pixel) in out.iter_mut().enumerate() {
            *pixel = average(&mut span.iter().map(|&(y, weight)| (across[y * width as usize + x], weight)));
        }
    });
    LinearImage { width, height, pixels, ..image }
}

/// For each of `to` pixels along a side `from` pixels long, the old pixels
/// it covers and how much each counts towards it.
fn spans(from: u32, to: u32) -> Vec<Vec<(usize, f32)>> {
    let scale = from as f64 / to as f64;
    (0..to)
        .map(|i| {
            let (start, end) = (i as f64 * scale, (i + 1) as f64 * scale);
            (start.floor() as usize..(end.ceil() as usize).min(from as usize))
                .map(|j| (j, ((end.min(j as f64 + 1.0) - start.max(j as f64)) / scale) as f32))
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Crop;

    /// A small picture that changes smoothly both ways.
    fn gradient(gpu: &Gpu) -> Session {
        let (width, height) = (96u32, 80u32);
        let pixels = (0..width * height)
            .map(|i| [(i % width) as f32 / width as f32, (i / width) as f32 / height as f32, 0.5])
            .collect();
        gpu.open(LinearImage { width, height, pixels, scene_referred: false }).unwrap()
    }

    fn gpu() -> Option<&'static Gpu> {
        shared().inspect_err(|error| eprintln!("skipping: {error:#}")).ok()
    }

    /// A picture of one flat grey, which has no local contrast anywhere.
    fn flat(gpu: &Gpu, value: f32) -> Session {
        let (width, height) = (64u32, 48u32);
        let pixels = vec![[value; 3]; (width * height) as usize];
        gpu.open(LinearImage { width, height, pixels, scene_referred: false }).unwrap()
    }

    /// The largest difference between two renders, in 8-bit steps.
    fn largest_difference(a: &image::RgbImage, b: &image::RgbImage) -> u8 {
        assert_eq!(a.dimensions(), b.dimensions());
        a.as_raw().iter().zip(b.as_raw()).map(|(a, b)| a.abs_diff(*b)).max().unwrap()
    }

    #[test]
    fn clarity_leaves_a_flat_dehazed_picture_alone() {
        let Some(gpu) = gpu() else { return };
        let session = flat(gpu, 0.3);
        for dehaze in [100.0, -100.0] {
            let dehazed = Adjustments { dehaze, ..Default::default() };
            let with_clarity = Adjustments { clarity: 100.0, ..dehazed.clone() };
            let dehazed = gpu.render_image(&session, &dehazed, u32::MAX).unwrap();
            let with_clarity = gpu.render_image(&session, &with_clarity, u32::MAX).unwrap();
            let difference = largest_difference(&dehazed, &with_clarity);
            assert!(difference <= 1, "clarity moved a flat picture with dehaze {dehaze} by {difference} steps");
        }
    }

    /// `DEHAZE_STRENGTH` in develop.wgsl.
    const DEHAZE_STRENGTH: f32 = 0.6 / 0.9;

    /// What dehaze at `dehaze` (-1..1) makes of a flat grey `value` with no
    /// haze around it but its own.
    fn dehazed_flat(value: f32, dehaze: f32) -> f32 {
        let clear = (-dehaze * DEHAZE_STRENGTH * value).exp2();
        (value - (1.0 - clear)) / clear
    }

    #[test]
    fn the_shader_dehazes_as_strongly_as_the_tests_expect() {
        let shader = include_str!("shaders/develop.wgsl");
        assert!(shader.contains("const DEHAZE_STRENGTH = 0.6 / 0.9;"));
    }

    #[test]
    fn shadows_and_highlights_judge_a_dehazed_picture_by_how_it_now_looks() {
        let Some(gpu) = gpu() else { return };
        let hazy = flat(gpu, 0.3);
        let clear = flat(gpu, dehazed_flat(0.3, 1.0));
        for (shadows, highlights) in [(100.0, 0.0), (0.0, -100.0)] {
            let edit = Adjustments { shadows, highlights, ..Default::default() };
            let dehazed = gpu.render_image(&hazy, &Adjustments { dehaze: 100.0, ..edit.clone() }, u32::MAX).unwrap();
            let already_clear = gpu.render_image(&clear, &edit, u32::MAX).unwrap();
            let difference = largest_difference(&dehazed, &already_clear);
            assert!(difference <= 1, "shadows {shadows} and highlights {highlights} differ by {difference} steps");
        }
    }

    #[test]
    fn exposure_does_not_change_how_much_haze_dehaze_finds() {
        let Some(gpu) = gpu() else { return };
        // Brightening a hazy photo used to make the same haze look thicker,
        // so the same slider took more off (#33).
        for scene_referred in [false, true] {
            let (width, height) = (64u32, 48u32);
            let open = |value: f32| {
                let pixels = vec![[value; 3]; (width * height) as usize];
                gpu.open(LinearImage { width, height, pixels, scene_referred }).unwrap()
            };
            let hazy = open(0.3);
            for dehaze in [100.0f32, -100.0] {
                let clear = open(dehazed_flat(0.3, dehaze / 100.0));
                for exposure in [-1.0, 0.0, 1.0] {
                    let edit = Adjustments { exposure, ..Default::default() };
                    let dehazed = gpu.render_image(&hazy, &Adjustments { dehaze, ..edit.clone() }, u32::MAX).unwrap();
                    let already_clear = gpu.render_image(&clear, &edit, u32::MAX).unwrap();
                    let difference = largest_difference(&dehazed, &already_clear);
                    assert!(
                        difference <= 1,
                        "dehaze {dehaze} at exposure {exposure} (RAW: {scene_referred}) is {difference} steps off"
                    );
                }
            }
        }
    }

    #[test]
    fn dehaze_adds_contrast_in_even_steps_and_gently() {
        let Some(gpu) = gpu() else { return };
        // Fine stripes of 0.92 and 1.0 under haze as thick as it is ever
        // taken to be (0.9 of white): dehaze lifts the same veil off both,
        // so the gap between them grows by exactly its contrast gain.
        let (width, height) = (64u32, 48u32);
        let pixels = (0..width * height).map(|i| [if i % 2 == 0 { 0.92 } else { 1.0 }; 3]).collect();
        let session = gpu.open(LinearImage { width, height, pixels, scene_referred: false }).unwrap();
        let gap = |edit: &Adjustments| {
            let picture = gpu.render_deep_image(&session, edit, u32::MAX).unwrap();
            let linear = |x: u32| {
                let v = picture.get_pixel(x, height / 2).0[1] as f32 / 65535.0;
                if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            linear(width / 2 + 1) - linear(width / 2)
        };
        for exposure in [0.0, -1.0] {
            let plain = gap(&Adjustments { exposure, ..Default::default() });
            for (dehaze, expected) in [(50.0, 2f32.powf(0.3)), (100.0, 2f32.powf(0.6))] {
                let gain = gap(&Adjustments { exposure, dehaze, ..Default::default() }) / plain;
                // It used to reach 2.2 at +100, most of it in the last stretch.
                assert!((gain - expected).abs() < 0.03, "dehaze {dehaze} at exposure {exposure}: {gain:.3}, not {expected:.3}");
            }
        }
    }

    #[test]
    fn a_picture_drawn_in_bands_has_no_seams() {
        let Some(gpu) = gpu() else { return };
        let session = gradient(gpu);
        // Grain and a vignette would both show a seam if bands were drawn from the wrong place.
        let edit = Adjustments { grain: 60.0, vignette: -50.0, sharpening: 40.0, ..Default::default() };
        let draw = |band_pixels| {
            let mut pixels = Vec::new();
            let size = gpu.render_in_bands(&session, &edit, u32::MAX, band_pixels, false, |band| pixels.extend(band)).unwrap();
            (size, pixels)
        };

        let whole = draw(u32::MAX);
        assert_eq!(whole.0, (96, 80));
        // Seven rows at a time, leaving a short band at the bottom.
        assert!(whole == draw(96 * 7), "bands differ from the picture drawn in one go");
    }

    #[test]
    fn the_crop_tool_draws_the_vignette_around_the_crop() {
        let Some(gpu) = gpu() else { return };
        let (width, height) = (120u32, 80u32);
        let flat = LinearImage { width, height, pixels: vec![[0.5; 3]; (width * height) as usize], scene_referred: false };
        let session = gpu.open(flat).unwrap();
        // Half the photo, off to the left and down: 60 x 40 pixels from (12, 28).
        let crop = Crop { x: 0.35, y: 0.6, width: 0.5, height: 0.5 };
        let edit = Adjustments { vignette: -80.0, crop, ..Default::default() };
        let cropped = gpu.render(&session, &edit, Region::FULL, (60, 40), Guides::default()).unwrap();
        let whole =
            gpu.render(&session, &edit, Region::FULL, (width, height), Guides { uncropped: true, ..Default::default() }).unwrap();
        let grey = |rgba: &[u8], row: u32, x: u32, y: u32| rgba[((y * row + x) * 4) as usize] as i32;

        let middle = grey(&cropped, 60, 30, 20);
        assert!(grey(&cropped, 60, 0, 0) < middle - 40, "the vignette darkens the crop's corners");
        for (x, y) in [(0, 0), (59, 0), (0, 39), (59, 39), (30, 20), (10, 30)] {
            let (in_crop, in_whole) = (grey(&cropped, 60, x, y), grey(&whole, width, 12 + x, 28 + y));
            assert!((in_crop - in_whole).abs() <= 2, "at ({x}, {y}) of the crop: {in_crop} cropped, {in_whole} in the crop tool");
        }
    }

    #[test]
    fn frames_drawn_into_kept_targets_match_frames_drawn_afresh() {
        let Some(gpu) = gpu() else { return };
        let session = gradient(gpu);
        let grainy = Adjustments { grain: 60.0, exposure: 0.5, ..Default::default() };
        let darker = Adjustments { exposure: -1.0, vignette: -40.0, ..Default::default() };
        let draw = |session: &Session, edit: &Adjustments, size| {
            gpu.render(session, edit, Region::FULL, size, Guides::default()).unwrap()
        };
        let target_of = |width, height| {
            let targets = session.targets.lock().unwrap();
            targets.iter().find(|t| (t.width, t.height, t.format) == (width, height, OUTPUT_FORMAT)).map(|t| t.texture.clone())
        };

        draw(&session, &grainy, (96, 80));
        let first = target_of(96, 80).expect("the frame's target is kept");
        // Another size in between, as the histogram's small picture is.
        draw(&session, &darker, (48, 40));
        let again = draw(&session, &darker, (96, 80));
        assert!(target_of(96, 80) == Some(first), "the second frame of the same size reused the target");
        assert!(again == draw(&gradient(gpu), &darker, (96, 80)), "and came out as a fresh session draws it");

        // A deep picture of the same size has its own target, and doesn't disturb the plain one.
        if gpu.develop_deep.is_some() {
            gpu.render_deep_image(&session, &grainy, u32::MAX).unwrap();
        }
        assert!(draw(&session, &grainy, (96, 80)) == draw(&gradient(gpu), &grainy, (96, 80)));

        // A string of sizes keeps only the most recent few.
        for width in 10..20 {
            draw(&session, &grainy, (width, 10));
        }
        assert_eq!(session.targets.lock().unwrap().len(), TARGETS_KEPT);
        assert!(target_of(19, 10).is_some() && target_of(96, 80).is_none());
    }

    #[test]
    fn the_histogram_is_counted_again_only_when_the_picture_changes() {
        use crate::edit::{LocalAdjustments, Mask, MaskPart, Mode, Shape};
        let Some(gpu) = gpu() else { return };
        let session = gradient(gpu);
        let counted = |edit: &Adjustments| Histogram::of(&gpu.render_image(&session, edit, Histogram::EDGE).unwrap());
        let kept = || session.histogram.lock().unwrap().as_ref().map(|(_, _, histogram)| histogram.clone());

        let edit = Adjustments { contrast: 30.0, ..Default::default() };
        let histogram = gpu.histogram(&session, &edit).unwrap();
        assert_eq!(histogram, counted(&edit));
        assert_eq!(kept(), Some(histogram.clone()));
        // Kept: a stand-in left in its place is what comes back for the same recipe.
        let stand_in = Histogram { bins: [[7; 256]; 4], blown: true, crushed: true };
        session.histogram.lock().unwrap().as_mut().unwrap().2 = stand_in.clone();
        assert_eq!(gpu.histogram(&session, &edit).unwrap(), stand_in);

        // Another recipe is counted afresh.
        let brighter = Adjustments { exposure: 1.0, ..edit.clone() };
        assert_eq!(gpu.histogram(&session, &brighter).unwrap(), counted(&brighter));

        // So is the same recipe once a part it masks is found.
        let subject = Mask {
            id: 1,
            parts: vec![MaskPart { mode: Mode::Add, shape: Shape::Subject }],
            adjustments: LocalAdjustments { exposure: 2.0, ..Default::default() },
            ..Default::default()
        };
        let masked = Adjustments { masks: vec![subject], ..edit };
        // Not found yet: it draws as empty, without running the model.
        session.missing.lock().unwrap().insert(session.key(&Found::Subject));
        let before = gpu.histogram(&session, &masked).unwrap();
        session.set_matte(&Found::Subject, image::GrayImage::from_pixel(8, 8, image::Luma([255])));
        let after = gpu.histogram(&session, &masked).unwrap();
        assert_ne!(before, after);
        assert_eq!(after, counted(&masked));
    }

    #[test]
    fn a_deep_picture_is_the_same_picture_with_finer_steps() {
        let Some(gpu) = gpu() else { return };
        deep_matches_plain(gpu);
    }

    /// Draws a picture both ways and checks the deep one rounds to the plain one.
    fn deep_matches_plain(gpu: &Gpu) {
        let session = gradient(gpu);
        let edit = Adjustments { contrast: 20.0, ..Default::default() };
        let plain = gpu.render_image(&session, &edit, u32::MAX).unwrap();
        let deep = gpu.render_deep_image(&session, &edit, u32::MAX).unwrap();
        assert_eq!(plain.dimensions(), deep.dimensions());

        for (plain, deep) in plain.as_raw().iter().zip(deep.as_raw()) {
            // Rounded to 8 bits, the deep picture is the plain one.
            assert!((*deep as f32 / 257.0 - *plain as f32).abs() <= 0.75, "{deep} is not {plain}");
        }
        let reds: std::collections::HashSet<u16> = deep.pixels().map(|pixel| pixel.0[0]).collect();
        let coarse: std::collections::HashSet<u8> = plain.pixels().map(|pixel| pixel.0[0]).collect();
        assert!(reds.len() > coarse.len(), "{} levels where 8 bits gave {}", reds.len(), coarse.len());
    }

    #[test]
    fn deep_pictures_use_the_finest_format_the_device_can_draw_into() {
        use wgpu::TextureFormat::*;
        assert_eq!(deep_format(|_| true), Some(Rgba32Float));
        assert_eq!(deep_format(|format| format != Rgba32Float), Some(Rgba16Unorm));
        assert_eq!(deep_format(|format| format == Rgba16Float || format == Rgba8Unorm), Some(Rgba16Float));
        assert_eq!(deep_format(|format| format == Rgba8Unorm), None);
    }

    #[test]
    fn every_deep_format_reads_back_as_the_same_levels() {
        use wgpu::TextureFormat::*;
        for value in [0.0f32, 0.25, 0.5, 1.0, 1.5, -0.25] {
            let level = (value.clamp(0.0, 1.0) * 65535.0).round() as u16;
            assert_eq!(deep_level(Rgba32Float, &value.to_le_bytes()), level);
            assert_eq!(deep_level(Rgba16Unorm, &level.to_le_bytes()), level);
            assert_eq!(deep_level(Rgba16Float, &f16::from_f32(value).to_le_bytes()), level);
        }
    }

    #[test]
    fn the_editor_draws_through_opengl() {
        // Some drivers wgpu only reaches through OpenGL can't draw into
        // 32-bit float textures; the editor and 16-bit exports still work (#79).
        let mut instance = wgpu::InstanceDescriptor::new_without_display_handle();
        instance.backends = wgpu::Backends::GL;
        let gpu = match Gpu::on(instance) {
            Ok(gpu) => gpu,
            Err(error) => return eprintln!("skipping, no OpenGL adapter: {error:#}"),
        };
        let (format, _) = gpu.develop_deep.as_ref().expect("no format to draw 16-bit pictures in");
        eprintln!("drawing 16-bit pictures through OpenGL in {format:?}");
        deep_matches_plain(&gpu);
    }

    #[test]
    fn a_photo_too_large_for_the_device_is_scaled_to_fit_not_halved() {
        // A 250 x 120 photo on a device that holds 100 px: halving would give 62 x 30.
        let (width, height) = (250u32, 120u32);
        let pixels = (0..width * height).map(|i| [(i % width) as f32 / width as f32, 0.25, (i / width % 2) as f32]).collect();
        let held = shrink_to_fit(LinearImage { width, height, pixels, scene_referred: true }, 100);
        assert_eq!((held.width, held.height), (100, 48));
        assert_eq!(held_size(width, height, 100), (100, 48));
        assert!(held.scene_referred);
        // Averaged, not picked: flat colour stays flat, a ramp stays a ramp
        // and alternating rows blend to their middle.
        for (i, pixel) in held.pixels.iter().enumerate() {
            let x = (i % 100) as f32;
            assert!((pixel[0] - (x + 0.5) / 100.0).abs() < 0.005, "{pixel:?} at {x}");
            assert!((pixel[1] - 0.25).abs() < 1e-5);
            assert!((pixel[2] - 0.5).abs() < 0.11, "{pixel:?}");
        }
        let mean = held.pixels.iter().map(|pixel| pixel[2]).sum::<f32>() / held.pixels.len() as f32;
        assert!((mean - 0.5).abs() < 1e-3);
    }

    #[test]
    fn the_shader_caps_exposure_where_the_sliders_do() {
        let shader = include_str!("shaders/develop.wgsl");
        assert!(shader.contains(&format!("const MAX_EXPOSURE = {MAX_EXPOSURE:.1};")));
    }

    #[test]
    fn a_photo_that_fits_is_left_alone() {
        let image = LinearImage { width: 3, height: 2, pixels: vec![[0.5; 3]; 6], scene_referred: false };
        let held = shrink_to_fit(image, 3);
        assert_eq!((held.width, held.height, held.pixels.len()), (3, 2, 6));
        assert_eq!(held_size(3, 2, 3), (3, 2));
    }
}
