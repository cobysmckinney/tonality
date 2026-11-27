//! App settings, release channels and in-app updates.
//!
//! Updates come from GitHub Releases, through the Tauri updater: each release carries a signed
//! `latest.json` naming its installers. The stable channel follows the latest full release; the
//! nightly channel follows the rolling `nightly` prerelease, rebuilt from `main` each night (see
//! `.github/workflows/nightly.yml`). Nightlies are versioned `X.Y.(Z+1)-nightly.DATE.RUN` after
//! stable `X.Y.Z`, so they sort above the stable they follow and below the next one.
//!
//! An update is checked for, downloaded in the background, and installed when the user chooses
//! to restart, never out from under them.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Emitter, Manager, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::CmdResult;

const STABLE_FEED: &str =
    "https://github.com/cobysmckinney/tonality/releases/latest/download/latest.json";
const NIGHTLY_FEED: &str =
    "https://github.com/cobysmckinney/tonality/releases/download/nightly/latest.json";

/// How long the feed gets to answer a check, start to finish. It's a few hundred bytes.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a check or a download waits to connect, and then for the next bytes to arrive. A
/// download has no limit on its whole length (a slow one that's still moving is fine), but a
/// connection that has gone quiet (the laptop slept, the VPN dropped) has to end, or the update
/// stays "checking" or "downloading" until the app is restarted.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Which builds the app updates itself to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ReleaseChannel {
    /// Full releases.
    #[default]
    Stable,
    /// Built from `main` every night.
    Nightly,
}

/// Preferences that live outside the library, in the app's config folder.
#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Settings {
    pub channel: ReleaseChannel,
    /// Check for and download updates in the background.
    pub auto_update: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            channel: ReleaseChannel::Stable,
            auto_update: true,
        }
    }
}

/// An update offered by the current channel's feed.
#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
    /// RFC 3339, as the feed states it.
    pub date: Option<String>,
}

/// Download progress, sent as `update-progress` events.
#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateProgress {
    pub downloaded: f64,
    pub total: Option<f64>,
}

pub struct Updates {
    path: Option<PathBuf>,
    settings: Mutex<Settings>,
    /// The update the last check found.
    found: Mutex<Option<Update>>,
    /// The downloaded installer for `found`, waiting for a restart.
    ready: Mutex<Option<(String, Vec<u8>)>>,
}

impl Updates {
    pub fn load(app: &AppHandle) -> Self {
        let path = app
            .path()
            .app_config_dir()
            .ok()
            .map(|d| d.join("settings.json"));
        let stored = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok());
        // A nightly someone installed by hand keeps following nightlies until told otherwise.
        let settings = stored.unwrap_or_else(|| Settings {
            channel: if !app.package_info().version.pre.is_empty() {
                ReleaseChannel::Nightly
            } else {
                ReleaseChannel::Stable
            },
            ..Settings::default()
        });
        Updates {
            path,
            settings: Mutex::new(settings),
            found: Mutex::default(),
            ready: Mutex::default(),
        }
    }

    fn save(&self, s: &Settings) -> CmdResult<()> {
        let path = self.path.as_ref().ok_or("no config folder")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?;
        // Write then rename, so a crash mid-write can't leave half a file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn settings_get(app: AppHandle) -> Settings {
    app.state::<Updates>().settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn settings_set(settings: Settings, app: AppHandle) -> CmdResult<()> {
    let u = app.state::<Updates>();
    let channel_changed = u.settings.lock().unwrap().channel != settings.channel;
    u.save(&settings)?;
    *u.settings.lock().unwrap() = settings;
    if channel_changed {
        // Whatever was found belongs to the other channel.
        *u.found.lock().unwrap() = None;
        *u.ready.lock().unwrap() = None;
    }
    Ok(())
}

#[tauri::command]
pub fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Whether an install running `current` on `channel` takes the version its feed offers.
///
/// Nightly follows whatever is newer. Stable does too, and a nightly build on the stable channel
/// also takes the latest stable when that's *older*, so switching back to stable works; once
/// it's running that stable, the same version isn't offered again.
fn offers(channel: ReleaseChannel, current: &Version, remote: &Version) -> bool {
    match channel {
        ReleaseChannel::Nightly => remote > current,
        ReleaseChannel::Stable => {
            remote > current || (!current.pre.is_empty() && remote != current)
        }
    }
}

/// Why this install can't replace itself with an update, when it can't.
///
/// On Linux the feed only carries the AppImage, and the updater installs it by moving the
/// running AppImage aside and writing the new one in its place. A `.deb` or `.rpm` install, or
/// a plain binary, has nothing that can take it; an AppImage in a folder that can't be written
/// to (`/opt`, say) can't be replaced. Without this they'd download the update at every launch
/// and fail to install it each time.
fn cannot_self_update(
    linux: bool,
    bundle: Option<BundleType>,
    appimage: Option<&Path>,
    writable: impl Fn(&Path) -> bool,
) -> Option<String> {
    if !linux {
        return None;
    }
    match (bundle, appimage) {
        (Some(BundleType::AppImage), Some(path)) => {
            let dir = path.parent().unwrap_or(Path::new("/"));
            (!writable(dir)).then(|| {
                format!(
                    "tonality can't update itself here: {} can't be written to. Move the \
                     AppImage somewhere you can write, or replace it by hand.",
                    dir.display()
                )
            })
        }
        (Some(BundleType::Deb | BundleType::Rpm), _) => Some(
            "This install came from a package, so it doesn't update itself. Update it the way \
             it was installed."
                .into(),
        ),
        _ => {
            Some("This build wasn't installed as an AppImage, so it doesn't update itself.".into())
        }
    }
}

/// Whether a file could be created in `dir`. Only asked on Linux.
fn dir_writable(dir: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
            return false;
        };
        // SAFETY: `c` is a valid NUL-terminated path for the length of the call.
        unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dir;
        true
    }
}

