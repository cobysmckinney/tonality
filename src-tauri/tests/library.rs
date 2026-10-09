//! End-to-end checks of the library: scan, import, mark, delete.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use image::{Rgb, RgbImage};
use tempfile::TempDir;
use tonality_lib::edit::Adjustments;
use tonality_lib::gpu::LOOK_VERSION;
use tonality_lib::import::{self, ScanSession};
use tonality_lib::library::{self, Library, LibraryInUse, View};
use tonality_lib::thumbs;

/// 2026-03-14 around midday UTC; far enough from midnight to be the same day in any timezone that matters here.
const SHOT_AT: u64 = 1_773_489_600;

fn write_image(path: &Path, shade: u8) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    RgbImage::from_fn(96, 64, |x, y| Rgb([shade, (x * 2) as u8, (y * 3) as u8])).save(path).unwrap();
    set_modified(path);
}

fn write_bytes(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    set_modified(path);
}

fn set_modified(path: &Path) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(SHOT_AT)).unwrap();
}

/// The EXIF of a photo taken at `taken` (`YYYY:MM:DD HH:MM:SS`), as a bare TIFF.
fn exif_taken(taken: &str) -> Vec<u8> {
    let field = exif::Field {
        tag: exif::Tag::DateTimeOriginal,
        ifd_num: exif::In::PRIMARY,
        value: exif::Value::Ascii(vec![taken.as_bytes().to_vec()]),
    };
    let mut writer = exif::experimental::Writer::new();
    writer.push_field(&field);
    let mut tiff = std::io::Cursor::new(Vec::new());
    writer.write(&mut tiff, false).unwrap();
    tiff.into_inner()
}

/// A JPEG that says when it was taken, which is not when its file is dated.
fn write_dated_image(path: &Path, shade: u8, taken: &str) {
    write_image(path, shade);
    let jpeg = fs::read(path).unwrap();
    let exif = exif_taken(taken);
    // An APP1 segment straight after the start-of-image marker.
    let mut dated = jpeg[..2].to_vec();
    dated.extend([0xFF, 0xE1]);
    dated.extend(((exif.len() + 8) as u16).to_be_bytes());
    dated.extend(b"Exif\0\0");
    dated.extend(&exif);
    dated.extend(&jpeg[2..]);
    write_bytes(path, &dated);
}

fn scan(library: &Library, paths: &[PathBuf]) -> ScanSession {
    import::scan(library, 1, paths, None, &|_, _| {}).unwrap()
}

fn import_all(library: &Library, session: &ScanSession) -> import::ImportSummary {
    let all: Vec<usize> = (0..session.items.len()).collect();
    import::run(library, session, &all, &AtomicBool::new(false), &|_, _| {}).unwrap()
}

struct Fixture {
    _dir: TempDir,
    library: Library,
    card: PathBuf,
}

/// A library plus a "card" holding a JPEG, a PNG and a RAW+JPEG pair.
fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let library = Library::open(&dir.path().join("Tonality")).unwrap();
    let card = dir.path().join("card");
    write_image(&card.join("DCIM/100CANON/IMG_0001.JPG"), 10);
    write_image(&card.join("DCIM/100CANON/screenshot.png"), 20);
    // Not a decodable RAW, which is the point: the pair must still import,
    // dated and thumbnailed from its JPEG.
    write_bytes(&card.join("DCIM/100CANON/IMG_0002.CR2"), b"not really a raw file");
    write_image(&card.join("DCIM/100CANON/IMG_0002.JPG"), 30);
    write_bytes(&card.join("DCIM/100CANON/notes.txt"), b"ignored");
    write_image(&card.join("DCIM/.hidden/IMG_0009.JPG"), 40);
    Fixture { _dir: dir, library, card }
}

#[test]
fn scan_pairs_raw_with_jpeg_and_ignores_other_files() {
    let f = fixture();
    let session = scan(&f.library, std::slice::from_ref(&f.card));
    let view = session.view();

    let mut names: Vec<_> = view.items.iter().map(|i| (i.file_name.as_str(), i.kind, i.has_jpeg)).collect();
    names.sort();
    assert_eq!(
        names,
        [("IMG_0001.JPG", "image", false), ("IMG_0002.CR2", "raw", true), ("screenshot.png", "image", false)]
    );
    assert!(view.items.iter().all(|i| i.status == "new"));
    assert_eq!(view.source, "card");
}

