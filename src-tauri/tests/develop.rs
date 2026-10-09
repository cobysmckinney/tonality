//! Opening originals for editing: files cut short are refused rather than drawn
//! half blank, and DNGs laid out the way some converters write them still open.

use std::path::PathBuf;

use rawler::rawsource::RawSource;
use tonality_lib::media;

const STRIP_OFFSETS: u16 = 273;
const STRIP_BYTE_COUNTS: u16 = 279;
const SUB_IFDS: u16 = 330;
const LONG: u16 = 4;

/// A little-endian TIFF directory of LONG entries, with no next directory.
fn directory(entries: &[(u16, u32)]) -> Vec<u8> {
    let mut bytes = (entries.len() as u16).to_le_bytes().to_vec();
    for &(tag, value) in entries {
        bytes.extend(tag.to_le_bytes());
        bytes.extend(LONG.to_le_bytes());
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(0u32.to_le_bytes());
    bytes
}

/// A TIFF laid out like a camera file: a header, a preview strip in the
/// first directory, and `data` bytes of sensor data in a directory of its
/// own, either chained after the first or under it as a sub-directory.
fn camera_file(data: u32, as_sub_directory: bool) -> Vec<u8> {
    const PREVIEW: u32 = 64;
    // Header (8), then the first directory, the second, the preview and the data.
    let first_entries = if as_sub_directory { 3 } else { 2 };
    let first_at = 8u32;
    let second_at = first_at + 2 + 12 * first_entries + 4;
    let preview_at = second_at + 2 + 12 * 2 + 4;
    let data_at = preview_at + PREVIEW;

    let mut first = vec![(STRIP_OFFSETS, preview_at), (STRIP_BYTE_COUNTS, PREVIEW)];
    if as_sub_directory {
        first.push((SUB_IFDS, second_at));
    }
    let mut first = directory(&first);
    if !as_sub_directory {
        // Chain the second directory after the first.
        let next = first.len() - 4;
        first[next..].copy_from_slice(&second_at.to_le_bytes());
    }

    let mut bytes = b"II*\0".to_vec();
    bytes.extend(first_at.to_le_bytes());
    bytes.extend(first);
    bytes.extend(directory(&[(STRIP_OFFSETS, data_at), (STRIP_BYTE_COUNTS, data)]));
    assert_eq!(bytes.len() as u32, preview_at);
    bytes.extend(vec![0x80; (PREVIEW + data) as usize]);
    bytes
}

#[test]
fn a_whole_file_passes() {
    for as_sub_directory in [false, true] {
        let file = camera_file(4096, as_sub_directory);
        media::check_whole(&RawSource::new_from_slice(&file)).unwrap();
    }
}

#[test]
fn a_file_cut_short_is_refused_before_it_is_decoded() {
    for as_sub_directory in [false, true] {
        let file = camera_file(4096, as_sub_directory);
        let length = file.len();
        let error = media::check_whole(&RawSource::new_from_slice(&file[..length - 1000])).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("the file is cut short: it ends at byte {}, but its image data runs to byte {length}", length - 1000),
        );
    }
}

#[test]
fn a_file_that_isnt_a_tiff_is_left_to_its_decoder() {
    media::check_whole(&RawSource::new_from_slice(b"ftypcrx not a tiff at all")).unwrap();
    media::check_whole(&RawSource::new_from_slice(b"")).unwrap();
}