/// Ask the current channel's feed for an update. On the stable channel a nightly build always
/// takes the latest stable, even when that's older, so switching back to stable works.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> CmdResult<Option<UpdateInfo>> {
    // `Env::appimage` only exists on Linux.
    #[cfg(target_os = "linux")]
    let appimage = app.env().appimage.map(PathBuf::from);
    #[cfg(not(target_os = "linux"))]
    let appimage: Option<PathBuf> = None;
    if let Some(why) = cannot_self_update(
        cfg!(target_os = "linux"),
        tauri::utils::platform::bundle_type(),
        appimage.as_deref(),
        dir_writable,
    ) {
        return Err(why);
    }
    let channel = app.state::<Updates>().settings.lock().unwrap().channel;
    let feed = match channel {
        ReleaseChannel::Stable => STABLE_FEED,
        ReleaseChannel::Nightly => NIGHTLY_FEED,
    };
    let updater = app
        .updater_builder()
        .endpoints(vec![Url::parse(feed).map_err(|e| e.to_string())?])
        .map_err(|e| e.to_string())?
        .version_comparator(move |current, remote| offers(channel, &current, &remote.version))
        // The check only. The download made from its result has no overall limit.
        .timeout(CHECK_TIMEOUT)
        // Both the check and the download.
        .configure_client(|client| {
            client
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(STALL_TIMEOUT)
        })
        .build()
        .map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| match e {
        // No latest.json: a channel nothing has been published to yet (or offline GitHub).
        tauri_plugin_updater::Error::ReleaseNotFound => match channel {
            ReleaseChannel::Stable => "No stable release with in-app updates yet.".to_string(),
            ReleaseChannel::Nightly => "No nightly build is available yet.".to_string(),
        },
        e => e.to_string(),
    })?;
    // The stable channel offers a nightly build the latest stable even when that's older. An
    // older build is only safe if it reads what this one has written: otherwise it refuses the
    // catalog (and the app can't start, to switch back), or opens it and drops the settings it
    // doesn't know from every photo it saves.
    if let Some(u) = &update {
        let current = &app.package_info().version;
        let older = crate::formats::parsed_like(current, &u.version)
            .is_some_and(|remote| &remote < current);
        if older && !crate::formats::reads_this_library(&u.raw_json) {
            return Err(format!(
                "tonality {} can’t open this library: this version ({current}) saves it in a \
                 newer format. Stay on Nightly until a newer stable release is out.",
                u.version
            ));
        }
    }
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        notes: u.body.clone().filter(|b| !b.trim().is_empty()),
        date: u
            .raw_json
            .get("pub_date")
            .and_then(|d| d.as_str())
            .map(String::from),
    });
    let u = app.state::<Updates>();
    // The settings may have changed channel while the feed was being asked.
    if u.settings.lock().unwrap().channel != channel {
        return Err("the release channel changed during the check".into());
    }
    let mut ready = u.ready.lock().unwrap();
    if ready.as_ref().map(|r| &r.0) != info.as_ref().map(|i| &i.version) {
        *ready = None;
    }
    *u.found.lock().unwrap() = update;
    Ok(info)
}

