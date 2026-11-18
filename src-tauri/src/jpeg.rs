//! JPEG header surgery for exports: putting EXIF and XMP where readers look for them (right after
//! SOI, as cameras write them), dropping JFIF, and keeping an Ultra HDR file's MPF index right
//! after its segments move.

const SOI: [u8; 2] = [0xFF, 0xD8];
const APP0: u8 = 0xE0;
const APP1: u8 = 0xE1;
const APP2: u8 = 0xE2;
const EXIF_ID: &[u8] = b"Exif\0\0";
const XMP_ID: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const MPF_ID: &[u8] = b"MPF\0";

/// A header segment: marker (the byte after 0xFF) and payload (without the length).
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub marker: u8,
    pub payload: Vec<u8>,
}

impl Segment {
    fn is(&self, marker: u8, id: &[u8]) -> bool {
        self.marker == marker && self.payload.starts_with(id)
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = vec![0xFF, self.marker];
        out.extend_from_slice(&((self.payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&self.payload);
        out
    }
}

/// The APPn / COM segments after SOI, and where the rest of the file (tables, frame, scan)
/// starts.
pub fn split_head(jpeg: &[u8]) -> Result<(Vec<Segment>, usize), String> {
    if !jpeg.starts_with(&SOI) {
        return Err("not a JPEG".into());
    }
    let mut segs = Vec::new();
    let mut at = 2;
    while at + 4 <= jpeg.len() && jpeg[at] == 0xFF {
        let marker = jpeg[at + 1];
        if !((0xE0..=0xEF).contains(&marker) || marker == 0xFE) {
            break;
        }
        let len = u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]) as usize;
        if len < 2 || at + 2 + len > jpeg.len() {
            return Err("truncated JPEG segment".into());
        }
        segs.push(Segment {
            marker,
            payload: jpeg[at + 4..at + 2 + len].to_vec(),
        });
        at += 2 + len;
    }
    Ok((segs, at))
}

fn assemble(segs: &[Segment], rest: &[u8]) -> Vec<u8> {
    let mut out = SOI.to_vec();
    for s in segs {
        out.extend_from_slice(&s.bytes());
    }
    out.extend_from_slice(rest);
    out
}

pub fn exif_segment(tiff_exif: &[u8]) -> Segment {
    Segment {
        marker: APP1,
        payload: [EXIF_ID, tiff_exif].concat(),
    }
}

pub fn xmp_segment(packet: &str) -> Segment {
    Segment {
        marker: APP1,
        payload: [XMP_ID, packet.as_bytes()].concat(),
    }
}

/// `jpeg` without JFIF, with `front` (EXIF, XMP) first after SOI, then its other segments.
pub fn with_front(jpeg: &[u8], front: &[Segment]) -> Result<Vec<u8>, String> {
    let (segs, rest) = split_head(jpeg)?;
    let mut all: Vec<Segment> = front.to_vec();
    all.extend(segs.into_iter().filter(|s| !s.is(APP0, b"JFIF\0")));
    Ok(assemble(&all, &jpeg[rest..]))
}

/// An Ultra HDR file (primary JPEG, then the gain map JPEG, located by the primary's MPF index)
/// with `exif` inserted first after SOI and `rating` added to the primary's XMP; the MPF index
/// is rewritten for the new primary size.
pub fn ultra_hdr_with(
    file: &[u8],
    exif: Option<Segment>,
    rating: Option<u8>,
) -> Result<Vec<u8>, String> {
    let (segs, rest) = split_head(file)?;
    let mpf_i = segs
        .iter()
        .position(|s| s.is(APP2, MPF_ID))
        .ok_or("no MPF index")?;
    let entries = mp_entries(&segs[mpf_i].payload[MPF_ID.len()..])?;
    // Where the gain map starts now: its MPF offset counts from the MPF TIFF header.
    let gain_at = mpf_tiff_at(&segs, mpf_i)
        + entries.read(&segs[mpf_i].payload[MPF_ID.len()..], 1, 8)? as usize;
    if gain_at > file.len() || !file[gain_at..].starts_with(&SOI) {
        return Err("MPF offset doesn't point at the gain map".into());
    }
    let (primary_rest, gain_map) = (&file[rest..gain_at], &file[gain_at..]);

    let mut new_segs: Vec<Segment> = exif.into_iter().collect();
    for mut s in segs {
        if let Some(stars) = rating.filter(|r| (1..=5).contains(r)) {
            if s.is(APP1, XMP_ID) {
                s.payload = add_rating(&s.payload, stars);
            }
        }
        new_segs.push(s);
    }
    let mut primary = assemble(&new_segs, primary_rest);
    let mpf_i = new_segs.iter().position(|s| s.is(APP2, MPF_ID)).unwrap();
    let at = mpf_tiff_at(&new_segs, mpf_i);
    let primary_len = primary.len() as u32;
    let gain_offset = (primary.len() - at) as u32;
    entries.write(&mut primary[at..], 0, 4, primary_len)?;
    entries.write(&mut primary[at..], 1, 8, gain_offset)?;
    primary.extend_from_slice(gain_map);
    Ok(primary)
}

/// `xmp:Rating` added to an XMP segment's first `rdf:Description`.
fn add_rating(payload: &[u8], stars: u8) -> Vec<u8> {
    let text = String::from_utf8_lossy(&payload[XMP_ID.len()..]);
    let tag = "<rdf:Description ";
    if text.contains("xmp:Rating") || !text.contains(tag) {
        return payload.to_vec();
    }
    let with = text.replacen(
        tag,
        &format!("{tag}xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmp:Rating=\"{stars}\" "),
        1,
    );
    [XMP_ID, with.as_bytes()].concat()
}

