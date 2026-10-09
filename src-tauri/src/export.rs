//! Exporting: writing a finished picture out of the library as an ordinary
//! image file.
//!
//! An export draws one branch of a photo's edits, at the step that branch is
//! on. So that there is never a doubt afterwards about what was written or
//! where it went:
//!
//! - unless it is given another name, the file is named after the photo, and
//!   after the branch too when the photo has more than one;
//! - nothing in the destination folder is ever replaced: a name that is
//!   taken gets a number instead;
//! - the step that was exported is marked in the photo's history with the
//!   file it became.
//!
//! `plan` works out the folder and the names without writing anything, so
//! the interface can show them before the export is run.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, ensure, Context, Result};
use exif::{Field, In, Tag, Value};
use image::{DynamicImage, RgbImage};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::edit::Adjustments;
use crate::gpu::{self, DeepImage};
use crate::history::Branch;
use crate::library::{FilmDetails, Library, FILM_COLUMNS};
use crate::xmp::{self, Marks};

pub(crate) const SCHEMA: &str = "
-- One row per file an export wrote.
CREATE TABLE exports (
    id          INTEGER PRIMARY KEY,
    photo_id    INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    -- The step that was drawn. The record goes when the step does.
    step_id     INTEGER NOT NULL REFERENCES edit_steps(id) ON DELETE CASCADE,
    path        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX exports_photo ON exports(photo_id);
CREATE INDEX exports_step ON exports(step_id);
";

/// The name the export settings are remembered under.
const SETTINGS_KEY: &str = "export";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    #[default]
    Jpeg,
    /// Lossless, and several times the size.
    Png,
    /// 16 bits a channel, uncompressed: for more editing somewhere else.
    Tiff,
}

impl Format {
    fn extension(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::Png => "png",
            Format::Tiff => "tif",
        }
    }

    /// Whether the picture is wanted with 16 bits a channel rather than 8.
    fn deep(self) -> bool {
        self == Format::Tiff
    }
}

/// Which branches of each photo an export draws.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Branches {
    /// The one each photo is on, which is what the library shows.
    #[default]
    Current,
    /// Every branch of every photo.
    All,
    /// These branches of a single photo.
    Chosen(Vec<i64>),
}

/// What to export, as opposed to how (`Settings`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Job {
    pub ids: Vec<i64>,
    pub branches: Branches,
    /// What to call the files, without the extension. `{name}` stands for
    /// the photo's own name, `{branch}` for the branch, `{date}` for the day
    /// it was taken and `{n}` for the file's place in this export. `None`
    /// names each file after its photo, and its branch if it has several.
    pub name: Option<String>,
}

impl Job {
    /// The photos' current branches, under their own names.
    pub fn of(ids: &[i64]) -> Self {
        Self { ids: ids.to_vec(), ..Default::default() }
    }

    fn pattern(&self) -> Option<&str> {
        self.name.as_deref().map(str::trim).filter(|name| !name.is_empty())
    }

    /// Each (photo, branch) to draw, in order; no branch means the current one.
    fn targets(&self, db: &Connection) -> Result<Vec<(i64, Option<i64>)>> {
        match &self.branches {
            Branches::Current => Ok(self.ids.iter().map(|&id| (id, None)).collect()),
            Branches::Chosen(branches) => {
                let [id] = self.ids[..] else { bail!("branches can only be chosen for one photo") };
                ensure!(!branches.is_empty(), "no branch was chosen");
                Ok(branches.iter().map(|&branch| (id, Some(branch))).collect())
            }
            Branches::All => {
                let mut of_photo = db.prepare("SELECT id FROM edit_branches WHERE photo_id = ?1 ORDER BY created_at, id")?;
                let mut targets = Vec::new();
                for &id in &self.ids {
                    let branches: Vec<i64> = of_photo.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                    match branches.is_empty() {
                        // No history yet: the edits the library shows are all there is.
                        true => targets.push((id, None)),
                        false => targets.extend(branches.into_iter().map(|branch| (id, Some(branch)))),
                    }
                }
                Ok(targets)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// The folder the files are written to; `None` is the library's own
    /// Exports folder.
    pub folder: Option<PathBuf>,
    pub format: Format,
    /// JPEG quality, 1..100.
    pub quality: u8,
    /// The longer side of the exported picture, in pixels; `None` keeps the
    /// picture at full size.
    pub long_edge: Option<u32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { folder: None, format: Format::Jpeg, quality: 90, long_edge: None }
    }
}

impl Settings {
    /// The settings with anything out of range brought back in.
    fn tidied(&self) -> Self {
        Self {
            folder: self.folder.clone().filter(|folder| folder.is_absolute()),
            format: self.format,
            quality: self.quality.clamp(1, 100),
            long_edge: self.long_edge.filter(|edge| *edge > 0),
        }
    }
}

/// One file an export would write.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedFile {
    pub photo_id: i64,
    /// The branch it is drawn from; `None` for a photo with no history yet.
    pub branch_id: Option<i64>,
    /// That branch's name, for a photo that has more than one.
    pub branch: Option<String>,
    /// The name the file will be given.
    pub name: String,
    /// True when the name asked for is taken, so this one carries a number.
    pub numbered: bool,
    /// The picture's size in pixels, where the library knows the photo's.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// True when the photo is larger than the graphics card can hold, so the
    /// picture comes out smaller than asked for.
    pub shrunk: bool,
}

/// What an export would do, worked out without writing anything.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub settings: Settings,
    /// The folder the files go to, in full.
    pub folder: String,
    pub files: Vec<PlannedFile>,
    /// For a single photo, its branches to choose between, and the one it
    /// is on (which is what the library shows). Empty otherwise.
    pub branches: Vec<Branch>,
    pub current_branch_id: Option<i64>,
    /// True when some photo has more than one branch, so "all branches"
    /// would export more than "current" does.
    pub several_branches: bool,
}