#[test]
fn scan_dates_photos_by_when_they_were_taken() {
    let f = fixture();
    // Two more shoots on the card, either side of midnight; every file is dated the same.
    write_dated_image(&f.card.join("DCIM/101CANON/IMG_0100.JPG"), 50, "2026:02:01 23:30:00");
    write_bytes(&f.card.join("DCIM/101CANON/IMG_0101.CR2"), &exif_taken("2026:02:02 00:10:00"));
    // A RAW that says nothing itself goes by the JPEG shot with it.
    write_bytes(&f.card.join("DCIM/101CANON/IMG_0102.CR2"), b"not really a raw file either");
    write_dated_image(&f.card.join("DCIM/101CANON/IMG_0102.JPG"), 60, "2026:02:02 00:20:00");

    let session = scan(&f.library, std::slice::from_ref(&f.card));
    let view = session.view();
    let (undated, dated) = view.items.split_at(3);
    // Newest first, and photos that carry no date go by their file's.
    assert!(undated.iter().all(|i| i.taken_at.starts_with("2026-03-14T")));
    let dated: Vec<_> = dated.iter().map(|i| (i.file_name.as_str(), i.taken_at.as_str())).collect();
    assert_eq!(
        dated,
        [
            ("IMG_0102.CR2", "2026-02-02T00:20:00"),
            ("IMG_0101.CR2", "2026-02-02T00:10:00"),
            ("IMG_0100.JPG", "2026-02-01T23:30:00"),
        ]
    );

    // Each lands in the day the review showed it under.
    import_all(&f.library, &session);
    let originals = f.library.originals_dir();
    assert!(originals.join("2026/2026-02-01/IMG_0100.JPG").is_file());
    assert!(originals.join("2026/2026-02-02/IMG_0101.CR2").is_file());
    assert!(originals.join("2026/2026-02-02/IMG_0102.CR2").is_file());
}

#[test]
fn import_files_photos_by_capture_date_and_then_sees_them_as_duplicates() {
    let f = fixture();
    let session = scan(&f.library, std::slice::from_ref(&f.card));
    let summary = import_all(&f.library, &session);
    assert_eq!((summary.imported, summary.restored, summary.failed.len()), (3, 0, 0));

    let day = f.library.originals_dir().join("2026/2026-03-14");
    for name in ["IMG_0001.JPG", "IMG_0002.CR2", "IMG_0002.JPG", "screenshot.png"] {
        assert!(day.join(name).is_file(), "{name} should be in {}", day.display());
    }
    // The card is left alone.
    assert!(f.card.join("DCIM/100CANON/IMG_0002.CR2").is_file());

    let photos = f.library.list_photos(View::Library).unwrap();
    assert_eq!(photos.len(), 3);
    let pair = photos.iter().find(|p| p.file_name == "IMG_0002.CR2").unwrap();
    assert!(pair.has_jpeg);
    assert!(pair.taken_at.starts_with("2026-03-14T"), "dated from its JPEG: {}", pair.taken_at);
    for photo in &photos {
        assert!(f.library.thumb_path(photo.id).is_file(), "{} should have a thumbnail", photo.file_name);
    }
    let plain = photos.iter().find(|p| p.file_name == "IMG_0001.JPG").unwrap();
    assert_eq!((plain.width, plain.height), (Some(96), Some(64)));
    assert!(thumbs::ensure_preview(&f.library, plain.id).unwrap().is_file());

    let overview = f.library.overview().unwrap();
    assert_eq!((overview.photo_count, overview.imports.len()), (3, 1));

    let again = scan(&f.library, std::slice::from_ref(&f.card));
    assert!(again.view().items.iter().all(|i| i.status == "duplicate"));
    let summary = import_all(&f.library, &again);
    assert_eq!((summary.imported, summary.failed.len()), (0, 3));
    assert_eq!(f.library.overview().unwrap().imports.len(), 1, "an import that adds nothing leaves no trace");
}

#[test]
fn different_photos_with_the_same_name_both_survive() {
    let f = fixture();
    let other = f.card.join("DCIM/101CANON/IMG_0001.JPG");
    write_image(&other, 200);
    let summary = import_all(&f.library, &scan(&f.library, std::slice::from_ref(&f.card)));
    assert_eq!(summary.imported, 4);

    let day = f.library.originals_dir().join("2026/2026-03-14");
    assert!(day.join("IMG_0001.JPG").is_file());
    assert!(day.join("IMG_0001-1.JPG").is_file());
}

