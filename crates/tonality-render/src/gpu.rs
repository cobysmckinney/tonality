//! Headless wgpu renderer for export: runs `develop.wgsl` natively at full resolution.
//!
//! The image is rendered in tiles, so any size works within the GPU's texture limits. Each
//! tile's texture carries an apron of neighbouring pixels so Detail (sharpening, noise
//! reduction) sees the same neighbourhood as it would in one big pass: tiles join seamlessly.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use rayon::prelude::*;

use tonality_core::geometry::Geometry;
use tonality_core::mask::BrushLayers;

use crate::presence::PresenceMaps;
use crate::{DevelopParams, DEVELOP_WGSL};

/// Output tile size (pixels).
const TILE: u32 = 2048;
/// Extra source pixels around each tile: covers the widest Detail kernel.
const APRON: u32 = 24;
const HALF_ONE: u16 = 0x3C00;

/// Camera-space input to render (the whole, unframed image; `params` carries the framing): RGB half floats (bit patterns), row-major.
pub struct Source<'a> {
    pub width: u32,
    pub height: u32,
    pub pixels: &'a [u16],
    /// Size of the full-resolution image this source was made from. Detail sizes are in
    /// full-resolution pixels, so a downscaled source renders like the downscaled original.
    pub frame: [u32; 2],
}

/// Rendered display-referred sRGB, 16 bits per channel, interleaved RGB.
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u16>,
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    /// Set when the device is lost or reports an error outside `render`'s error scopes: the
    /// device can't be trusted any more, and the owner should make a new `Gpu`.
    lost: Arc<AtomicBool>,
    /// Adapter name, for diagnostics.
    pub adapter: String,
}

const TARGET: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