/// One file an export wrote.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    pub photo_id: i64,
    pub name: String,
    pub path: String,
    /// The branch it was drawn from, for a photo that has more than one.
    pub branch: Option<String>,
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFailure {
    pub file_name: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// The folder the files went to, in full.
    pub folder: String,
    pub exported: Vec<Exported>,
    pub failed: Vec<ExportFailure>,
    pub cancelled: bool,
}

/// Draws a photo for export: its id, the recipe to apply, the longest side
/// wanted in pixels (anything larger than the picture means full size), and
/// whether to draw 16 bits a channel (`ImageRgb16`) rather than 8 (`ImageRgb8`).
pub type Draw<'a> = dyn Fn(i64, &Adjustments, u32, bool) -> Result<DynamicImage> + 'a;

/// What the camera recorded about a photo, to carry into the exported file.
struct Capture {
    taken_at: String,
    make: Option<String>,
    model: Option<String>,
    lens: Option<String>,
    iso: Option<u32>,
    aperture: Option<f64>,
    shutter: Option<f64>,
    focal_length: Option<f64>,
    /// What the picture shows besides: the film stock and frame of a film photo.
    description: Option<String>,
}

impl Capture {
    /// The capture as the film it is of was shot, if it has film details: a
    /// scan's own details describe the scanning camera and lens. The film
    /// camera, lens and ISO stand in for the scanner's, and the scanner's
    /// aperture, shutter speed and focal length go, since they were the
    /// scan's. The film stock and frame number become the description.
    fn on_film(self, film: FilmDetails) -> Self {
        if !film.is_set() {
            return self;
        }
        // Cameras give their make as a word of its own: "Nikon", then "Nikon FM2".
        let make = film.camera.as_deref().and_then(|camera| camera.split_whitespace().next()).map(str::to_string);
        let description = match (film.stock, film.frame) {
            (Some(stock), Some(frame)) => Some(format!("{stock}, frame {frame}")),
            (Some(stock), None) => Some(stock),
            (None, Some(frame)) => Some(format!("Frame {frame}")),
            (None, None) => None,
        };
        Capture {
            taken_at: self.taken_at,
            make,
            model: film.camera,
            lens: film.lens,
            iso: film.iso,
            aperture: None,
            shutter: None,
            focal_length: None,
            description,
        }
    }
}

/// What one photo contributes to an export.
struct Source {
    file_name: String,
    /// The photo's size in pixels before any crop, if the library knows it.
    size: Option<(u32, u32)>,
    /// The branch being drawn; `None` for a photo whose history has not been
    /// started, which has only the edits the library shows.
    branch: Option<Branch>,
    /// The branch the photo is on, whichever one is being drawn.
    current_branch: Option<i64>,
    /// How many branches the photo has.
    branches: u32,
    /// The step the branch is on.
    step_id: Option<i64>,
    recipe: Adjustments,
    capture: Capture,
    /// The photo's flag and favorite, which the file carries as XMP.
    marks: Marks,
}

