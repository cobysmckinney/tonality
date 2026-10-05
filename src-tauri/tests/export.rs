//! Exporting: where the files go, what they are called, which edits they
//! hold, and what the library remembers. The drawing itself is stood in for
//! here (tests/editor.rs covers the real pipeline), so these run anywhere.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use image::{DynamicImage, ImageBuffer, Rgb, RgbImage};
use tempfile::TempDir;
use tonality_lib::edit::Adjustments;
use tonality_lib::export::{self, Branches, Format, Job, Settings, Summary};
use tonality_lib::import;
use tonality_lib::library::{Library, View};

struct Fixture {
    dir: TempDir,
    library: Library,
    /// IMG_0462, then IMG_0463.
    photos: Vec<i64>,
}

/// A library holding two photos.
fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let library = Library::open(&dir.path().join("Tonality")).unwrap();
    let card = dir.path().join("card");
    fs::create_dir_all(&card).unwrap();
    // Different pictures: identical files would be taken for one photo.
    RgbImage::from_pixel(8, 8, Rgb([40, 40, 40])).save(card.join("IMG_0462.png")).unwrap();
    RgbImage::from_pixel(8, 8, Rgb([90, 90, 90])).save(card.join("IMG_0463.png")).unwrap();
    let session = import::scan(&library, 1, &[card], None, &|_, _| {}).unwrap();
    let all: Vec<usize> = (0..session.items.len()).collect();
    import::run(&library, &session, &all, &AtomicBool::new(false), &|_, _| {}).unwrap();
    let mut photos = library.list_photos(View::Library).unwrap();
    photos.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Fixture { dir, library, photos: photos.iter().map(|photo| photo.id).collect() }
}

fn exposure(stops: f32) -> Adjustments {
    Adjustments { exposure: stops, ..Default::default() }
}

/// Stands in for the GPU: a picture whose red channel says which recipe it
/// was drawn with (ten per stop of exposure, or a thousand in a deep
/// picture) and whose width is the size asked for.
fn draw(_id: i64, recipe: &Adjustments, long_edge: u32, deep: bool) -> anyhow::Result<DynamicImage> {
    let width = long_edge.min(12);
    Ok(match deep {
        true => ImageBuffer::from_pixel(width, 8, Rgb([(recipe.exposure * 1000.0) as u16, 0, 0])).into(),
        false => RgbImage::from_pixel(width, 8, Rgb([(recipe.exposure * 10.0) as u8, 0, 0])).into(),
    })
}

fn png() -> Settings {
    Settings { format: Format::Png, ..Default::default() }
}

/// Which recipe an exported PNG was drawn with, in stops of exposure.
fn stops_in(path: &str) -> f32 {
    image::open(path).unwrap().into_rgb8().get_pixel(0, 0).0[0] as f32 / 10.0
}

impl Fixture {
    /// Exports photos' current branches, or one particular branch of a single photo.
    fn export(&self, ids: &[i64], branch: Option<i64>, settings: &Settings) -> Summary {
        let branches = branch.map_or(Branches::Current, |branch| Branches::Chosen(vec![branch]));
        self.run(&Job { branches, ..Job::of(ids) }, settings)
    }

    fn run(&self, job: &Job, settings: &Settings) -> Summary {
        export::run(&self.library, job, settings, &draw, &AtomicBool::new(false), &|_, _| {}).unwrap()
    }

    fn plan(&self, job: &Job, settings: Option<&Settings>) -> Vec<String> {
        export::plan(&self.library, job, settings).unwrap().files.into_iter().map(|file| file.name).collect()
    }

    fn commit(&self, stops: f32) {
        self.library.history_commit(self.photos[0], &exposure(stops), &format!("Exposure {stops}")).unwrap();
    }

    fn exports_dir(&self) -> PathBuf {
        self.dir.path().join("Tonality/Exports")
    }
}

fn names(summary: &Summary) -> Vec<&str> {
    summary.exported.iter().map(|exported| exported.name.as_str()).collect()
}

#[test]
fn an_export_lands_in_the_exports_folder_under_the_photos_name() {
    let f = fixture();
    let summary = f.export(&f.photos, None, &Settings::default());

    assert_eq!(Path::new(&summary.folder), f.exports_dir());
    assert_eq!(names(&summary), ["IMG_0462.jpg", "IMG_0463.jpg"]);
    assert!(summary.failed.is_empty() && !summary.cancelled);
    for exported in &summary.exported {
        assert_eq!(Path::new(&exported.path), f.exports_dir().join(&exported.name));
        assert_eq!(fs::metadata(&exported.path).unwrap().len(), exported.file_size);
        assert_eq!(image::open(&exported.path).unwrap().width(), exported.width);
        assert_eq!(exported.branch, None, "a photo with one branch has nothing to tell apart");
    }
}