impl Gpu {
    pub fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let name = adapter.get_info().name;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("tonality export"),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| format!("cannot open GPU device: {e}"))?;

        // wgpu's default handlers panic; record the failure instead, so callers get an error and
        // can rebuild the device (a driver reset, suspend/resume, running out of video memory).
        let lost = Arc::new(AtomicBool::new(false));
        let flag = lost.clone();
        device.set_device_lost_callback(move |reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                eprintln!("GPU device lost ({reason:?}): {message}");
            }
            flag.store(true, Ordering::SeqCst);
        });
        let flag = lost.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            eprintln!("uncaptured GPU error: {e}");
            flag.store(true, Ordering::SeqCst);
        }));

        let scopes = ErrorScopes::push(&device);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("develop"),
            source: wgpu::ShaderSource::Wgsl(DEVELOP_WGSL.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("develop"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TARGET,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        scopes
            .pop()
            .map_err(|e| format!("cannot build the develop pipeline: {e}"))?;
        Ok(Self {
            device,
            queue,
            pipeline,
            sampler,
            lost,
            adapter: name,
        })
    }

    /// Whether the device has been lost (or failed outside an error scope): every further call
    /// will fail, so make a new `Gpu`.
    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }

    /// Develop `src` with `params`, the tone-curve LUT (`curve_lut_rgba16f`) and the brush
    /// mask rasters (`tonality_core::mask::brush_layers`). `progress` receives the fraction of
    /// tiles done. GPU errors (out of memory, validation, a lost device) come back as `Err`.
    pub fn render(
        &self,
        src: &Source,
        params: &DevelopParams,
        curve_lut: &[u8],
        brush: &BrushLayers,
        presence: Option<&PresenceMaps>,
        progress: impl FnMut(f32),
    ) -> Result<Rendered, String> {
        if self.is_lost() {
            return Err("GPU device lost".into());
        }
        let scopes = ErrorScopes::push(&self.device);
        let result = self.render_tiles(src, params, curve_lut, brush, presence, progress);
        // A GPU error explains a failure better than what it led to (a failed readback, say).
        scopes.pop().map_err(|e| format!("GPU error: {e}"))?;
        if self.is_lost() {
            return Err("GPU device lost".into());
        }
        result
    }

    fn render_tiles(
        &self,
        src: &Source,
        params: &DevelopParams,
        curve_lut: &[u8],
        brush: &BrushLayers,
        presence: Option<&PresenceMaps>,
        mut progress: impl FnMut(f32),
    ) -> Result<Rendered, String> {
        let (sw_all, sh_all) = (src.width, src.height);
        if src.pixels.len() != (sw_all * sh_all * 3) as usize {
            return Err("source size mismatch".into());
        }
        // Output size: the framed (cropped) size, at the source's scale.
        let scale = sw_all as f32 / src.frame[0] as f32;
        let w = ((params.geo0[3] * scale).round() as u32).max(1);
        let h = ((params.geo1[3] * scale).round() as u32).max(1);
        let geo = Geometry {
            width: params.geo0[3],
            height: params.geo1[3],
            m: [
                [params.geo0[0], params.geo0[1], params.geo0[2]],
                [params.geo1[0], params.geo1[1], params.geo1[2]],
            ],
        };
        let d = &self.device;
        let params_buf = uniform(d, "params", bytes_f32(&params.to_vec()));
        let curve = self.texture("curve", crate::params::CURVE_LUT_SIZE as u32, 1);
        self.queue.write_texture(
            curve.as_image_copy(),
            curve_lut,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(crate::params::CURVE_LUT_SIZE as u32 * 8),
                rows_per_image: None,
            },
            curve.size(),
        );
        let curve_view = curve.create_view(&Default::default());
        let brush_view = self.brush_texture(brush);
        let presence_view = self.presence_texture(presence);

        let mut out = vec![0u16; (w * h * 3) as usize];
        let tiles: Vec<(u32, u32)> = (0..h.div_ceil(TILE))
            .flat_map(|ty| (0..w.div_ceil(TILE)).map(move |tx| (tx * TILE, ty * TILE)))
            .collect();
        let mut inflight: Option<InFlight> = None;
        for (n, &(x, y)) in tiles.iter().enumerate() {
            let tw = TILE.min(w - x);
            let th = TILE.min(h - y);
            // The source area this tile reads (rotation makes it larger), plus an apron for
            // Detail's neighbourhood, clamped to the image.
            let (fw, fh) = (w as f32, h as f32);
            let b = geo.source_bounds(
                [x as f32 / fw, y as f32 / fh],
                [(x + tw) as f32 / fw, (y + th) as f32 / fh],
            );
            // Lens correction reads up to this far from where framing points.
            let m = params.lens_head[1];
            let b = [b[0] - m, b[1] - m, b[2] + m, b[3] + m];
            let px = |v: f32, size: u32| (v * size as f32).clamp(0.0, size as f32);
            let sx = (px(b[0], sw_all).floor() as u32)
                .saturating_sub(APRON)
                .min(sw_all - 1);
            let sy = (px(b[1], sh_all).floor() as u32)
                .saturating_sub(APRON)
                .min(sh_all - 1);
            let sw = ((px(b[2], sw_all).ceil() as u32 + APRON).min(sw_all) - sx).max(1);
            let sh = ((px(b[3], sh_all).ceil() as u32 + APRON).min(sh_all) - sy).max(1);

            let image = self.texture("image", sw, sh);
            // RGB halves → RGBA half bytes, a row per task.
            let mut rgba = vec![0u8; (sw * sh * 8) as usize];
            rgba.par_chunks_mut((sw * 8) as usize)
                .enumerate()
                .for_each(|(r, dst)| {
                    let start = (((sy + r as u32) * sw_all + sx) * 3) as usize;
                    let row = &src.pixels[start..start + (sw * 3) as usize];
                    for (p, o) in row
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .zip(dst.as_chunks_mut::<8>().0)
                    {
                        o[0..2].copy_from_slice(&p[0].to_le_bytes());
                        o[2..4].copy_from_slice(&p[1].to_le_bytes());
                        o[4..6].copy_from_slice(&p[2].to_le_bytes());
                        o[6..8].copy_from_slice(&HALF_ONE.to_le_bytes());
                    }
                });
            self.queue.write_texture(
                image.as_image_copy(),
                &rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(sw * 8),
                    rows_per_image: None,
                },
                image.size(),
            );
            drop(rgba);

            let (uw, uh) = (sw_all as f32, sh_all as f32);
            let view = [
                1.0,
                1.0,
                0.0,
                0.0, // xform: the quad fills the tile
                x as f32 / fw,
                y as f32 / fh,
                tw as f32 / fw,
                th as f32 / fh, // quad (frame)
                sx as f32 / uw,
                sy as f32 / uh,
                sw as f32 / uw,
                sh as f32 / uh, // tex (source)
                src.frame[0] as f32,
                src.frame[1] as f32,
                0.0,
                0.0, // frame
            ];
            let view_buf = uniform(d, "view", bytes_f32(&view));
            let image_view = image.create_view(&Default::default());
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("develop"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: view_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&image_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&curve_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(&brush_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(&presence_view),
                    },
                ],
            });

            let target = d.create_texture(&wgpu::TextureDescriptor {
                label: Some("tile"),
                size: wgpu::Extent3d {
                    width: tw,
                    height: th,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TARGET,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let target_view = target.create_view(&Default::default());
            let row_bytes = (tw * 16).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
            let readback = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: (row_bytes * th) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            let mut enc = d.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("develop"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.draw(0..6, 0..1);
            }
            enc.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row_bytes),
                        rows_per_image: None,
                    },
                },
                target.size(),
            );
            let index = self.queue.submit([enc.finish()]);
            let (tx, rx) = mpsc::channel();
            readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            // While the GPU renders this tile, copy out the previous one.
            let this = InFlight {
                readback,
                rx,
                index,
                x,
                y,
                tw,
                th,
                row_bytes,
                _target: target,
            };
            if let Some(prev) = inflight.replace(this) {
                self.finish(prev, &mut out, w)?;
                progress(n as f32 / tiles.len() as f32);
            }
        }
        if let Some(last) = inflight.take() {
            self.finish(last, &mut out, w)?;
        }
        progress(1.0);
        Ok(Rendered {
            width: w,
            height: h,
            pixels: out,
        })
    }

    /// Wait for a tile's readback and convert it into `out` (`w` pixels wide, RGB u16).
    fn finish(&self, t: InFlight, out: &mut [u16], w: u32) -> Result<(), String> {
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(t.index),
                timeout: None,
            })
            .map_err(|e| e.to_string())?;
        t.rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        {
            let slice = t.readback.slice(..);
            let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
            let rows = out
                .par_chunks_mut((w * 3) as usize)
                .skip(t.y as usize)
                .take(t.th as usize);
            rows.enumerate().for_each(|(row, dst)| {
                let line = &data[row * t.row_bytes as usize..][..(t.tw * 16) as usize];
                let dst = &mut dst[(t.x * 3) as usize..((t.x + t.tw) * 3) as usize];
                for (px, o) in line
                    .as_chunks::<16>()
                    .0
                    .iter()
                    .zip(dst.as_chunks_mut::<3>().0)
                {
                    for c in 0..3 {
                        let v = f32::from_le_bytes(px[c * 4..c * 4 + 4].try_into().unwrap());
                        o[c] = (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
                    }
                }
            });
        }
        t.readback.unmap();
        Ok(())
    }

    /// The brush layers as a texture array (one empty layer when there are none).
    fn brush_texture(&self, b: &BrushLayers) -> wgpu::TextureView {
        let (w, h, layers) = if b.layers == 0 {
            (1, 1, 1)
        } else {
            (b.width, b.height, b.layers)
        };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("brush masks"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let zero = [0u8];
        let data = if b.layers == 0 {
            &zero[..]
        } else {
            &b.data[..]
        };
        self.queue.write_texture(
            tex.as_image_copy(),
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w),
                rows_per_image: Some(h),
            },
            tex.size(),
        );
        tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        })
    }

    /// The Clarity / Dehaze maps (a 1×1 placeholder without them; `params` says which).
    fn presence_texture(&self, maps: Option<&PresenceMaps>) -> wgpu::TextureView {
        let (w, h, bytes) = match maps {
            Some(m) => (m.width, m.height, m.rgba16f()),
            None => (1, 1, vec![0u8; 8]),
        };
        let tex = self.texture("presence maps", w, h);
        self.queue.write_texture(
            tex.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 8),
                rows_per_image: Some(h),
            },
            tex.size(),
        );
        tex.create_view(&Default::default())
    }

    fn texture(&self, label: &str, width: u32, height: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }
}