impl Source {
    /// Reads a photo's `branch_id` branch, or its current one.
    fn read(db: &Connection, photo_id: i64, branch_id: Option<i64>) -> Result<Self> {
        let (source, own_edits, step_recipe) = db
            .query_row(
                &format!(
                    "SELECT p.file_name, p.edits, p.taken_at, p.make, p.model, p.lens, p.iso, p.aperture, p.shutter,
                            p.focal_length, (SELECT COUNT(*) FROM edit_branches WHERE photo_id = p.id),
                            b.id, b.name, s.id, s.recipe, p.branch_id, p.width, p.height, {FILM_COLUMNS},
                            p.flag, p.favorite
                     FROM photos p
                     LEFT JOIN edit_branches b ON b.photo_id = p.id AND b.id = COALESCE(?2, p.branch_id)
                     LEFT JOIN edit_steps s ON s.id = b.head_id
                     WHERE p.id = ?1"
                ),
                params![photo_id, branch_id],
                |r| {
                    let branch = match (r.get::<_, Option<i64>>(11)?, r.get::<_, Option<String>>(12)?) {
                        (Some(id), Some(name)) => Some(Branch { id, name }),
                        _ => None,
                    };
                    let source = Source {
                        file_name: r.get(0)?,
                        size: r.get::<_, Option<u32>>(16)?.zip(r.get::<_, Option<u32>>(17)?),
                        branch,
                        current_branch: r.get(15)?,
                        branches: r.get(10)?,
                        step_id: r.get(13)?,
                        recipe: Adjustments::default(),
                        capture: Capture {
                            taken_at: r.get(2)?,
                            make: r.get(3)?,
                            model: r.get(4)?,
                            lens: r.get(5)?,
                            iso: r.get(6)?,
                            aperture: r.get(7)?,
                            shutter: r.get(8)?,
                            focal_length: r.get(9)?,
                            description: None,
                        }
                        .on_film(FilmDetails::from_row(r, 18)?),
                        marks: Marks { flag: r.get(23)?, favorite: r.get(24)? },
                    };
                    Ok((source, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(14)?))
                },
            )
            .with_context(|| format!("photo {photo_id} is not in the library"))?;
        if source.branch.is_none() && (source.branches > 0 || branch_id.is_some()) {
            bail!("that branch of {} is no longer there", source.file_name);
        }
        let recipe = if source.branch.is_some() { step_recipe } else { own_edits };
        Ok(Source { recipe: Adjustments::from_json(recipe.as_deref()), ..source })
    }

    /// The branch's name where it needs saying: on a photo that has others.
    fn named_branch(&self) -> Option<&str> {
        self.branch.as_ref().filter(|_| self.branches > 1).map(|branch| branch.name.as_str())
    }

    /// The photo's name without its extension.
    fn stem(&self) -> &str {
        Path::new(&self.file_name).file_stem().and_then(|stem| stem.to_str()).unwrap_or(&self.file_name)
    }

    /// The name the exported file asks for. Left to itself that is the
    /// photo's own, and the branch's when there are several to tell apart:
    /// `IMG_0462 (Warm).jpg`. A `pattern` replaces it, with `{name}`,
    /// `{branch}`, `{date}` and `{n}` filled in for this file, the `number`th
    /// of `total`.
    fn output_name(&self, pattern: Option<&str>, number: usize, total: usize, format: Format) -> String {
        let named = pattern.and_then(|pattern| {
            let digits = total.to_string().len().max(2);
            let filled = pattern
                .replace("{name}", self.stem())
                .replace("{branch}", self.branch.as_ref().map_or("Main", |branch| &branch.name))
                .replace("{date}", self.capture.taken_at.get(..10).unwrap_or_default())
                .replace("{n}", &format!("{number:0digits$}"));
            file_safe(without_extension(&filled))
        });
        let stem = named.unwrap_or_else(|| match self.named_branch().and_then(file_safe) {
            Some(branch) => format!("{} ({branch})", self.stem()),
            None => self.stem().to_string(),
        });
        format!("{stem}.{}", format.extension())
    }

    /// The exported picture's size in pixels, if the library knows the
    /// photo's, drawn from the photo as a graphics card that holds `largest`
    /// pixels a side has it.
    fn output_size(&self, settings: &Settings, largest: u32) -> Option<(u32, u32)> {
        let (width, height) = self.size?;
        let (width, height) = gpu::held_size(width, height, largest);
        Some(gpu::picture_size(width, height, &self.recipe, settings.long_edge.unwrap_or(u32::MAX)))
    }
}

/// A typed name without the image extension someone may have typed after it.
fn without_extension(name: &str) -> &str {
    let name = name.trim();
    match name.rsplit_once('.') {
        Some((stem, extension)) if ["jpg", "jpeg", "png", "tif", "tiff"].contains(&extension.to_ascii_lowercase().as_str()) => stem,
        _ => name,
    }
}

