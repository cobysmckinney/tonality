//! Tethered capture with a fake camera: shots come off the camera, into the
//! library as one import, and follow the frame before.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use image::{Rgb, RgbImage};
use tempfile::TempDir;
use tonality_lib::capture::fake::FakeCamera;
use tonality_lib::capture::{self, Camera, Capture, Found, Report, Tether};
use tonality_lib::edit::Adjustments;
use tonality_lib::film::{self, Kind};
use tonality_lib::library::{FilmDetails, FilmPhoto, Library, View};

fn write_image(path: &Path, shade: u8) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    RgbImage::from_fn(96, 64, |x, y| Rgb([shade, (x * 2) as u8, (y * 3) as u8])).save(path).unwrap();
}

struct Fixture {
    _dir: TempDir,
    library: Arc<Library>,
    /// What the fake camera holds.
    camera: PathBuf,
}

/// A library, and a camera holding three shots: two JPEGs and a RAW+JPEG pair.
fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let library = Arc::new(Library::open(&dir.path().join("Tonality")).unwrap());
    let camera = dir.path().join("camera");
    write_image(&camera.join("IMG_0001.JPG"), 10);
    // Not a decodable RAW: the pair still comes in, drawn from its JPEG.
    fs::write(camera.join("IMG_0002.CR2"), b"not really a raw file").unwrap();
    write_image(&camera.join("IMG_0002.JPG"), 20);
    write_image(&camera.join("IMG_0003.JPG"), 30);
    Fixture { _dir: dir, library, camera }
}

fn photos(library: &Library) -> Vec<(String, bool, i64)> {
    let mut photos: Vec<_> = library
        .list_photos(View::Library)
        .unwrap()
        .into_iter()
        .map(|p| (p.file_name, p.has_jpeg, p.import_id))
        .collect();
    photos.sort();
    photos
}

fn quiet(_: Report) {}

#[test]
fn each_shot_comes_into_one_import_named_after_the_camera() {
    let f = fixture();
    let mut camera = FakeCamera::new(&f.camera);
    let mut capture = Capture::new(f.library.clone(), "Canon EOS 600D", true, None).unwrap();

    capture.trigger(&mut camera, &quiet).unwrap();
    let first = capture.bring_in().unwrap();
    assert_eq!(first.len(), 1);
    // The RAW comes off first, its JPEG after, announced by the camera: one photo.
    capture.trigger(&mut camera, &quiet).unwrap();
    let jpeg = camera.wait(Duration::ZERO).unwrap().expect("the JPEG of the pair is announced");
    capture.announced(&mut camera, &jpeg, &quiet).unwrap();
    let second = capture.bring_in().unwrap();
    assert_eq!(second.len(), 1, "a RAW and its JPEG are one photo");
    assert!(capture.bring_in().unwrap().is_empty(), "nothing more to bring in");
    capture.finish().unwrap();

    let all = photos(&f.library);
    assert_eq!(
        all.iter().map(|(name, jpeg, _)| (name.as_str(), *jpeg)).collect::<Vec<_>>(),
        [("IMG_0001.JPG", false), ("IMG_0002.CR2", true)]
    );
    assert_eq!(all[0].2, all[1].2, "one import for the whole session");
    let imports = f.library.overview().unwrap().imports;
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].source, "Canon EOS 600D");

    // The camera's files are left as they were, and nothing is left behind.
    assert!(f.camera.join("IMG_0001.JPG").exists() && f.camera.join("IMG_0002.CR2").exists());
    assert!(!f.library.root().join(".tonality/capture").exists());
}

