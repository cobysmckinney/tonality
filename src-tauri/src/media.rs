//! Reading photo files: what kind they are, their metadata, and rendering
//! them down to thumbnails and previews.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Cursor, Read, Seek, SeekFrom};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDateTime;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, RgbImage};
use rawler::decoders::RawDecodeParams;
use rawler::imgop::develop::RawDevelop;
use rawler::rawsource::RawSource;

/// Long edge of grid thumbnails, in pixels.
pub const THUMB_EDGE: u32 = 720;
/// Long edge of the image shown in the viewer.
pub const PREVIEW_EDGE: u32 = 2560;

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "tif", "tiff", "webp"];
#[cfg(feature = "heif")]
const HEIF_EXTENSIONS: &[&str] = &["heic", "heif"];
#[cfg(not(feature = "heif"))]
const HEIF_EXTENSIONS: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Raw,
    Image,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Raw => "raw",
            Kind::Image => "image",
        }
    }
}

fn extension(path: &Path) -> Option<String> {
    Some(path.extension()?.to_str()?.to_ascii_lowercase())
}

/// The kind of photo a file is, or `None` if the library does not accept it.
pub fn kind_of(path: &Path) -> Option<Kind> {
    let ext = extension(path)?;
    if IMAGE_EXTENSIONS.contains(&ext.as_str()) || HEIF_EXTENSIONS.contains(&ext.as_str()) {
        Some(Kind::Image)
    } else if rawler::decoders::supported_extensions().contains(&ext.to_ascii_uppercase().as_str()) {
        Some(Kind::Raw)
    } else {
        None
    }
}

pub fn is_jpeg(path: &Path) -> bool {
    matches!(extension(path).as_deref(), Some("jpg" | "jpeg"))
}

/// Call once at startup so the `image` crate can open HEIC files.
pub fn init() {
    #[cfg(feature = "heif")]
    libheif_rs::integration::image::register_all_decoding_hooks();
}

/// How much of the start of a file is read to tell it apart and to date it.
const HEAD: u64 = 256 * 1024;

/// A cheap identity for a file: its size plus a hash of its head and tail.
/// Camera files carry their capture metadata and embedded preview up front,
/// so this tells photos apart without reading tens of megabytes off a card.
pub fn fingerprint(path: &Path) -> Result<(String, u64)> {
    const TAIL: u64 = 64 * 1024;
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let size = file.metadata()?.len();
    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());
    let mut buffer = Vec::with_capacity(HEAD as usize);
    (&mut file).take(HEAD).read_to_end(&mut buffer)?;
    hasher.update(&buffer);
    if size > HEAD + TAIL {
        file.seek(SeekFrom::End(-(TAIL as i64)))?;
        buffer.clear();
        file.read_to_end(&mut buffer)?;
        hasher.update(&buffer);
    }
    Ok((hasher.finalize().to_hex()[..32].to_string(), size))
}

#[derive(Debug, Default, Clone)]
pub struct Meta {
    pub taken_at: Option<NaiveDateTime>,
    /// Pixel size as displayed, after applying the camera's rotation.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<u32>,
    pub aperture: Option<f64>,
    /// Exposure time in seconds.
    pub shutter: Option<f64>,
    pub focal_length: Option<f64>,
}

fn parse_exif_date(text: &str) -> Option<NaiveDateTime> {
    let text = text.trim().trim_end_matches('\0');
    NaiveDateTime::parse_from_str(text, "%Y:%m:%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S"))
        .ok()
        // Cameras with an unset clock write zeroes or a date decades off.
        .filter(|date| date.and_utc().timestamp() > 0)
}

fn exif_text(parsed: &exif::Exif, tag: exif::Tag) -> Option<String> {
    match &parsed.get_field(tag, exif::In::PRIMARY)?.value {
        exif::Value::Ascii(parts) => clean(String::from_utf8_lossy(parts.first()?)),
        _ => None,
    }
}

fn exif_taken_at(parsed: &exif::Exif) -> Option<NaiveDateTime> {
    use exif::Tag;
    exif_text(parsed, Tag::DateTimeOriginal)
        .or_else(|| exif_text(parsed, Tag::DateTimeDigitized))
        .or_else(|| exif_text(parsed, Tag::DateTime))
        .as_deref()
        .and_then(parse_exif_date)
}

/// When a photo was taken, going by the start of the file alone. Reading a
/// RAW's metadata properly maps the whole file, far too much to do for every
/// photo on a card before any is chosen; most formats keep the date up front.
pub fn quick_taken_at(path: &Path) -> Option<NaiveDateTime> {
    let mut head = Vec::new();
    File::open(path).ok()?.take(HEAD).read_to_end(&mut head).ok()?;
    // A Fuji RAW opens with a header of its own, which says where its JPEG
    // preview starts; the date is in there.
    let start = match head.get(84..88) {
        Some(offset) if head.starts_with(b"FUJIFILMCCD-RAW") => u32::from_be_bytes(offset.try_into().ok()?) as usize,
        _ => 0,
    };
    // Whatever the cut-off start of a file still describes is enough here.
    let parsed = exif::Reader::new()
        .continue_on_error(true)
        .read_from_container(&mut Cursor::new(head.get(start..)?))
        .or_else(|error| error.distill_partial_result(|_| {}))
        .ok()?;
    exif_taken_at(&parsed)
}