/// Text made fit to be (part of) a file name, or `None` if nothing is left of it.
fn file_safe(text: &str) -> Option<String> {
    let safe: String = text
        .chars()
        .take(120)
        .map(|c| if c.is_control() || r#"/\:*?"<>|"#.contains(c) { '-' } else { c })
        .collect();
    let safe = safe.trim_matches(|c: char| c == '.' || c.is_whitespace());
    (!safe.is_empty()).then(|| safe.to_string())
}

/// `name` itself the first time, then `name-2`, `name-3` and so on, ahead of
/// the extension.
fn numbered(name: &str, attempt: u32) -> String {
    match name.rsplit_once('.') {
        _ if attempt <= 1 => name.to_string(),
        Some((stem, extension)) => format!("{stem}-{attempt}.{extension}"),
        None => format!("{name}-{attempt}"),
    }
}

/// Writes `bytes` to a new file in `folder`, called `name` or, if that is
/// taken, the first numbered variant that is free. Never replaces a file.
fn write_new(folder: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    for attempt in 1.. {
        let path = folder.join(numbered(name, attempt));
        // `create_new` fails rather than opening a file that is already there.
        let mut file = match fs::File::options().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).with_context(|| format!("creating {}", path.display())),
        };
        if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            // Half a picture is worse than none.
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error).with_context(|| format!("writing {}", path.display()));
        }
        return Ok(path);
    }
    unreachable!("the loop only ends by returning")
}

fn field(tag: Tag, value: Value) -> Field {
    Field { tag, ifd_num: In::PRIMARY, value }
}

/// The capture details as EXIF fields. The orientation is left out on
/// purpose: an exported picture is already the right way up.
fn capture_fields(capture: &Capture) -> Vec<Field> {
    let text = |tag, value: &str| value.is_ascii().then(|| field(tag, Value::Ascii(vec![value.as_bytes().to_vec()])));
    let ratio = |tag, num: u32, denom: u32| field(tag, Value::Rational(vec![exif::Rational { num, denom }]));
    let tenths = |value: f64| (value * 10.0).round() as u32;
    let hundredths = |value: f64| (value * 100.0).round() as u32;

    let mut fields = vec![
        text(Tag::Software, "Tonality"),
        // 1 is sRGB, which is what the develop shader produces.
        Some(field(Tag::ColorSpace, Value::Short(vec![1]))),
        capture.description.as_deref().and_then(|description| text(Tag::ImageDescription, description)),
        capture.make.as_deref().and_then(|make| text(Tag::Make, make)),
        capture.model.as_deref().and_then(|model| text(Tag::Model, model)),
        capture.lens.as_deref().and_then(|lens| text(Tag::LensModel, lens)),
        capture.iso.map(|iso| field(Tag::PhotographicSensitivity, Value::Short(vec![iso.min(u16::MAX as u32) as u16]))),
        // Hundredths, as cameras write it: f/0.95 lenses exist.
        capture.aperture.map(|f| ratio(Tag::FNumber, hundredths(f), 100)),
        capture.focal_length.map(|mm| ratio(Tag::FocalLength, tenths(mm), 10)),
        // Shutter speeds under a second are fractions when the camera's are:
        // 1/250 and 1/3, but 0.3 and 0.8 as tenths.
        capture.shutter.map(|seconds| {
            let per_second = 1.0 / seconds;
            let whole = per_second.round();
            if seconds < 0.25 || (seconds < 1.0 && (per_second - whole).abs() < 0.02 * per_second) {
                ratio(Tag::ExposureTime, 1, whole as u32)
            } else {
                ratio(Tag::ExposureTime, tenths(seconds), 10)
            }
        }),
    ];
    // 'YYYY-MM-DDTHH:MM:SS' in the library, 'YYYY:MM:DD HH:MM:SS' in EXIF.
    if capture.taken_at.len() == 19 {
        fields.push(text(Tag::DateTimeOriginal, &capture.taken_at.replace('-', ":").replace('T', " ")));
    }
    fields.into_iter().flatten().collect()
}

/// EXIF is a small TIFF file of its own, and the EXIF writer will write a
/// whole one: `fields`, and after them `strips` of image data if there are any.
fn write_tiff(fields: &[Field], strips: Option<&[&[u8]]>, size_hint: usize) -> Result<Vec<u8>> {
    let mut writer = exif::experimental::Writer::new();
    for field in fields {
        writer.push_field(field);
    }
    if let Some(strips) = strips {
        writer.set_strips(strips, In::PRIMARY);
    }
    let mut bytes = std::io::Cursor::new(Vec::with_capacity(size_hint + 4096));
    // Samples are stored as this machine keeps them, and the file says which way round that is.
    writer.write(&mut bytes, cfg!(target_endian = "little")).context("laying out the file")?;
    Ok(bytes.into_inner())
}