/// Where the MP entries are in an MPF TIFF block, and its byte order.
#[derive(Debug, Clone, Copy)]
struct MpEntries {
    big: bool,
    /// Offset of the first 16-byte entry from the TIFF header.
    at: usize,
    count: usize,
}

impl MpEntries {
    fn slot(&self, entry: usize, field: usize) -> Result<usize, String> {
        if entry >= self.count {
            return Err("missing MP entry".into());
        }
        Ok(self.at + 16 * entry + field)
    }

    /// Field at byte `field` (4 = size, 8 = offset) of entry `entry`.
    fn read(&self, tiff: &[u8], entry: usize, field: usize) -> Result<u32, String> {
        let i = self.slot(entry, field)?;
        let b: [u8; 4] = tiff.get(i..i + 4).ok_or("short MPF")?.try_into().unwrap();
        Ok(if self.big {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    }

    fn write(&self, tiff: &mut [u8], entry: usize, field: usize, v: u32) -> Result<(), String> {
        let i = self.slot(entry, field)?;
        let b = if self.big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        };
        tiff.get_mut(i..i + 4)
            .ok_or("short MPF")?
            .copy_from_slice(&b);
        Ok(())
    }
}

/// Parse the MPF TIFF block (after `MPF\0`) far enough to find the MP Entry array (tag 0xB002).
fn mp_entries(tiff: &[u8]) -> Result<MpEntries, String> {
    let big = match tiff.get(..2) {
        Some(b"MM") => true,
        Some(b"II") => false,
        _ => return Err("bad MPF byte order".into()),
    };
    let u16_at = |i: usize| -> Result<u16, String> {
        let b: [u8; 2] = tiff.get(i..i + 2).ok_or("short MPF")?.try_into().unwrap();
        Ok(if big {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        })
    };
    let u32_at = |i: usize| -> Result<u32, String> {
        let b: [u8; 4] = tiff.get(i..i + 4).ok_or("short MPF")?.try_into().unwrap();
        Ok(if big {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    };
    let ifd = u32_at(4)? as usize;
    for k in 0..u16_at(ifd)? as usize {
        let e = ifd + 2 + 12 * k;
        if u16_at(e)? == 0xB002 {
            let len = u32_at(e + 4)? as usize;
            let at = u32_at(e + 8)? as usize;
            if !len.is_multiple_of(16) || at + len > tiff.len() {
                return Err("bad MP entries".into());
            }
            return Ok(MpEntries {
                big,
                at,
                count: len / 16,
            });
        }
    }
    Err("no MP entries".into())
}

/// Offset of the MPF TIFF header in a file whose head segments are `segs`, the MPF one at `i`.
fn mpf_tiff_at(segs: &[Segment], i: usize) -> usize {
    2 + segs[..i].iter().map(|s| s.payload.len() + 4).sum::<usize>() + 4 + MPF_ID.len()
}

/// For tests: the primary size an Ultra HDR file's MPF records, and where it says the gain map is.
#[cfg(test)]
pub fn mpf_targets(file: &[u8]) -> (usize, usize) {
    let (segs, _) = split_head(file).unwrap();
    let i = segs.iter().position(|s| s.is(APP2, MPF_ID)).unwrap();
    let tiff = &segs[i].payload[MPF_ID.len()..];
    let e = mp_entries(tiff).unwrap();
    (
        e.read(tiff, 0, 4).unwrap() as usize,
        mpf_tiff_at(&segs, i) + e.read(tiff, 1, 8).unwrap() as usize,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_jpeg() -> Vec<u8> {
        use image::{codecs::jpeg::JpegEncoder, ExtendedColorType, ImageEncoder};
        let mut buf = Vec::new();
        let mut e = JpegEncoder::new_with_quality(&mut buf, 90);
        e.set_icc_profile(vec![7; 40]).unwrap();
        e.write_image(&[128; 4 * 4 * 3], 4, 4, ExtendedColorType::Rgb8)
            .unwrap();
        buf
    }

    #[test]
    fn puts_exif_and_xmp_first_and_drops_jfif() {
        let j = tiny_jpeg();
        let out = with_front(&j, &[exif_segment(b"II*\0abc"), xmp_segment("<x/>")]).unwrap();
        let (segs, _) = split_head(&out).unwrap();
        assert!(segs[0].is(APP1, EXIF_ID) && segs[1].is(APP1, XMP_ID));
        assert!(segs.iter().all(|s| !s.is(APP0, b"JFIF\0")));
        assert!(
            segs.iter().any(|s| s.is(APP2, b"ICC_PROFILE\0")),
            "the encoder's ICC stays"
        );
        assert!(image::load_from_memory(&out).is_ok());
    }

    #[test]
    fn adds_a_rating_to_xmp_once() {
        let x = xmp_segment(
            r#"<x:xmpmeta><rdf:RDF><rdf:Description rdf:about="" a="1"/></rdf:RDF></x:xmpmeta>"#,
        );
        let once = add_rating(&x.payload, 4);
        let text = String::from_utf8_lossy(&once);
        assert!(text.contains(r#"xmp:Rating="4""#) && text.contains("xmlns:xmp="));
        assert_eq!(
            add_rating(&once, 2),
            once,
            "an existing rating is left alone"
        );
    }
}
