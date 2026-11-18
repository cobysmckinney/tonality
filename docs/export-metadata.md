# Metadata in exported files

Every export (JPEG, PNG, TIFF, Ultra HDR JPEG) carries EXIF, so galleries sort it by when the
photo was taken and show the camera and settings.

Written (`crates/tonality-core/src/exif.rs`, from `tonality_raw::read_capture`):

- **Camera:** make, model; lens make and model.
- **Exposure:** exposure time, f-number, ISO, focal length, exposure compensation, program,
  metering mode, flash. Rationals are copied as the camera wrote them, not rounded.
- **When:** DateTimeOriginal and DateTimeDigitized (the capture time), its UTC offset and
  sub-seconds; DateTime is the export time.
- **Who:** artist and copyright, if the camera has them set.
- **The file:** Orientation 1 (the pixels are already turned and cropped), pixel size, Software
  "tonality", ColorSpace (1 for sRGB; 0xFFFF "uncalibrated" for Display P3, as Apple's cameras
  write it, with the ICC profile saying which space).
- **Rating:** tonality's stars, as EXIF Rating / RatingPercent (Windows, exiftool, many
  galleries) and `xmp:Rating` (Lightroom and most DAMs) in JPEG and TIFF. PNG has the EXIF ones
  only. Unrated photos carry no rating.
- **Location:** GPS latitude, longitude, altitude, time and date, only when **Location: Include** is
  chosen in the Export dialog (off by default).

Not written: maker notes, serial numbers, and anything the camera didn't record.

**Where it goes.** JPEG: an `Exif` APP1 segment first after SOI, then the XMP, as cameras write
them (no JFIF segment). PNG: an `eXIf` chunk. TIFF: IFD0 tags in the image's directory, the Exif
and GPS directories beside it, XMP in tag 700. Ultra HDR: the same `Exif` segment first, the
rating added to the container's XMP, and the MPF index (which locates the gain map relative to
itself) rewritten for the moved segments (`src-tauri/src/jpeg.rs`).

**Checked by** `exif::tests` and `export::tests`, which read every format back with an independent
reader (kamadak-exif), and once by hand with `exiv2` on real exports.
