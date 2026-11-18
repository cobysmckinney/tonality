//! EXIF for exported files: what the camera recorded (read from the RAW by
//! `tonality_raw::read_capture`) plus what the export is, written as a TIFF-structured EXIF block
//! (little-endian, no `Exif\0\0` prefix: the JPEG and PNG encoders add their own framing).
//!
//! Written: IFD0 (camera, Orientation 1 since the pixels are already turned and cropped, software,
//! export time, artist, copyright, rating), the Exif IFD (exposure, dates, lens, colour space,
//! pixel size) and, only when asked, the GPS IFD. No maker notes and no serial numbers.

/// A rational as numerator, denominator.
pub type Ratio = [u32; 2];
pub type SRatio = [i32; 2];

/// What the camera recorded, as exactly as the RAW has it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Capture {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens_make: Option<String>,
    pub lens_model: Option<String>,
    pub exposure_time: Option<Ratio>,
    pub f_number: Option<Ratio>,
    pub iso: Option<u32>,
    pub focal_length: Option<Ratio>,
    pub exposure_bias: Option<SRatio>,
    pub exposure_program: Option<u16>,
    pub metering_mode: Option<u16>,
    pub flash: Option<u16>,
    /// EXIF form, `2026:11:02 14:03:11`.
    pub date_time_original: Option<String>,
    /// `+01:00`.
    pub offset_time_original: Option<String>,
    pub sub_sec_time_original: Option<String>,
    pub artist: Option<String>,
    pub copyright: Option<String>,
    pub gps: Option<Gps>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Gps {
    /// `N` / `S`.
    pub latitude_ref: Option<String>,
    /// Degrees, minutes, seconds.
    pub latitude: Option<[Ratio; 3]>,
    /// `E` / `W`.
    pub longitude_ref: Option<String>,
    pub longitude: Option<[Ratio; 3]>,
    /// 0 above sea level, 1 below.
    pub altitude_ref: Option<u8>,
    pub altitude: Option<Ratio>,
    /// UTC hours, minutes, seconds.
    pub time_stamp: Option<[Ratio; 3]>,
    /// `2026:11:02`.
    pub date_stamp: Option<String>,
}

/// What the exported file is.
#[derive(Debug, Clone, PartialEq)]
pub struct Export {
    pub width: u32,
    pub height: u32,
    /// sRGB (EXIF ColorSpace 1); otherwise "uncalibrated" (0xFFFF), as Display P3 files from
    /// Apple's cameras say.
    pub srgb: bool,
    /// tonality's star rating, 1–5; `None` when unrated.
    pub rating: Option<u8>,
    pub include_location: bool,
    /// Local time of the export, EXIF form.
    pub exported_at: String,
}

enum Val {
    Byte(Vec<u8>),
    Ascii(String),
    Short(Vec<u16>),
    Long(Vec<u32>),
    Rational(Vec<Ratio>),
    Undefined(Vec<u8>),
    SRational(Vec<SRatio>),
}

impl Val {
    /// TIFF type code, count, and the value's bytes (little-endian).
    fn encode(&self) -> (u16, u32, Vec<u8>) {
        match self {
            Val::Byte(v) => (1, v.len() as u32, v.clone()),
            Val::Ascii(s) => {
                let mut b = s.as_bytes().to_vec();
                b.push(0);
                (2, b.len() as u32, b)
            }
            Val::Short(v) => (
                3,
                v.len() as u32,
                v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            ),
            Val::Long(v) => (
                4,
                v.len() as u32,
                v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            ),
            Val::Rational(v) => (
                5,
                v.len() as u32,
                v.iter()
                    .flat_map(|[n, d]| n.to_le_bytes().into_iter().chain(d.to_le_bytes()))
                    .collect(),
            ),
            Val::Undefined(v) => (7, v.len() as u32, v.clone()),
            Val::SRational(v) => (
                10,
                v.len() as u32,
                v.iter()
                    .flat_map(|[n, d]| n.to_le_bytes().into_iter().chain(d.to_le_bytes()))
                    .collect(),
            ),
        }
    }
}

type Ifd = Vec<(u16, Val)>;

/// One tag: TIFF type code, count, and its value's bytes (little-endian).
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub tag: u16,
    pub ty: u16,
    pub count: u32,
    pub bytes: Vec<u8>,
}