/// Error scopes catching out-of-memory, validation and internal errors on this thread.
struct ErrorScopes([wgpu::ErrorScopeGuard; 3]);

impl ErrorScopes {
    fn push(d: &wgpu::Device) -> Self {
        Self([
            d.push_error_scope(wgpu::ErrorFilter::OutOfMemory),
            d.push_error_scope(wgpu::ErrorFilter::Validation),
            d.push_error_scope(wgpu::ErrorFilter::Internal),
        ])
    }

    /// Pops the scopes (innermost first) and returns the first error any of them caught.
    fn pop(self) -> Result<(), wgpu::Error> {
        let mut first = None;
        for scope in self.0.into_iter().rev() {
            if let Some(e) = pollster::block_on(scope.pop()) {
                first.get_or_insert(e);
            }
        }
        first.map_or(Ok(()), Err)
    }
}

fn uniform(d: &wgpu::Device, label: &str, bytes: Vec<u8>) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: &bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

fn bytes_f32(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// A tile submitted to the GPU whose pixels haven't been read back yet.
struct InFlight {
    readback: wgpu::Buffer,
    rx: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    index: wgpu::SubmissionIndex,
    x: u32,
    y: u32,
    tw: u32,
    th: u32,
    row_bytes: u32,
    /// Kept alive until its copy has run.
    _target: wgpu::Texture,
}
