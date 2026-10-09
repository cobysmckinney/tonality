//! Tethered capture: a camera on USB whose every shot comes straight into
//! the library, as part of one import, and opens to be checked before the
//! next. Made for scanning film with a camera, a roll at a time.
//!
//! The camera is behind [`Camera`], so the whole flow (download, import,
//! follow the last frame) runs against [`fake::FakeCamera`] in tests. Real
//! cameras go through libgphoto2, loaded when first needed, on Linux only
//! for now; elsewhere no camera is found.

pub mod fake;
#[cfg(target_os = "linux")]
mod gphoto;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::edit::Adjustments;
use crate::import::{self, Status};
use crate::library::{FilmPhoto, Library};
use crate::thumbs;

/// A file a camera has just made, where the camera keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    pub folder: String,
    pub name: String,
}

/// A camera Tonality can take photos with and download them from.
pub trait Camera: Send {
    /// Takes a photo and returns the file it made. Any other file the same
    /// shot makes (the JPEG of RAW+JPEG) is reported by [`Camera::wait`].
    fn trigger(&mut self) -> Result<Shot>;
    /// Waits up to `timeout` for the camera to report a new file, as it does
    /// when its own shutter button is pressed. `None`: nothing new.
    fn wait(&mut self, timeout: Duration) -> Result<Option<Shot>>;
    /// Copies a file off the camera to `to`, leaving the camera's copy alone.
    fn download(&mut self, shot: &Shot, to: &Path) -> Result<()>;
}

/// A camera found connected, to capture with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    /// Its model, as it names itself: "Canon EOS 600D".
    pub name: String,
    /// Where it is connected, as libgphoto2 names it ("usb:001,005").
    pub port: String,
}

/// Where the fake camera's port names its folder.
const FAKE_PORT: &str = "fake:";

/// The cameras connected right now. Looking is slow-ish (libgphoto2 loads
/// a driver for every camera it knows), so [`watch`] does it only when
/// something on USB changes; this answers from what it last found.
pub fn list() -> Vec<Found> {
    let mut known = KNOWN.lock().unwrap();
    known.get_or_insert_with(detect).clone()
}

static KNOWN: Mutex<Option<Vec<Found>>> = Mutex::new(None);
/// Set while a camera is in use: looking for cameras then could disturb it.
static IN_USE: AtomicBool = AtomicBool::new(false);

fn detect() -> Vec<Found> {
    #[cfg_attr(not(any(target_os = "linux", debug_assertions)), allow(unused_mut))]
    let mut found = Vec::new();
    #[cfg(target_os = "linux")]
    found.extend(gphoto::detect());
    // Development aid: a folder of photos named in TONALITY_FAKE_CAMERA
    // shows as a camera (see fake.rs).
    #[cfg(debug_assertions)]
    if let Some(folder) = std::env::var_os("TONALITY_FAKE_CAMERA") {
        found.push(Found { name: "Fake camera".into(), port: format!("{FAKE_PORT}{}", folder.to_string_lossy()) });
    }
    found
}

/// What is plugged in on USB, cheaply: changes when anything is plugged in
/// or out, which is when cameras are looked for again.
fn usb_devices() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir("/sys/bus/usb/devices")
        .map(|entries| entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    names
}

/// Calls `on_change` with the cameras connected whenever that changes.
pub fn watch(on_change: impl Fn(&[Found]) + Send + 'static) {
    std::thread::spawn(move || {
        let mut usb = usb_devices();
        let mut cameras = list();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            if IN_USE.load(Ordering::Relaxed) {
                // Look again once the camera is let go.
                usb.clear();
                continue;
            }
            let now = usb_devices();
            if now == usb {
                continue;
            }
            usb = now;
            let found = detect();
            *KNOWN.lock().unwrap() = Some(found.clone());
            if found != cameras {
                on_change(&found);
                cameras = found;
            }
        }
    });
}

/// Why no camera can be used at all on this computer, if that is so.
pub fn unavailable() -> Option<String> {
    #[cfg(target_os = "linux")]
    return gphoto::unavailable();
    #[cfg(not(target_os = "linux"))]
    Some("Tethered capture works on Linux only for now.".into())
}