/// The tags to write, per directory, sorted, without the pointers between them (a TIFF writer
/// places the Exif and GPS directories itself).
#[derive(Debug, Clone, PartialEq)]
pub struct Ifds {
    pub ifd0: Vec<Field>,
    pub exif: Vec<Field>,
    pub gps: Option<Vec<Field>>,
}

pub const EXIF_POINTER: u16 = 0x8769;
pub const GPS_POINTER: u16 = 0x8825;

fn fields(mut ifd: Ifd) -> Vec<Field> {
    ifd.sort_by_key(|(tag, _)| *tag);
    ifd.into_iter()
        .map(|(tag, v)| {
            let (ty, count, bytes) = v.encode();
            Field {
                tag,
                ty,
                count,
                bytes,
            }
        })
        .collect()
}

/// Bytes an IFD takes: count, entries, next-IFD offset, then values over 4 bytes (each padded to
/// an even length).
fn ifd_len(ifd: &[Field]) -> usize {
    let data: usize = ifd
        .iter()
        .map(|f| f.bytes.len())
        .filter(|&n| n > 4)
        .map(|n| n + n % 2)
        .sum();
    2 + 12 * ifd.len() + 4 + data
}

/// `ifd` (sorted) serialised for offset `at` in the block.
fn write_ifd(out: &mut Vec<u8>, ifd: &[Field], at: usize) {
    let mut data_at = at + 2 + 12 * ifd.len() + 4;
    let mut data = Vec::new();
    out.extend_from_slice(&(ifd.len() as u16).to_le_bytes());
    for f in ifd {
        out.extend_from_slice(&f.tag.to_le_bytes());
        out.extend_from_slice(&f.ty.to_le_bytes());
        out.extend_from_slice(&f.count.to_le_bytes());
        if f.bytes.len() <= 4 {
            let mut inline = f.bytes.clone();
            inline.resize(4, 0);
            out.extend_from_slice(&inline);
        } else {
            out.extend_from_slice(&(data_at as u32).to_le_bytes());
            data.extend_from_slice(&f.bytes);
            if f.bytes.len() % 2 == 1 {
                data.push(0);
            }
            data_at += f.bytes.len() + f.bytes.len() % 2;
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data);
}

fn long(tag: u16, v: u32) -> Field {
    Field {
        tag,
        ty: 4,
        count: 1,
        bytes: v.to_le_bytes().to_vec(),
    }
}

fn text(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(|s| s.trim_matches(|c: char| c == '\0' || c.is_whitespace()))
        .filter(|s| !s.is_empty() && s.is_ascii())
        .map(str::to_string)
}

fn valid(r: &Option<Ratio>) -> Option<Ratio> {
    r.filter(|[_, d]| *d != 0)
}

/// Windows' RatingPercent for 1–5 stars.
fn rating_percent(stars: u8) -> u16 {
    [0, 1, 25, 50, 75, 99][stars.min(5) as usize]
}

/// The EXIF block for an exported JPEG or PNG.
pub fn tiff_exif(c: &Capture, e: &Export) -> Vec<u8> {
    let Ifds {
        mut ifd0,
        exif,
        gps,
    } = ifds(c, e);
    // Pointers are 4-byte values, so the layout is known before their values are.
    ifd0.push(long(EXIF_POINTER, 0));
    if gps.is_some() {
        ifd0.push(long(GPS_POINTER, 0));
    }
    ifd0.sort_by_key(|f| f.tag);
    let exif_at = 8 + ifd_len(&ifd0);
    let gps_at = exif_at + ifd_len(&exif);
    for f in ifd0.iter_mut() {
        match f.tag {
            EXIF_POINTER => *f = long(EXIF_POINTER, exif_at as u32),
            GPS_POINTER => *f = long(GPS_POINTER, gps_at as u32),
            _ => {}
        }
    }
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes());
    write_ifd(&mut out, &ifd0, 8);
    debug_assert_eq!(out.len(), exif_at);
    write_ifd(&mut out, &exif, exif_at);
    if let Some(gps) = gps {
        debug_assert_eq!(out.len(), gps_at);
        write_ifd(&mut out, &gps, gps_at);
    }
    out
}

