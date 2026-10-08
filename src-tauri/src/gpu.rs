//! The GPU side of the editor. A photo is uploaded once at full resolution
//! (`Session`); after that every slider change is one draw of the develop
//! shader, at whatever size and crop the screen needs.

use anyhow::{anyhow, Context, Result};
use bytemuck::{Pod, Zeroable};
use half::f16;
use rayon::prelude::*;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::develop::LinearImage;
use crate::edit::{curve_table, Adjustments, Stroke, MAX_EXPOSURE};
use crate::geometry;
use crate::masks::{self, Coverage, COVERAGE_EDGE, MAX_BRUSHES, MAX_FOUND, MAX_MASKS, MAX_PARTS};
use crate::segment::{self, Found};

const WORKING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// What 16-bit exports are drawn into: the same picture before it is rounded
/// to 256 levels.
const DEEP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// Brush and found coverage: 256 levels is far finer than any slider step it scales.
const COVERAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Long edge and blur width (in their own pixels) of the two blurred copies.
/// Fixed sizes make the blurs cover the same share of any photo.
const MEDIUM_BLUR: (u32, f32) = (768, 4.0);
const LARGE_BLUR: (u32, f32) = (192, 5.0);

/// The most pixels read back from the GPU in one go. A full-size export is
/// drawn in bands of this many, so no photo is too big for one buffer.
const BAND_PIXELS: u32 = 16 << 20;
/// Deep pixels are four times the size, so their bands are a quarter as tall.
const DEEP_BAND_PIXELS: u32 = BAND_PIXELS / 4;

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
    /// The develop shader again, drawing into `DEEP_FORMAT`.
    develop_deep: wgpu::RenderPipeline,
    copy: wgpu::RenderPipeline,
    gaussian: wgpu::RenderPipeline,
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
    views: Views,
    coverages: Mutex<Coverages>,
    /// The photo as the models see it, kept to find its subject, sky and
    /// circled objects in (`segment::guide`).
    picture: Arc<segment::Picture>,
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
}

impl Session {
    /// The photo as the models see it, to find parts in.
    pub fn picture(&self) -> Arc<segment::Picture> {
        self.picture.clone()
    }

    /// Whether this part has been found (or set) for this session.
    pub fn has_matte(&self, found: &Found) -> bool {
        self.mattes.lock().unwrap().contains_key(&found.key())
    }

    /// Uses `matte` for this found part: white where it is. Any size; it is
    /// stretched over the whole photo.
    pub fn set_matte(&self, found: &Found, matte: image::GrayImage) {
        let key = found.key();
        self.missing.lock().unwrap().remove(&key);
        self.mattes.lock().unwrap().insert(key, Arc::new(matte));
    }

    /// Where this part's matte is kept between sessions, if anywhere.
    pub fn matte_path(&self, found: &Found) -> Option<PathBuf> {
        let start = self.matte_files.as_ref()?;
        let mut name = start.file_name()?.to_os_string();
        name.push(format!("{}.png", found.key()));
        Some(start.with_file_name(name))
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
    deep_bind_group: wgpu::BindGroup,
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
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| anyhow!("no usable graphics adapter: {e}"))?;
        let limits = adapter.limits();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("tonality"),
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
            develop_deep: pipeline("develop deep", &develop_shader, "fragment", DEEP_FORMAT),
            copy: pipeline("copy", &prepare_shader, "copy", WORKING_FORMAT),
            gaussian: pipeline("gaussian", &prepare_shader, "gaussian", WORKING_FORMAT),
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
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
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

    /// A blurred, shrunken copy of `source`, `long_edge` pixels along its longer side.
    fn blurred_copy(&self, source: &wgpu::TextureView, width: u32, height: u32, (long_edge, sigma): (u32, f32)) -> wgpu::TextureView {
        use wgpu::util::DeviceExt;
        let (w, h) = fit_within(width, height, long_edge.min(width.max(height)));
        let make = |label| self.texture(label, w, h, 1, WORKING_FORMAT).create_view(&Default::default());
        let (first, second) = (make("blur"), make("blur scratch"));
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.draw(&mut encoder, &self.copy, &self.copy_bindings(source), &first);

        // Horizontal into the scratch texture, then vertical back.
        for (input, target, step) in [(&first, &second, [1.0 / w as f32, 0.0]), (&second, &first, [0.0, 1.0 / h as f32])] {
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
            self.draw(&mut encoder, &self.gaussian, &bindings, target);
        }
        self.queue.submit([encoder.finish()]);
        first
    }

    /// Uploads a photo and prepares everything the develop shader reads.
    pub fn open(&self, image: LinearImage) -> Result<Session> {
        let image = shrink_to_fit(image, self.max_texture_size);
        let (width, height) = (image.width, image.height);
        let picture = Arc::new(segment::Picture::new(segment::guide(&image)));

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

        // Mipmaps, each level averaged down from the one above, so a
        // zoomed-out view is a properly filtered one.
        let level_view = |level| {
            source.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for level in 1..mips {
            self.draw(&mut encoder, &self.copy, &self.copy_bindings(&level_view(level - 1)), &level_view(level));
        }
        self.queue.submit([encoder.finish()]);

        let source_view = source.create_view(&Default::default());
        let blur_medium = self.blurred_copy(&source_view, width, height, MEDIUM_BLUR);
        let blur_large = self.blurred_copy(&source_view, width, height, LARGE_BLUR);

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
            deep_bind_group: self.develop_bindings(&self.develop_deep, &views, &params, &brush_view, &found_view),
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
            views,
            coverages: Mutex::new(coverages),
            picture,
            matte_files: None,
            mattes: Mutex::new(HashMap::new()),
            finding: Mutex::new(()),
            missing: Mutex::new(HashSet::new()),
        })
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