/// Connects to a camera found by [`list`].
pub fn open(camera: &Found) -> Result<Box<dyn Camera>> {
    if let Some(folder) = camera.port.strip_prefix(FAKE_PORT) {
        return Ok(Box::new(fake::FakeCamera::new(folder)));
    }
    #[cfg(target_os = "linux")]
    return gphoto::open(&camera.port);
    #[cfg(not(target_os = "linux"))]
    bail!("Tethered capture works on Linux only for now.")
}

/// What happened, for the window to show.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Report {
    /// A file is coming off the camera.
    Downloading { name: String },
    /// New frames are in the library, in the order they were taken.
    Added { ids: Vec<i64> },
    /// Something went wrong with one shot; capture carries on.
    Failed { message: String },
    /// Capture has stopped: asked to (no message), or because the camera
    /// went away.
    Ended { message: Option<String> },
}

/// Turns a camera's shots into photos in the library.
pub struct Capture {
    library: Arc<Library>,
    import_id: i64,
    source: String,
    /// Where shots wait between the camera and the library.
    staging: PathBuf,
    /// Downloaded and waiting for the camera to go quiet, so the RAW and JPEG
    /// of one shot come in together as one photo.
    pending: Vec<PathBuf>,
    /// Files `trigger` returned, so the camera announcing them again later
    /// doesn't download them twice.
    taken: Vec<Shot>,
    /// Whether each new frame takes the settings of the one before.
    pub follow: bool,
    /// The frame the next one follows.
    last: Option<i64>,
    scans: u64,
}

impl Capture {
    /// Starts an import named after the camera. With `follow`, the first
    /// frame follows `first_follows` (the photo open when capture began).
    pub fn new(library: Arc<Library>, camera_name: &str, follow: bool, first_follows: Option<i64>) -> Result<Self> {
        let import_id = library.create_import(camera_name)?;
        let staging = library.root().join(".tonality/capture");
        fs::create_dir_all(&staging)?;
        Ok(Self {
            library,
            import_id,
            source: camera_name.to_string(),
            staging,
            pending: Vec::new(),
            taken: Vec::new(),
            follow,
            last: first_follows,
            scans: 0,
        })
    }

    /// Takes a photo with the camera and downloads it.
    pub fn trigger(&mut self, camera: &mut dyn Camera, report: &dyn Fn(Report)) -> Result<()> {
        let shot = camera.trigger()?;
        self.taken.push(shot.clone());
        // Only the last few can still be announced.
        if self.taken.len() > 8 {
            self.taken.remove(0);
        }
        self.download(camera, &shot, report)
    }

    /// Downloads a shot the camera announced, unless it was already.
    pub fn announced(&mut self, camera: &mut dyn Camera, shot: &Shot, report: &dyn Fn(Report)) -> Result<()> {
        if let Some(at) = self.taken.iter().position(|taken| taken == shot) {
            self.taken.remove(at);
            return Ok(());
        }
        self.download(camera, shot, report)
    }

    fn download(&mut self, camera: &mut dyn Camera, shot: &Shot, report: &dyn Fn(Report)) -> Result<()> {
        report(Report::Downloading { name: shot.name.clone() });
        let to = free_name(&self.staging, &shot.name);
        if let Err(error) = camera.download(shot, &to) {
            let _ = fs::remove_file(&to);
            return Err(error.context(format!("downloading {}", shot.name)));
        }
        self.pending.push(to);
        Ok(())
    }

