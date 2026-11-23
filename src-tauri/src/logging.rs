//! The log file: what the app (and the libraries under it) had to say, and any panic, kept
//! where a user can find it and attach it to a bug report. Everything also still goes to
//! stderr, for `tauri dev`.
//!
//! One file, `tonality.log`, in the system's log folder for the app. When it has grown past
//! [`MAX_BYTES`] at launch it's set aside as `tonality.old.log` (replacing the last one), so
//! the two together never take more than a couple of megabytes.

use std::backtrace::Backtrace;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const NAME: &str = "tonality.log";
const OLD: &str = "tonality.old.log";
const MAX_BYTES: u64 = 1 << 20;

pub struct FileLog {
    path: PathBuf,
    file: Mutex<File>,
}

impl FileLog {
    /// Open the log in `dir` (created if missing), setting a full one aside first.
    pub fn open(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(NAME);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
            std::fs::rename(&path, dir.join(OLD))?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    /// Append one entry: local time, `level`, then `text` (which may run over several lines).
    pub fn line(&self, level: &str, text: &str) {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        // A thread that panicked while logging mustn't silence the log for everyone else.
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        // Nowhere to report a failure to write the log.
        let _ = writeln!(file, "{now} {level:<5} {text}");
    }
}

static LOG: OnceLock<FileLog> = OnceLock::new();

/// Forwards the `log` crate's records (ours, and warnings from wgpu, tauri and the rest) to
/// the file and to stderr.
struct Logger;

impl log::Log for Logger {
    fn enabled(&self, meta: &log::Metadata) -> bool {
        // Our own crates at Info; everything else only when something's wrong.
        let ours = meta.target().starts_with("tonality");
        meta.level()
            <= if ours {
                log::Level::Info
            } else {
                log::Level::Warn
            }
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let text = format!("[{}] {}", record.target(), record.args());
        eprintln!("{} {text}", record.level());
        if let Some(log) = LOG.get() {
            log.line(record.level().as_str(), &text);
        }
    }

    fn flush(&self) {}
}

/// Start logging to `dir`, and record panics there too. Call once, before anything logs.
pub fn init(dir: &Path) -> io::Result<()> {
    let log = FileLog::open(dir)?;
    log.line(
        "INFO",
        &format!(
            "tonality {} on {} {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    );
    if LOG.set(log).is_err() {
        return Ok(());
    }
    // Only fails if a logger is already installed, which is then the one to keep.
    if log::set_logger(&Logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(log) = LOG.get() {
            let thread = std::thread::current();
            let name = thread.name().unwrap_or("unnamed");
            log.line(
                "PANIC",
                &format!("thread '{name}': {info}\n{}", Backtrace::force_capture()),
            );
        }
        // Then what Rust would have done anyway: the message on stderr.
        default(info);
    }));
    Ok(())
}

/// Where the log file is, once [`init`] has run.
pub fn path() -> Option<&'static Path> {
    LOG.get().map(|l| l.path.as_path())
}

/// The log file, for the Settings dialog to show in the file manager.
#[tauri::command]
pub fn log_path() -> Option<String> {
    path().map(|p| p.to_string_lossy().into_owned())
}

/// Messages from the interface: uncaught errors always, and its console in development.
#[tauri::command]
pub fn ui_log(level: String, message: String) {
    match level.as_str() {
        "error" => log::error!(target: "tonality::ui", "{message}"),
        "warn" => log::warn!(target: "tonality::ui", "{message}"),
        _ => log::info!(target: "tonality::ui", "{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tonality-log-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn entries_are_appended_with_their_level() {
        let dir = scratch("append");
        FileLog::open(&dir).unwrap().line("WARN", "first");
        // A second launch adds to the same file.
        FileLog::open(&dir).unwrap().line("ERROR", "second");
        let text = std::fs::read_to_string(dir.join(NAME)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].ends_with("WARN  first"), "{text}");
        assert!(lines[1].ends_with("ERROR second"), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_full_log_is_set_aside_at_launch() {
        let dir = scratch("rotate");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(NAME), vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        std::fs::write(dir.join(OLD), b"older still").unwrap();
        FileLog::open(&dir).unwrap().line("INFO", "fresh");
        let old = std::fs::metadata(dir.join(OLD)).unwrap().len();
        assert_eq!(
            old,
            MAX_BYTES + 1,
            "the full log replaces the last one set aside"
        );
        let text = std::fs::read_to_string(dir.join(NAME)).unwrap();
        assert!(
            text.ends_with("INFO  fresh\n") && text.lines().count() == 1,
            "{text}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn panics_and_log_records_reach_the_file() {
        let dir = scratch("panic");
        init(&dir).unwrap();
        log::warn!(target: "tonality::test", "careful-7c1");
        log::info!(target: "some_library", "chatter-7c1");
        let caught = std::panic::catch_unwind(|| panic!("kaboom-7c1"));
        assert!(caught.is_err());
        let text = std::fs::read_to_string(path().unwrap()).unwrap();
        assert!(
            text.contains("tonality "),
            "starts with the version line: {text}"
        );
        assert!(
            text.contains("WARN  [tonality::test] careful-7c1"),
            "{text}"
        );
        assert!(
            !text.contains("chatter-7c1"),
            "other crates only at warn and up: {text}"
        );
        assert!(
            text.contains("PANIC") && text.contains("kaboom-7c1"),
            "{text}"
        );
        assert!(
            text.contains("logging.rs"),
            "says where it panicked: {text}"
        );
    }
}