#[test]
fn nothing_already_in_the_folder_is_ever_replaced() {
    let f = fixture();
    fs::create_dir_all(f.exports_dir()).unwrap();
    fs::write(f.exports_dir().join("IMG_0462.jpg"), b"somebody else's file").unwrap();

    let plan = export::plan(&f.library, &Job::of(&f.photos[..1]), None).unwrap();
    assert_eq!(plan.files[0].name, "IMG_0462-2.jpg");
    assert!(plan.files[0].numbered);

    assert_eq!(names(&f.export(&f.photos[..1], None, &Settings::default())), ["IMG_0462-2.jpg"]);
    assert_eq!(names(&f.export(&f.photos[..1], None, &Settings::default())), ["IMG_0462-3.jpg"]);
    assert_eq!(fs::read(f.exports_dir().join("IMG_0462.jpg")).unwrap(), b"somebody else's file");
}

#[test]
fn a_photo_with_branches_exports_its_current_one_and_names_it() {
    let f = fixture();
    let photo = f.photos[0];
    f.commit(1.0);
    let main = f.library.history(photo).unwrap().branch_id;
    let head = f.library.history(photo).unwrap().head_id;
    let warm = f.library.history_branch(photo, head, "Warm").unwrap().branch_id;
    f.commit(2.0);

    let current = f.export(&[photo], None, &png());
    assert_eq!(names(&current), ["IMG_0462 (Warm).png"]);
    assert_eq!(current.exported[0].branch.as_deref(), Some("Warm"));
    assert_eq!(stops_in(&current.exported[0].path), 2.0);

    // Another branch can be asked for by name, without switching to it.
    let other = f.export(&[photo], Some(main), &png());
    assert_eq!(names(&other), ["IMG_0462 (Main).png"]);
    assert_eq!(stops_in(&other.exported[0].path), 1.0);
    assert_eq!(f.library.history(photo).unwrap().branch_id, warm, "exporting does not change branch");

    // The photo that has one branch keeps its plain name in the same export.
    assert_eq!(names(&f.export(&f.photos, None, &png())), ["IMG_0462 (Warm)-2.png", "IMG_0463.png"]);
}

#[test]
fn a_branch_is_exported_at_the_step_it_is_on() {
    let f = fixture();
    let photo = f.photos[0];
    f.commit(1.0);
    let first = f.library.history(photo).unwrap().head_id;
    f.commit(2.0);
    f.library.history_goto(photo, first).unwrap();
    let summary = f.export(&[photo], None, &png());
    assert_eq!(stops_in(&summary.exported[0].path), 1.0, "the undone step is not in the picture");
}