    /// Brings everything downloaded into the library, and returns the new
    /// photos in the order they were taken. A shot that fails stays in the
    /// staging folder, so nothing taken is lost.
    pub fn bring_in(&mut self) -> Result<Vec<i64>> {
        if self.pending.is_empty() {
            return Ok(Vec::new());
        }
        let pending = std::mem::take(&mut self.pending);
        self.scans += 1;
        let session_id = self.import_id as u64 * 1000 + self.scans;
        let session = import::scan(&self.library, session_id, &pending, Some(self.source.clone()), &|_, _| {})?;
        // A shot already in the library (the camera announcing an old file)
        // is left out quietly.
        let chosen: Vec<usize> =
            (0..session.items.len()).filter(|&i| session.items[i].status != Status::Duplicate).collect();
        let incoming = self.staging.join("incoming");
        let summary = import::run_into(
            &self.library,
            &session,
            &chosen,
            self.import_id,
            &incoming,
            &AtomicBool::new(false),
            &|_, _| {},
        );
        let _ = fs::remove_dir_all(&incoming);
        let summary = summary?;
        let failed: Vec<&str> = summary.failed.iter().map(|failure| failure.file_name.as_str()).collect();

        let done: Vec<&import::ScanItem> =
            session.items.iter().filter(|item| !failed.contains(&file_name(&item.path).as_str())).collect();
        for item in &done {
            let _ = fs::remove_file(&item.path);
            if let Some(jpeg) = &item.jpeg {
                let _ = fs::remove_file(jpeg);
            }
        }
        let known = self.library.fingerprints()?;
        let mut added: Vec<(&import::ScanItem, i64)> = done
            .into_iter()
            .filter(|item| item.status != Status::Duplicate)
            .filter_map(|item| Some((item, known.get(&item.fingerprint)?.0)))
            .collect();
        added.sort_by(|(a, _), (b, _)| a.taken_at.cmp(&b.taken_at).then_with(|| a.path.cmp(&b.path)));
        let mut ids = Vec::with_capacity(added.len());
        for (item, id) in added {
            if self.follow && item.status == Status::New {
                if let Some(last) = self.last {
                    // The frame is in; settings it couldn't take can be pasted later.
                    let _ = follow(&self.library, last, id);
                }
            }
            self.last = Some(id);
            ids.push(id);
        }
        if let Some(failure) = summary.failed.first() {
            bail!(
                "Couldn’t bring in {}: {}. The file is kept in {}.",
                failure.file_name,
                failure.reason,
                self.staging.display()
            );
        }
        Ok(ids)
    }

    /// Ends the import; one that brought nothing in is dropped.
    pub fn finish(self) -> Result<()> {
        // Leaves the staging folder if a failed shot is kept there.
        let _ = fs::remove_dir(&self.staging);
        self.library.discard_import_if_empty(self.import_id)
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// `name` in `dir`, or `name` with `-1`, `-2`… if a file has it already.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let path = Path::new(name);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path.extension().map(|ext| format!(".{}", ext.to_string_lossy())).unwrap_or_default();
    (0..)
        .map(|n| if n == 0 { dir.join(name) } else { dir.join(format!("{stem}-{n}{ext}")) })
        .find(|candidate| !candidate.exists())
        .expect("an unbounded search always finds a free name")
}

/// Gives frame `to` what frame `from` was set up with, if it is a frame of
/// film: its film settings and framing (crop, straightening, turns and
/// flips) as a step in its history, and its film details with the frame
/// number counted on by one. A photo that isn't of film passes nothing on.
pub fn follow(library: &Library, from: i64, to: i64) -> Result<()> {
    let before = Adjustments::from_json(library.edits(from)?.as_deref());
    let details = library.film_details(&[from])?.pop().map(|photo| photo.film).filter(|film| film.is_set());
    if !before.film.is_negative() && details.is_none() {
        return Ok(());
    }
    let own = Adjustments::from_json(library.edits(to)?.as_deref());
    let mut recipe = own.clone();
    recipe.film = before.film;
    recipe.keep_framing_of(&before);
    if recipe != own {
        library.history_commit(to, &recipe, "Settings of the last frame")?;
        redraw(library, to, &recipe);
    }
    if let Some(mut film) = details {
        film.frame = film.frame.map(|frame| frame.saturating_add(1));
        library.set_film_details(&[FilmPhoto { id: to, taken_at: String::new(), file_name: String::new(), film }])?;
    }
    Ok(())
}

/// Draws a new frame's thumbnail and preview with its edits. One that
/// can't be drawn now is drawn when first shown.
fn redraw(library: &Library, id: i64, recipe: &Adjustments) {
    let drawn = thumbs::open_session(library, id)
        .and_then(|session| thumbs::EditedImages::draw(&session, recipe))
        .and_then(|images| images.write(library, id));
    if drawn.is_err() {
        thumbs::clear_rendered(library, id);
    }
    let _ = library.bump_version(id);
}

/// What the window can ask of a running capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Take a photo.
    Fire,
    /// Whether new frames follow the last one.
    Follow(bool),
    Stop,
}