/// Download the update the last check found, reporting `update-progress`. The installer's
/// signature is checked against the app's public key when it's installed.
#[tauri::command]
pub async fn update_download(app: AppHandle) -> CmdResult<()> {
    let update = app
        .state::<Updates>()
        .found
        .lock()
        .unwrap()
        .clone()
        .ok_or("no update to download")?;
    if app
        .state::<Updates>()
        .ready
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|r| r.0 == update.version)
    {
        return Ok(());
    }
    let mut downloaded = 0usize;
    let bytes = update
        .download(
            |chunk, total| {
                downloaded += chunk;
                let _ = app.emit(
                    "update-progress",
                    UpdateProgress {
                        downloaded: downloaded as f64,
                        total: total.map(|t| t as f64),
                    },
                );
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    // Same order as `update_check` takes them.
    let u = app.state::<Updates>();
    let mut ready = u.ready.lock().unwrap();
    let found = u.found.lock().unwrap();
    // A channel switch or a newer check may have replaced what was being downloaded; its
    // installer mustn't then take the place of the one that's wanted now.
    if found.as_ref().map(|f| &f.version) != Some(&update.version) {
        return Err("the update changed while it was downloading".into());
    }
    *ready = Some((update.version.clone(), bytes));
    Ok(())
}

/// Install the downloaded update and restart into it. On Windows the installer takes over and
/// relaunches the app itself.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> CmdResult<()> {
    let update = app
        .state::<Updates>()
        .found
        .lock()
        .unwrap()
        .clone()
        .ok_or("no update to install")?;
    let u = app.state::<Updates>();
    let Some(bytes) = take_ready(&u.ready, &update.version) else {
        return Err("the update hasn't finished downloading".into());
    };
    // The app exits without its usual exit event, so write pending sidecars first.
    if let Err(e) = app.state::<crate::sidecar::Sidecars>().flush() {
        // Whatever stops the install, the download is kept for the next try.
        *u.ready.lock().unwrap() = Some((update.version, bytes));
        return Err(e);
    }
    let version = update.version.clone();
    let (result, bytes) = tauri::async_runtime::spawn_blocking(move || {
        let result = update.install(&bytes).map_err(|e| e.to_string());
        (result, bytes)
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Err(e) = result {
        *u.ready.lock().unwrap() = Some((version, bytes));
        return Err(e);
    }
    app.restart()
}

/// The downloaded installer for `version`, taken out of `ready`. A download for any other
/// version stays where it is.
fn take_ready(ready: &Mutex<Option<(String, Vec<u8>)>>, version: &str) -> Option<Vec<u8>> {
    let mut ready = ready.lock().unwrap();
    match ready.take() {
        Some((v, bytes)) if v == version => Some(bytes),
        other => {
            *ready = other;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ReleaseChannel::{Nightly, Stable};

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn which_version_an_install_takes() {
        const NIGHT: &str = "0.1.2-nightly.20261126.42";
        const NEXT_NIGHT: &str = "0.1.2-nightly.20261127.43";
        // (channel, running, the feed's version, offered?)
        let cases = [
            // A stable install follows newer stables, and nothing else.
            (Stable, "0.1.1", "0.1.1", false),
            (Stable, "0.1.1", "0.1.2", true),
            (Stable, "0.1.2", "0.1.1", false),
            // A nightly on the nightly channel follows newer nightlies.
            (Nightly, NIGHT, NEXT_NIGHT, true),
            (Nightly, NIGHT, NIGHT, false),
            (Nightly, NEXT_NIGHT, NIGHT, false),
            // Switching a nightly back to stable takes the latest stable though it's older...
            (Stable, NIGHT, "0.1.1", true),
            // ...and once that stable is running it isn't offered again, so there's no loop.
            (Stable, "0.1.1", "0.1.1", false),
            // A nightly on the stable channel also takes the stable release it led up to.
            (Stable, NIGHT, "0.1.2", true),
            // A stable on the nightly channel waits for a nightly newer than itself: the last
            // one built before its release is older.
            (Nightly, "0.1.2", NIGHT, false),
            (Nightly, "0.1.2", "0.1.3-nightly.20261201.50", true),
        ];
        for (channel, current, remote, expected) in cases {
            assert_eq!(
                offers(channel, &v(current), &v(remote)),
                expected,
                "{channel:?}: running {current}, feed has {remote}"
            );
        }
    }

    /// `scripts/version.mjs --nightly` makes the version every nightly is stamped with. The
    /// rule above only works if it sorts above the stable it follows and below the next one.
    #[test]
    fn nightly_versions_sort_between_two_stables() {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/version.mjs");
        let run = |args: &[&str]| {
            let out = std::process::Command::new("node")
                .arg(&script)
                .args(args)
                .env("NIGHTLY_BUILD", "42")
                .output()
                .expect("node runs scripts/version.mjs");
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        let stable = v(&run(&[]));
        let nightly = v(&run(&["--nightly"]));
        let next = Version::new(stable.major, stable.minor, stable.patch + 1);
        assert!(
            stable < nightly && nightly < next,
            "{stable} < {nightly} < {next}"
        );
        assert!(
            nightly.to_string().ends_with(".42"),
            "{nightly} carries the run number"
        );
        // A later run the same night, and a two-digit run after a one-digit one, sort later:
        // the parts are compared as numbers, not as text.
        let mut later = nightly.clone();
        later.pre = semver::Prerelease::new(&nightly.pre.as_str().replace(".42", ".100")).unwrap();
        assert!(later > nightly, "{later} > {nightly}");
        assert!(offers(Nightly, &nightly, &later));
        assert!(offers(Stable, &nightly, &stable) && offers(Stable, &nightly, &next));
    }

    #[test]
    fn only_a_writable_appimage_updates_itself_on_linux() {
        let appimage = Path::new("/home/someone/Apps/tonality.AppImage");
        let yes = |_: &Path| true;
        let no = |_: &Path| false;
        // Windows and macOS installs replace themselves through their own installers.
        assert_eq!(cannot_self_update(false, None, None, no), None);
        assert_eq!(
            cannot_self_update(true, Some(BundleType::AppImage), Some(appimage), yes),
            None
        );
        let read_only = cannot_self_update(true, Some(BundleType::AppImage), Some(appimage), no);
        assert!(
            read_only.is_some_and(|why| why.contains("/home/someone/Apps")),
            "names the folder that can't be written to"
        );
        for bundle in [Some(BundleType::Deb), Some(BundleType::Rpm), None] {
            assert!(
                cannot_self_update(true, bundle.clone(), None, yes).is_some(),
                "{bundle:?} can't install the feed's AppImage"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_read_only_folder_is_not_writable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("tonality-ro-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(dir_writable(&dir));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        // root can write anywhere, whatever the mode says.
        let root = unsafe { libc::geteuid() } == 0;
        assert_eq!(dir_writable(&dir), root);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(!dir_writable(&dir), "a folder that isn't there");
    }

    #[test]
    fn a_failed_install_keeps_the_download() {
        let ready = Mutex::new(Some(("1.2.3".to_string(), vec![1, 2, 3])));
        // Asking for another version leaves the download alone.
        assert_eq!(take_ready(&ready, "1.2.4"), None);
        assert!(ready.lock().unwrap().is_some());
        assert_eq!(take_ready(&ready, "1.2.3"), Some(vec![1, 2, 3]));
        assert!(ready.lock().unwrap().is_none());
    }
}