/// Cuts each real camera file short and checks it is refused, and that the
/// whole file isn't:
/// `TONALITY_SAMPLES=/path/to/raws cargo test cut_short_samples -- --ignored --nocapture`
#[test]
#[ignore = "needs TONALITY_SAMPLES pointing at a folder of real photos"]
fn cut_short_samples() {
    let samples = PathBuf::from(std::env::var_os("TONALITY_SAMPLES").expect("set TONALITY_SAMPLES"));
    let dir = tempfile::TempDir::new().unwrap();
    for entry in walkdir::WalkDir::new(samples).into_iter().flatten().filter(|e| e.file_type().is_file()) {
        if media::kind_of(entry.path()) != Some(media::Kind::Raw) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let bytes = std::fs::read(entry.path()).unwrap();
        media::check_whole(&RawSource::new_from_slice(&bytes)).unwrap_or_else(|error| panic!("{name}: {error:#}"));
        assert!(tonality_lib::develop::load(entry.path(), true).is_ok(), "{name} whole doesn't open");

        let cut = dir.path().join(&name);
        std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let error = tonality_lib::develop::load(&cut, true).err();
        println!("{name}: cut in half: {:?}", error.as_ref().map(|error| format!("{error:#}")));
        assert!(error.is_some(), "{name} cut in half opens as if whole");
    }
}

/// XYZ to linear sRGB: as a DNG colour matrix, it makes the "camera" see in linear sRGB.
const XYZ_TO_SRGB: [[f32; 3]; 3] = [[3.2406, -1.5372, -0.4986], [-0.9689, 1.8758, 0.0415], [0.0557, -0.2040, 1.0570]];
const SRGB_TO_XYZ: [[f32; 3]; 3] = [[0.4124, 0.3576, 0.1805], [0.2126, 0.7152, 0.0722], [0.0193, 0.1192, 0.9505]];
/// The colour of the light, as the camera sees it: warm.
const LIGHT: [f32; 3] = [0.6, 0.5, 0.3];
const TILE: u32 = 16;
/// How bright each of the four tiles is, left to right and top to bottom.
const TILE_SHADES: [f32; 4] = [1.0, 0.75, 0.5, 0.25];

/// A DNG as converters such as Adobe's write it: 32 x 32 pixels of a grey
/// card under a warm light, in four tiles that are each an 8-bit JPEG; one
/// colour matrix for no particular illuminant; and the white point, if
/// given, as a chromaticity rather than a camera neutral.
fn converted_dng(white_as_xy: bool) -> Vec<u8> {
    use rawler::formats::tiff::writer::{DirectoryWriter, TiffWriter};
    use rawler::formats::tiff::{Rational, SRational};
    use rawler::tags::{DngTag, ExifTag, TiffCommonTag};

    let mut bytes = Vec::new();
    let mut tiff = TiffWriter::new(std::io::Cursor::new(&mut bytes)).unwrap();
    let (mut offsets, mut counts) = (Vec::new(), Vec::new());
    for shade in TILE_SHADES {
        let level = LIGHT.map(|v| (v * shade * 320.0).round() as u8);
        let tile = image::RgbImage::from_pixel(TILE, TILE, image::Rgb(level));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100).encode_image(&tile).unwrap();
        offsets.push(tiff.write_data(&jpeg).unwrap());
        counts.push(jpeg.len() as u32);
    }

    let mut root = DirectoryWriter::new();
    root.add_tag(TiffCommonTag::NewSubFileType, 0u32);
    root.add_tag(TiffCommonTag::ImageWidth, TILE * 2);
    root.add_tag(TiffCommonTag::ImageLength, TILE * 2);
    root.add_tag(TiffCommonTag::BitsPerSample, [8u16; 3]);
    root.add_tag(TiffCommonTag::Compression, 7u16);
    root.add_tag(TiffCommonTag::PhotometricInt, 34892u16); // LinearRaw
    root.add_tag(TiffCommonTag::Make, "Tonality");
    root.add_tag(TiffCommonTag::Model, "Test card");
    root.add_tag(TiffCommonTag::SamplesPerPixel, 3u16);
    root.add_tag(ExifTag::PlanarConfiguration, 1u16);
    root.add_tag(TiffCommonTag::TileWidth, TILE);
    root.add_tag(TiffCommonTag::TileLength, TILE);
    root.add_tag(TiffCommonTag::TileOffsets, &offsets);
    root.add_tag(TiffCommonTag::TileByteCounts, &counts);
    root.add_tag(DngTag::DNGVersion, [1u8, 4, 0, 0]);
    root.add_tag(DngTag::DNGBackwardVersion, [1u8, 1, 0, 0]);
    root.add_tag(DngTag::UniqueCameraModel, "Tonality Test card");
    root.add_tag(DngTag::WhiteLevel, [255u16; 3]);
    // No particular illuminant: the matrix applies to any light.
    root.add_tag(DngTag::CalibrationIlluminant1, 0u16);
    let matrix: Vec<SRational> = XYZ_TO_SRGB.iter().flatten().map(|v| SRational::new((v * 10_000.0).round() as i32, 10_000)).collect();
    root.add_tag(DngTag::ColorMatrix1, matrix.as_slice());
    if white_as_xy {
        let xyz = SRGB_TO_XYZ.map(|row| row.iter().zip(LIGHT).map(|(m, v)| m * v).sum::<f32>());
        let sum: f32 = xyz.iter().sum();
        let xy = [xyz[0] / sum, xyz[1] / sum].map(|v| Rational::new((v * 1_000_000.0).round() as u32, 1_000_000));
        root.add_tag(DngTag::AsShotWhiteXY, xy);
    }
    tiff.build(root).unwrap();
    bytes
}