/// A 16-bit TIFF: uncompressed, which everything that opens TIFFs can read.
fn tiff(image: &DeepImage, mut fields: Vec<Field>) -> Result<Vec<u8>> {
    /// About how much picture goes in each strip; TIFF readers take a strip at a time.
    const STRIP_BYTES: u32 = 1 << 20;
    const SAMPLE_BYTES: u32 = 3 * 2;

    let (width, height) = image.dimensions();
    let rows_per_strip = (STRIP_BYTES / (width * SAMPLE_BYTES)).max(1);
    let short = |tag, values: &[u16]| field(tag, Value::Short(values.to_vec()));
    let pixels_per_inch = |tag| field(tag, Value::Rational(vec![exif::Rational { num: 300, denom: 1 }]));
    fields.extend([
        field(Tag::ImageWidth, Value::Long(vec![width])),
        field(Tag::ImageLength, Value::Long(vec![height])),
        short(Tag::BitsPerSample, &[16, 16, 16]),
        short(Tag::Compression, &[1]),
        // 2 is RGB.
        short(Tag::PhotometricInterpretation, &[2]),
        short(Tag::SamplesPerPixel, &[3]),
        field(Tag::RowsPerStrip, Value::Long(vec![rows_per_strip])),
        short(Tag::PlanarConfiguration, &[1]),
        pixels_per_inch(Tag::XResolution),
        pixels_per_inch(Tag::YResolution),
        short(Tag::ResolutionUnit, &[2]),
    ]);
    let samples: &[u8] = bytemuck::cast_slice(image.as_raw());
    let strips: Vec<&[u8]> = samples.chunks((rows_per_strip * width * SAMPLE_BYTES) as usize).collect();
    write_tiff(&fields, Some(&strips), samples.len())
}

/// The XMP an exported file carries: the photo's marks, and the details
/// EXIF can only hold in ASCII.
fn xmp_packet(capture: &Capture, marks: Marks) -> String {
    let details = xmp::Details {
        description: capture.description.as_deref(),
        make: capture.make.as_deref(),
        model: capture.model.as_deref(),
        lens: capture.lens.as_deref(),
    };
    xmp::packet(marks, &details)
}

fn encode(image: &DynamicImage, settings: &Settings, capture: &Capture, marks: Marks) -> Result<Vec<u8>> {
    use image::codecs::jpeg::JpegEncoder;
    use image::codecs::png::PngEncoder;
    use image::{ExtendedColorType, ImageEncoder};

    fn write(mut encoder: impl ImageEncoder, image: &RgbImage, exif: Vec<u8>) -> Result<()> {
        encoder.set_exif_metadata(exif)?;
        Ok(encoder.write_image(image.as_raw(), image.width(), image.height(), ExtendedColorType::Rgb8)?)
    }

    let mut fields = capture_fields(capture);
    let packet = xmp_packet(capture, marks);
    if settings.format == Format::Tiff {
        // TIFF keeps XMP in a tag of its own, XMLPacket.
        fields.push(field(Tag(exif::Context::Tiff, 700), Value::Byte(packet.into_bytes())));
        return match image {
            DynamicImage::ImageRgb16(deep) => tiff(deep, fields),
            other => tiff(&other.to_rgb16(), fields),
        };
    }
    let converted;
    let image = match image {
        DynamicImage::ImageRgb8(image) => image,
        other => {
            converted = other.to_rgb8();
            &converted
        }
    };
    let exif = write_tiff(&fields, None, 0)?;
    let mut bytes = Vec::new();
    match settings.format {
        Format::Jpeg => {
            write(JpegEncoder::new_with_quality(&mut bytes, settings.quality), image, exif)?;
            xmp::into_jpeg(&bytes, &packet)
        }
        _ => {
            write(PngEncoder::new(&mut bytes), image, exif)?;
            xmp::into_png(&bytes, &packet)
        }
    }
}

impl Library {
    /// Where exports go unless another folder is chosen.
    pub fn exports_dir(&self) -> PathBuf {
        self.root().join("Exports")
    }

    /// The export settings used last time, or the defaults.
    pub fn export_settings(&self) -> Result<Settings> {
        let stored = self.setting(SETTINGS_KEY)?;
        Ok(stored.and_then(|json| serde_json::from_str::<Settings>(&json).ok()).unwrap_or_default().tidied())
    }
}

fn folder_of(library: &Library, settings: &Settings) -> PathBuf {
    settings.folder.clone().unwrap_or_else(|| library.exports_dir())
}

