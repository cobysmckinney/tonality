//! Export: develop photos at full resolution on the GPU (wgpu, same WGSL as the preview) and
//! write JPEG, PNG or 16-bit TIFF files.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
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
                let mut params = DevelopParams::new(edit, &full.color, full.lens.as_ref(), full.width, full.height);
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
                let rendered = gpu()?.render(&src, &params, &curve_lut_rgba16f(edit), &layers, presence.as_ref(), |f| emit(done as f32 + f * 0.9, &label))?;
                let target = unique_path(&folder, &sanitize(&label), settings.format.extension());
                write(&rendered, &target, &settings)?;
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

fn write(r: &Rendered, target: &Path, s: &ExportSettings) -> Result<(), String> {
    use image::codecs::{jpeg::JpegEncoder, png::PngEncoder, tiff::TiffEncoder};
    use image::{ExtendedColorType, ImageEncoder};

    let part = target.with_extension(format!("{}.part", s.format.extension()));
    let file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut out = std::io::BufWriter::new(file);
    let eight = || -> Vec<u8> { r.pixels.iter().map(|&v| ((v as u32 * 255 + 32767) / 65535) as u8).collect() };
    let res = match s.format {
        ExportFormat::Jpeg => JpegEncoder::new_with_quality(&mut out, s.quality.clamp(1, 100)).write_image(
            &eight(),
            r.width,
            r.height,
            ExtendedColorType::Rgb8,
        ),
        ExportFormat::Png => PngEncoder::new(&mut out).write_image(&eight(), r.width, r.height, ExtendedColorType::Rgb8),
        ExportFormat::Tiff => {
            let bytes: Vec<u8> = r.pixels.iter().flat_map(|v| v.to_ne_bytes()).collect();
            TiffEncoder::new(&mut out).write_image(&bytes, r.width, r.height, ExtendedColorType::Rgb16)
        }
    };
    let res = res.map_err(|e| e.to_string()).and_then(|_| {
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
        for format in [ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::Tiff] {
            let s = ExportSettings {
                folder: dir.display().to_string(),
                format,
                quality: 90,
                long_edge: None,
                versions: ExportVersions::Current,
            };
            let p = dir.join(format!("t.{}", format.extension()));
            write(&r, &p, &s).unwrap();
            let img = image::open(&p).unwrap();
            assert_eq!((img.width(), img.height()), (4, 3));
            if format == ExportFormat::Tiff {
                assert_eq!(img.color(), image::ColorType::Rgb16);
            }
        }
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().path().to_string_lossy().ends_with(".part")));
        std::fs::remove_dir_all(&dir).unwrap();
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