#[test]
fn deleted_photos_can_be_recovered_reimported_or_purged() {
    let f = fixture();
    import_all(&f.library, &scan(&f.library, std::slice::from_ref(&f.card)));
    let ids: Vec<i64> = f.library.list_photos(View::Library).unwrap().iter().map(|p| p.id).collect();

    // Purging only ever touches photos that are in Recently Deleted.
    assert_eq!(f.library.purge(&ids).unwrap(), 0);
    assert_eq!(f.library.list_photos(View::Library).unwrap().len(), 3);

    f.library.trash(&ids).unwrap();
    assert_eq!(f.library.list_photos(View::Library).unwrap().len(), 0);
    assert_eq!(f.library.list_photos(View::Deleted).unwrap().len(), 3);
    assert_eq!(f.library.purge_expired().unwrap(), 0, "fresh deletions are inside the retention period");

    // Importing a deleted photo again brings the existing one back.
    let session = scan(&f.library, std::slice::from_ref(&f.card));
    assert!(session.view().items.iter().all(|i| i.status == "deleted"));
    let summary = import_all(&f.library, &session);
    assert_eq!((summary.imported, summary.restored), (0, 3));
    assert_eq!(f.library.list_photos(View::Library).unwrap().len(), 3);

    f.library.trash(&ids).unwrap();
    f.library.restore(&ids[..1]).unwrap();
    assert_eq!(f.library.purge(&ids).unwrap(), 2);
    let left = f.library.list_photos(View::Library).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(f.library.list_photos(View::Deleted).unwrap().len(), 0);

    let day = f.library.originals_dir().join("2026/2026-03-14");
    let remaining: Vec<_> = fs::read_dir(&day).unwrap().flatten().map(|e| e.file_name()).collect();
    let expected = if left[0].has_jpeg { 2 } else { 1 };
    assert_eq!(remaining.len(), expected, "only the kept photo's files remain: {remaining:?}");
    for id in &ids {
        assert_eq!(f.library.thumb_path(*id).exists(), *id == left[0].id);
    }
}

#[test]
fn expired_photos_are_purged_while_the_app_is_open() {
    let f = fixture();
    import_all(&f.library, &scan(&f.library, std::slice::from_ref(&f.card)));
    let ids: Vec<i64> = f.library.list_photos(View::Library).unwrap().iter().map(|p| p.id).collect();
    f.library.trash(&ids).unwrap();
    let expired = ids[0];
    let original = f.library.photo_files(expired).unwrap().path;

    // Backdate one deletion past the retention period, as if the app had been open that long.
    let past = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() as i64
        - (library::TRASH_RETENTION_DAYS * 24 * 60 * 60 + 60);
    let db = rusqlite::Connection::open(f.library.root().join(".tonality/library.db")).unwrap();
    db.execute("UPDATE photos SET deleted_at = ?1 WHERE id = ?2", [past, expired]).unwrap();

    // Looking at Recently Deleted removes it for good, and the others stay.
    let deleted: Vec<i64> = f.library.list_photos(View::Deleted).unwrap().iter().map(|p| p.id).collect();
    assert_eq!(deleted.len(), 2);
    assert!(!deleted.contains(&expired));
    assert!(!original.exists(), "its original is gone");
    assert!(!f.library.thumb_path(expired).exists());

    // So does the sidebar's count.
    db.execute("UPDATE photos SET deleted_at = ?1 WHERE id = ?2", [past, deleted[0]]).unwrap();
    assert_eq!(f.library.overview().unwrap().deleted_count, 1);
}

#[test]
fn originals_moved_or_deleted_outside_the_app_are_noticed() {
    let f = fixture();
    import_all(&f.library, &scan(&f.library, std::slice::from_ref(&f.card)));
    let photos = f.library.list_photos(View::Library).unwrap();
    assert!(photos.iter().all(|p| !p.missing));

    let gone = photos.iter().find(|p| p.file_name == "IMG_0001.JPG").unwrap().id;
    let original = f.library.photo_files(gone).unwrap().path;
    let moved = f.library.root().join("elsewhere.jpg");
    fs::rename(&original, &moved).unwrap();
    for photo in f.library.list_photos(View::Library).unwrap() {
        assert_eq!(photo.missing, photo.id == gone, "{}", photo.file_name);
    }
    let error = format!("{:#}", f.library.photo_files(gone).unwrap_err());
    assert!(error.contains("missing from the library folder"), "{error}");
    assert!(error.contains("IMG_0001.JPG"), "names the file: {error}");

    // Putting it back is all it takes.
    fs::rename(&moved, &original).unwrap();
    assert!(f.library.list_photos(View::Library).unwrap().iter().all(|p| !p.missing));
    assert!(f.library.photo_files(gone).is_ok());
}