/// The tags for an exported file.
pub fn ifds(c: &Capture, e: &Export) -> Ifds {
    let mut ifd0: Ifd = vec![
        (0x0112, Val::Short(vec![1])),
        (0x0131, Val::Ascii("tonality".into())),
    ];
    let push = |ifd: &mut Ifd, tag: u16, v: Option<Val>| {
        if let Some(v) = v {
            ifd.push((tag, v));
        }
    };
    push(&mut ifd0, 0x010F, text(&c.make).map(Val::Ascii));
    push(&mut ifd0, 0x0110, text(&c.model).map(Val::Ascii));
    push(&mut ifd0, 0x0132, Some(Val::Ascii(e.exported_at.clone())));
    push(&mut ifd0, 0x013B, text(&c.artist).map(Val::Ascii));
    push(&mut ifd0, 0x8298, text(&c.copyright).map(Val::Ascii));
    if let Some(stars) = e.rating.filter(|r| (1..=5).contains(r)) {
        ifd0.push((0x4746, Val::Short(vec![stars as u16])));
        ifd0.push((0x4749, Val::Short(vec![rating_percent(stars)])));
    }

    let mut exif: Ifd = vec![
        (0x9000, Val::Undefined(b"0232".to_vec())),
        (0xA001, Val::Short(vec![if e.srgb { 1 } else { 0xFFFF }])),
        (0xA002, Val::Long(vec![e.width])),
        (0xA003, Val::Long(vec![e.height])),
    ];
    push(
        &mut exif,
        0x829A,
        valid(&c.exposure_time).map(|r| Val::Rational(vec![r])),
    );
    push(
        &mut exif,
        0x829D,
        valid(&c.f_number).map(|r| Val::Rational(vec![r])),
    );
    push(
        &mut exif,
        0x8822,
        c.exposure_program.map(|v| Val::Short(vec![v])),
    );
    push(
        &mut exif,
        0x8827,
        c.iso
            .filter(|&v| v > 0)
            .map(|v| Val::Short(vec![v.min(65535) as u16])),
    );
    let date = text(&c.date_time_original);
    push(&mut exif, 0x9003, date.clone().map(Val::Ascii));
    push(&mut exif, 0x9004, date.map(Val::Ascii));
    let offset = text(&c.offset_time_original);
    push(&mut exif, 0x9011, offset.clone().map(Val::Ascii));
    push(&mut exif, 0x9012, offset.map(Val::Ascii));
    push(
        &mut exif,
        0x9204,
        c.exposure_bias
            .filter(|[_, d]| *d != 0)
            .map(|r| Val::SRational(vec![r])),
    );
    push(
        &mut exif,
        0x9207,
        c.metering_mode.map(|v| Val::Short(vec![v])),
    );
    push(&mut exif, 0x9209, c.flash.map(|v| Val::Short(vec![v])));
    push(
        &mut exif,
        0x920A,
        valid(&c.focal_length).map(|r| Val::Rational(vec![r])),
    );
    let sub = text(&c.sub_sec_time_original);
    push(&mut exif, 0x9291, sub.clone().map(Val::Ascii));
    push(&mut exif, 0x9292, sub.map(Val::Ascii));
    push(&mut exif, 0xA433, text(&c.lens_make).map(Val::Ascii));
    push(&mut exif, 0xA434, text(&c.lens_model).map(Val::Ascii));

    let gps: Option<Ifd> = c.gps.as_ref().filter(|_| e.include_location).and_then(|g| {
        let (lat, lon) = (g.latitude?, g.longitude?);
        let ok = |r: &[Ratio; 3]| r.iter().all(|[_, d]| *d != 0);
        if !ok(&lat) || !ok(&lon) {
            return None;
        }
        let mut ifd: Ifd = vec![
            (0x0000, Val::Byte(vec![2, 3, 0, 0])),
            (
                0x0001,
                Val::Ascii(text(&g.latitude_ref).unwrap_or_else(|| "N".into())),
            ),
            (0x0002, Val::Rational(lat.to_vec())),
            (
                0x0003,
                Val::Ascii(text(&g.longitude_ref).unwrap_or_else(|| "E".into())),
            ),
            (0x0004, Val::Rational(lon.to_vec())),
        ];
        if let Some(alt) = valid(&g.altitude) {
            ifd.push((0x0005, Val::Byte(vec![g.altitude_ref.unwrap_or(0)])));
            ifd.push((0x0006, Val::Rational(vec![alt])));
        }
        if let Some(t) = g.time_stamp.filter(|t| t.iter().all(|[_, d]| *d != 0)) {
            ifd.push((0x0007, Val::Rational(t.to_vec())));
        }
        if let Some(d) = text(&g.date_stamp) {
            ifd.push((0x001D, Val::Ascii(d)));
        }
        Some(ifd)
    });

    Ifds {
        ifd0: fields(ifd0),
        exif: fields(exif),
        gps: gps.map(fields),
    }
}

