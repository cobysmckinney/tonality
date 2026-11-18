//! Capture metadata (EXIF) without decoding pixels.

use std::path::Path;

use rawler::decoders::RawDecodeParams;
use rawler::formats::tiff::Rational;
use rawler::rawsource::RawSource;
use tonality_core::library::ImageMeta;

use crate::{RawError, Result};

fn ratio(r: Rational) -> Option<f32> {
    (r.d != 0)
        .then(|| r.n as f32 / r.d as f32)
        .filter(|v| v.is_finite() && *v > 0.0)
}

/// `"2026:11:02 14:03:11"` → `"2026-11-02T14:03:11"`.
fn exif_date(s: &str) -> Option<String> {
    let s = s.trim_end_matches('\0').trim();
    let (date, time) = s.split_once([' ', 'T'])?;
    let date = date.replace(':', "-");
    let time = time.get(..8)?;
    // `YYYY-MM-DD` and `HH:MM:SS`, ASCII only (callers slice these by byte).
    let shaped = |s: &str, seps: [usize; 2], sep: u8| {
        s.bytes().enumerate().all(|(i, b)| {
            if seps.contains(&i) {
                b == sep
            } else {
                b.is_ascii_digit()
            }
        })
    };
    let ok = date.len() == 10
        && shaped(&date, [4, 7], b'-')
        && shaped(time, [2, 5], b':')
        && !date.starts_with("0000");
    ok.then(|| format!("{date}T{time}"))
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    (!s.is_empty()).then(|| s.to_string())
}

pub fn read_meta(path: &Path) -> Result<ImageMeta> {
    let src = RawSource::new(path).map_err(|e| RawError::Decode(e.to_string()))?;
    let decoder = rawler::get_decoder(&src).map_err(|e| RawError::Decode(e.to_string()))?;
    let m = decoder
        .raw_metadata(&src, &RawDecodeParams::default())
        .map_err(|e| RawError::Decode(e.to_string()))?;
    let e = &m.exif;
    Ok(ImageMeta {
        make: m.make.clone(),
        model: m.model.clone(),
        lens: e
            .lens_model
            .as_deref()
            .and_then(non_empty)
            .or_else(|| m.lens.as_ref().and_then(|l| non_empty(&l.lens_model))),
        iso: e
            .iso_speed_ratings
            .map(u32::from)
            .or(e.iso_speed)
            .or(e.recommended_exposure_index)
            .filter(|v| *v > 0),
        exposure_time: e.exposure_time.and_then(ratio),
        f_number: e.fnumber.and_then(ratio),
        focal_length: e.focal_length.and_then(ratio),
        captured_at: e
            .date_time_original
            .as_deref()
            .and_then(exif_date)
            .or_else(|| e.create_date.as_deref().and_then(exif_date)),
        rating: m.rating.map(|r| r.min(5) as u8),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exif_dates() {
        assert_eq!(
            exif_date("2026:11:02 14:03:11").as_deref(),
            Some("2026-11-02T14:03:11")
        );
        assert_eq!(
            exif_date("2026:11:02 14:03:11\0").as_deref(),
            Some("2026-11-02T14:03:11")
        );
        assert_eq!(exif_date("0000:00:00 00:00:00"), None);
        assert_eq!(exif_date("garbage"), None);
        // Multi-byte text across byte 8 of the time must not panic.
        assert_eq!(exif_date("2026:11:02 14:03:1é"), None);
        assert_eq!(exif_date("2026:11:02 1é:03:11"), None);
        assert_eq!(exif_date("2026:1é:02 14:03:11"), None);
        assert_eq!(exif_date("abcd:ef:gh 14:03:11"), None);
    }
}