#[test]
fn the_plan_names_exactly_what_the_export_writes() {
    let f = fixture();
    let photo = f.photos[0];
    let head = f.library.history(photo).unwrap().head_id;
    f.library.history_branch(photo, head, "B/W").unwrap();
    f.export(&f.photos, None, &png());

    // Both names are now taken, and one photo is listed twice.
    let ids = [f.photos[0], f.photos[1], f.photos[1]];
    let plan = export::plan(&f.library, &Job::of(&ids), Some(&png())).unwrap();
    let planned: Vec<&str> = plan.files.iter().map(|file| file.name.as_str()).collect();
    assert_eq!(planned, ["IMG_0462 (B-W)-2.png", "IMG_0463-2.png", "IMG_0463-3.png"]);
    assert_eq!(plan.files[0].branch.as_deref(), Some("B/W"));
    assert_eq!(Path::new(&plan.folder), f.exports_dir());
    assert_eq!(names(&f.export(&ids, None, &png())), planned);

    // For one photo the plan also lists the branches there are to choose from.
    let single = export::plan(&f.library, &Job::of(&[photo]), Some(&png())).unwrap();
    assert_eq!(single.branches.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Main", "B/W"]);
    let (main, black_and_white) = (single.branches[0].id, single.branches[1].id);
    assert_eq!((single.files[0].branch_id, single.current_branch_id), (Some(black_and_white), Some(black_and_white)));
    let other = Job { branches: Branches::Chosen(vec![main]), ..Job::of(&[photo]) };
    let other = export::plan(&f.library, &other, Some(&png())).unwrap();
    assert_eq!((other.files[0].branch_id, other.current_branch_id), (Some(main), Some(black_and_white)));
    assert_eq!(other.files[0].name, "IMG_0462 (Main).png");
    assert!(plan.branches.is_empty() && plan.several_branches);
    // The fixture's photos are 8 pixels square, and the plan says so.
    assert_eq!((other.files[0].width, other.files[0].height), (Some(8), Some(8)));
}

#[test]
fn planning_writes_nothing() {
    let f = fixture();
    for branches in [Branches::Current, Branches::All] {
        export::plan(&f.library, &Job { branches, ..Job::of(&f.photos) }, None).unwrap();
    }
    assert!(!f.exports_dir().exists());
    assert_eq!(f.library.list_photos(View::Library).unwrap()[0].branches, 0, "no history is started either");
}

#[test]
fn the_exported_step_is_marked_in_the_history() {
    let f = fixture();
    let photo = f.photos[0];
    f.commit(1.0);
    let summary = f.export(&[photo], None, &Settings::default());

    let history = f.library.history(photo).unwrap();
    let marks = |index: usize| history.steps[index].exports.iter().map(|mark| mark.path.as_str()).collect::<Vec<_>>();
    assert_eq!(marks(1), [summary.exported[0].path.as_str()]);
    assert!(marks(0).is_empty(), "the original was not what got exported");

    // Undoing and editing again replaces the exported step; its mark goes
    // with it, and the file stays where it is.
    f.library.history_goto(photo, history.steps[0].id).unwrap();
    f.commit(3.0);
    let history = f.library.history(photo).unwrap();
    assert!(history.steps.iter().all(|step| step.exports.is_empty()));
    assert!(Path::new(&summary.exported[0].path).exists());
}

#[test]
fn a_photo_that_was_exported_can_still_be_deleted() {
    let f = fixture();
    f.export(&f.photos, None, &Settings::default());
    f.library.trash(&f.photos).unwrap();
    assert_eq!(f.library.purge(&f.photos).unwrap(), 2);
    assert!(f.exports_dir().join("IMG_0462.jpg").exists(), "exports are yours to keep");
}

#[test]
fn the_folder_and_settings_are_remembered() {
    let f = fixture();
    assert_eq!(f.library.export_settings().unwrap(), Settings::default());

    let elsewhere = f.dir.path().join("For the client");
    let settings = Settings { folder: Some(elsewhere.clone()), format: Format::Png, quality: 80, long_edge: Some(6) };
    let summary = f.export(&f.photos[..1], None, &settings);
    assert_eq!(Path::new(&summary.exported[0].path), elsewhere.join("IMG_0462.png"));
    assert_eq!(summary.exported[0].width, 6, "the size asked for reaches the renderer");
    assert!(!f.exports_dir().exists());

    assert_eq!(f.library.export_settings().unwrap(), settings);
    let plan = export::plan(&f.library, &Job::of(&f.photos[1..]), None).unwrap();
    assert_eq!(Path::new(&plan.folder), elsewhere);
    assert_eq!(plan.files[0].name, "IMG_0463.png");
}

#[test]
fn exported_files_carry_the_capture_date_but_no_rotation() {
    let f = fixture();
    let taken_at = f.library.list_photos(View::Library).unwrap().into_iter().find(|p| p.id == f.photos[0]).unwrap().taken_at;
    for settings in [Settings::default(), png(), Settings { format: Format::Tiff, ..Default::default() }] {
        let summary = f.export(&f.photos[..1], None, &settings);
        let file = fs::File::open(&summary.exported[0].path).unwrap();
        let exif = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)).unwrap();
        let date = exif.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY).unwrap();
        assert_eq!(date.display_value().to_string(), taken_at.replace('T', " "));
        assert!(exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY).is_none());
    }
}

#[test]
fn one_photo_failing_does_not_stop_the_rest() {
    let f = fixture();
    let bad = f.photos[0];
    let flaky = |id: i64, recipe: &Adjustments, long_edge: u32, deep: bool| {
        anyhow::ensure!(id != bad, "the file is damaged");
        draw(id, recipe, long_edge, deep)
    };
    let summary =
        export::run(&f.library, &Job::of(&f.photos), &Settings::default(), &flaky, &AtomicBool::new(false), &|_, _| {})
            .unwrap();
    assert_eq!(names(&summary), ["IMG_0463.jpg"]);
    assert_eq!(summary.failed.len(), 1);
    assert_eq!(summary.failed[0].file_name, "IMG_0462.png");
    assert!(summary.failed[0].reason.contains("damaged"));
    assert!(!f.exports_dir().join("IMG_0462.jpg").exists());
}

