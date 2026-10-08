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
    card_mounts(&std::fs::read_to_string("/proc/self/mounts").unwrap_or_default())
}

/// The mount points in a mount table (laid out like `/proc/self/mounts`)
/// that could be a camera card.
#[cfg(any(target_os = "linux", test))]
fn card_mounts(table: &str) -> Vec<PathBuf> {
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
    table
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_are_found_by_filesystem_or_where_they_are_mounted() {
        let table = "\
proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
/dev/nvme0n1p1 /boot vfat rw,relatime,fmask=0022 0 0
/dev/sdb1 /run/media/coby/EOS_DIGITAL exfat rw,nosuid,nodev,relatime 0 0
/dev/sdc1 /media/backup ext4 rw,relatime 0 0
/dev/sdd1 /mnt/card ntfs3 rw,relatime 0 0
/dev/sde1 /home/coby/odd\\040place fuseblk rw,relatime 0 0
tmpfs /run/user/1000 tmpfs rw,nosuid,nodev 0 0
";
        let found: Vec<String> = card_mounts(table).iter().map(|path| path.to_string_lossy().into_owned()).collect();
        assert_eq!(
            found,
            ["/boot", "/run/media/coby/EOS_DIGITAL", "/media/backup", "/mnt/card", "/home/coby/odd place"],
            "card filesystems anywhere, and anything under the usual removable roots",
        );
    }

    #[test]
    fn escaped_characters_in_mount_points_are_read_back() {
        let table = "/dev/sdb1 /run/media/coby/My\\040Card\\011(2)\\134x vfat rw 0 0\n";
        assert_eq!(card_mounts(table), [PathBuf::from("/run/media/coby/My Card\t(2)\\x")]);
        // A backslash not followed by three octal digits is kept as it is.
        let table = "/dev/sdb1 /media/a\\b\\9 vfat rw 0 0\n";
        assert_eq!(card_mounts(table), [PathBuf::from("/media/a\\b\\9")]);
    }

    #[test]
    fn short_or_empty_lines_are_skipped() {
        assert!(card_mounts("").is_empty());
        assert!(card_mounts("\n\nnonsense\n/dev/sdb1\n").is_empty());
        assert_eq!(card_mounts("/dev/sdb1 /media/card\n/dev/sdc1 /media/other vfat\n"), [PathBuf::from("/media/other")]);
    }

    #[test]
    fn a_card_is_a_volume_with_a_dcim_folder() {
        let dir = tempfile::TempDir::new().unwrap();
        let card = dir.path().join("EOS_DIGITAL");
        std::fs::create_dir_all(card.join("DCIM/100CANON")).unwrap();
        let drive = dir.path().join("BACKUP");
        std::fs::create_dir_all(&drive).unwrap();
        let odd = dir.path().join("ODD");
        std::fs::create_dir_all(&odd).unwrap();
        std::fs::write(odd.join("DCIM"), b"a file, not a folder").unwrap();

        let volume = camera_volume(&card).unwrap();
        assert_eq!(volume, Volume { name: "EOS_DIGITAL".into(), path: card.to_string_lossy().into_owned() });
        assert_eq!(camera_volume(&drive), None);
        assert_eq!(camera_volume(&odd), None);
        assert_eq!(camera_volume(&dir.path().join("gone")), None);
    }
}
