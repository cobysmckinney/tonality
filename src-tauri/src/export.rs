//! Export: develop photos at full resolution on the GPU (wgpu, same WGSL as the preview) and
//! write JPEG, PNG or 16-bit TIFF files, in sRGB or Display P3 (with the profile embedded), and
//! optionally as Ultra HDR JPEG (the SDR image plus a gain map to AgX's 1000-nit HDR).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tonality_core::agx::{Display, GAIN_LOG2, GAIN_OFFSET, HDR_RATIO};
use tonality_core::{EditState, ImageId};
use tonality_raw::FullImage;
use tonality_render::{curve_lut_rgba16f, DevelopParams, Gpu, Rendered, Source};
use ts_rs::TS;

use crate::history::CatalogState;
use crate::images::Images;
use crate::store::Store;
use crate::CmdResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExportFormat {
    Jpeg,
    Png,
    Tiff,
}

impl ExportFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Tiff => "tif",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExportColorSpace {
    #[default]
    Srgb,
    DisplayP3,
}

impl ExportColorSpace {
    fn display(self) -> Display {
        match self {
            Self::Srgb => Display::Srgb,
            Self::DisplayP3 => Display::DisplayP3,
        }
    }

    fn gamut(self) -> ultrahdr_rs::ColorGamut {
        match self {
            Self::Srgb => ultrahdr_rs::ColorGamut::Bt709,
            Self::DisplayP3 => ultrahdr_rs::ColorGamut::DisplayP3,
        }
    }