#[test]
fn a_damaged_original_says_so_in_plain_words() {
    let f = fixture();
    import_all(&f.library, &scan(&f.library, std::slice::from_ref(&f.card)));
    // The pair's RAW isn't really one, as if it had been damaged after import.
    let photos = f.library.list_photos(View::Library).unwrap();
    let damaged = photos.iter().find(|p| p.file_name == "IMG_0002.CR2").unwrap().id;
    // What the editor and every export of the photo are told.
    let error = format!("{:#}", thumbs::open_session(&f.library, damaged).err().unwrap());
    assert_eq!(
        error,
        "The original file (IMG_0002.CR2) is damaged, or isn't a photo Tonality can read. \
         If it's still on the camera card, deleting this photo permanently and importing it again may help."
    );
}

#[test]
fn purging_a_photo_whose_original_is_already_gone_finishes_the_job() {
    let f = fixture();
    import_all(&f.library, &scan(&f.library, std::slice::from_ref(&f.card)));
    let id = f.library.list_photos(View::Library).unwrap()[0].id;
    fs::remove_file(f.library.photo_files(id).unwrap().path).unwrap();
    f.library.trash(&[id]).unwrap();
    assert_eq!(f.library.purge(&[id]).unwrap(), 1);
    assert!(f.library.list_photos(View::Deleted).unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn a_photo_whose_original_cant_be_deleted_stays_in_recently_deleted() {
    use std::os::unix::fs::PermissionsExt;

    let f = fixture();
    import_all(&f.library, &scan(&f.library, std::slice::from_ref(&f.card)));
    let ids: Vec<i64> = f.library.list_photos(View::Library).unwrap().iter().map(|p| p.id).collect();
    let day = f.library.originals_dir().join("2026/2026-03-14");
    let files = fs::read_dir(&day).unwrap().count();
    fs::set_permissions(&day, fs::Permissions::from_mode(0o555)).unwrap();

    f.library.trash(&ids).unwrap();
    let result = f.library.purge(&ids);
    fs::set_permissions(&day, fs::Permissions::from_mode(0o755)).unwrap();
    if fs::read_dir(&day).unwrap().count() < files {
        eprintln!("skipping: running as a user that can delete from read-only folders");
        return;
    }
    let error = format!("{:#}", result.unwrap_err());
    assert!(error.contains("Couldn’t delete 3 photos, so they stay in Recently Deleted"), "{error}");
    assert!(error.contains("IMG_0001.JPG"), "names the files: {error}");
    assert_eq!(f.library.list_photos(View::Deleted).unwrap().len(), 3, "still tracked");
    for id in &ids {
        assert!(f.library.thumb_path(*id).exists(), "thumbnails kept while the photo is");
    }

    // Once the files can go, the next purge takes them.
    assert_eq!(f.library.purge(&ids).unwrap(), 3);
    assert!(!day.exists());
}

#[test]
fn favorites_flags_and_albums() {
    let f = fixture();
    import_all(&f.library, &scan(&f.library, std::slice::from_ref(&f.card)));
    let ids: Vec<i64> = f.library.list_photos(View::Library).unwrap().iter().map(|p| p.id).collect();

    f.library.set_favorite(&ids[..2], true).unwrap();
    f.library.set_flag(&ids[..1], 1).unwrap();
    f.library.set_flag(&ids[2..], -1).unwrap();
    assert_eq!(f.library.list_photos(View::Favorites).unwrap().len(), 2);
    let flags: Vec<i8> = f.library.list_photos(View::Library).unwrap().iter().map(|p| p.flag).collect();
    assert_eq!(flags, [1, 0, -1]);

    let album = f.library.create_album("  Portfolio ").unwrap();
    f.library.add_to_album(album, &ids).unwrap();
    f.library.add_to_album(album, &ids[..1]).unwrap();
    assert_eq!(f.library.list_photos(View::Album { id: album }).unwrap().len(), 3);
    assert_eq!(f.library.photo_info(ids[0]).unwrap().albums, ["Portfolio"]);

    f.library.remove_from_album(album, &ids[..1]).unwrap();
    f.library.trash(&ids[1..2]).unwrap();
    let overview = f.library.overview().unwrap();
    assert_eq!((overview.albums[0].name.as_str(), overview.albums[0].count), ("Portfolio", 1));
    assert_eq!((overview.photo_count, overview.favorite_count, overview.deleted_count), (2, 1, 1));

    f.library.delete_album(album).unwrap();
    assert_eq!(f.library.list_photos(View::Library).unwrap().len(), 2, "deleting an album keeps its photos");
}

#[test]
fn a_library_opens_in_one_copy_at_a_time() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("Tonality");
    let library = Library::open(&root).unwrap();
    let error = Library::open(&root).err().expect("a second open is refused while the first is open");
    assert!(error.is::<LibraryInUse>(), "unexpected error: {error:#}");
    drop(library);
    Library::open(&root).expect("the library opens again once the first copy closes");
}

#[test]
fn an_update_that_changes_the_look_redraws_edited_photos_only() {
    let Fixture { _dir, library, card } = fixture();
    import_all(&library, &scan(&library, &[card]));
    let photos = library.list_photos(View::Library).unwrap();
    let (edited, plain) = (photos[0].id, photos[1].id);
    library.history_commit(edited, &Adjustments { exposure: 1.0, ..Default::default() }, "Exposure +1.00").unwrap();
    // Stand-ins for images drawn earlier.
    let edited_images = [library.thumb_path(edited), library.preview_path(edited)];
    let plain_images = [library.thumb_path(plain), library.preview_path(plain)];
    for path in edited_images.iter().chain(&plain_images) {
        write_bytes(path, b"drawn earlier");
    }
    let root = library.root().to_path_buf();
    let version = |library: &Library, id| library.list_photos(View::Library).unwrap().iter().find(|p| p.id == id).unwrap().version;
    let before = version(&library, edited);

    // Opening again with the same look keeps them all.
    drop(library);
    let library = Library::open(&root).unwrap();
    assert!(edited_images.iter().chain(&plain_images).all(|path| path.is_file()));
    assert_eq!(version(&library, edited), before);

    // As if they were drawn by an older version.
    library.set_setting(thumbs::LOOK_SETTING, "0").unwrap();
    drop(library);
    let library = Library::open(&root).unwrap();
    assert!(edited_images.iter().all(|path| !path.exists()), "the edited photo's images are dropped");
    assert!(plain_images.iter().all(|path| path.is_file()), "the camera's own picture is kept");
    assert!(version(&library, edited) > before, "so the window doesn't show a copy it kept");
    assert_eq!(library.setting(thumbs::LOOK_SETTING).unwrap(), Some(LOOK_VERSION.to_string()));
}

#[test]
fn an_import_cancelled_before_it_starts_leaves_no_trace() {
    let f = fixture();
    let session = scan(&f.library, std::slice::from_ref(&f.card));
    let all: Vec<usize> = (0..session.items.len()).collect();
    let summary = import::run(&f.library, &session, &all, &AtomicBool::new(true), &|_, _| {}).unwrap();
    assert!(summary.cancelled);
    assert_eq!((summary.imported, summary.restored, summary.failed.len()), (0, 0, 0));
    assert!(f.library.list_photos(View::Library).unwrap().is_empty());
    assert!(f.library.list_photos(View::Imports).unwrap().is_empty());
    let db = rusqlite::Connection::open(f.library.root().join(".tonality/library.db")).unwrap();
    let imports: i64 = db.query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0)).unwrap();
    assert_eq!(imports, 0, "an import with nothing in it isn't kept");
    assert!(!f.library.incoming_dir().exists());
    assert_eq!(fs::read_dir(f.library.originals_dir()).unwrap().count(), 0);
}

