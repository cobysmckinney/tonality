//! A pretend camera, for tests and for trying capture without a camera.
//!
//! Its shots are the photos in a folder, taken in name order; a RAW and a
//! JPEG of the same name are one shot, as a camera set to RAW+JPEG makes.
//! Putting a file named `press` in the folder presses the camera's own
//! shutter button (the file is taken away again), and one named `unplug`
//! unplugs it. In debug builds, `TONALITY_FAKE_CAMERA=<folder>` shows it in
//! the app as "Fake camera".

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};

use super::{Camera, Shot};
use crate::media;

pub struct FakeCamera {
    folder: PathBuf,
    /// Shots taken so far.
    taken: usize,
    /// Files of the last shot not yet announced.
    announcing: VecDeque<Shot>,
}

impl FakeCamera {
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        Self { folder: folder.into(), taken: 0, announcing: VecDeque::new() }
    }

    /// The photos in the folder, one entry per shot, in name order.
    fn shots(&self) -> Vec<Vec<String>> {
        let mut shots: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for entry in fs::read_dir(&self.folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_file() && media::kind_of(&path).is_some() {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
                shots.entry(stem).or_default().push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        shots
            .into_values()
            .map(|mut files| {
                // The RAW first, as a camera usually announces it.
                files.sort_by_key(|name| (media::is_jpeg(Path::new(name)), name.clone()));
                files
            })
            .collect()
    }

    fn signal(&self, name: &str) -> bool {
        self.folder.join(name).exists()
    }
}

impl Camera for FakeCamera {
    fn trigger(&mut self) -> Result<Shot> {
        if self.signal("unplug") {
            bail!("the camera isn’t connected");
        }
        let Some(files) = self.shots().into_iter().nth(self.taken) else {
            bail!("the fake camera has taken every photo in {}", self.folder.display());
        };
        self.taken += 1;
        let folder = self.folder.to_string_lossy().into_owned();
        let mut shots = files.into_iter().map(|name| Shot { folder: folder.clone(), name });
        let first = shots.next().expect("a shot has at least one file");
        self.announcing.extend(shots);
        Ok(first)
    }

    fn wait(&mut self, timeout: Duration) -> Result<Option<Shot>> {
        if let Some(shot) = self.announcing.pop_front() {
            return Ok(Some(shot));
        }
        if self.signal("unplug") {
            bail!("the camera was unplugged");
        }
        if self.signal("press") {
            let _ = fs::remove_file(self.folder.join("press"));
            return self.trigger().map(Some);
        }
        std::thread::sleep(timeout.min(Duration::from_millis(50)));
        Ok(None)
    }

    fn download(&mut self, shot: &Shot, to: &Path) -> Result<()> {
        fs::copy(Path::new(&shot.folder).join(&shot.name), to).with_context(|| format!("copying {}", shot.name))?;
        Ok(())
    }
}