    /// The ICC profile embedded in every file (sRGB or Display P3, sRGB curve).
    fn icc(self) -> Vec<u8> {
        ultrahdr_rs::jpeg::get_icc_profile_for_gamut(self.gamut())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExportVersions {
    /// The version on screen (HEAD).
    Current,
    /// Every branch, one file each.
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportSettings {
    pub folder: String,
    pub format: ExportFormat,
    /// JPEG quality, 1..=100.
    pub quality: u8,
    /// Long edge in pixels; `None` = full resolution.
    pub long_edge: Option<u32>,
    pub versions: ExportVersions,
    #[serde(default)]
    pub color_space: ExportColorSpace,
    /// JPEG only: write Ultra HDR (a gain map to AgX HDR, 1000 nits).
    #[serde(default)]
    pub hdr: bool,
}

impl ExportSettings {
    fn ultra_hdr(&self) -> bool {
        self.hdr && self.format == ExportFormat::Jpeg
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportSummary {
    pub written: Vec<String>,
    pub failed: Vec<String>,
    pub folder: String,
}

#[derive(Debug, Clone, Serialize)]
struct Progress {
    done: f32,
    total: usize,
    name: String,
}

/// One GPU for the app's lifetime (creating a device costs a few hundred ms).
static GPU: OnceLock<Result<Mutex<Gpu>, String>> = OnceLock::new();

fn gpu() -> Result<std::sync::MutexGuard<'static, Gpu>, String> {
    match GPU.get_or_init(|| Gpu::new().map(Mutex::new)) {
        Ok(m) => m.lock().map_err(|_| "GPU lock poisoned".into()),
        Err(e) => Err(e.clone()),
    }
}

/// A version to export: file name suffix (None for `main`) and its edit.
struct Job {
    image: ImageId,
    path: PathBuf,
    /// Contents hash (keys the AI mask cache).
    hash: String,
    stem: String,
    versions: Vec<(Option<String>, EditState)>,
}

#[tauri::command]
pub async fn export_images(
    ids: Vec<ImageId>,
    settings: ExportSettings,
    app: AppHandle,
    images: State<'_, Images>,
) -> CmdResult<ExportSummary> {
    let folder = PathBuf::from(&settings.folder);
    let images = images.inner().clone();
    // Off the async runtime: gathering reads the catalog (every version of every photo), and
    // rendering is long.
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&folder).map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
        let jobs = gather(&app, &ids, &settings)?;
        run(jobs, settings, folder, images, app)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What to render, read from the catalog one photo at a time (so the UI's own catalog calls
/// aren't held up behind a big export).
fn gather(app: &AppHandle, ids: &[ImageId], settings: &ExportSettings) -> CmdResult<Vec<Job>> {
    let catalog = app.state::<CatalogState>();
    let store = app.state::<Store>();
    let mut jobs = Vec::new();
    for &id in ids {
        let (rel, hash, name) = catalog.with(|c| c.image_file(id))?;
        // `edit`, not `with`: opening a never-edited photo's history creates its first commit,
        // which its sidecar should record.
        let versions = catalog.edit(&[id], |c| {
            let co = c.history_open(id)?;
            let view = &co.view;
            let mut out = Vec::new();
            for b in &view.branches {
                let current = b.name == view.head.branch;
                if settings.versions == ExportVersions::Current && !current {
                    continue;
                }
                // The current branch exports what's on screen (HEAD may be behind the tip after undo).
                let state = if current { co.state.clone() } else { c.state_at(b.tip)? };
                // `IMG_1234.jpg` for main, `IMG_1234 — B&W.jpg` for other branches.
                out.push(((b.name != "main").then(|| b.name.clone()), state));
            }
            Ok(out)
        })?;
        let stem = Path::new(&name).file_stem().map_or(name.clone(), |s| s.to_string_lossy().into_owned());
        jobs.push(Job { image: id, path: store.abs(&rel), hash, stem, versions });
    }
    Ok(jobs)
}

/// A downscaled source: width, height, RGB halves.
type Scaled = (u32, u32, Vec<u16>);

fn run(
    jobs: Vec<Job>,
    settings: ExportSettings,
    folder: PathBuf,
    images: Images,
    app: AppHandle,
) -> CmdResult<ExportSummary> {
    let total: usize = jobs.iter().map(|j| j.versions.len()).sum();
    let mut summary = ExportSummary { written: vec![], failed: vec![], folder: folder.display().to_string() };
    let mut done = 0usize;
    let emit = |done: f32, name: &str| {
        let _ = app.emit("export-progress", Progress { done, total, name: name.to_string() });
    };

    for job in jobs {
        emit(done as f32, &job.stem);
        // Reuse the decode of the photo open in Edit; decode others just for this export.
        let full: Result<Arc<FullImage>, String> = match images.cached_full(job.image) {
            Some(f) => Ok(f),
            None => tonality_raw::decode_full(&job.path).map(Arc::new).map_err(|e| e.to_string()),
        };
        let full = match full {
            Ok(f) => f,
            Err(e) => {
                summary.failed.push(format!("{}: {e}", job.stem));
                done += job.versions.len();
                continue;
            }
        };
        // Downscaled sources, by source long edge (versions with the same crop share one).
        let mut scaled: Vec<(u32, Scaled)> = Vec::new();
        // Clarity / Dehaze maps: made once per photo, if some version uses them.
        let presence = job
            .versions
            .iter()
            .any(|(_, e)| tonality_render::presence::uses_maps(e))
            .then(|| tonality_render::presence::presence_maps(&full.pixels, full.width, full.height, &full.color));

        for (suffix, edit) in &job.versions {
            let label = match suffix {
                Some(s) => format!("{} — {s}", job.stem),
                None => job.stem.clone(),
            };
            emit(done as f32, &label);
            let result = (|| -> Result<PathBuf, String> {
                let mut params = DevelopParams::new(edit, &full.color, full.lens.as_ref(), full.width, full.height)
                    .for_display(settings.color_space.display());
                if let Some(m) = &presence {
                    params = params.with_presence(m);
                }
                // The long-edge limit applies to the framed output; scale the source to match.
                let out_long = params.geo0[3].max(params.geo1[3]);
                let full_long = full.width.max(full.height);
                let src_long = match settings.long_edge {
                    Some(edge) if (edge as f32) < out_long => ((full_long as f32 * edge as f32 / out_long).round() as u32).max(1),
                    _ => full_long,
                };
                let src = if src_long >= full_long {
                    Source { width: full.width, height: full.height, pixels: &full.pixels, frame: [full.width, full.height] }
                } else {
                    if !scaled.iter().any(|(l, _)| *l == src_long) {
                        scaled.push((src_long, full.scaled(src_long)));
                    }
                    let (_, (w, h, px)) = scaled.iter().find(|(l, _)| *l == src_long).unwrap();
                    Source { width: *w, height: *h, pixels: px, frame: [full.width, full.height] }
                };
                let store = app.state::<crate::store::Store>();
                let mut layers = crate::ai::layers(&app, &job.hash, edit, full.width, full.height, full.lens.as_ref());
                if !layers.missing.is_empty() {
                    // AI masks whose raster isn't cached (another library, a cleared cache):
                    // detect them now from this decode.
                    let (sw, sh, px) = full.scaled(2048);
                    let photo = crate::ai::Photo {
                        image: None,
                        hash: &job.hash,
                        pixels: &px,
                        width: sw,
                        height: sh,
                        color: &full.color,
                        profile: full.lens.as_ref(),
                        full_width: full.width,
                        full_height: full.height,
                    };
                    let mut ran: Vec<&str> = Vec::new();
                    for &i in &layers.missing {
                        if let tonality_core::mask::MaskShape::Ai { target, model, .. } = &edit.masks[i].shape {
                            // One scene pass caches both sky and people.
                            let scene = matches!(target, tonality_core::mask::AiTarget::Sky | tonality_core::mask::AiTarget::People);
                            if scene && ran.contains(&model.as_str()) {
                                continue;
                            }
                            ran.push(model);
                            crate::ai::detect(&app, &app.state::<crate::ai::Detector>(), &store, &photo, &edit.lens, target, model)?;
                        }
                    }
                    layers = crate::ai::layers(&app, &job.hash, edit, full.width, full.height, full.lens.as_ref());
                }
                let hdr = settings.ultra_hdr();
                // With HDR the image and its gain map share the progress.
                let share = if hdr { 0.45 } else { 0.9 };
                let curve = curve_lut_rgba16f(edit);
                let rendered = gpu()?.render(&src, &params, &curve, &layers, presence.as_ref(), |f| emit(done as f32 + f * share, &label))?;
                let gain = if hdr {
                    let p = params.gain_map();
                    Some(gpu()?.render(&src, &p, &curve, &layers, presence.as_ref(), |f| emit(done as f32 + share + f * share, &label))?)
                } else {
                    None
                };
                let target = unique_path(&folder, &sanitize(&label), settings.format.extension());
                write(&rendered, gain.as_ref(), &target, &settings)?;
                Ok(target)
            })();
            match result {
                Ok(p) => summary.written.push(p.display().to_string()),
                Err(e) => summary.failed.push(format!("{label}: {e}")),
            }
            done += 1;
            emit(done as f32, &label);
        }
    }
    Ok(summary)
}

fn eight(pixels: &[u16]) -> Vec<u8> {
    pixels.iter().map(|&v| ((v as u32 * 255 + 32767) / 65535) as u8).collect()
}

/// `r` written to `target` as `s` says; `gain`: the Ultra HDR gain map render, for HDR JPEGs.
fn write(r: &Rendered, gain: Option<&Rendered>, target: &Path, s: &ExportSettings) -> Result<(), String> {
    use image::codecs::{jpeg::JpegEncoder, png::PngEncoder, tiff::TiffEncoder};
    use image::{ExtendedColorType, ImageEncoder};

    let part = target.with_extension(format!("{}.part", s.format.extension()));
    let file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut out = std::io::BufWriter::new(file);
    let icc = s.color_space.icc();
    let quality = s.quality.clamp(1, 100);
    let res = match (s.format, gain) {
        (ExportFormat::Jpeg, Some(gain)) => ultra_hdr(r, gain, quality, s.color_space).and_then(|bytes| {
            use std::io::Write;
            out.write_all(&bytes).map_err(|e| e.to_string())
        }),
        (ExportFormat::Jpeg, None) => {
            let mut e = JpegEncoder::new_with_quality(&mut out, quality);
            e.set_icc_profile(icc).map_err(|e| e.to_string())?;
            e.write_image(&eight(&r.pixels), r.width, r.height, ExtendedColorType::Rgb8).map_err(|e| e.to_string())
        }
        (ExportFormat::Png, _) => {
            let mut e = PngEncoder::new(&mut out);
            e.set_icc_profile(icc).map_err(|e| e.to_string())?;
            e.write_image(&eight(&r.pixels), r.width, r.height, ExtendedColorType::Rgb8).map_err(|e| e.to_string())
        }
        (ExportFormat::Tiff, _) => {
            let bytes: Vec<u8> = r.pixels.iter().flat_map(|v| v.to_ne_bytes()).collect();
            let mut e = TiffEncoder::new(&mut out);
            e.set_icc_profile(icc).map_err(|e| e.to_string())?;
            e.write_image(&bytes, r.width, r.height, ExtendedColorType::Rgb16).map_err(|e| e.to_string())
        }
    };
    let res = res.and_then(|_| {
        use std::io::Write;
        out.flush().map_err(|e| e.to_string())
    });
    drop(out);
    match res {
        Ok(()) => std::fs::rename(&part, target).map_err(|e| e.to_string()),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// An Ultra HDR JPEG: `r` as the SDR base image, and the gain map render at half size (as
/// Apple's and Android's cameras store theirs), both at `quality`. The container (MPF, the
/// Ultra HDR XMP and ISO 21496-1 metadata, the ICC profile) is `ultrahdr-rs`'s.
fn ultra_hdr(r: &Rendered, gain: &Rendered, quality: u8, space: ExportColorSpace) -> Result<Vec<u8>, String> {
    use image::codecs::jpeg::JpegEncoder;
    use image::{ExtendedColorType, ImageEncoder};

    let jpeg = |px: &[u8], w: u32, h: u32| -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        JpegEncoder::new_with_quality(&mut buf, quality)
            .write_image(px, w, h, ExtendedColorType::Rgb8)
            .map_err(|e| e.to_string())?;
        Ok(buf)
    };
    let base = jpeg(&eight(&r.pixels), r.width, r.height)?;
    let (gw, gh, gpx) = halve(gain);
    let map = jpeg(&eight(&gpx), gw, gh)?;
    let mut metadata = ultrahdr_rs::GainMapMetadata::default();
    metadata.gain_map_min = [GAIN_LOG2[0]; 3];
    metadata.gain_map_max = [GAIN_LOG2[1]; 3];
    metadata.gamma = [1.0; 3];
    metadata.base_offset = [GAIN_OFFSET; 3];
    metadata.alternate_offset = [GAIN_OFFSET; 3];
    metadata.base_hdr_headroom = 0.0;
    metadata.alternate_hdr_headroom = HDR_RATIO.log2();
    metadata.use_base_color_space = true;
    metadata.backward_direction = false;
    ultrahdr_rs::encode_ultrahdr(&base, &map, &metadata, space.gamut()).map_err(|e| e.to_string())
}

/// A render at half size (2×2 means; the last row or column alone when odd).
fn halve(r: &Rendered) -> (u32, u32, Vec<u16>) {
    let (w, h) = (r.width.div_ceil(2), r.height.div_ceil(2));
    let mut out = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                let (mut sum, mut n) = (0u32, 0u32);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = (2 * x + dx, 2 * y + dy);
                    if sx < r.width && sy < r.height {
                        sum += r.pixels[((sy * r.width + sx) * 3 + c) as usize] as u32;
                        n += 1;
                    }
                }
                out.push(((sum + n / 2) / n) as u16);
            }
        }
    }
    (w, h, out)
}

/// File-name-safe version of a label (branch names may contain `/` and friends).
fn sanitize(s: &str) -> String {
    let cleaned: String =
        s.chars().map(|c| if c.is_control() || r#"/\:*?"<>|"#.contains(c) { '-' } else { c }).collect();
    cleaned.trim().trim_matches('.').to_string()
}

/// `folder/name.ext`, or `name (2).ext` etc. if taken — exports never overwrite.
fn unique_path(folder: &Path, name: &str, ext: &str) -> PathBuf {
    let mut p = folder.join(format!("{name}.{ext}"));
    let mut n = 2;
    while p.exists() {
        p = folder.join(format!("{name} ({n}).{ext}"));
        n += 1;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_every_format() {
        let dir = std::env::temp_dir().join(format!("tonality-export-fmt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let r = Rendered { width: 4, height: 3, pixels: (0..36).map(|i| i * 1800).collect() };
        for space in [ExportColorSpace::Srgb, ExportColorSpace::DisplayP3] {
            for format in [ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::Tiff] {
                let s = settings(&dir, format, space, false);
                let p = dir.join(format!("t.{}", format.extension()));
                write(&r, None, &p, &s).unwrap();
                let img = image::open(&p).unwrap();
                assert_eq!((img.width(), img.height()), (4, 3));
                if format == ExportFormat::Tiff {
                    assert_eq!(img.color(), image::ColorType::Rgb16);
                }
                // The profile goes in, and says which space.
                let icc = icc_of(&p, format);
                assert_eq!(icc, space.icc(), "{format:?} {space:?}");
                std::fs::remove_file(&p).unwrap();
            }
        }
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().path().to_string_lossy().ends_with(".part")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn settings(dir: &Path, format: ExportFormat, color_space: ExportColorSpace, hdr: bool) -> ExportSettings {
        ExportSettings {
            folder: dir.display().to_string(),
            format,
            quality: 90,
            long_edge: None,
            versions: ExportVersions::Current,
            color_space,
            hdr,
        }
    }

    fn icc_of(p: &Path, format: ExportFormat) -> Vec<u8> {
        use image::ImageDecoder;
        let f = std::io::BufReader::new(std::fs::File::open(p).unwrap());
        let icc = match format {
            ExportFormat::Jpeg => image::codecs::jpeg::JpegDecoder::new(f).unwrap().icc_profile(),
            ExportFormat::Png => image::codecs::png::PngDecoder::new(f).unwrap().icc_profile(),
            ExportFormat::Tiff => image::codecs::tiff::TiffDecoder::new(f).unwrap().icc_profile(),
        };
        icc.unwrap().expect("an embedded profile")
    }

    #[test]
    fn writes_ultra_hdr() {
        let dir = std::env::temp_dir().join(format!("tonality-export-uhdr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (9u32, 6u32);
        let r = Rendered { width: w, height: h, pixels: (0..w * h * 3).map(|i| (i * 1200) as u16).collect() };
        // A gain map: 0 at the left, full at the right.
        let gain = Rendered { width: w, height: h, pixels: (0..w * h * 3).map(|i| ((i / 3 % w) * 65535 / (w - 1)) as u16).collect() };
        for space in [ExportColorSpace::Srgb, ExportColorSpace::DisplayP3] {
            let p = dir.join("hdr.jpg");
            write(&r, Some(&gain), &p, &settings(&dir, ExportFormat::Jpeg, space, true)).unwrap();
            let bytes = std::fs::read(&p).unwrap();
            // Still an ordinary JPEG of the SDR image, with the space's profile.
            let img = image::load_from_memory(&bytes).unwrap();
            assert_eq!((img.width(), img.height()), (w, h));
            assert_eq!(icc_of(&p, ExportFormat::Jpeg), space.icc());
            // And an Ultra HDR one: the gain map at half size, the metadata as written.
            let d = ultrahdr_rs::Decoder::new(&bytes).unwrap();
            assert!(d.is_ultrahdr());
            let m = d.metadata().unwrap();
            assert!((m.gain_map_min[0] - GAIN_LOG2[0]).abs() < 1e-3 && (m.gain_map_max[2] - GAIN_LOG2[1]).abs() < 1e-3, "{m:?}");
            assert!((m.alternate_hdr_headroom - HDR_RATIO.log2()).abs() < 1e-3, "{m:?}");
            let map = image::load_from_memory(d.gainmap_jpeg().unwrap()).unwrap().to_rgb8();
            assert_eq!(map.dimensions(), (w.div_ceil(2), h.div_ceil(2)));
            let (left, right) = (map.get_pixel(0, 1)[0], map.get_pixel(map.width() - 1, 1)[0]);
            assert!(left < 40 && right > 215, "{left} {right}");
            std::fs::remove_file(&p).unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A real photo through the export path (decode, render, write), for looking at:
    /// `TONALITY_EXPORT_RAW=photo.ARW TONALITY_EXPORT_OUT=dir cargo test -p tonality --lib
    /// export_real_photo -- --ignored` writes sRGB and Display P3 Ultra HDR JPEGs and a P3 PNG.
    #[test]
    #[ignore]
    fn export_real_photo() {
        let (Ok(raw), Ok(out)) = (std::env::var("TONALITY_EXPORT_RAW"), std::env::var("TONALITY_EXPORT_OUT")) else {
            return;
        };
        let out = PathBuf::from(out);
        let full = tonality_raw::decode_full(raw.as_ref()).unwrap();
        let edit = EditState::default();
        let maps = tonality_render::presence::presence_maps(&full.pixels, full.width, full.height, &full.color);
        let (w, h, px) = full.scaled(2400);
        let src = Source { width: w, height: h, pixels: &px, frame: [full.width, full.height] };
        let layers = tonality_core::mask::brush_layers(&[], full.width, full.height);
        let gpu = Gpu::new().unwrap();
        for (space, format, hdr, name) in [
            (ExportColorSpace::Srgb, ExportFormat::Jpeg, true, "srgb-hdr.jpg"),
            (ExportColorSpace::DisplayP3, ExportFormat::Jpeg, true, "p3-hdr.jpg"),
            (ExportColorSpace::DisplayP3, ExportFormat::Png, false, "p3.png"),
        ] {
            let params = DevelopParams::new(&edit, &full.color, full.lens.as_ref(), full.width, full.height)
                .for_display(space.display())
                .with_presence(&maps);
            let curve = curve_lut_rgba16f(&edit);
            let r = gpu.render(&src, &params, &curve, &layers, Some(&maps), |_| {}).unwrap();
            let g = hdr.then(|| gpu.render(&src, &params.gain_map(), &curve, &layers, Some(&maps), |_| {}).unwrap());
            write(&r, g.as_ref(), &out.join(name), &settings(&out, format, space, hdr)).unwrap();
        }
    }

    #[test]
    fn ultra_hdr_is_jpeg_only() {
        let dir = Path::new("/");
        assert!(settings(dir, ExportFormat::Jpeg, ExportColorSpace::Srgb, true).ultra_hdr());
        assert!(!settings(dir, ExportFormat::Png, ExportColorSpace::Srgb, true).ultra_hdr());
        // Settings saved before HDR existed still load.
        let old = r#"{"folder":"/x","format":"jpeg","quality":90,"longEdge":null,"versions":"current"}"#;
        let s: ExportSettings = serde_json::from_str(old).unwrap();
        assert_eq!((s.color_space, s.hdr), (ExportColorSpace::Srgb, false));
    }

    #[test]
    fn names_are_safe_and_unique() {
        assert_eq!(sanitize("DSC0001 — B&W/v2"), "DSC0001 — B&W-v2");
        let dir = std::env::temp_dir().join(format!("tonality-export-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = unique_path(&dir, "x", "jpg");
        std::fs::write(&a, b"").unwrap();
        let b = unique_path(&dir, "x", "jpg");
        assert_eq!(b.file_name().unwrap(), "x (2).jpg");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