#[test]
fn an_import_stopped_partway_keeps_what_it_finished_and_can_pick_up_the_rest() {
    let dir = TempDir::new().unwrap();
    let library = Library::open(&dir.path().join("Tonality")).unwrap();
    let card = dir.path().join("card");
    const PHOTOS: u32 = 24;
    for i in 0..PHOTOS {
        write_image(&card.join(format!("DCIM/100CANON/IMG_{i:04}.JPG")), i as u8 * 10);
    }
    let session = scan(&library, std::slice::from_ref(&card));
    let all: Vec<usize> = (0..session.items.len()).collect();
    assert_eq!(all.len(), PHOTOS as usize);

    // Stop as soon as the first photo is in.
    let cancel = AtomicBool::new(false);
    let summary = import::run(&library, &session, &all, &cancel, &|_, _| cancel.store(true, Ordering::Relaxed)).unwrap();
    assert!(summary.cancelled);
    assert!(summary.failed.is_empty(), "{:?}", summary.failed.iter().map(|f| &f.reason).collect::<Vec<_>>());
    // Photos already on their way when it stopped are finished, not left half copied.
    assert!((1..PHOTOS / 2).contains(&summary.imported), "{} imported", summary.imported);
    let photos = library.list_photos(View::Library).unwrap();
    assert_eq!(photos.len() as u32, summary.imported);
    for photo in &photos {
        assert!(library.photo_files(photo.id).unwrap().path.exists());
    }
    assert!(!library.incoming_dir().exists(), "nothing is left in the holding folder");

    // Scanning the card again offers just the rest, and importing them finishes the job.
    let session = scan(&library, std::slice::from_ref(&card));
    let new = session.view().items.iter().filter(|item| item.status == "new").count() as u32;
    assert_eq!(new, PHOTOS - summary.imported);
    let summary = import_all(&library, &session);
    assert_eq!(summary.imported, PHOTOS - photos.len() as u32);
    assert_eq!(library.list_photos(View::Library).unwrap().len() as u32, PHOTOS);
}

