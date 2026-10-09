//! XMP: the metadata other photo apps read, written so that a photo's flag
//! and favorite reach them.
//!
//! Each photo that is flagged or a favorite gets a sidecar next to its
//! original, named after the whole file (`IMG_0462.CR2.xmp`, as darktable
//! and digiKam name theirs) so that a RAW and an unrelated JPEG of the same
//! name never share one. Exports carry the same in their own embedded XMP.
//!
//! There is no one standard for picks and favorites, so each is written the
//! ways the most apps read:
//!
//! - `xmp:Rating`: -1 for a reject (Bridge, darktable, digiKam and most
//!   others show it as rejected), 5 stars for a favorite, 0 otherwise.
//! - `xmpDM:pick` and `xmpDM:good`: Lightroom's pick and reject flags.
//! - `digiKam:PickLabel`: digiKam's accepted and rejected.
//! - `tonality:Flag` and `tonality:Favorite`: exactly what the library holds.
//!
//! A sidecar is only ever replaced or removed if Tonality wrote it, which
//! its toolkit name (`x:xmptk="Tonality"`) says; another app that rewrites
//! it puts its own name there, and from then on the file is left alone.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};

/// What says a sidecar is Tonality's own.
const TOOLKIT: &str = r#"x:xmptk="Tonality""#;

/// A photo's flag and favorite.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Marks {
    /// 1 = pick, -1 = reject, 0 = unflagged.
    pub flag: i8,
    pub favorite: bool,
}

impl Marks {
    pub fn is_set(&self) -> bool {
        *self != Self::default()
    }

    /// The star rating other apps are shown: a reject is -1 whatever else it
    /// is, and a favorite is five stars.
    pub fn rating(&self) -> i8 {
        match (self.flag.signum(), self.favorite) {
            (-1, _) => -1,
            (_, true) => 5,
            _ => 0,
        }
    }
}

/// What an export says about the picture besides its marks, in full: EXIF
/// text can only be ASCII, so a film stock or camera with other letters in
/// it only reaches the file here.
#[derive(Debug, Clone, Default)]
pub struct Details<'a> {
    pub description: Option<&'a str>,
    pub make: Option<&'a str>,
    pub model: Option<&'a str>,
    pub lens: Option<&'a str>,
}

/// Text made safe to go in XML, as an attribute or between tags.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // Control characters other than tabs and line breaks aren't allowed in XML at all.
            c if c.is_control() && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// A whole XMP packet holding `marks` and `details`.