/// Works out what a job would write: the folder, each file's name and size,
/// and the branch it comes from. Nothing is written or changed. Without
/// `settings`, the ones used last time apply. `largest` is the longest side
/// of a photo the graphics card can hold (`Gpu::largest_picture`).
pub fn plan(library: &Library, job: &Job, settings: Option<&Settings>, largest: u32) -> Result<Plan> {
    let settings = match settings {
        Some(settings) => settings.tidied(),
        None => library.export_settings()?,
    };
    let folder = folder_of(library, &settings);
    let db = library.db();
    let targets = job.targets(&db)?;

    let mut taken = HashSet::new();
    let mut files = Vec::with_capacity(targets.len());
    let (mut current, mut several_branches) = (None, false);
    for (index, &(id, branch_id)) in targets.iter().enumerate() {
        let source = Source::read(&db, id, branch_id)?;
        let wanted = source.output_name(job.pattern(), index + 1, targets.len(), settings.format);
        // The same search `write_new` makes, plus the names this export has already claimed.
        let name = (1..)
            .map(|attempt| numbered(&wanted, attempt))
            .find(|name| !taken.contains(name) && fs::symlink_metadata(folder.join(name)).is_err())
            .expect("there is always another number");
        taken.insert(name.clone());
        let size = source.output_size(&settings, largest);
        files.push(PlannedFile {
            photo_id: id,
            branch_id: source.branch.as_ref().map(|branch| branch.id),
            branch: source.named_branch().map(str::to_string),
            numbered: name != wanted,
            name,
            width: size.map(|(width, _)| width),
            height: size.map(|(_, height)| height),
            shrunk: size != source.output_size(&settings, u32::MAX),
        });
        current = source.current_branch;
        several_branches |= source.branches > 1;
    }

    let mut branches = Vec::new();
    if let [id] = job.ids[..] {
        branches = db
            .prepare("SELECT id, name FROM edit_branches WHERE photo_id = ?1 ORDER BY created_at, id")?
            .query_map([id], |r| Ok(Branch { id: r.get(0)?, name: r.get(1)? }))?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(Plan {
        settings,
        folder: folder.to_string_lossy().into_owned(),
        files,
        branches,
        current_branch_id: current.filter(|_| job.ids.len() == 1),
        several_branches,
    })
}

/// The photo's name for a failure report, whatever went wrong with it.
fn name_of(library: &Library, id: i64) -> String {
    library
        .db()
        .query_row("SELECT file_name FROM photos WHERE id = ?1", [id], |r| r.get(0))
        .unwrap_or_else(|_| format!("Photo {id}"))
}

/// Exports one branch of one photo, the `number`th file of `total`.
#[allow(clippy::too_many_arguments)]
fn export_one(
    library: &Library,
    (id, branch_id): (i64, Option<i64>),
    pattern: Option<&str>,
    (number, total): (usize, usize),
    settings: &Settings,
    folder: &Path,
    draw: &Draw,
) -> Result<Exported> {
    // Starts the photo's history if it has none, so that every export has a
    // step to be marked on.
    library.history(id)?;
    let source = Source::read(&library.db(), id, branch_id)?;
    let step_id = source.step_id.context("the photo has no history")?;

    let image = draw(id, &source.recipe, settings.long_edge.unwrap_or(u32::MAX), settings.format.deep())?;
    let bytes = encode(&image, settings, &source.capture, source.marks)?;
    let path = write_new(folder, &source.output_name(pattern, number, total, settings.format), &bytes)?;
    let shown = path.to_string_lossy().into_owned();
    library
        .db()
        .execute(
            "INSERT INTO exports (photo_id, step_id, path, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, step_id, shown, chrono::Utc::now().timestamp()],
        )
        .with_context(|| format!("{shown} was written, but could not be noted in the photo's history"))?;

    Ok(Exported {
        photo_id: id,
        name: path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
        path: shown,
        branch: source.named_branch().map(str::to_string),
        width: image.width(),
        height: image.height(),
        file_size: bytes.len() as u64,
    })
}

/// Runs a job, writing its files one after another as `plan` describes. The
/// settings are remembered for next time. `progress` is called with (files
/// finished, files asked for); a file that fails is reported and the rest
/// carry on.
pub fn run(
    library: &Library,
    job: &Job,
    settings: &Settings,
    draw: &Draw,
    cancel: &AtomicBool,
    progress: &dyn Fn(usize, usize),
) -> Result<Summary> {
    let settings = settings.tidied();
    let folder = folder_of(library, &settings);
    if job.branches == Branches::All {
        // Every photo needs its history started before its branches can be listed.
        for &id in &job.ids {
            let _ = library.history(id);
        }
    }
    let targets = job.targets(&library.db())?;
    fs::create_dir_all(&folder).with_context(|| format!("creating the folder {}", folder.display()))?;
    library.set_setting(SETTINGS_KEY, &serde_json::to_string(&settings)?)?;

    let mut summary = Summary {
        folder: folder.to_string_lossy().into_owned(),
        exported: Vec::new(),
        failed: Vec::new(),
        cancelled: false,
    };
    for (done, &target) in targets.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
        match export_one(library, target, job.pattern(), (done + 1, targets.len()), &settings, &folder, draw) {
            Ok(exported) => summary.exported.push(exported),
            Err(error) => summary
                .failed
                .push(ExportFailure { file_name: name_of(library, target.0), reason: format!("{error:#}") }),
        }
        progress(done + 1, targets.len());
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture() -> Capture {
        Capture {
            taken_at: "2026-09-19T17:42:05".into(),
            make: Some("Canon".into()),
            model: Some("Canon EOS R6".into()),
            lens: Some("RF24-70mm F2.8 L IS USM".into()),
            iso: Some(400),
            aperture: Some(2.8),
            shutter: Some(1.0 / 250.0),
            focal_length: Some(35.0),
            description: None,
        }
    }

    fn source(file_name: &str, branch: &str, branches: u32) -> Source {
        Source {
            file_name: file_name.into(),
            size: Some((6000, 4000)),
            branch: Some(Branch { id: 1, name: branch.into() }),
            current_branch: Some(1),
            branches,
            step_id: Some(1),
            recipe: Adjustments::default(),
            capture: capture(),
            marks: Marks::default(),
        }
    }

    #[test]
    fn taken_names_get_a_number_before_the_extension() {
        assert_eq!(numbered("IMG_0462.jpg", 1), "IMG_0462.jpg");
        assert_eq!(numbered("IMG_0462.jpg", 2), "IMG_0462-2.jpg");
        assert_eq!(numbered("IMG_0462 (v1.2).jpg", 3), "IMG_0462 (v1.2)-3.jpg");
    }

    #[test]
    fn a_file_is_named_after_its_photo_and_its_branch_if_there_are_several() {
        let name = |source: &Source| source.output_name(None, 1, 1, Format::Jpeg);
        assert_eq!(name(&source("IMG_0462.CR2", "Main", 1)), "IMG_0462.jpg");
        assert_eq!(name(&source("IMG_0462.CR2", "Warm", 2)), "IMG_0462 (Warm).jpg");
        assert_eq!(name(&source("IMG_0462.CR2", "B/W: high contrast?", 2)), "IMG_0462 (B-W- high contrast-).jpg");
        assert_eq!(name(&source("IMG_0462.CR2", " .. ", 2)), "IMG_0462.jpg");
    }

    #[test]
    fn a_given_name_is_used_as_typed_with_its_placeholders_filled_in() {
        let warm = source("IMG_0462.CR2", "Warm", 2);
        let name = |pattern, number, total| warm.output_name(Some(pattern), number, total, Format::Tiff);
        assert_eq!(name("Portfolio cover", 1, 1), "Portfolio cover.tif", "no branch is added to a name you chose");
        assert_eq!(name("{name}-web", 1, 1), "IMG_0462-web.tif");
        assert_eq!(name("{date} {branch} {n}", 3, 12), "2026-09-19 Warm 03.tif");
        assert_eq!(name("Wedding {n}", 7, 250), "Wedding 007.tif");
        // An extension typed out of habit is not doubled, and nothing can climb out of the folder.
        assert_eq!(name("cover.JPG", 1, 1), "cover.tif");
        assert_eq!(name("../../etc/cover", 1, 1), "-..-etc-cover.tif");
        assert_eq!(name(" . ", 1, 1), "IMG_0462 (Warm).tif", "an empty name falls back to the usual one");
    }

    #[test]
    fn the_planned_size_follows_the_crop_and_the_longest_side_asked_for() {
        let mut photo = source("IMG_0462.CR2", "Main", 1);
        let size = |photo: &Source, long_edge| photo.output_size(&Settings { long_edge, ..Default::default() }, u32::MAX);
        assert_eq!(size(&photo, None), Some((6000, 4000)));
        assert_eq!(size(&photo, Some(3000)), Some((3000, 2000)));
        assert_eq!(size(&photo, Some(9000)), Some((6000, 4000)), "never enlarged");
        photo.recipe.rotation = 1;
        photo.recipe.crop.width = 0.5;
        assert_eq!(size(&photo, None), Some((2000, 6000)));
        photo.size = None;
        assert_eq!(size(&photo, None), None);
    }

    #[test]
    fn the_planned_size_is_what_the_graphics_card_can_hold() {
        let mut panorama = source("PANO.CR2", "Main", 1);
        panorama.size = Some((20000, 5000));
        let size = |photo: &Source, long_edge| photo.output_size(&Settings { long_edge, ..Default::default() }, 16384);
        assert_eq!(size(&panorama, None), Some((16384, 4096)), "scaled to fit, not halved to 10000");
        assert_eq!(size(&panorama, Some(8000)), Some((8000, 2000)));
        panorama.recipe.crop.width = 0.5;
        assert_eq!(size(&panorama, None), Some((8192, 4096)), "the crop is of the photo as held");
    }

    #[test]
    fn capture_details_survive_as_exif() {
        let bytes = write_tiff(&capture_fields(&capture()), None, 0).unwrap();
        let read = exif::Reader::new().read_raw(bytes).unwrap();
        let shown = |tag| read.get_field(tag, In::PRIMARY).unwrap().display_value().to_string();
        assert_eq!(shown(Tag::DateTimeOriginal), "2026-09-19 17:42:05");
        assert_eq!(shown(Tag::Model), "\"Canon EOS R6\"");
        assert_eq!(shown(Tag::LensModel), "\"RF24-70mm F2.8 L IS USM\"");
        assert_eq!(shown(Tag::PhotographicSensitivity), "400");
        assert_eq!(shown(Tag::FNumber), "2.8");
        assert_eq!(shown(Tag::ExposureTime), "1/250");
        assert_eq!(shown(Tag::FocalLength), "35");
        assert!(read.get_field(Tag::Orientation, In::PRIMARY).is_none());

        let fast = Capture { aperture: Some(0.95), ..capture() };
        let bytes = write_tiff(&capture_fields(&fast), None, 0).unwrap();
        let read = exif::Reader::new().read_raw(bytes).unwrap();
        assert_eq!(read.get_field(Tag::FNumber, In::PRIMARY).unwrap().display_value().to_string(), "0.95", "not rounded to f/1");
    }

    #[test]
    fn a_film_photo_carries_the_film_cameras_details_rather_than_the_scanners() {
        let film = FilmDetails { camera: Some("Nikon FM2".into()), iso: Some(800), ..Default::default() };
        let on_film = capture().on_film(film);
        assert_eq!((on_film.make.as_deref(), on_film.model.as_deref()), (Some("Nikon"), Some("Nikon FM2")));
        assert_eq!((on_film.lens, on_film.iso), (None, Some(800)), "the scanner's macro lens is not the film camera's");
        assert_eq!((on_film.aperture, on_film.shutter, on_film.focal_length), (None, None, None));
        assert_eq!(on_film.taken_at, capture().taken_at);

        let described = |stock: Option<&str>, frame| {
            let film = FilmDetails { stock: stock.map(str::to_string), frame, ..Default::default() };
            capture().on_film(film).description
        };
        assert_eq!(described(Some("Kodak Portra 400"), Some(12)).as_deref(), Some("Kodak Portra 400, frame 12"));
        assert_eq!(described(Some("Ilford HP5 Plus"), None).as_deref(), Some("Ilford HP5 Plus"));
        assert_eq!(described(None, Some(0)).as_deref(), Some("Frame 0"));

        let digital = capture().on_film(FilmDetails::default());
        assert_eq!((digital.model.as_deref(), digital.aperture), (Some("Canon EOS R6"), Some(2.8)), "a digital photo is as shot");
    }

    #[test]
    fn shutter_speeds_are_written_as_the_camera_wrote_them() {
        let cases = [
            (1.0 / 250.0, (1, 250)),
            (1.0 / 4.0, (1, 4)),
            (1.0 / 3.0, (1, 3)),
            (0.3, (3, 10)),
            (0.4, (4, 10)),
            (0.5, (1, 2)),
            (0.8, (8, 10)),
            (1.3, (13, 10)),
            (30.0, (300, 10)),
        ];
        for (seconds, expected) in cases {
            let bytes = write_tiff(&capture_fields(&Capture { shutter: Some(seconds), ..capture() }), None, 0).unwrap();
            let read = exif::Reader::new().read_raw(bytes).unwrap();
            // The rational itself: kamadak-exif shows 3/10 as 1/3.33, which would hide the difference.
            let Value::Rational(ref written) = read.get_field(Tag::ExposureTime, In::PRIMARY).unwrap().value else {
                panic!("the exposure time is not a rational")
            };
            assert_eq!((written[0].num, written[0].denom), expected, "{seconds} s");
        }
    }

    #[test]
    fn a_tiff_holds_every_one_of_its_sixteen_bits() {
        // Wide enough for several strips, with values 8 bits could not tell apart.
        let image = DeepImage::from_fn(700, 900, |x, y| image::Rgb([(x * 93 + y) as u16, (y * 71) as u16, 65535 - x as u16]));
        let bytes = tiff(&image, capture_fields(&capture())).unwrap();
        assert!(bytes.len() > 700 * 900 * 6);

        let read = image::load_from_memory_with_format(&bytes, image::ImageFormat::Tiff).unwrap();
        let DynamicImage::ImageRgb16(read) = read else { panic!("not read back as 16-bit RGB: {:?}", read.color()) };
        assert!(read == image, "the pixels changed on the way through");

        let exif = exif::Reader::new().read_raw(bytes).unwrap();
        let model = exif.get_field(Tag::Model, In::PRIMARY).unwrap();
        assert_eq!(model.display_value().to_string(), "\"Canon EOS R6\"");
    }
}