#[test]
fn purging_a_photo_takes_its_cached_mattes_with_it() {
    let f = fixture();
    import_all(&f.library, &scan(&f.library, std::slice::from_ref(&f.card)));
    let ids: Vec<i64> = f.library.list_photos(View::Library).unwrap().iter().map(|p| p.id).collect();
    let matte = |id: i64, key: &str| {
        let start = f.library.matte_files(id);
        start.with_file_name(format!("{}{key}.png", start.file_name().unwrap().to_string_lossy()))
    };
    for &id in &ids {
        fs::create_dir_all(matte(id, "subject").parent().unwrap()).unwrap();
        for key in ["subject-x", "sky-y"] {
            image::GrayImage::new(4, 4).save(matte(id, key)).unwrap();
        }
    }
    f.library.trash(&ids[..1]).unwrap();
    assert_eq!(f.library.purge(&ids[..1]).unwrap(), 1);
    assert!(!matte(ids[0], "subject-x").exists() && !matte(ids[0], "sky-y").exists());
    for &id in &ids[1..] {
        assert!(matte(id, "subject-x").exists(), "other photos keep theirs");
    }
}

#[test]
fn a_deleted_photo_stays_until_its_time_is_up() {
    let f = fixture();
    import_all(&f.library, &scan(&f.library, std::slice::from_ref(&f.card)));
    let ids: Vec<i64> = f.library.list_photos(View::Library).unwrap().iter().map(|p| p.id).collect();
    f.library.trash(&ids).unwrap();
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() as i64;
    let retention = library::TRASH_RETENTION_DAYS * 24 * 60 * 60;
    let db = rusqlite::Connection::open(f.library.root().join(".tonality/library.db")).unwrap();
    // One a minute short of the limit, one a minute past it.
    db.execute("UPDATE photos SET deleted_at = ?1 WHERE id = ?2", [now - retention + 60, ids[0]]).unwrap();
    db.execute("UPDATE photos SET deleted_at = ?1 WHERE id = ?2", [now - retention - 60, ids[1]]).unwrap();
    assert_eq!(f.library.purge_expired().unwrap(), 1);
    let left: Vec<i64> = f.library.list_photos(View::Deleted).unwrap().iter().map(|p| p.id).collect();
    assert!(left.contains(&ids[0]) && !left.contains(&ids[1]), "{left:?}");
}