pub fn packet(marks: Marks, details: &Details) -> String {
    let flag = marks.flag.signum();
    let mut attributes = vec![format!(r#"xmp:Rating="{}""#, marks.rating())];
    if flag != 0 {
        attributes.push(format!(r#"xmpDM:pick="{flag}""#));
        attributes.push(format!(r#"xmpDM:good="{}""#, if flag > 0 { "True" } else { "False" }));
        attributes.push(format!(r#"digiKam:PickLabel="{}""#, if flag > 0 { 3 } else { 1 }));
    }
    attributes.push(format!(r#"tonality:Flag="{flag}""#));
    attributes.push(format!(r#"tonality:Favorite="{}""#, if marks.favorite { "True" } else { "False" }));
    let text = |name: &str, value: Option<&str>| {
        value.map(str::trim).filter(|value| !value.is_empty()).map(|value| format!(r#"{name}="{}""#, escaped(value)))
    };
    attributes.extend(text("tiff:Make", details.make));
    attributes.extend(text("tiff:Model", details.model));
    attributes.extend(text("exifEX:LensModel", details.lens));

    let description = details.description.map(str::trim).filter(|value| !value.is_empty()).map(|value| {
        format!(
            "\n   <dc:description>\n    <rdf:Alt>\n     <rdf:li xml:lang=\"x-default\">{}</rdf:li>\n    </rdf:Alt>\n   </dc:description>\n  ",
            escaped(value)
        )
    });
    let body = match description {
        Some(description) => format!(">{description}</rdf:Description>"),
        None => "/>".to_string(),
    };
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" {TOOLKIT}>
 <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">
  <rdf:Description rdf:about=\"\"
    xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"
    xmlns:xmpDM=\"http://ns.adobe.com/xmp/1.0/DynamicMedia/\"
    xmlns:digiKam=\"http://www.digikam.org/ns/1.0/\"
    xmlns:dc=\"http://purl.org/dc/elements/1.1/\"
    xmlns:tiff=\"http://ns.adobe.com/tiff/1.0/\"
    xmlns:exifEX=\"http://cipa.jp/exif/1.0/\"
    xmlns:tonality=\"https://github.com/cobysmckinney/tonality/xmp/1.0/\"
    {}{body}
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end=\"w\"?>
",
        attributes.join("\n    ")
    )
}

/// The sidecar of the original at `original`.
pub fn sidecar_path(original: &Path) -> PathBuf {
    let mut name = original.as_os_str().to_owned();
    name.push(".xmp");
    PathBuf::from(name)
}

/// Whether the file at `path` is a sidecar Tonality wrote. A file that can't
/// be read is not taken to be one.
fn is_ours(path: &Path) -> bool {
    fs::read(path).is_ok_and(|bytes| bytes.windows(TOOLKIT.len()).any(|window| window == TOOLKIT.as_bytes()))
}

/// What happened to a sidecar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Written,
    Removed,
    /// There was nothing to say and no sidecar to take away.
    Nothing,
    /// Another app's file is in the way, and was left as it was.
    LeftAlone,
}

/// Brings the sidecar of the original at `original` up to date with `marks`:
/// written if there is anything to say, removed if there isn't. The new
/// file is written whole beside the old and then put in its place, so an
/// app reading it never finds half a file.
pub fn update_sidecar(original: &Path, marks: Marks) -> Result<Outcome> {
    let path = sidecar_path(original);
    let exists = fs::symlink_metadata(&path).is_ok();
    if exists && !is_ours(&path) {
        return Ok(Outcome::LeftAlone);
    }
    if !marks.is_set() {
        if !exists {
            return Ok(Outcome::Nothing);
        }
        fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        return Ok(Outcome::Removed);
    }
    write_atomically(&path, packet(marks, &Details::default()).as_bytes())?;
    Ok(Outcome::Written)
}

/// Removes the sidecar of the original at `original`, if Tonality wrote it.
pub fn remove_sidecar(original: &Path) -> Result<()> {
    let path = sidecar_path(original);
    if is_ours(&path) {
        fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

/// Replaces the file at `path` with `bytes` in one step.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().context("a sidecar needs a folder")?;
    let name = path.file_name().context("a sidecar needs a name")?.to_string_lossy();
    // Hidden, so a look through the folder for originals passes over it.
    let temporary =
        dir.join(format!(".{name}.{}-{}.tmp", std::process::id(), WRITES.fetch_add(1, Ordering::Relaxed)));
    let written = (|| {
        let mut file = fs::File::create_new(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("writing {}", path.display()));
    }
    if let Ok(dir) = fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// A JPEG with `packet` added as its XMP, after the segments at its start
/// that must come first (JFIF and EXIF).
pub fn into_jpeg(jpeg: &[u8], packet: &str) -> Result<Vec<u8>> {
    const SIGNATURE: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
    let length = 2 + SIGNATURE.len() + packet.len();
    anyhow::ensure!(length <= u16::MAX as usize, "the XMP is too long for a JPEG segment");
    anyhow::ensure!(jpeg.starts_with(&[0xFF, 0xD8]), "not a JPEG");
    // Past the start-of-image marker and every application segment after it.
    let mut at = 2;
    while at + 4 <= jpeg.len() && jpeg[at] == 0xFF && (0xE0..=0xEF).contains(&jpeg[at + 1]) {
        at += 2 + u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]) as usize;
    }
    anyhow::ensure!(at <= jpeg.len(), "the JPEG is cut short");
    let mut out = Vec::with_capacity(jpeg.len() + length + 2);
    out.extend_from_slice(&jpeg[..at]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&(length as u16).to_be_bytes());
    out.extend_from_slice(SIGNATURE);
    out.extend_from_slice(packet.as_bytes());
    out.extend_from_slice(&jpeg[at..]);
    Ok(out)
}

/// A PNG with `packet` added as its XMP: an `iTXt` chunk straight after the
/// header, where readers look for it.
pub fn into_png(png: &[u8], packet: &str) -> Result<Vec<u8>> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    anyhow::ensure!(png.starts_with(SIGNATURE) && png.get(12..16) == Some(b"IHDR"), "not a PNG");
    let header_end = 8 + 12 + u32::from_be_bytes(png[8..12].try_into()?) as usize;
    anyhow::ensure!(header_end <= png.len(), "the PNG is cut short");
    // Keyword, then no compression, the method, and no language or translated keyword.
    let mut chunk = b"iTXtXML:com.adobe.xmp\0\0\0\0\0".to_vec();
    chunk.extend_from_slice(packet.as_bytes());
    let mut out = Vec::with_capacity(png.len() + chunk.len() + 8);
    out.extend_from_slice(&png[..header_end]);
    out.extend_from_slice(&((chunk.len() - 4) as u32).to_be_bytes());
    out.extend_from_slice(&chunk);
    out.extend_from_slice(&crc32(&chunk).to_be_bytes());
    out.extend_from_slice(&png[header_end..]);
    Ok(out)
}

/// The CRC that closes each PNG chunk.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_become_a_rating_other_apps_show() {
        let rating = |flag, favorite| Marks { flag, favorite }.rating();
        assert_eq!(rating(0, false), 0);
        assert_eq!(rating(1, false), 0, "a pick is carried by the pick fields, not stars");
        assert_eq!(rating(0, true), 5);
        assert_eq!(rating(1, true), 5);
        assert_eq!(rating(-1, false), -1);
        assert_eq!(rating(-1, true), -1, "a reject stays a reject");
    }

    #[test]
    fn the_packet_holds_each_apps_fields() {
        let pick = packet(Marks { flag: 1, favorite: true }, &Details::default());
        for field in [r#"xmp:Rating="5""#, r#"xmpDM:pick="1""#, r#"xmpDM:good="True""#, r#"digiKam:PickLabel="3""#, TOOLKIT] {
            assert!(pick.contains(field), "{field} is missing from\n{pick}");
        }
        let reject = packet(Marks { flag: -1, favorite: false }, &Details::default());
        for field in [r#"xmp:Rating="-1""#, r#"xmpDM:pick="-1""#, r#"xmpDM:good="False""#, r#"digiKam:PickLabel="1""#] {
            assert!(reject.contains(field), "{field} is missing from\n{reject}");
        }
        let plain = packet(Marks::default(), &Details::default());
        assert!(!plain.contains("xmpDM:pick") && plain.contains(r#"xmp:Rating="0""#));
    }

    #[test]
    fn details_are_written_in_full_and_escaped() {
        let details = Details {
            description: Some("Фотопленка <Svema> & co, frame 3"),
            model: Some("Зенит-E"),
            ..Default::default()
        };
        let packet = packet(Marks::default(), &details);
        assert!(packet.contains("Фотопленка &lt;Svema&gt; &amp; co, frame 3"));
        assert!(packet.contains(r#"tiff:Model="Зенит-E""#));
        assert!(!packet.contains("tiff:Make"), "nothing is written for a detail that isn't known");
    }

    #[test]
    fn the_crc_is_pngs() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }
}
