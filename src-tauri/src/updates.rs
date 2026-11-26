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

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::CmdResult;

const STABLE_FEED: &str =
    "https://github.com/cobysmckinney/tonality/releases/latest/download/latest.json";
const NIGHTLY_FEED: &str =
    "https://github.com/cobysmckinney/tonality/releases/download/nightly/latest.json";

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

/// Ask the current channel's feed for an update. On the stable channel a nightly build always
/// takes the latest stable, even when that's older, so switching back to stable works.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> CmdResult<Option<UpdateInfo>> {
    let channel = app.state::<Updates>().settings.lock().unwrap().channel;
    let feed = match channel {
        ReleaseChannel::Stable => STABLE_FEED,
        ReleaseChannel::Nightly => NIGHTLY_FEED,
    };
    let updater = app
        .updater_builder()
        .endpoints(vec![Url::parse(feed).map_err(|e| e.to_string())?])
        .map_err(|e| e.to_string())?
        .version_comparator(move |current, remote| match channel {
            ReleaseChannel::Nightly => remote.version > current,
            ReleaseChannel::Stable => {
                remote.version > current || (!current.pre.is_empty() && remote.version != current)
            }
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
        let older = parsed_like(current, &u.version).is_some_and(|remote| &remote < current);
        if older && !reads_this_library(&u.raw_json) {
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

/// `text` as the same kind of value as `like` (a version, without naming its type here).
fn parsed_like<T: std::str::FromStr>(_like: &T, text: &str) -> Option<T> {
    text.parse().ok()
}

/// Whether the build a feed (`latest.json`) describes reads everything this build writes. The
/// feed says what its build reads under `tonality` (see `tonality_core::format`); one that
/// doesn't say (every release before this was added) is taken not to.
fn reads_this_library(feed: &serde_json::Value) -> bool {
    use tonality_core::Formats;
    feed.get("tonality")
        .and_then(|f| serde_json::from_value::<Formats>(f.clone()).ok())
        .is_some_and(|theirs| theirs.reads(&Formats::CURRENT))
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
    *app.state::<Updates>().ready.lock().unwrap() = Some((update.version.clone(), bytes));
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
    let bytes = match app.state::<Updates>().ready.lock().unwrap().take() {
        Some((v, b)) if v == update.version => b,
        _ => return Err("the update hasn't finished downloading".into()),
    };
    // The app exits without its usual exit event, so write pending sidecars first.
    app.state::<crate::sidecar::Sidecars>().flush()?;
    crate::blocking(move || update.install(&bytes).map_err(|e| e.to_string())).await?;
    app.restart()
}

#[cfg(test)]
mod format_tests {
    use super::*;
    use serde_json::json;
    use tonality_core::Formats;

    #[test]
    fn an_older_build_is_only_offered_if_it_reads_this_library() {
        let ours = Formats::CURRENT;
        let feed = |f: Formats| json!({ "version": "0.1.1", "tonality": f, "platforms": {} });
        assert!(reads_this_library(&feed(ours)));
        assert!(reads_this_library(&feed(Formats {
            catalog: ours.catalog + 1,
            ..ours
        })));
        // Behind in any one of them: it would refuse the catalog, or lose settings.
        for behind in [
            Formats {
                catalog: ours.catalog - 1,
                ..ours
            },
            Formats {
                edits: ours.edits - 1,
                ..ours
            },
        ] {
            assert!(!reads_this_library(&feed(behind)), "{behind:?}");
        }
        // A release from before feeds said (v0.1.1 and earlier), or one that says nonsense.
        assert!(!reads_this_library(&json!({ "version": "0.1.1" })));
        assert!(!reads_this_library(
            &json!({ "version": "0.1.1", "tonality": "yes" })
        ));
    }

    #[test]
    fn the_formats_flag_prints_what_the_feed_carries() {
        let printed: serde_json::Value = serde_json::from_str(&crate::formats_json()).unwrap();
        assert!(reads_this_library(&json!({ "tonality": printed })));
    }
}