/// The colour in the middle of each tile of a developed picture.
fn tile_colours(image: &tonality_lib::develop::LinearImage) -> Vec<[f32; 3]> {
    (0..4u32)
        .map(|tile| {
            let (x, y) = ((tile % 2) * TILE + TILE / 2, (tile / 2) * TILE + TILE / 2);
            image.pixels[(y * image.width + x) as usize]
        })
        .collect()
}

#[test]
fn a_dng_whose_tiles_are_jpegs_opens_with_each_tile_in_its_place() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("converted.dng");
    std::fs::write(&path, converted_dng(true)).unwrap();
    let image = tonality_lib::develop::load(&path, true).unwrap();
    assert!(image.scene_referred);
    assert_eq!((image.width, image.height), (TILE * 2, TILE * 2));
    let colours = tile_colours(&image);
    let brightness: Vec<f32> = colours.iter().map(|[r, g, b]| (r + g + b) / 3.0).collect();
    for (tile, shade) in TILE_SHADES.iter().enumerate() {
        let ratio = brightness[tile] / brightness[0];
        assert!((ratio - shade).abs() < 0.03, "tile {tile} is {ratio} as bright as the first, not {shade}: {colours:?}");
    }
}

#[test]
fn a_dng_that_gives_its_white_as_a_chromaticity_is_white_balanced() {
    let dir = tempfile::TempDir::new().unwrap();
    let develop = |white_as_xy: bool| {
        let path = dir.path().join(format!("white-{white_as_xy}.dng"));
        std::fs::write(&path, converted_dng(white_as_xy)).unwrap();
        tile_colours(&tonality_lib::develop::load(&path, true).unwrap())[0]
    };
    // The grey card comes out grey under the warm light...
    let [r, g, b] = develop(true);
    assert!((r / g - 1.0).abs() < 0.03 && (b / g - 1.0).abs() < 0.03, "{:?}", [r, g, b]);
    // ...where without the white point it would keep the light's colour.
    let [r, g, b] = develop(false);
    assert!(r > b * 1.5, "{:?}", [r, g, b]);
}

// ---- developed RAWs kept on disk ----

use tonality_lib::develop::LinearImage;
use tonality_lib::developed::Developed;

fn same(a: &LinearImage, b: &LinearImage) -> bool {
    (a.width, a.height, a.scene_referred) == (b.width, b.height, b.scene_referred) && a.pixels == b.pixels
}

fn developed_now(path: &std::path::Path) -> LinearImage {
    tonality_lib::develop::load(path, true).unwrap()
}