/// A minimal XMP packet carrying `xmp:Rating` (what Lightroom and most DAMs read).
pub fn xmp_rating(stars: u8) -> String {
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
<rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmp:Rating=\"{}\"/>\
</rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>",
        stars.min(5)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use exif::{In, Reader, Tag, Value};

    fn capture() -> Capture {
        Capture {
            make: Some("SONY".into()),
            model: Some("ILCE-7M3\0".into()),
            lens_make: Some("Sony".into()),
            lens_model: Some("FE 35mm F1.8".into()),
            exposure_time: Some([1, 250]),
            f_number: Some([18, 10]),
            iso: Some(800),
            focal_length: Some([350, 10]),
            exposure_bias: Some([-1, 3]),
            exposure_program: Some(3),
            metering_mode: Some(5),
            flash: Some(16),
            date_time_original: Some("2026:11:02 14:03:11".into()),
            offset_time_original: Some("-08:00".into()),
            sub_sec_time_original: Some("42".into()),
            artist: Some("Coby".into()),
            copyright: Some("(c) 2026".into()),
            gps: Some(Gps {
                latitude_ref: Some("N".into()),
                latitude: Some([[37, 1], [46, 1], [3012, 100]]),
                longitude_ref: Some("W".into()),
                longitude: Some([[122, 1], [25, 1], [912, 100]]),
                altitude_ref: Some(0),
                altitude: Some([152, 10]),
                time_stamp: Some([[22, 1], [3, 1], [11, 1]]),
                date_stamp: Some("2026:11:02".into()),
            }),
        }
    }

    fn export(include_location: bool, rating: Option<u8>) -> Export {
        Export {
            width: 6000,
            height: 4000,
            srgb: false,
            rating,
            include_location,
            exported_at: "2026:11:17 20:00:00".into(),
        }
    }

    fn read(block: &[u8]) -> exif::Exif {
        Reader::new().read_raw(block.to_vec()).expect("valid EXIF")
    }

    fn ascii(x: &exif::Exif, tag: Tag) -> String {
        match &x
            .get_field(tag, In::PRIMARY)
            .unwrap_or_else(|| panic!("{tag} missing"))
            .value
        {
            Value::Ascii(v) => String::from_utf8(v[0].clone()).unwrap(),
            v => panic!("{tag}: {v:?}"),
        }
    }

    fn uint(x: &exif::Exif, tag: Tag) -> u32 {
        x.get_field(tag, In::PRIMARY)
            .unwrap_or_else(|| panic!("{tag} missing"))
            .value
            .get_uint(0)
            .unwrap()
    }

    fn ratio(x: &exif::Exif, tag: Tag) -> Vec<(u32, u32)> {
        match &x
            .get_field(tag, In::PRIMARY)
            .unwrap_or_else(|| panic!("{tag} missing"))
            .value
        {
            Value::Rational(v) => v.iter().map(|r| (r.num, r.denom)).collect(),
            v => panic!("{tag}: {v:?}"),
        }
    }

    #[test]
    fn writes_what_the_camera_recorded() {
        let x = read(&tiff_exif(&capture(), &export(false, Some(4))));
        assert_eq!(ascii(&x, Tag::Make), "SONY");
        assert_eq!(ascii(&x, Tag::Model), "ILCE-7M3");
        assert_eq!(ascii(&x, Tag::Software), "tonality");
        assert_eq!(ascii(&x, Tag::DateTime), "2026:11:17 20:00:00");
        assert_eq!(ascii(&x, Tag::DateTimeOriginal), "2026:11:02 14:03:11");
        assert_eq!(ascii(&x, Tag::DateTimeDigitized), "2026:11:02 14:03:11");
        assert_eq!(ascii(&x, Tag::OffsetTimeOriginal), "-08:00");
        assert_eq!(ascii(&x, Tag::SubSecTimeOriginal), "42");
        assert_eq!(ascii(&x, Tag::Artist), "Coby");
        assert_eq!(ascii(&x, Tag::Copyright), "(c) 2026");
        assert_eq!(ascii(&x, Tag::LensMake), "Sony");
        assert_eq!(ascii(&x, Tag::LensModel), "FE 35mm F1.8");
        assert_eq!(ratio(&x, Tag::ExposureTime), [(1, 250)]);
        assert_eq!(ratio(&x, Tag::FNumber), [(18, 10)]);
        assert_eq!(ratio(&x, Tag::FocalLength), [(350, 10)]);
        assert_eq!(uint(&x, Tag::PhotographicSensitivity), 800);
        assert_eq!(uint(&x, Tag::ExposureProgram), 3);
        assert_eq!(uint(&x, Tag::MeteringMode), 5);
        assert_eq!(uint(&x, Tag::Flash), 16);
        assert_eq!(uint(&x, Tag::Orientation), 1);
        assert_eq!(uint(&x, Tag::ColorSpace), 0xFFFF);
        assert_eq!(uint(&x, Tag::PixelXDimension), 6000);
        assert_eq!(uint(&x, Tag::PixelYDimension), 4000);
        match &x
            .get_field(Tag::ExposureBiasValue, In::PRIMARY)
            .unwrap()
            .value
        {
            Value::SRational(v) => assert_eq!((v[0].num, v[0].denom), (-1, 3)),
            v => panic!("{v:?}"),
        }
        match &x.get_field(Tag::ExifVersion, In::PRIMARY).unwrap().value {
            Value::Undefined(v, _) => assert_eq!(v, b"0232"),
            v => panic!("{v:?}"),
        }
        // Rating: Windows' tags (Lightroom reads XMP, written separately).
        let rating = |tag: u16| {
            x.fields()
                .find(|f| f.tag.number() == tag)
                .map(|f| f.value.get_uint(0).unwrap())
        };
        assert_eq!((rating(0x4746), rating(0x4749)), (Some(4), Some(75)));
        // No location unless asked.
        assert!(x.get_field(Tag::GPSLatitude, In::PRIMARY).is_none());
    }

    #[test]
    fn location_only_when_asked() {
        let x = read(&tiff_exif(&capture(), &export(true, None)));
        assert_eq!(ascii(&x, Tag::GPSLatitudeRef), "N");
        assert_eq!(ratio(&x, Tag::GPSLatitude), [(37, 1), (46, 1), (3012, 100)]);
        assert_eq!(ascii(&x, Tag::GPSLongitudeRef), "W");
        assert_eq!(
            ratio(&x, Tag::GPSLongitude),
            [(122, 1), (25, 1), (912, 100)]
        );
        assert_eq!(ratio(&x, Tag::GPSAltitude), [(152, 10)]);
        assert_eq!(ratio(&x, Tag::GPSTimeStamp), [(22, 1), (3, 1), (11, 1)]);
        assert_eq!(ascii(&x, Tag::GPSDateStamp), "2026:11:02");
        assert!(
            x.fields().all(|f| f.tag.number() != 0x4746),
            "unrated: no rating"
        );
        // Asked, but the camera recorded none.
        let none = Capture {
            gps: None,
            ..capture()
        };
        assert!(read(&tiff_exif(&none, &export(true, None)))
            .get_field(Tag::GPSLatitude, In::PRIMARY)
            .is_none());
    }

    #[test]
    fn an_empty_capture_is_still_valid() {
        let x = read(&tiff_exif(
            &Capture::default(),
            &Export {
                srgb: true,
                ..export(true, None)
            },
        ));
        assert_eq!(ascii(&x, Tag::Software), "tonality");
        assert_eq!(uint(&x, Tag::ColorSpace), 1);
        assert!(x.get_field(Tag::Make, In::PRIMARY).is_none());
        // Text that isn't ASCII or is blank is left out rather than written wrong.
        let odd = Capture {
            artist: Some("Zoë".into()),
            copyright: Some("  \0".into()),
            ..Capture::default()
        };
        let x = read(&tiff_exif(&odd, &export(false, None)));
        assert!(
            x.get_field(Tag::Artist, In::PRIMARY).is_none()
                && x.get_field(Tag::Copyright, In::PRIMARY).is_none()
        );
    }

    #[test]
    fn xmp_rating_packet() {
        let p = xmp_rating(3);
        assert!(
            p.contains("xmp:Rating=\"3\"")
                && p.starts_with("<?xpacket")
                && p.ends_with("<?xpacket end=\"w\"?>")
        );
    }
}