    /// Points both pipelines' bindings at the session's current coverage maps.
    fn rebind(&self, session: &Session, coverages: &mut Coverages) {
        let (brushes, found) = (&coverages.brush_view, &coverages.found_view);
        coverages.bind_group = self.develop_bindings(&self.develop, &session.views, &session.params, brushes, found);
        coverages.deep_bind_group =
            self.develop_bindings(&self.develop_deep, &session.views, &session.params, brushes, found);
    }

    /// Makes sure these found parts' mattes are ready for the shader: read
    /// from the cache, or found by a model, which takes a second or two each.
    /// A part that can't be found draws as empty rather than stopping the
    /// photo drawing; those found missing by this call are returned.
    pub fn ensure_found(&self, session: &Session, found: &[Found]) -> Vec<(Found, anyhow::Error)> {
        let mut missing = Vec::new();
        for part in found {
            let ready = || session.has_matte(part) || session.missing.lock().unwrap().contains(&part.key());
            if ready() {
                continue;
            }
            let _finding = session.finding.lock().unwrap();
            // Another thread may have found it while this one waited.
            if ready() {
                continue;
            }
            match part.find_cached(&session.picture, session.matte_path(part).as_deref()) {
                Ok(matte) => session.set_matte(part, matte),
                Err(error) => {
                    session.missing.lock().unwrap().insert(part.key());
                    missing.push((part.clone(), error));
                }
            }
        }
        missing
    }

    /// Puts the mattes of `found` into the layers the shader reads them from,
    /// in order, uploading only those that changed. They must be ready (`ensure_found`).
    fn upload_found(&self, session: &Session, coverages: &mut Coverages, found: &[Found]) {
        let (width, height) = session.picture.guide.dimensions();
        if coverages.found.is_none() {
            let (texture, view) = self.coverage_maps(width, height, MAX_FOUND as u32);
            coverages.found = Some(texture);
            coverages.found_view = view;
            self.rebind(session, coverages);
        }
        let texture = coverages.found.as_ref().expect("just made");
        let mattes = session.mattes.lock().unwrap();
        for (layer, part) in found.iter().enumerate() {
            let key = part.key();
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

    /// `render`, or with `deep` the same frame as four little-endian f32s a
    /// pixel, each 0..1.
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
        for (part, error) in self.ensure_found(session, &found) {
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
                session.scene_referred as u8 as f32,
            ],
            light: [a.exposure.clamp(-MAX_EXPOSURE, MAX_EXPOSURE), unit(a.contrast), unit(a.highlights), unit(a.shadows)],
            tone: [unit(a.whites), unit(a.blacks), unit(a.temperature), unit(a.tint)],
            color: [unit(a.vibrance), unit(a.saturation), unit(a.clarity), unit(a.dehaze)],
            detail: [unit(a.sharpening).max(0.0), unit(a.noise_reduction).max(0.0), unit(a.vignette), unit(a.grain).max(0.0)],
            flags: [show_clipping as u8 as f32, uncropped as u8 as f32, matte as u8 as f32, 0.0],
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

        let (format, pipeline, bindings, pixel_bytes) = match deep {
            false => (OUTPUT_FORMAT, &self.develop, &coverages.bind_group, 4),
            true => (DEEP_FORMAT, &self.develop_deep, &coverages.deep_bind_group, 16),
        };
        let target = self.texture("frame", width, height, 1, format);
        // Rows in a readback buffer must be padded to a multiple of 256 bytes.
        let row = width * pixel_bytes;
        let padded_row = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: padded_row as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.draw(&mut encoder, pipeline, bindings, &target.create_view(&Default::default()));
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded_row), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);

        let (sender, receiver) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).context("waiting for the graphics device")?;
        receiver.recv().context("the graphics device dropped the frame")??;

        let mapped = readback.slice(..).get_mapped_range().context("reading the frame back")?;
        let mut pixels = Vec::with_capacity((row * height) as usize);
        for line in mapped.chunks_exact(padded_row as usize) {
            pixels.extend_from_slice(&line[..row as usize]);
        }
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
        let mut rgb = Vec::new();
        let (width, height) = self.render_in_bands(session, adjustments, long_edge, DEEP_BAND_PIXELS, true, |rgba| {
            let level = |bytes: &[u8]| {
                let value = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                (value.clamp(0.0, 1.0) * 65535.0).round() as u16
            };
            rgb.extend(rgba.as_chunks::<16>().0.iter().flat_map(|p| [level(&p[0..4]), level(&p[4..8]), level(&p[8..12])]));
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
    fn a_deep_picture_is_the_same_picture_with_finer_steps() {
        let Some(gpu) = gpu() else { return };
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
