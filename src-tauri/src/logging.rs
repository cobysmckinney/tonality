//! The log file: what the app (and the libraries under it) had to say, and any panic, kept
//! where a user can find it and attach it to a bug report. Everything also still goes to
//! stderr, for `tauri dev`.
//!
//! One file, `tonality.log`, in the system's log folder for the app. When the next entry would
//! take it past [`MAX_BYTES`] it's set aside as `tonality.old.log` (replacing the last one), so
//! the two together never take more than a couple of megabytes, however long the app is open.

use std::backtrace::Backtrace;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const NAME: &str = "tonality.log";
const OLD: &str = "tonality.old.log";
const MAX_BYTES: u64 = 1 << 20;
/// The most of one message from the interface that is kept: an error there can carry a whole
/// serialised object, and can repeat on every frame.
const MAX_UI_MESSAGE: usize = 4096;

pub struct FileLog {
    path: PathBuf,
    open: Mutex<Open>,
}

/// The file being appended to, and how big it is.
struct Open {
    file: File,
    bytes: u64,
}

impl FileLog {
    /// Open the log in `dir` (created if missing), setting a full one aside first.
    pub fn open(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(NAME);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
            std::fs::rename(&path, dir.join(OLD))?;
        }
        let open = Self::append(&path)?;
        Ok(Self {
            path,
            open: Mutex::new(open),
        })
    }

    fn append(path: &Path) -> io::Result<Open> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let bytes = file.metadata()?.len();
        Ok(Open { file, bytes })
    }

    /// Append one entry: local time, `level`, then `text` (which may run over several lines).
    pub fn line(&self, level: &str, text: &str) {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let entry = format!("{now} {level:<5} {text}\n");
        // A thread that panicked while logging mustn't silence the log for everyone else.
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        // Set a full file aside before adding to it, so it's capped while the app runs and not
        // only at launch. If that fails, keep writing to the one that's open.
        if open.bytes > 0 && open.bytes + entry.len() as u64 > MAX_BYTES {
            let old = self.path.with_file_name(OLD);
            if std::fs::rename(&self.path, old).is_ok() {
                if let Ok(fresh) = Self::append(&self.path) {
                    *open = fresh;
                }
            }
        }
        // Nowhere to report a failure to write the log.
        if open.file.write_all(entry.as_bytes()).is_ok() {
            open.bytes += entry.len() as u64;
        }
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
        // The file first: it's the copy that lasts.
        if let Some(log) = LOG.get() {
            log.line(record.level().as_str(), &text);
        }
        // Not `eprintln!`, which panics when the write fails: stderr can be a terminal that has
        // since closed, and a log message must never take down the thread that wrote it.
        let _ = writeln!(io::stderr().lock(), "{} {text}", record.level());
    }

    fn flush(&self) {}
}

/// Start logging to `dir`, and record panics there too. Call once, before anything logs.
/// `version` is the app's own (the Tauri config's): a nightly is only versioned there, and the
/// Cargo package version would name the stable release before it.
pub fn init(dir: &Path, version: &str) -> io::Result<()> {
    let log = FileLog::open(dir)?;
    log.line(
        "INFO",
        &format!(
            "tonality {version} on {} {}",
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
    let message = clip(&message, MAX_UI_MESSAGE);
    match level.as_str() {
        "error" => log::error!(target: "tonality::ui", "{message}"),
        "warn" => log::warn!(target: "tonality::ui", "{message}"),
        _ => log::info!(target: "tonality::ui", "{message}"),
    }
}

/// At most `max` bytes of `text`, cut at a character boundary, with a mark when it was cut.
fn clip(text: &str, max: usize) -> std::borrow::Cow<'_, str> {
    if text.len() <= max {
        return text.into();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… ({} bytes in all)", &text[..end], text.len()).into()
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
    fn a_log_that_fills_while_the_app_is_open_is_set_aside() {
        let dir = scratch("session");
        let log = FileLog::open(&dir).unwrap();
        let entry = "x".repeat(1000);
        // Three times the limit, in one session.
        for _ in 0..(3 * MAX_BYTES as usize / entry.len()) {
            log.line("WARN", &entry);
        }
        let now = std::fs::metadata(dir.join(NAME)).unwrap().len();
        let old = std::fs::metadata(dir.join(OLD)).unwrap().len();
        assert!(now > 0 && now <= MAX_BYTES, "current log is {now} bytes");
        assert!(old > 0 && old <= MAX_BYTES, "set-aside log is {old} bytes");
        // Nothing lost at the seam: the newest entry is in the current file.
        log.line("INFO", "newest");
        let text = std::fs::read_to_string(dir.join(NAME)).unwrap();
        assert!(text.ends_with("INFO  newest\n"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_long_message_from_the_interface_is_cut_at_a_character() {
        assert_eq!(clip("short", 10), "short");
        // "é" is two bytes; a cut in the middle of one backs up to before it.
        let long = "é".repeat(10);
        let cut = clip(&long, 5);
        assert!(cut.starts_with("éé…"), "{cut}");
        assert!(cut.contains("20 bytes"), "{cut}");
    }

    #[test]
    fn panics_and_log_records_reach_the_file() {
        let dir = scratch("panic");
        init(&dir, "9.8.7-nightly.20261126.1").unwrap();
        log::warn!(target: "tonality::test", "careful-7c1");
        log::info!(target: "some_library", "chatter-7c1");
        let caught = std::panic::catch_unwind(|| panic!("kaboom-7c1"));
        assert!(caught.is_err());
        let text = std::fs::read_to_string(path().unwrap()).unwrap();
        assert!(
            text.contains("tonality 9.8.7-nightly.20261126.1 on "),
            "starts with the app's version, not the Cargo package's: {text}"
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