fn clean(text: impl AsRef<str>) -> Option<String> {
    let text = text.as_ref().trim().trim_matches('\0').trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn positive(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

/// Whether an EXIF orientation swaps width and height.
fn transposes(orientation: u16) -> bool {
    (5..=8).contains(&orientation)
}

/// Best-effort metadata. Anything that cannot be read is left empty rather
/// than failing the import.
pub fn read_meta(path: &Path, kind: Kind) -> Meta {
    match kind {
        Kind::Raw => catch_unwind(AssertUnwindSafe(|| raw_meta(path)))
            .ok()
            .and_then(Result::ok)
            .unwrap_or_else(|| image_meta(path)),
        Kind::Image => image_meta(path),
    }
}

fn raw_meta(path: &Path) -> Result<Meta> {
    let source = RawSource::new(path)?;
    let decoder = rawler::get_decoder(&source).map_err(|e| anyhow!("{e}"))?;
    let params = RawDecodeParams::default();
    let md = decoder.raw_metadata(&source, &params).map_err(|e| anyhow!("{e}"))?;
    let exif = &md.exif;
    let ratio = |r: &rawler::formats::tiff::Rational| (r.d != 0).then(|| r.n as f64 / r.d as f64).and_then(positive);

    let mut meta = Meta {
        taken_at: exif
            .date_time_original
            .as_deref()
            .or(exif.create_date.as_deref())
            .and_then(parse_exif_date),
        make: clean(&md.make),
        model: clean(&md.model),
        lens: md
            .lens
            .as_ref()
            .and_then(|lens| clean(&lens.lens_name))
            .or_else(|| exif.lens_model.as_ref().and_then(clean)),
        // Above ISO 65535 the 16-bit tag saturates and the real value moves.
        iso: match exif.iso_speed_ratings {
            Some(iso) if iso > 0 && iso < u16::MAX => Some(iso as u32),
            _ => exif.iso_speed.or(exif.recommended_exposure_index).filter(|iso| *iso > 0),
        },
        aperture: exif.fnumber.as_ref().and_then(ratio),
        shutter: exif.exposure_time.as_ref().and_then(ratio),
        focal_length: exif.focal_length.as_ref().and_then(ratio),
        ..Meta::default()
    };

    // A dummy decode reads the sensor layout without unpacking any pixels.
    if let Ok(raw) = decoder.raw_image(&source, &params, true) {
        let (mut width, mut height) = match raw.crop_area.or(raw.active_area) {
            Some(area) => (area.d.w as u32, area.d.h as u32),
            None => (raw.width as u32, raw.height as u32),
        };
        if transposes(exif.orientation.unwrap_or(raw.orientation.to_u16())) {
            std::mem::swap(&mut width, &mut height);
        }
        if width > 0 && height > 0 {
            meta.width = Some(width);
            meta.height = Some(height);
        }
    }
    Ok(meta)
}

fn image_meta(path: &Path) -> Meta {
    use exif::{In, Tag, Value};
    let mut meta = Meta::default();
    let mut orientation = 1;

    let parsed = File::open(path)
        .ok()
        .and_then(|file| exif::Reader::new().read_from_container(&mut BufReader::new(file)).ok());
    if let Some(parsed) = parsed {
        let text = |tag| exif_text(&parsed, tag);
        let number = |tag| match &parsed.get_field(tag, In::PRIMARY)?.value {
            Value::Rational(v) => positive(v.first()?.to_f64()),
            Value::SRational(v) => positive(v.first()?.to_f64()),
            other => other.get_uint(0).map(f64::from).and_then(positive),
        };
        meta.taken_at = exif_taken_at(&parsed);
        meta.make = text(Tag::Make);
        meta.model = text(Tag::Model);
        meta.lens = text(Tag::LensModel);
        meta.iso = number(Tag::PhotographicSensitivity).map(|iso| iso as u32);
        meta.aperture = number(Tag::FNumber);
        meta.shutter = number(Tag::ExposureTime);
        meta.focal_length = number(Tag::FocalLength);
        orientation = number(Tag::Orientation).map_or(1, |o| o as u16);
    }

    let dimensions = ImageReader::open(path)
        .and_then(ImageReader::with_guessed_format)
        .ok()
        .and_then(|reader| reader.into_dimensions().ok());
    if let Some((mut width, mut height)) = dimensions {
        if transposes(orientation) {
            std::mem::swap(&mut width, &mut height);
        }
        meta.width = Some(width);
        meta.height = Some(height);
    }
    meta
}

fn long_edge(image: &DynamicImage) -> u32 {
    image.width().max(image.height())
}

/// Decoders hand back the pixels as stored plus the rotation still to apply,
/// so the rotation can be done after shrinking, when it is nearly free.
type Decoded = (DynamicImage, Orientation);

pub(crate) fn decode_image(path: &Path) -> Result<Decoded> {
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    Ok((DynamicImage::from_decoder(decoder)?, orientation))
}

/// Gets a displayable image out of a RAW file: the camera's embedded JPEG
/// when it is big enough for `edge`, otherwise a basic develop of the sensor
/// data.
fn decode_raw(path: &Path, edge: u32) -> Result<Decoded> {
    let source = RawSource::new(path)?;
    let decoder = rawler::get_decoder(&source).map_err(|e| anyhow!("{e}"))?;
    let params = RawDecodeParams::default();
    let orientation = decoder
        .raw_metadata(&source, &params)
        .ok()
        .and_then(|md| md.exif.orientation)
        .and_then(|o| Orientation::from_exif(o as u8))
        .unwrap_or(Orientation::NoTransforms);

    let big_enough = |image: &DynamicImage| long_edge(image) * 100 >= edge * 55;
    let mut embedded = decoder.preview_image(&source, &params).ok().flatten();
    if !embedded.as_ref().is_some_and(big_enough) {
        if let Ok(Some(full)) = decoder.full_image(&source, &params) {
            if embedded.as_ref().is_none_or(|e| long_edge(&full) > long_edge(e)) {
                embedded = Some(full);
            }
        }
    }

    let image = match embedded {
        Some(image) if big_enough(&image) => image,
        fallback => {
            let developed = decoder
                .raw_image(&source, &params, false)
                .and_then(|raw| RawDevelop::default().develop_intermediate(&raw))
                .ok()
                .and_then(|intermediate| intermediate.to_dynamic_image());
            match developed.or(fallback) {
                Some(image) => image,
                None => decoder
                    .thumbnail_image(&source, &params)
                    .ok()
                    .flatten()
                    .ok_or_else(|| anyhow!("no image data found in {}", path.display()))?,
            }
        }
    };
    Ok((image, orientation))
}

/// Renders a photo so its long edge is at most `edge` pixels. When the photo
/// has a paired camera JPEG, that is used in preference to decoding the RAW.
pub fn render(path: &Path, is_raw: bool, jpeg: Option<&Path>, edge: u32) -> Result<RgbImage> {
    let decode = || -> Result<Decoded> {
        if let Some(jpeg) = jpeg {
            if let Ok(decoded) = decode_image(jpeg) {
                return Ok(decoded);
            }
        }
        if is_raw {
            decode_raw(path, edge)
        } else {
            decode_image(path)
        }
    };
    // RAW decoders index into untrusted files and can panic on damaged ones.
    let (image, orientation) = match catch_unwind(AssertUnwindSafe(decode)) {
        Ok(decoded) => decoded.with_context(|| format!("decoding {}", path.display()))?,
        Err(_) => bail!("the decoder crashed on {}", path.display()),
    };
    let mut image = DynamicImage::ImageRgb8(shrink(image.into_rgb8(), edge)?);
    image.apply_orientation(orientation);
    Ok(image.into_rgb8())
}

/// Scales `image` down so its long edge is at most `edge`.
fn shrink(image: RgbImage, edge: u32) -> Result<RgbImage> {
    use fast_image_resize::images::Image;
    use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};

    let (width, height) = image.dimensions();
    let long = width.max(height);
    if long <= edge {
        return Ok(image);
    }
    let scaled = |side: u32| ((side as u64 * edge as u64 + long as u64 / 2) / long as u64).max(1) as u32;
    let (new_width, new_height) = (scaled(width), scaled(height));
    let source = Image::from_vec_u8(width, height, image.into_raw(), PixelType::U8x3)?;
    let mut target = Image::new(new_width, new_height, PixelType::U8x3);
    let filter = if edge <= THUMB_EDGE { FilterType::Bilinear } else { FilterType::CatmullRom };
    Resizer::new().resize(&source, &mut target, &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(filter)))?;
    RgbImage::from_raw(new_width, new_height, target.into_vec()).context("resized image has the wrong size")
}

/// Writes `image` as a JPEG, atomically, so a half-written file is never served.
pub fn write_jpeg(image: &RgbImage, dest: &Path, quality: u8) -> Result<()> {
    let dir = dest.parent().context("destination has no parent folder")?;
    fs::create_dir_all(dir)?;
    // Two requests can render the same photo at once; give each its own partial file.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let partial = dest.with_extension(format!("{}.part", COUNTER.fetch_add(1, Ordering::Relaxed)));
    let mut writer = BufWriter::new(File::create(&partial)?);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, quality).encode(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    writer.into_inner().map_err(|e| e.into_error())?;
    fs::rename(&partial, dest)?;
    Ok(())
}