#[test]
fn a_file_the_camera_announces_after_taking_it_is_downloaded_once() {
    let f = fixture();
    let mut camera = FakeCamera::new(&f.camera);
    let mut capture = Capture::new(f.library.clone(), "Camera", true, None).unwrap();
    let downloads = Mutex::new(Vec::new());
    let report = |report: Report| {
        if let Report::Downloading { name } = report {
            downloads.lock().unwrap().push(name);
        }
    };
    capture.trigger(&mut camera, &report).unwrap();
    let again = capture::Shot { folder: f.camera.to_string_lossy().into_owned(), name: "IMG_0001.JPG".into() };
    capture.announced(&mut camera, &again, &report).unwrap();
    assert_eq!(*downloads.lock().unwrap(), ["IMG_0001.JPG"]);
    assert_eq!(capture.bring_in().unwrap().len(), 1);

    // A shot already in the library is left out quietly.
    capture.announced(&mut camera, &again, &report).unwrap();
    assert!(capture.bring_in().unwrap().is_empty());
    assert_eq!(photos(&f.library).len(), 1);
}

fn negative(library: &Library, id: i64, frame: Option<u32>) {
    let mut recipe = Adjustments { exposure: 0.7, ..Default::default() };
    recipe.film = film::Film { kind: Kind::BlackAndWhite, base: Some([0.6, 0.5, 0.55]), range: None };
    recipe.crop.width = 0.8;
    recipe.flip_horizontal = true;
    library.history_commit(id, &recipe, "Film").unwrap();
    let film = FilmDetails {
        stock: Some("Ilford HP5 Plus".into()),
        iso: Some(800),
        camera: Some("Nikon FM2".into()),
        lens: None,
        frame,
    };
    library.set_film_details(&[FilmPhoto { id, taken_at: String::new(), file_name: String::new(), film }]).unwrap();
}

fn shoot(capture: &mut Capture, camera: &mut FakeCamera) -> i64 {
    capture.trigger(camera, &quiet).unwrap();
    while let Some(shot) = camera.wait(Duration::ZERO).unwrap() {
        capture.announced(camera, &shot, &quiet).unwrap();
    }
    let ids = capture.bring_in().unwrap();
    assert_eq!(ids.len(), 1);
    ids[0]
}

#[test]
fn a_new_frame_follows_the_film_settings_framing_and_details_of_the_last() {
    let f = fixture();
    let mut camera = FakeCamera::new(&f.camera);
    let mut capture = Capture::new(f.library.clone(), "Camera", true, None).unwrap();
    let first = shoot(&mut capture, &mut camera);
    negative(&f.library, first, Some(5));

    let second = shoot(&mut capture, &mut camera);
    let history = f.library.history(second).unwrap();
    assert_eq!(history.steps.last().unwrap().label, "Settings of the last frame", "a step in its history");
    let recipe = history.adjustments;
    assert_eq!(recipe.film.kind, Kind::BlackAndWhite);
    assert_eq!(recipe.film.base, Some([0.6, 0.5, 0.55]));
    assert_eq!((recipe.crop.width, recipe.flip_horizontal), (0.8, true), "the framing too");
    assert_eq!(recipe.exposure, 0.0, "not the rest of the look");
    let details = f.library.film_details(&[second]).unwrap().pop().unwrap().film;
    assert_eq!(details.stock.as_deref(), Some("Ilford HP5 Plus"));
    assert_eq!((details.iso, details.frame), (Some(800), Some(6)), "the frame number counts on");

    // And the next follows that one.
    let third = shoot(&mut capture, &mut camera);
    assert_eq!(f.library.film_details(&[third]).unwrap()[0].film.frame, Some(7));
}

#[test]
fn the_first_frame_follows_the_photo_open_when_capture_began() {
    let f = fixture();
    let mut camera = FakeCamera::new(&f.camera);
    let mut capture = Capture::new(f.library.clone(), "Camera", true, None).unwrap();
    let earlier = shoot(&mut capture, &mut camera);
    negative(&f.library, earlier, None);
    capture.finish().unwrap();

    let mut capture = Capture::new(f.library.clone(), "Camera", true, Some(earlier)).unwrap();
    let next = shoot(&mut capture, &mut camera);
    assert_eq!(f.library.history(next).unwrap().adjustments.film.kind, Kind::BlackAndWhite);
    assert_eq!(f.library.film_details(&[next]).unwrap()[0].film.frame, None, "no frame number to count on");
}