#[test]
fn a_stopped_export_says_so() {
    let f = fixture();
    let stop = AtomicBool::new(false);
    // Stop as soon as the first photo is done.
    let progress = |done: usize, _total: usize| stop.store(done >= 1, std::sync::atomic::Ordering::Relaxed);
    let summary = export::run(&f.library, &Job::of(&f.photos), &Settings::default(), &draw, &stop, &progress).unwrap();
    assert!(summary.cancelled);
    assert_eq!(names(&summary), ["IMG_0462.jpg"]);
}

#[test]
fn a_tiff_is_drawn_and_kept_at_sixteen_bits() {
    let f = fixture();
    f.commit(2.5);
    let summary = f.export(&f.photos[..1], None, &Settings { format: Format::Tiff, ..Default::default() });
    assert_eq!(names(&summary), ["IMG_0462.tif"]);
    let DynamicImage::ImageRgb16(read) = image::open(&summary.exported[0].path).unwrap() else { panic!("not 16-bit") };
    assert_eq!(read.get_pixel(3, 3).0, [2500, 0, 0], "a level 8 bits has no room for");
}

#[test]
fn an_export_can_be_given_a_name() {
    let f = fixture();
    let photo = f.photos[0];
    let head = f.library.history(photo).unwrap().head_id;
    f.library.history_branch(photo, head, "Warm").unwrap();

    // One file, called what you typed: the branch is not added to a name you chose.
    let cover = Job { name: Some("Portfolio cover".into()), ..Job::of(&[photo]) };
    assert_eq!(f.plan(&cover, None), ["Portfolio cover.jpg"]);
    assert_eq!(names(&f.run(&cover, &Settings::default())), ["Portfolio cover.jpg"]);
    // Asked for again, the name is taken, and still nothing is replaced.
    assert_eq!(f.plan(&cover, None), ["Portfolio cover-2.jpg"]);

    // Several files share a pattern.
    let numbered = Job { name: Some("Shoot {n} {name}".into()), ..Job::of(&f.photos) };
    let planned = f.plan(&numbered, Some(&png()));
    assert_eq!(planned, ["Shoot 01 IMG_0462.png", "Shoot 02 IMG_0463.png"]);
    assert_eq!(names(&f.run(&numbered, &png())), planned);

    // A name with nothing in it that differs between files still gives each its own.
    let same = Job { name: Some("Proof".into()), ..Job::of(&f.photos) };
    assert_eq!(names(&f.run(&same, &png())), ["Proof.png", "Proof-2.png"]);
    // A blank name is no name.
    assert_eq!(f.plan(&Job { name: Some("  ".into()), ..Job::of(&[photo]) }, Some(&png())), ["IMG_0462 (Warm).png"]);
}

#[test]
fn every_branch_can_be_exported_at_once() {
    let f = fixture();
    let photo = f.photos[0];
    f.commit(1.0);
    let main = f.library.history(photo).unwrap();
    let warm = f.library.history_branch(photo, main.head_id, "Warm").unwrap().branch_id;
    f.commit(2.0);
    let cool = f.library.history_branch(photo, main.head_id, "Cool").unwrap().branch_id;
    f.commit(3.0);

    // All of them, for every photo: one that has no branches to speak of exports as usual.
    let all = Job { branches: Branches::All, ..Job::of(&f.photos) };
    let expected = ["IMG_0462 (Main).png", "IMG_0462 (Warm).png", "IMG_0462 (Cool).png", "IMG_0463.png"];
    assert_eq!(f.plan(&all, Some(&png())), expected);
    let summary = f.run(&all, &png());
    assert_eq!(names(&summary), expected);
    let stops: Vec<f32> = summary.exported.iter().map(|exported| stops_in(&exported.path)).collect();
    assert_eq!(stops, [1.0, 2.0, 3.0, 0.0]);
    assert_eq!(f.library.history(photo).unwrap().branch_id, cool, "the photo stays on its branch");

    // Or a chosen few of one photo's.
    let two = Job { branches: Branches::Chosen(vec![cool, warm]), name: Some("{branch} look".into()), ..Job::of(&[photo]) };
    assert_eq!(names(&f.run(&two, &png())), ["Cool look.png", "Warm look.png"]);
    let none = Job { branches: Branches::Chosen(vec![]), ..Job::of(&[photo]) };
    assert!(export::plan(&f.library, &none, None).is_err());
    let two_photos = Job { branches: Branches::Chosen(vec![warm]), ..Job::of(&f.photos) };
    assert!(export::plan(&f.library, &two_photos, None).is_err(), "branches are chosen for one photo at a time");
}
