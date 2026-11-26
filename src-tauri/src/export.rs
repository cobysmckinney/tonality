//! Export: develop photos at full resolution on the GPU (wgpu, same WGSL as the preview) and
//! write JPEG, PNG or 16-bit TIFF files, in sRGB or Display P3 (with the profile embedded), and
//! optionally as Ultra HDR JPEG (the SDR image plus a gain map to AgX's 1000-nit HDR).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tonality_core::agx::{Display, GAIN_LOG2, GAIN_OFFSET, HDR_RATIO};
use tonality_core::exif::{Capture, Export};
use tonality_core::{EditState, ImageId};
use tonality_raw::FullImage;
use tonality_render::{curve_lut_rgba16f, DevelopParams, Gpu, Rendered, Source};
use ts_rs::TS;

use crate::cancel::Cancel;
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

    /// The ICC profile embedded in every file (sRGB or Display P3, sRGB curve). Made once:
    /// moxcms stamps the time of making in the header, so every file gets the same bytes.
    fn icc(self) -> Vec<u8> {
        static SRGB: OnceLock<Vec<u8>> = OnceLock::new();
        static P3: OnceLock<Vec<u8>> = OnceLock::new();
        let cell = match self {
            Self::Srgb => &SRGB,
            Self::DisplayP3 => &P3,
        };
        cell.get_or_init(|| ultrahdr_rs::jpeg::get_icc_profile_for_gamut(self.gamut()))
            .clone()
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
    /// Write the GPS position the camera recorded.
    #[serde(default)]
    pub include_location: bool,
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
    /// Stopped by the user before every file was written.
    pub cancelled: bool,
}

/// Raised by [`export_cancel`].
static CANCEL: Cancel = Cancel::new();

/// Stop the export that's running, once the file it is on is written. Files already written
/// stay.
#[tauri::command]
pub fn export_cancel() {
    CANCEL.request();
}

#[derive(Debug, Clone, Serialize)]
struct Progress {
    done: f32,
    total: usize,
    name: String,
}

/// One GPU, kept between exports (creating a device costs a few hundred ms). Rebuilt when the
/// last attempt to make it failed, the device was lost, or a render panicked holding the lock.
static GPU: Mutex<Option<Gpu>> = Mutex::new(None);