#[test]
fn frames_follow_only_when_asked_and_only_film() {
    let f = fixture();
    let mut camera = FakeCamera::new(&f.camera);
    let mut capture = Capture::new(f.library.clone(), "Camera", false, None).unwrap();
    let first = shoot(&mut capture, &mut camera);
    negative(&f.library, first, Some(1));
    let second = shoot(&mut capture, &mut camera);
    assert!(f.library.history(second).unwrap().adjustments.is_default(), "following is off");
    assert!(!f.library.film_details(&[second]).unwrap()[0].film.is_set());

    // A photo that isn't of film passes nothing on, not even its crop.
    capture.follow = true;
    let mut cropped = Adjustments::default();
    cropped.crop.width = 0.5;
    f.library.history_commit(second, &cropped, "Crop").unwrap();
    let third = shoot(&mut capture, &mut camera);
    assert!(f.library.history(third).unwrap().adjustments.is_default());
}

/// Collects what a running capture reports.
#[derive(Clone, Default)]
struct Reports(Arc<Mutex<Vec<Report>>>);

impl Reports {
    fn wait_for(&self, what: impl Fn(&Report) -> bool) -> Report {
        let start = Instant::now();
        loop {
            if let Some(found) = self.0.lock().unwrap().iter().find(|r| what(r)).cloned() {
                return found;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "waited too long; got {:?}", self.0.lock().unwrap());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn added(&self) -> Vec<i64> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .flat_map(|r| if let Report::Added { ids } = r { ids.clone() } else { Vec::new() })
            .collect()
    }
}

fn start(f: &Fixture, tether: &Tether) -> Reports {
    let reports = Reports::default();
    let sink = reports.clone();
    let camera = Found { name: "Fake camera".into(), port: format!("fake:{}", f.camera.display()) };
    tether.start(f.library.clone(), &camera, true, None, move |r| sink.0.lock().unwrap().push(r)).unwrap();
    reports
}

#[test]
fn shots_fired_from_the_app_or_the_camera_come_in_until_capture_stops() {
    let f = fixture();
    let tether = Tether::default();
    let reports = start(&f, &tether);
    assert_eq!(tether.camera().unwrap().name, "Fake camera");
    assert!(tether.start(f.library.clone(), &tether.camera().unwrap(), true, None, quiet).is_err(), "one at a time");

    tether.send(capture::Command::Fire).unwrap();
    reports.wait_for(|r| matches!(r, Report::Added { .. }));
    // The camera's own shutter button.
    fs::write(f.camera.join("press"), b"").unwrap();
    let start = Instant::now();
    while reports.added().len() < 2 {
        assert!(start.elapsed() < Duration::from_secs(20), "the second shot never came: {:?}", reports.0.lock().unwrap());
        std::thread::sleep(Duration::from_millis(20));
    }
    tether.stop();
    assert_eq!(reports.wait_for(|r| matches!(r, Report::Ended { .. })), Report::Ended { message: None });
    assert!(tether.camera().is_none());
    assert!(tether.send(capture::Command::Fire).is_err(), "nothing to fire once stopped");
    assert_eq!(photos(&f.library).len(), 2);
}

#[test]
fn unplugging_the_camera_ends_capture_and_says_so() {
    let f = fixture();
    let tether = Tether::default();
    let reports = start(&f, &tether);
    fs::write(f.camera.join("unplug"), b"").unwrap();
    let ended = reports.wait_for(|r| matches!(r, Report::Ended { .. }));
    let Report::Ended { message: Some(message) } = ended else { panic!("no reason given: {ended:?}") };
    assert!(message.contains("unplugged"), "{message}");
    assert!(f.library.overview().unwrap().imports.is_empty(), "an import that brought nothing in is dropped");

    // Taking a photo with nothing connected says so, and capture can start again.
    fs::remove_file(f.camera.join("unplug")).unwrap();
    let reports = start(&f, &tether);
    fs::write(f.camera.join("unplug"), b"").unwrap();
    tether.send(capture::Command::Fire).ok();
    reports.wait_for(|r| matches!(r, Report::Ended { .. }));
}