/// A folder with a converted DNG in it, and somewhere to keep developed photos.
fn kept_fixture() -> (tempfile::TempDir, PathBuf, Developed) {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("photo.dng");
    std::fs::write(&path, converted_dng(true)).unwrap();
    let developed = Developed::new(dir.path().join("developed"), 1 << 20);
    (dir, path, developed)
}

#[test]
fn a_developed_raw_is_kept_and_comes_back_exactly_as_developed() {
    let (_dir, path, developed) = kept_fixture();
    assert!(developed.kept(&path).is_none());
    let first = developed.load(&path, true).unwrap();
    developed.finish_writing();
    let kept = developed.kept(&path).expect("kept after the first open");
    let fresh = developed_now(&path);
    assert!(same(&first, &fresh) && same(&kept, &fresh));
    assert!(same(&developed.load(&path, true).unwrap(), &fresh));
}

#[test]
fn a_replaced_original_is_developed_again() {
    let (_dir, path, developed) = kept_fixture();
    let before = developed.load(&path, true).unwrap();
    developed.finish_writing();
    std::fs::write(&path, converted_dng(false)).unwrap();
    assert!(developed.kept(&path).is_none());
    let after = developed.load(&path, true).unwrap();
    assert!(!same(&before, &after));
    assert!(same(&after, &developed_now(&path)));
}

#[cfg(unix)]
#[test]
fn a_file_copied_over_the_original_with_its_size_and_date_is_still_new() {
    let (_dir, path, developed) = kept_fixture();
    developed.load(&path, true).unwrap();
    developed.finish_writing();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, converted_dng(true)).unwrap();
    std::fs::File::options().write(true).open(&path).unwrap().set_modified(modified).unwrap();
    assert!(developed.kept(&path).is_none());
}

#[test]
fn a_damaged_copy_is_developed_again() {
    let (dir, path, developed) = kept_fixture();
    developed.load(&path, true).unwrap();
    developed.finish_writing();
    let entry = std::fs::read_dir(dir.path().join("developed")).unwrap().flatten().next().unwrap().path();
    let bytes = std::fs::read(&entry).unwrap();
    std::fs::write(&entry, &bytes[..bytes.len() - 1]).unwrap();
    assert!(developed.kept(&path).is_none());
    assert!(same(&developed.load(&path, true).unwrap(), &developed_now(&path)));
}

#[test]
fn the_photos_used_longest_ago_are_forgotten_first() {
    let dir = tempfile::TempDir::new().unwrap();
    let photos: Vec<PathBuf> = ["a", "b", "c"].iter().map(|name| dir.path().join(format!("{name}.dng"))).collect();
    for path in &photos {
        std::fs::write(path, converted_dng(true)).unwrap();
    }
    // Room for two: 32 x 32 pixels of three half-floats each, and a header.
    let entry = 32 * 32 * 6 + 16;
    let developed = Developed::new(dir.path().join("developed"), entry * 2 + entry / 2);
    let open = |path: &PathBuf| {
        developed.load(path, true).unwrap();
        developed.finish_writing();
        // Far enough apart for any file system to tell which came first.
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    open(&photos[0]);
    open(&photos[1]);
    // Opening the first again makes the second the one used longest ago.
    open(&photos[0]);
    open(&photos[2]);
    assert!(developed.kept(&photos[0]).is_some());
    assert!(developed.kept(&photos[1]).is_none());
    assert!(developed.kept(&photos[2]).is_some());
}

#[test]
fn photos_that_arent_raw_are_not_kept() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("photo.png");
    image::RgbImage::from_pixel(8, 8, image::Rgb([200, 100, 50])).save(&path).unwrap();
    let developed = Developed::new(dir.path().join("developed"), 1 << 20);
    developed.load(&path, false).unwrap();
    developed.finish_writing();
    assert!(!dir.path().join("developed").exists());
}