/// The library as the first version of Tonality made it.
const FIRST_SCHEMA: &str = "
CREATE TABLE imports (id INTEGER PRIMARY KEY, created_at INTEGER NOT NULL, source TEXT NOT NULL);
CREATE TABLE photos (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    path          TEXT NOT NULL UNIQUE,
    jpeg_path     TEXT,
    file_name     TEXT NOT NULL,
    kind          TEXT NOT NULL,
    fingerprint   TEXT NOT NULL,
    file_size     INTEGER NOT NULL,
    taken_at      TEXT NOT NULL,
    width         INTEGER,
    height        INTEGER,
    make          TEXT,
    model         TEXT,
    lens          TEXT,
    iso           INTEGER,
    aperture      REAL,
    shutter       REAL,
    focal_length  REAL,
    favorite      INTEGER NOT NULL DEFAULT 0,
    flag          INTEGER NOT NULL DEFAULT 0,
    import_id     INTEGER NOT NULL REFERENCES imports(id),
    deleted_at    INTEGER
);
CREATE INDEX photos_taken_at ON photos(taken_at);
CREATE INDEX photos_fingerprint ON photos(fingerprint);
CREATE INDEX photos_import ON photos(import_id);
CREATE TABLE albums (id INTEGER PRIMARY KEY, name TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE album_photos (
    album_id  INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
    photo_id  INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    PRIMARY KEY (album_id, photo_id)
) WITHOUT ROWID;
CREATE INDEX album_photos_photo ON album_photos(photo_id);
PRAGMA user_version = 1;
";

#[test]
fn a_library_from_the_first_version_is_brought_up_to_date() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("Tonality");
    let original = root.join("Originals/2026/2026-03-14/IMG_0001.png");
    write_image(&original, 50);
    fs::create_dir_all(root.join(".tonality")).unwrap();
    {
        let db = rusqlite::Connection::open(root.join(".tonality/library.db")).unwrap();
        db.execute_batch(FIRST_SCHEMA).unwrap();
        db.execute_batch(
            "INSERT INTO imports (id, created_at, source) VALUES (1, 1773489600, '/media/card');
             INSERT INTO photos (path, file_name, kind, fingerprint, file_size, taken_at, width, height, favorite, import_id)
                 VALUES ('Originals/2026/2026-03-14/IMG_0001.png', 'IMG_0001.png', 'image', 'abc', 100,
                         '2026-03-14T12:00:00', 96, 64, 1, 1);
             INSERT INTO albums (id, name, created_at) VALUES (1, 'Trip', 1773489600);
             INSERT INTO album_photos (album_id, photo_id) VALUES (1, 1);",
        )
        .unwrap();
    }

    let library = Library::open(&root).unwrap();
    let photos = library.list_photos(View::Library).unwrap();
    assert_eq!(photos.len(), 1, "photos survive the upgrade");
    let id = photos[0].id;
    assert!(photos[0].favorite);
    assert_eq!(library.list_photos(View::Album { id: 1 }).unwrap().len(), 1, "and so do albums");
    assert_eq!(library.edits(id).unwrap(), None, "an old photo is as shot");

    // Everything later versions added works.
    let edited = Adjustments { exposure: 0.5, ..Default::default() };
    let history = library.history_commit(id, &edited, "Exposure +0.50").unwrap();
    assert!(history.steps.len() >= 2, "edits are kept as history");
    assert!(library.edits(id).unwrap().is_some());
    library.set_setting("test", "1").unwrap();
    assert_eq!(library.setting("test").unwrap().as_deref(), Some("1"));
    let bright = tonality_lib::presets::Settings { exposure: Some(0.3), ..Default::default() };
    let preset = library.create_preset("Bright", bright).unwrap();
    assert_eq!(library.set_preset_favorite(preset.id, true).unwrap(), [preset.id]);
    drop(library);

    // Opening it again changes nothing.
    let db = rusqlite::Connection::open(root.join(".tonality/library.db")).unwrap();
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert!(version >= 6, "at version {version}");
    let library = Library::open(&root).unwrap();
    assert!(library.edits(id).unwrap().is_some());
    assert_eq!(library.favorite_presets().unwrap(), [preset.id]);
}

/// Runs the real decoders over a folder of camera files:
/// `TONALITY_SAMPLES=/path/to/raws cargo test real_samples -- --ignored --nocapture`
#[test]
#[ignore = "needs TONALITY_SAMPLES pointing at a folder of real photos"]
fn real_samples() {
    let samples = PathBuf::from(std::env::var_os("TONALITY_SAMPLES").expect("set TONALITY_SAMPLES"));
    let dir = TempDir::new().unwrap();
    let library = Library::open(&dir.path().join("Tonality")).unwrap();
    tonality_lib::media::init();

    let session = scan(&library, &[samples]);
    let started = std::time::Instant::now();
    let summary = import_all(&library, &session);
    println!("imported {} in {:?}, failed: {:?}", summary.imported, started.elapsed(), summary.failed);

    for photo in library.list_photos(View::Library).unwrap() {
        let info = library.photo_info(photo.id).unwrap();
        let started = std::time::Instant::now();
        let preview = thumbs::ensure_preview(&library, photo.id);
        println!(
            "{:<22} {} {:?}x{:?} {:?} {:?} | {:?} iso {:?} f/{:?} {:?}s {:?}mm | thumb {} preview {:?} in {:?}",
            photo.file_name,
            photo.taken_at,
            photo.width,
            photo.height,
            info.make,
            info.model,
            info.lens,
            info.iso,
            info.aperture,
            info.shutter,
            info.focal_length,
            library.thumb_path(photo.id).is_file(),
            preview.as_ref().map(|p| image::image_dimensions(p).unwrap()).map_err(|e| format!("{e:#}")),
            started.elapsed(),
        );
        assert!(library.thumb_path(photo.id).is_file(), "{} has no thumbnail", photo.file_name);
        assert!(preview.is_ok(), "{} has no preview", photo.file_name);
    }
    if let Some(keep) = std::env::var_os("TONALITY_KEEP") {
        let keep = PathBuf::from(keep);
        let _ = fs::remove_dir_all(&keep);
        fs::rename(dir.path().join("Tonality"), keep).unwrap();
    }
}