struct GpuGuard(MutexGuard<'static, Option<Gpu>>);

impl std::ops::Deref for GpuGuard {
    type Target = Gpu;
    fn deref(&self) -> &Gpu {
        self.0.as_ref().expect("gpu() fills the slot")
    }
}

fn gpu() -> Result<GpuGuard, String> {
    let mut slot = GPU.lock().unwrap_or_else(|poisoned| {
        // A panic mid-render leaves the device in an unknown state: start again.
        GPU.clear_poison();
        let mut slot = poisoned.into_inner();
        *slot = None;
        slot
    });
    if slot.as_ref().is_some_and(Gpu::is_lost) {
        *slot = None;
    }
    if slot.is_none() {
        *slot = Some(Gpu::new()?);
    }
    Ok(GpuGuard(slot))
}

/// A version to export: file name suffix (None for `main`) and its edit.
struct Job {
    image: ImageId,
    path: PathBuf,
    /// Contents hash (keys the AI mask cache).
    hash: String,
    stem: String,
    versions: Vec<(Option<String>, EditState)>,
    /// tonality's star rating, 0–5.
    rating: u8,
}

/// What an exported file says about itself besides pixels.
struct Meta {
    capture: Capture,
    rating: u8,
}

impl Meta {
    fn export(&self, r: &Rendered, s: &ExportSettings) -> Export {
        Export {
            width: r.width,
            height: r.height,
            srgb: s.color_space == ExportColorSpace::Srgb,
            rating: (self.rating > 0).then_some(self.rating),
            include_location: s.include_location,
            exported_at: chrono::Local::now().format("%Y:%m:%d %H:%M:%S").to_string(),
        }
    }
}

#[tauri::command]
pub async fn export_images(
    ids: Vec<ImageId>,
    settings: ExportSettings,
    app: AppHandle,
    images: State<'_, Images>,
) -> CmdResult<ExportSummary> {
    let folder = export_folder(&settings.folder)?;
    CANCEL.clear();
    let images = images.inner().clone();
    // Off the async runtime: gathering reads the catalog (every version of every photo), and
    // rendering is long.
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&folder)
            .map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
        let jobs = gather(&app, &ids, &settings)?;
        run(jobs, settings, folder, images, app, &CANCEL)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The folder to export into. It must be absolute: an empty or relative path would resolve
/// against the app's working directory and scatter files there.
fn export_folder(folder: &str) -> CmdResult<PathBuf> {
    if folder.trim().is_empty() {
        return Err("Choose a folder to export to".into());
    }
    let path = PathBuf::from(folder);
    if !path.is_absolute() {
        return Err(format!("Export folder must be a full path: {folder}"));
    }
    Ok(path)
}

/// What to render, read from the catalog one photo at a time (so the UI's own catalog calls
/// aren't held up behind a big export).
fn gather(app: &AppHandle, ids: &[ImageId], settings: &ExportSettings) -> CmdResult<Vec<Job>> {
    let catalog = app.state::<CatalogState>();
    let store = app.state::<Store>();
    let mut jobs = Vec::new();
    for &id in ids {
        let (rel, hash, name) = catalog.with(|c| c.image_file(id))?;
        let rating = catalog.with(|c| c.image_rating(id))?;
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
                let state = if current {
                    co.state.clone()
                } else {
                    c.state_at(b.tip)?
                };
                // `IMG_1234.jpg` for main, `IMG_1234 — B&W.jpg` for other branches.
                out.push(((b.name != "main").then(|| b.name.clone()), state));
            }
            Ok(out)
        })?;
        let stem = Path::new(&name)
            .file_stem()
            .map_or(name.clone(), |s| s.to_string_lossy().into_owned());
        jobs.push(Job {
            image: id,
            path: store.abs(&rel),
            hash,
            stem,
            versions,
            rating,
        });
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
    cancel: &Cancel,
) -> CmdResult<ExportSummary> {
    let total: usize = jobs.iter().map(|j| j.versions.len()).sum();
    let mut summary = ExportSummary {
        written: vec![],
        failed: vec![],
        folder: folder.display().to_string(),
        cancelled: false,
    };
    let mut done = 0usize;
    let emit = |done: f32, name: &str| {
        let _ = app.emit(
            "export-progress",
            Progress {
                done,
                total,
                name: name.to_string(),
            },
        );
    };

    // Checked between files only, so one being written is always finished.
    'jobs: for job in jobs {
        if cancel.requested() {
            summary.cancelled = true;
            break;
        }
        emit(done as f32, &job.stem);
        // Reuse the decode of the photo open in Edit; decode others just for this export.
        let full: Result<Arc<FullImage>, String> = match images.cached_full(job.image) {
            Some(f) => Ok(f),
            None => tonality_raw::decode_full(&job.path)
                .map(Arc::new)
                .map_err(|e| e.to_string()),
        };
        let full = match full {
            Ok(f) => f,
            Err(e) => {
                summary.failed.push(format!("{}: {e}", job.stem));
                done += job.versions.len();
                continue;
            }
        };
        // What the camera recorded (no pixel decode); a file it can't be read from exports
        // without it.
        let meta = Meta {
            capture: tonality_raw::read_capture(&job.path).unwrap_or_default(),
            rating: job.rating,
        };
        // Downscaled sources, by source long edge (versions with the same crop share one).
        let mut scaled: Vec<(u32, Scaled)> = Vec::new();
        // Clarity / Dehaze maps: made once per photo, if some version uses them.
        let presence = job
            .versions
            .iter()
            .any(|(_, e)| tonality_render::presence::uses_maps(e))
            .then(|| {
                tonality_render::presence::presence_maps(
                    &full.pixels,
                    full.width,
                    full.height,
                    &full.color,
                )
            });

        for (suffix, edit) in &job.versions {
            if cancel.requested() {
                summary.cancelled = true;
                break 'jobs;
            }
            let label = match suffix {
                Some(s) => format!("{} — {s}", job.stem),
                None => job.stem.clone(),
            };
            emit(done as f32, &label);
            let result = (|| -> Result<PathBuf, String> {
                let mut params = DevelopParams::new(
                    edit,
                    &full.color,
                    full.lens.as_ref(),
                    full.width,
                    full.height,
                )
                .for_display(settings.color_space.display());
                if let Some(m) = &presence {
                    params = params.with_presence(m);
                }
                // The long-edge limit applies to the framed output; scale the source to match.
                let out_long = params.geo0[3].max(params.geo1[3]);
                let full_long = full.width.max(full.height);
                let src_long = match settings.long_edge {
                    Some(edge) if (edge as f32) < out_long => {
                        ((full_long as f32 * edge as f32 / out_long).round() as u32).max(1)
                    }
                    _ => full_long,
                };
                let src = if src_long >= full_long {
                    Source {
                        width: full.width,
                        height: full.height,
                        pixels: &full.pixels,
                        frame: [full.width, full.height],
                    }
                } else {
                    if !scaled.iter().any(|(l, _)| *l == src_long) {
                        scaled.push((src_long, full.scaled(src_long)));
                    }
                    let (_, (w, h, px)) = scaled.iter().find(|(l, _)| *l == src_long).unwrap();
                    Source {
                        width: *w,
                        height: *h,
                        pixels: px,
                        frame: [full.width, full.height],
                    }
                };
                let store = app.state::<crate::store::Store>();
                let mut layers = crate::ai::layers(
                    &app,
                    &job.hash,
                    edit,
                    full.width,
                    full.height,
                    full.lens.as_ref(),
                );
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
                        if let tonality_core::mask::MaskShape::Ai { target, model, .. } =
                            &edit.masks[i].shape
                        {
                            // One scene pass caches both sky and people.
                            let scene = matches!(
                                target,
                                tonality_core::mask::AiTarget::Sky
                                    | tonality_core::mask::AiTarget::People
                            );
                            if scene && ran.contains(&model.as_str()) {
                                continue;
                            }
                            ran.push(model);
                            crate::ai::detect(
                                &app,
                                &app.state::<crate::ai::Detector>(),
                                &store,
                                &photo,
                                &edit.lens,
                                target,
                                model,
                            )?;
                        }
                    }
                    layers = crate::ai::layers(
                        &app,
                        &job.hash,
                        edit,
                        full.width,
                        full.height,
                        full.lens.as_ref(),
                    );
                }
                let hdr = settings.ultra_hdr();
                // With HDR the image and its gain map share the progress.
                let share = if hdr { 0.45 } else { 0.9 };
                let curve = curve_lut_rgba16f(edit);
                let rendered =
                    gpu()?.render(&src, &params, &curve, &layers, presence.as_ref(), |f| {
                        emit(done as f32 + f * share, &label)
                    })?;
                let gain = if hdr {
                    let p = params.gain_map();
                    Some(
                        gpu()?.render(&src, &p, &curve, &layers, presence.as_ref(), |f| {
                            emit(done as f32 + share + f * share, &label)
                        })?,
                    )
                } else {
                    None
                };
                let target = folder.join(format!(
                    "{}.{}",
                    sanitize(&label),
                    settings.format.extension()
                ));
                write(&rendered, gain.as_ref(), &target, &settings, &meta)
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
    pixels
        .iter()
        .map(|&v| ((v as u32 * 255 + 32767) / 65535) as u8)
        .collect()
}

/// `r` written to `target` as `s` says, with `meta`'s EXIF (and XMP rating); `gain`: the Ultra
/// HDR gain map render, for HDR JPEGs. If `target` is taken, `name (2).ext` etc.: exports never
/// overwrite. Returns the path written.
fn write(
    r: &Rendered,
    gain: Option<&Rendered>,
    target: &Path,
    s: &ExportSettings,
    meta: &Meta,
) -> Result<PathBuf, String> {
    use image::codecs::{jpeg::JpegEncoder, png::PngEncoder};
    use image::{ExtendedColorType, ImageEncoder};
    use std::io::Write;

    let export = meta.export(r, s);
    let exif = tonality_core::exif::tiff_exif(&meta.capture, &export);
    let xmp = export.rating.map(tonality_core::exif::xmp_rating);
    let icc = s.color_space.icc();
    let quality = s.quality.clamp(1, 100);
    let bytes: Result<Vec<u8>, String> = match (s.format, gain) {
        (ExportFormat::Jpeg, Some(gain)) => {
            ultra_hdr(r, gain, quality, s.color_space).and_then(|f| {
                crate::jpeg::ultra_hdr_with(
                    &f,
                    Some(crate::jpeg::exif_segment(&exif)),
                    export.rating,
                )
            })
        }
        (ExportFormat::Jpeg, None) => {
            let mut buf = Vec::new();
            let mut e = JpegEncoder::new_with_quality(&mut buf, quality);
            e.set_icc_profile(icc).map_err(|e| e.to_string())?;
            e.write_image(
                &eight(&r.pixels),
                r.width,
                r.height,
                ExtendedColorType::Rgb8,
            )
            .map_err(|e| e.to_string())?;
            let mut front = vec![crate::jpeg::exif_segment(&exif)];
            front.extend(xmp.as_deref().map(crate::jpeg::xmp_segment));
            crate::jpeg::with_front(&buf, &front)
        }
        (ExportFormat::Png, _) => {
            let mut buf = Vec::new();
            let mut e = PngEncoder::new(&mut buf);
            e.set_icc_profile(icc).map_err(|e| e.to_string())?;
            e.set_exif_metadata(exif).map_err(|e| e.to_string())?;
            e.write_image(
                &eight(&r.pixels),
                r.width,
                r.height,
                ExtendedColorType::Rgb8,
            )
            .map_err(|e| e.to_string())?;
            Ok(buf)
        }
        (ExportFormat::Tiff, _) => {
            let ifds = tonality_core::exif::ifds(&meta.capture, &export);
            let mut buf = std::io::Cursor::new(Vec::new());
            tiff(&mut buf, r, &icc, &ifds, xmp.as_deref()).map(|_| buf.into_inner())
        }
    };
    let part = crate::store::temp_path(target);
    let res = bytes.and_then(|b| {
        std::fs::File::create_new(&part)
            .and_then(|mut f| f.write_all(&b))
            .map_err(|e| e.to_string())
    });
    let res = res.and_then(|()| {
        for n in 1..10_000 {
            let p = numbered(target, n);
            match crate::store::rename_noreplace(&part, &p) {
                Ok(()) => return Ok(p),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("no free file name".into())
    });
    if res.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    res
}

/// A 16-bit RGB TIFF with the ICC profile, the EXIF tags (IFD0 in the image's directory, the
/// Exif and GPS directories beside it) and the XMP rating.
fn tiff(
    out: &mut std::io::Cursor<Vec<u8>>,
    r: &Rendered,
    icc: &[u8],
    ifds: &tonality_core::exif::Ifds,
    xmp: Option<&str>,
) -> Result<(), String> {
    use tiff::encoder::{colortype::RGB16, DirectoryEncoder, TiffEncoder, TiffKindStandard};
    use tiff::tags::{Tag, Type};
    use tonality_core::exif::{Field, EXIF_POINTER, GPS_POINTER};
    type Dir<'a, 'b> = DirectoryEncoder<'a, &'b mut std::io::Cursor<Vec<u8>>, TiffKindStandard>;

    // The EXIF fields' bytes are little-endian, as the tiff crate writes on this machine.
    const _: () = assert!(cfg!(target_endian = "little"));
    let e = |e: tiff::TiffError| e.to_string();
    let add = |d: &mut Dir, fields: &[Field]| -> Result<(), String> {
        let mut dir = tiff::Directory::empty();
        for f in fields {
            let ty = Type::from_u16(f.ty).ok_or("bad EXIF type")?;
            dir.extend([(
                Tag::from_u16_exhaustive(f.tag),
                d.write_entry_bytes(ty, &f.bytes).map_err(e)?,
            )]);
        }
        d.extend_from(&dir);
        Ok(())
    };
    let mut enc = TiffEncoder::new(out).map_err(e)?;
    let exif_at = {
        let mut d = enc.extra_directory().map_err(e)?;
        add(&mut d, &ifds.exif)?;
        d.finish_with_offsets().map_err(e)?.offset
    };
    let gps_at = match &ifds.gps {
        Some(gps) => {
            let mut d = enc.extra_directory().map_err(e)?;
            add(&mut d, gps)?;
            Some(d.finish_with_offsets().map_err(e)?.offset)
        }
        None => None,
    };
    let mut img = enc.new_image::<RGB16>(r.width, r.height).map_err(e)?;
    let d = img.encoder();
    add(d, &ifds.ifd0)?;
    let mut extra = tiff::Directory::empty();
    extra.extend([(
        Tag::IccProfile,
        d.write_entry_bytes(Type::UNDEFINED, icc).map_err(e)?,
    )]);
    if let Some(x) = xmp {
        extra.extend([(
            Tag::from_u16_exhaustive(700),
            d.write_entry_bytes(Type::BYTE, x.as_bytes()).map_err(e)?,
        )]);
    }
    d.extend_from(&extra);
    d.write_tag(Tag::from_u16_exhaustive(EXIF_POINTER), exif_at)
        .map_err(e)?;
    if let Some(at) = gps_at {
        d.write_tag(Tag::from_u16_exhaustive(GPS_POINTER), at)
            .map_err(e)?;
    }
    img.write_data(&r.pixels).map_err(e)
}

/// An Ultra HDR JPEG: `r` as the SDR base image, and the gain map render at half size (as
/// Apple's and Android's cameras store theirs), both at `quality`. The container (MPF, the
/// Ultra HDR XMP and ISO 21496-1 metadata, the ICC profile) is `ultrahdr-rs`'s.
fn ultra_hdr(
    r: &Rendered,
    gain: &Rendered,
    quality: u8,
    space: ExportColorSpace,
) -> Result<Vec<u8>, String> {
    use image::codecs::jpeg::JpegEncoder;
    use image::{ExtendedColorType, ImageEncoder};

    let jpeg = |px: &[u8], w: u32, h: u32| -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        JpegEncoder::new_with_quality(&mut buf, quality)
            .write_image(px, w, h, ExtendedColorType::Rgb8)
            .map_err(|e| e.to_string())?;
        crate::jpeg::with_front(&buf, &[])
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
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_control() || r#"/\:*?"<>|"#.contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    cleaned.trim().trim_matches('.').to_string()
}

/// `target` itself for `n` = 1, else `name (n).ext` beside it.
fn numbered(target: &Path, n: u32) -> PathBuf {
    if n == 1 {
        return target.to_path_buf();
    }
    let stem = target.file_stem().unwrap_or_default().to_string_lossy();
    let name = match target.extension() {
        Some(ext) => format!("{stem} ({n}).{}", ext.to_string_lossy()),
        None => format!("{stem} ({n})"),
    };
    target.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use exif::{In, Tag};
    use tonality_core::exif::Gps;

    #[test]
    fn export_folder_must_be_absolute() {
        assert!(export_folder("").is_err());
        assert!(export_folder("   ").is_err());
        assert!(export_folder("exports").is_err());
        assert!(export_folder("./exports").is_err());
        let abs = std::env::temp_dir().join("tonality Exports");
        assert_eq!(export_folder(abs.to_str().unwrap()).unwrap(), abs);
    }

    fn meta(rating: u8) -> Meta {
        Meta {
            capture: Capture {
                make: Some("SONY".into()),
                model: Some("ILCE-7M3".into()),
                lens_model: Some("FE 35mm F1.8".into()),
                exposure_time: Some([1, 250]),
                f_number: Some([18, 10]),
                iso: Some(800),
                focal_length: Some([35, 1]),
                date_time_original: Some("2026:11:02 14:03:11".into()),
                artist: Some("Coby".into()),
                copyright: Some("(c) 2026".into()),
                gps: Some(Gps {
                    latitude_ref: Some("N".into()),
                    latitude: Some([[37, 1], [46, 1], [30, 1]]),
                    longitude_ref: Some("W".into()),
                    longitude: Some([[122, 1], [25, 1], [9, 1]]),
                    ..Gps::default()
                }),
                ..Capture::default()
            },
            rating,
        }
    }

    fn settings(
        dir: &Path,
        format: ExportFormat,
        color_space: ExportColorSpace,
        hdr: bool,
    ) -> ExportSettings {
        ExportSettings {
            folder: dir.display().to_string(),
            format,
            quality: 90,
            long_edge: None,
            versions: ExportVersions::Current,
            color_space,
            hdr,
            include_location: false,
        }
    }

    fn icc_of(p: &Path, format: ExportFormat) -> Vec<u8> {
        use image::ImageDecoder;
        let f = std::io::BufReader::new(std::fs::File::open(p).unwrap());
        let icc = match format {
            ExportFormat::Jpeg => image::codecs::jpeg::JpegDecoder::new(f)
                .unwrap()
                .icc_profile(),
            ExportFormat::Png => image::codecs::png::PngDecoder::new(f)
                .unwrap()
                .icc_profile(),
            ExportFormat::Tiff => image::codecs::tiff::TiffDecoder::new(f)
                .unwrap()
                .icc_profile(),
        };
        icc.unwrap().expect("an embedded profile")
    }

    /// The file's EXIF, read by an independent reader (kamadak-exif).
    fn exif_of(p: &Path) -> exif::Exif {
        let mut f = std::io::BufReader::new(std::fs::File::open(p).unwrap());
        exif::Reader::new()
            .read_from_container(&mut f)
            .expect("EXIF")
    }

    fn text(x: &exif::Exif, tag: Tag) -> Option<String> {
        x.get_field(tag, In::PRIMARY)
            .map(|f| f.display_value().to_string().trim_matches('"').to_string())
    }

    /// What every export should say, whatever the format.
    fn check_exif(p: &Path, location: bool) {
        let x = exif_of(p);
        assert_eq!(text(&x, Tag::Make).as_deref(), Some("SONY"), "{p:?}");
        assert_eq!(text(&x, Tag::Model).as_deref(), Some("ILCE-7M3"));
        assert_eq!(text(&x, Tag::LensModel).as_deref(), Some("FE 35mm F1.8"));
        assert_eq!(
            text(&x, Tag::DateTimeOriginal).as_deref(),
            Some("2026-11-02 14:03:11")
        );
        assert_eq!(text(&x, Tag::Artist).as_deref(), Some("Coby"));
        assert_eq!(
            x.get_field(Tag::PhotographicSensitivity, In::PRIMARY)
                .unwrap()
                .value
                .get_uint(0),
            Some(800)
        );
        assert_eq!(
            x.get_field(Tag::Orientation, In::PRIMARY)
                .unwrap()
                .value
                .get_uint(0),
            Some(1)
        );
        let rating = x
            .fields()
            .find(|f| f.tag.number() == 0x4746)
            .and_then(|f| f.value.get_uint(0));
        assert_eq!(rating, Some(4), "{p:?}");
        assert_eq!(
            x.get_field(Tag::GPSLatitude, In::PRIMARY).is_some(),
            location,
            "{p:?}"
        );
    }

    #[test]
    fn writes_every_format() {
        let dir = std::env::temp_dir().join(format!("tonality-export-fmt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let r = Rendered {
            width: 4,
            height: 3,
            pixels: (0..36).map(|i| i * 1800).collect(),
        };
        for space in [ExportColorSpace::Srgb, ExportColorSpace::DisplayP3] {
            for format in [ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::Tiff] {
                for location in [false, true] {
                    let s = ExportSettings {
                        include_location: location,
                        ..settings(&dir, format, space, false)
                    };
                    let p = dir.join(format!("t.{}", format.extension()));
                    write(&r, None, &p, &s, &meta(4)).unwrap();
                    let img = image::open(&p).unwrap();
                    assert_eq!((img.width(), img.height()), (4, 3));
                    if format == ExportFormat::Tiff {
                        assert_eq!(img.color(), image::ColorType::Rgb16);
                    }
                    // The profile goes in, and says which space.
                    assert_eq!(icc_of(&p, format), space.icc(), "{format:?} {space:?}");
                    check_exif(&p, location);
                    // The rating in XMP too, where Lightroom reads it (PNG: EXIF only).
                    let bytes = std::fs::read(&p).unwrap();
                    let xmp = bytes.windows(14).any(|w| w == br#"xmp:Rating="4""#);
                    assert_eq!(xmp, format != ExportFormat::Png, "{format:?}");
                    std::fs::remove_file(&p).unwrap();
                }
            }
        }
        // Unrated: no rating anywhere.
        let p = dir.join("u.jpg");
        write(
            &r,
            None,
            &p,
            &settings(&dir, ExportFormat::Jpeg, ExportColorSpace::Srgb, false),
            &meta(0),
        )
        .unwrap();
        assert!(exif_of(&p).fields().all(|f| f.tag.number() != 0x4746));
        assert!(!std::fs::read(&p)
            .unwrap()
            .windows(10)
            .any(|w| w == b"xmp:Rating"));
        std::fs::remove_file(&p).unwrap();
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e
            .unwrap()
            .path()
            .to_string_lossy()
            .ends_with(".part")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn writes_ultra_hdr() {
        let dir = std::env::temp_dir().join(format!("tonality-export-uhdr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (9u32, 6u32);
        let r = Rendered {
            width: w,
            height: h,
            pixels: (0..w * h * 3).map(|i| (i * 1200) as u16).collect(),
        };
        // A gain map: 0 at the left, full at the right.
        let gain = Rendered {
            width: w,
            height: h,
            pixels: (0..w * h * 3)
                .map(|i| ((i / 3 % w) * 65535 / (w - 1)) as u16)
                .collect(),
        };
        for space in [ExportColorSpace::Srgb, ExportColorSpace::DisplayP3] {
            let p = dir.join("hdr.jpg");
            let s = ExportSettings {
                include_location: true,
                ..settings(&dir, ExportFormat::Jpeg, space, true)
            };
            write(&r, Some(&gain), &p, &s, &meta(4)).unwrap();
            let bytes = std::fs::read(&p).unwrap();
            // Still an ordinary JPEG of the SDR image, with the space's profile and the EXIF.
            let img = image::load_from_memory(&bytes).unwrap();
            assert_eq!((img.width(), img.height()), (w, h));
            assert_eq!(icc_of(&p, ExportFormat::Jpeg), space.icc());
            check_exif(&p, true);
            // EXIF first after SOI (as cameras write it), no JFIF in either image.
            let (segs, _) = crate::jpeg::split_head(&bytes).unwrap();
            assert!(segs[0].marker == 0xE1 && segs[0].payload.starts_with(b"Exif\0\0"));
            assert!(
                segs.iter().all(|s| !s.payload.starts_with(b"JFIF\0")),
                "JFIF in the primary"
            );
            // The MPF index follows the move: the primary size it records, and the gain map
            // where it says.
            let (primary, gain_at) = crate::jpeg::mpf_targets(&bytes);
            assert_eq!(primary, gain_at);
            assert_eq!(&bytes[gain_at..gain_at + 2], &[0xFF, 0xD8]);
            // The rating joined the primary's XMP; the Ultra HDR metadata is intact.
            assert!(bytes[..gain_at]
                .windows(14)
                .any(|w| w == br#"xmp:Rating="4""#));
            let d = ultrahdr_rs::Decoder::new(&bytes).unwrap();
            assert!(d.is_ultrahdr());
            let m = d.metadata().unwrap();
            assert!(
                (m.gain_map_min[0] - GAIN_LOG2[0]).abs() < 1e-3
                    && (m.gain_map_max[2] - GAIN_LOG2[1]).abs() < 1e-3,
                "{m:?}"
            );
            assert!(
                (m.alternate_hdr_headroom - HDR_RATIO.log2()).abs() < 1e-3,
                "{m:?}"
            );
            let map_jpeg = d.gainmap_jpeg().unwrap();
            assert_eq!(map_jpeg, &bytes[gain_at..]);
            assert!(
                !map_jpeg.windows(5).any(|w| w == b"JFIF\0"),
                "JFIF in the gain map"
            );
            let map = image::load_from_memory(map_jpeg).unwrap().to_rgb8();
            assert_eq!(map.dimensions(), (w.div_ceil(2), h.div_ceil(2)));
            let (left, right) = (map.get_pixel(0, 1)[0], map.get_pixel(map.width() - 1, 1)[0]);
            assert!(left < 40 && right > 215, "{left} {right}");
            std::fs::remove_file(&p).unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A real photo through the export path (decode, metadata, render, write), for looking at:
    /// `TONALITY_EXPORT_RAW=photo.ARW TONALITY_EXPORT_OUT=dir cargo test -p tonality --lib
    /// export_real_photo -- --ignored` writes Ultra HDR JPEGs (sRGB, P3 with location), a P3
    /// PNG and a TIFF, rated 3 stars.
    #[test]
    #[ignore]
    fn export_real_photo() {
        let (Ok(raw), Ok(out)) = (
            std::env::var("TONALITY_EXPORT_RAW"),
            std::env::var("TONALITY_EXPORT_OUT"),
        ) else {
            return;
        };
        let out = PathBuf::from(out);
        let full = tonality_raw::decode_full(raw.as_ref()).unwrap();
        let meta = Meta {
            capture: tonality_raw::read_capture(raw.as_ref()).unwrap(),
            rating: 3,
        };
        let edit = EditState::default();
        let maps = tonality_render::presence::presence_maps(
            &full.pixels,
            full.width,
            full.height,
            &full.color,
        );
        let (w, h, px) = full.scaled(2400);
        let src = Source {
            width: w,
            height: h,
            pixels: &px,
            frame: [full.width, full.height],
        };
        let layers = tonality_core::mask::brush_layers(&[], full.width, full.height);
        let gpu = Gpu::new().unwrap();
        for (space, format, hdr, location, name) in [
            (
                ExportColorSpace::Srgb,
                ExportFormat::Jpeg,
                true,
                false,
                "srgb-hdr.jpg",
            ),
            (
                ExportColorSpace::DisplayP3,
                ExportFormat::Jpeg,
                true,
                true,
                "p3-hdr-gps.jpg",
            ),
            (
                ExportColorSpace::DisplayP3,
                ExportFormat::Png,
                false,
                false,
                "p3.png",
            ),
            (
                ExportColorSpace::Srgb,
                ExportFormat::Tiff,
                false,
                true,
                "srgb-gps.tif",
            ),
        ] {
            let params = DevelopParams::new(
                &edit,
                &full.color,
                full.lens.as_ref(),
                full.width,
                full.height,
            )
            .for_display(space.display())
            .with_presence(&maps);
            let curve = curve_lut_rgba16f(&edit);
            let r = gpu
                .render(&src, &params, &curve, &layers, Some(&maps), |_| {})
                .unwrap();
            let g = hdr.then(|| {
                gpu.render(
                    &src,
                    &params.gain_map(),
                    &curve,
                    &layers,
                    Some(&maps),
                    |_| {},
                )
                .unwrap()
            });
            let s = ExportSettings {
                include_location: location,
                ..settings(&out, format, space, hdr)
            };
            write(&r, g.as_ref(), &out.join(name), &s, &meta).unwrap();
        }
    }

    /// Auto on real photos, for looking at: `TONALITY_AUTO_RAWS="a.ARW b.ARW"
    /// TONALITY_EXPORT_OUT=dir cargo test -p tonality --lib auto_on_real_photos -- --ignored
    /// --nocapture` prints the values and writes `<name>.before.jpg` / `<name>.auto.jpg`.
    #[test]
    #[ignore]
    fn auto_on_real_photos() {
        let (Ok(raws), Ok(out)) = (
            std::env::var("TONALITY_AUTO_RAWS"),
            std::env::var("TONALITY_EXPORT_OUT"),
        ) else {
            return;
        };
        let out = PathBuf::from(out);
        let gpu = Gpu::new().unwrap();
        for raw in raws.split_whitespace() {
            let p = tonality_raw::decode_preview(raw.as_ref(), 1600).unwrap();
            let mut edit = EditState::default();
            let cam = crate::auto_samples(&p, &edit);
            let wb = tonality_core::auto::white_balance(&cam.cam, &p.color);
            let (shot_t, shot_tint) = p.color.as_shot_temperature_tint();
            let before = edit.clone();
            if let Some((t, tint)) = wb {
                edit.white_balance.temperature = Some(t);
                edit.white_balance.tint = Some(tint);
            }
            edit.light = tonality_core::auto::tone(
                &cam,
                &p.color,
                p.color.wb_multipliers(&edit.white_balance),
            );
            let name = Path::new(raw)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            println!(
                "{name}: as shot {shot_t:.0} K {shot_tint:+.0} → auto WB {wb:?}; {:?}",
                edit.light
            );
            for (e, tag) in [(&before, "before"), (&edit, "auto")] {
                let params = DevelopParams::new(e, &p.color, None, p.full_width, p.full_height);
                let src = Source {
                    width: p.width,
                    height: p.height,
                    pixels: &p.pixels,
                    frame: [p.full_width, p.full_height],
                };
                let layers = tonality_core::mask::brush_layers(&[], p.full_width, p.full_height);
                let r = gpu
                    .render(&src, &params, &curve_lut_rgba16f(e), &layers, None, |_| {})
                    .unwrap();
                let s = settings(&out, ExportFormat::Jpeg, ExportColorSpace::Srgb, false);
                write(
                    &r,
                    None,
                    &out.join(format!("{name}.{tag}.jpg")),
                    &s,
                    &Meta {
                        capture: Capture::default(),
                        rating: 0,
                    },
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn ultra_hdr_is_jpeg_only() {
        let dir = Path::new("/");
        assert!(settings(dir, ExportFormat::Jpeg, ExportColorSpace::Srgb, true).ultra_hdr());
        assert!(!settings(dir, ExportFormat::Png, ExportColorSpace::Srgb, true).ultra_hdr());
        // Settings saved before HDR or location existed still load, without them.
        let old =
            r#"{"folder":"/x","format":"jpeg","quality":90,"longEdge":null,"versions":"current"}"#;
        let s: ExportSettings = serde_json::from_str(old).unwrap();
        assert_eq!(
            (s.color_space, s.hdr, s.include_location),
            (ExportColorSpace::Srgb, false, false)
        );
    }

    #[test]
    fn names_are_safe_and_unique() {
        assert_eq!(sanitize("DSC0001 — B&W/v2"), "DSC0001 — B&W-v2");
        let dir = std::env::temp_dir().join(format!("tonality-export-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("x.jpg");
        assert_eq!(numbered(&target, 1), target);
        assert_eq!(numbered(&target, 3), dir.join("x (3).jpg"));
        let r = Rendered {
            pixels: vec![0; 2 * 2 * 3],
            width: 2,
            height: 2,
        };
        let s = settings(&dir, ExportFormat::Png, ExportColorSpace::Srgb, false);
        std::fs::write(&target, b"earlier export").unwrap();
        let b = write(&r, None, &target, &s, &meta(0)).unwrap();
        assert_eq!(b, dir.join("x (2).jpg"));
        assert_eq!(std::fs::read(&target).unwrap(), b"earlier export");
        let c = write(&r, None, &target, &s, &meta(0)).unwrap();
        assert_eq!(c, dir.join("x (3).jpg"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