/// How long the camera is listened to before checking for commands.
const LISTEN: Duration = Duration::from_millis(200);

/// Runs a capture until it is stopped or the camera goes away: takes
/// photos when asked, downloads each shot, and brings them in once the
/// camera goes quiet. Returns why it stopped, if it wasn't asked to.
pub fn run(camera: &mut dyn Camera, capture: &mut Capture, commands: &Receiver<Command>, report: &dyn Fn(Report)) -> Option<String> {
    let failed = |error: anyhow::Error| report(Report::Failed { message: format!("{error:#}") });
    let bring_in = |capture: &mut Capture| match capture.bring_in() {
        Ok(ids) if ids.is_empty() => {}
        Ok(ids) => report(Report::Added { ids }),
        Err(error) => failed(error),
    };
    loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Fire) => {
                    if let Err(error) = capture.trigger(camera, report) {
                        failed(error.context("The camera didn’t take the photo"));
                    }
                }
                Ok(Command::Follow(follow)) => capture.follow = follow,
                Ok(Command::Stop) | Err(TryRecvError::Disconnected) => {
                    bring_in(capture);
                    return None;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        match camera.wait(LISTEN) {
            Ok(Some(shot)) => {
                if let Err(error) = capture.announced(camera, &shot, report) {
                    failed(error);
                }
            }
            Ok(None) => bring_in(capture),
            Err(error) => {
                bring_in(capture);
                return Some(format!("Lost the camera: {error:#}"));
            }
        }
    }
}

/// The capture running, if any.
#[derive(Default)]
pub struct Tether {
    running: Mutex<Option<Running>>,
}

struct Running {
    camera: Found,
    commands: Sender<Command>,
    thread: JoinHandle<()>,
}

impl Tether {
    /// Connects to `camera` and starts capturing into `library` on a thread
    /// of its own, which tells `report` what happens.
    pub fn start(
        &self,
        library: Arc<Library>,
        camera: &Found,
        follow: bool,
        first_follows: Option<i64>,
        report: impl Fn(Report) + Send + 'static,
    ) -> Result<()> {
        let mut running = self.running.lock().unwrap();
        if let Some(current) = running.as_ref().filter(|running| !running.thread.is_finished()) {
            bail!("Already capturing with {}.", current.camera.name);
        }
        IN_USE.store(true, Ordering::Relaxed);
        let opened = open(camera).and_then(|camera_handle| {
            Ok((camera_handle, Capture::new(library, &camera.name, follow, first_follows)?))
        });
        let (mut handle, mut capture) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                IN_USE.store(false, Ordering::Relaxed);
                return Err(error);
            }
        };
        let (commands, inbox) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let ended = run(handle.as_mut(), &mut capture, &inbox, &report);
            drop(handle);
            IN_USE.store(false, Ordering::Relaxed);
            let finished = capture.finish().err().map(|error| format!("{error:#}"));
            report(Report::Ended { message: ended.or(finished) });
        });
        *running = Some(Running { camera: camera.clone(), commands, thread });
        Ok(())
    }

    /// Passes a command to the capture running.
    pub fn send(&self, command: Command) -> Result<()> {
        let running = self.running.lock().unwrap();
        match running.as_ref() {
            Some(running) if running.commands.send(command).is_ok() => Ok(()),
            _ => bail!("No camera is capturing."),
        }
    }

    /// Stops capturing, once the shots already taken are in.
    pub fn stop(&self) {
        let running = self.running.lock().unwrap().take();
        if let Some(running) = running {
            let _ = running.commands.send(Command::Stop);
            let _ = running.thread.join();
        }
    }

    /// The camera capturing, if any.
    pub fn camera(&self) -> Option<Found> {
        let running = self.running.lock().unwrap();
        running.as_ref().filter(|running| !running.thread.is_finished()).map(|running| running.camera.clone())
    }
}