/// Times the editor's full-resolution load: `TONALITY_SAMPLES=… cargo test develop_timing -- --ignored --nocapture`
#[test]
#[ignore = "needs TONALITY_SAMPLES pointing at a folder of real photos"]
fn develop_timing() {
    let samples = PathBuf::from(std::env::var_os("TONALITY_SAMPLES").expect("set TONALITY_SAMPLES"));
    for entry in walkdir::WalkDir::new(samples).into_iter().flatten().filter(|e| e.file_type().is_file()) {
        let Some(kind) = tonality_lib::media::kind_of(entry.path()) else { continue };
        let started = std::time::Instant::now();
        let image = tonality_lib::develop::load(entry.path(), kind == tonality_lib::media::Kind::Raw).unwrap();
        let mean = image.pixels.iter().map(|p| (p[0] + p[1] + p[2]) as f64 / 3.0).sum::<f64>() / image.pixels.len() as f64;
        println!("{:<20} {}x{} in {:?}, mean {:.4}", entry.file_name().to_string_lossy(), image.width, image.height, started.elapsed(), mean);
    }
}

/// Renders real photos through the GPU pipeline and saves the results, next
/// to the camera's own JPEG, for a look:
/// `TONALITY_SAMPLES=… TONALITY_OUT=… cargo test gpu_render -- --ignored --nocapture`
#[test]
#[ignore = "needs TONALITY_SAMPLES and a GPU"]
fn gpu_render() {
    use tonality_lib::edit::Adjustments;
    let samples = PathBuf::from(std::env::var_os("TONALITY_SAMPLES").expect("set TONALITY_SAMPLES"));
    let out = PathBuf::from(std::env::var_os("TONALITY_OUT").expect("set TONALITY_OUT"));
    fs::create_dir_all(&out).unwrap();
    let recipe: Adjustments =
        std::env::var("TONALITY_RECIPE").map(|json| serde_json::from_str(&json).unwrap()).unwrap_or_default();

    let started = std::time::Instant::now();
    let gpu = tonality_lib::gpu::Gpu::new().unwrap();
    println!("gpu ready in {:?}", started.elapsed());

    for entry in walkdir::WalkDir::new(samples).into_iter().flatten().filter(|e| e.file_type().is_file()) {
        let Some(kind) = tonality_lib::media::kind_of(entry.path()) else { continue };
        let is_raw = kind == tonality_lib::media::Kind::Raw;
        let name = entry.path().file_stem().unwrap().to_string_lossy().into_owned();

        let started = std::time::Instant::now();
        let image = tonality_lib::develop::load(entry.path(), is_raw).unwrap();
        let loaded = started.elapsed();
        let session = gpu.open(image).unwrap();
        let opened = started.elapsed();
        let frame = gpu.render_image(&session, &recipe, 1600).unwrap();
        let first = started.elapsed();
        let again = std::time::Instant::now();
        for _ in 0..5 {
            gpu.render_image(&session, &recipe, 1600).unwrap();
        }
        println!(
            "{name:<16} load {loaded:?}, upload {:?}, first frame {:?}, then {:?} per frame",
            opened - loaded,
            first - opened,
            again.elapsed() / 5
        );
        frame.save(out.join(format!("{name}-ours.jpg"))).unwrap();
        let camera = tonality_lib::media::render(entry.path(), is_raw, None, 1600).unwrap();
        camera.save(out.join(format!("{name}-camera.jpg"))).unwrap();
    }
}

