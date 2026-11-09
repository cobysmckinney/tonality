//! Headless wgpu renderer for export: runs `develop.wgsl` natively at full resolution.
//!
//! The image is rendered in tiles, so any size works within the GPU's texture limits. Each
//! tile's texture carries an apron of neighbouring pixels so Detail (sharpening, noise
//! reduction) sees the same neighbourhood as it would in one big pass: tiles join seamlessly.

use std::sync::mpsc;

use tonality_core::geometry::Geometry;

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
        Ok(Self { device, queue, pipeline, sampler, adapter: name })
    }

    /// Develop `src` with `params` and the tone-curve LUT (`curve_lut_rgba16f`).
    /// `progress` receives the fraction of tiles done.
    pub fn render(
        &self,
        src: &Source,
        params: &DevelopParams,
        curve_lut: &[u8],
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

        let mut out = vec![0u16; (w * h * 3) as usize];
        let tiles: Vec<(u32, u32)> =
            (0..h.div_ceil(TILE)).flat_map(|ty| (0..w.div_ceil(TILE)).map(move |tx| (tx * TILE, ty * TILE))).collect();
        for (n, &(x, y)) in tiles.iter().enumerate() {
            let tw = TILE.min(w - x);
            let th = TILE.min(h - y);
            // The source area this tile reads (rotation makes it larger), plus an apron for
            // Detail's neighbourhood, clamped to the image.
            let (fw, fh) = (w as f32, h as f32);
            let b = geo.source_bounds([x as f32 / fw, y as f32 / fh], [(x + tw) as f32 / fw, (y + th) as f32 / fh]);
            let px = |v: f32, size: u32| (v * size as f32).clamp(0.0, size as f32);
            let sx = (px(b[0], sw_all).floor() as u32).saturating_sub(APRON).min(sw_all - 1);
            let sy = (px(b[1], sh_all).floor() as u32).saturating_sub(APRON).min(sh_all - 1);
            let sw = ((px(b[2], sw_all).ceil() as u32 + APRON).min(sw_all) - sx).max(1);
            let sh = ((px(b[3], sh_all).ceil() as u32 + APRON).min(sh_all) - sy).max(1);

            let image = self.texture("image", sw, sh);
            let mut rgba = Vec::with_capacity((sw * sh * 4) as usize);
            for row in sy..sy + sh {
                let start = ((row * sw_all + sx) * 3) as usize;
                for p in src.pixels[start..start + (sw * 3) as usize].as_chunks::<3>().0 {
                    rgba.extend_from_slice(&[p[0], p[1], p[2], HALF_ONE]);
                }
            }
            self.queue.write_texture(
                image.as_image_copy(),
                &bytes_u16(&rgba),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(sw * 8), rows_per_image: None },
                image.size(),
            );
            drop(rgba);

            let (uw, uh) = (sw_all as f32, sh_all as f32);
            let view = [
                1.0, 1.0, 0.0, 0.0, // xform: the quad fills the tile
                x as f32 / fw, y as f32 / fh, tw as f32 / fw, th as f32 / fh, // quad (frame)
                sx as f32 / uw, sy as f32 / uh, sw as f32 / uw, sh as f32 / uh, // tex (source)
                src.frame[0] as f32, src.frame[1] as f32, 0.0, 0.0, // frame
            ];
            let view_buf = uniform(d, "view", bytes_f32(&view));
            let image_view = image.create_view(&Default::default());
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("develop"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: params_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: view_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&image_view) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&curve_view) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });

            let target = d.create_texture(&wgpu::TextureDescriptor {
                label: Some("tile"),
                size: wgpu::Extent3d { width: tw, height: th, depth_or_array_layers: 1 },
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
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
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
            self.queue.submit([enc.finish()]);

            let slice = readback.slice(..);
            let (tx, rx) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            d.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).map_err(|e| e.to_string())?;
            rx.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
            {
                let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
                for row in 0..th {
                    let line = &data[(row * row_bytes) as usize..][..(tw * 16) as usize];
                    let dst = (((y + row) * w + x) * 3) as usize;
                    for (i, px) in line.as_chunks::<16>().0.iter().enumerate() {
                        for c in 0..3 {
                            let v = f32::from_le_bytes(px[c * 4..c * 4 + 4].try_into().unwrap());
                            out[dst + i * 3 + c] = (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
                        }
                    }
                }
            }
            readback.unmap();
            progress((n + 1) as f32 / tiles.len() as f32);
        }
        Ok(Rendered { width: w, height: h, pixels: out })
    }

    fn texture(&self, label: &str, width: u32, height: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
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

fn bytes_u16(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}
