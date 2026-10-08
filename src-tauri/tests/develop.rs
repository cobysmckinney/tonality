//! Opening originals for editing: files cut short are refused rather than drawn half blank.

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
