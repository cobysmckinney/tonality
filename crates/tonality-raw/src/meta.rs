//! Capture metadata (EXIF) without decoding pixels.

use std::path::Path;

use rawler::decoders::RawDecodeParams;
use rawler::formats::tiff::Rational;
use rawler::rawsource::RawSource;
use tonality_core::exif::{Capture, Gps};
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

/// What the camera recorded, for writing into exported files (`tonality_core::exif`).
/// Rationals stay exact; dates are checked and kept in EXIF form.
pub fn read_capture(path: &Path) -> Result<Capture> {
    let src = RawSource::new(path).map_err(|e| RawError::Decode(e.to_string()))?;
    let decoder = rawler::get_decoder(&src).map_err(|e| RawError::Decode(e.to_string()))?;
    let m = decoder
        .raw_metadata(&src, &RawDecodeParams::default())
        .map_err(|e| RawError::Decode(e.to_string()))?;
    let e = &m.exif;
    let r = |r: Rational| [r.n, r.d];
    let text = |s: &Option<String>| s.as_deref().and_then(non_empty);
    // EXIF form again (`2026:11:02 14:03:11`), after `exif_date` has checked its shape.
    let date = |s: &Option<String>| {
        s.as_deref()
            .and_then(exif_date)
            .map(|iso| iso.replacen('-', ":", 2).replacen('T', " ", 1))
    };
    Ok(Capture {
        make: non_empty(&m.make),
        model: non_empty(&m.model),
        lens_make: text(&e.lens_make),
        lens_model: text(&e.lens_model)
            .or_else(|| m.lens.as_ref().and_then(|l| non_empty(&l.lens_model))),
        exposure_time: e.exposure_time.map(r),
        f_number: e.fnumber.map(r),
        iso: e
            .iso_speed_ratings
            .map(u32::from)
            .or(e.iso_speed)
            .or(e.recommended_exposure_index)
            .filter(|v| *v > 0),
        focal_length: e.focal_length.map(r),
        exposure_bias: e.exposure_bias.map(|v| [v.n, v.d]),
        exposure_program: e.exposure_program,
        metering_mode: e.metering_mode,
        flash: e.flash,
        date_time_original: date(&e.date_time_original).or_else(|| date(&e.create_date)),
        offset_time_original: text(&e.offset_time_original),
        sub_sec_time_original: text(&e.sub_sec_time_original),
        artist: text(&e.artist),
        copyright: text(&e.copyright),
        gps: e.gps.as_ref().map(|g| Gps {
            latitude_ref: text(&g.gps_latitude_ref),
            latitude: g.gps_latitude.map(|v| v.map(r)),
            longitude_ref: text(&g.gps_longitude_ref),
            longitude: g.gps_longitude.map(|v| v.map(r)),
            altitude_ref: g.gps_altitude_ref,
            altitude: g.gps_altitude.map(r),
            time_stamp: g.gps_timestamp.map(|v| v.map(r)),
            date_stamp: text(&g.gps_date_stamp),
        }),
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
