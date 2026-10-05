//! Spotting camera cards: mounted volumes with a `DCIM` folder at their root.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    pub name: String,
    pub path: String,
}

fn camera_volume(mount: &Path) -> Option<Volume> {
    mount.join("DCIM").is_dir().then(|| Volume {
        name: mount
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| mount.to_string_lossy().into_owned()),
        path: mount.to_string_lossy().into_owned(),
    })
}

#[cfg(target_os = "linux")]
fn mount_points() -> Vec<PathBuf> {
    // Card filesystems, plus anything a desktop automounter put in the usual places.
    const CARD_FILESYSTEMS: &[&str] = &["vfat", "exfat", "msdos", "ntfs", "ntfs3", "fuseblk", "udf"];
    const REMOVABLE_ROOTS: &[&str] = &["/run/media/", "/media/", "/mnt/"];
    // Mount points escape spaces and friends as octal, e.g. `\040`.
    fn unescape(field: &str) -> String {
        let mut out = Vec::with_capacity(field.len());
        let bytes = field.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let octal = bytes.get(i + 1..i + 4).and_then(|d| std::str::from_utf8(d).ok());
            match (bytes[i], octal.and_then(|d| u8::from_str_radix(d, 8).ok())) {
                (b'\\', Some(byte)) => {
                    out.push(byte);
                    i += 4;
                }
                (byte, _) => {
                    out.push(byte);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }
    std::fs::read_to_string("/proc/self/mounts")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(' ');
            let (mount, fstype) = (unescape(fields.nth(1)?), fields.next()?);
            (CARD_FILESYSTEMS.contains(&fstype) || REMOVABLE_ROOTS.iter().any(|root| mount.starts_with(root)))
                .then(|| PathBuf::from(mount))
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn mount_points() -> Vec<PathBuf> {
    std::fs::read_dir("/Volumes")
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

#[cfg(target_os = "windows")]
fn mount_points() -> Vec<PathBuf> {
    ('D'..='Z').map(|letter| PathBuf::from(format!("{letter}:\\"))).filter(|drive| drive.exists()).collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn mount_points() -> Vec<PathBuf> {
    Vec::new()
}

/// Camera cards mounted right now.
pub fn list() -> Vec<Volume> {
    let mut mounts = mount_points();
    // Development aid: treat folders named in TONALITY_FAKE_VOLUMES as cards.
    #[cfg(debug_assertions)]
    if let Some(fake) = std::env::var_os("TONALITY_FAKE_VOLUMES") {
        mounts.extend(std::env::split_paths(&fake));
    }
    let mut volumes: Vec<Volume> = mounts.iter().filter_map(|mount| camera_volume(mount)).collect();
    volumes.sort_by(|a, b| a.path.cmp(&b.path));
    volumes.dedup();
    volumes
}

/// Calls `on_change` with the current cards whenever one is inserted or removed.
pub fn watch(on_change: impl Fn(&[Volume]) + Send + 'static) {
    std::thread::spawn(move || {
        let mut known = list();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let current = list();
            if current != known {
                on_change(&current);
                known = current;
            }
        }
    });
}
