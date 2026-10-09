//! The files and folders outside the library the person has pointed the app
//! at: chosen in one of its dialogs, or dropped on its window. Commands that
//! read or write outside the library take only these (or a camera card, or
//! the export folder used last time), so the interface can't be made to reach
//! anywhere else on the computer.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{bail, Result};

#[derive(Default)]
pub struct Grants(Mutex<HashSet<PathBuf>>);

impl Grants {
    /// Lets commands use `paths`, as the person chose them.
    pub fn allow(&self, paths: impl IntoIterator<Item = PathBuf>) {
        self.0.lock().unwrap().extend(paths);
    }

    /// Whether `path` was chosen or dropped. Only the path itself counts, not
    /// what is inside a folder or next to a file: an import reads inside a
    /// folder it was given, but nothing else may be reached through it.
    pub fn allows(&self, path: &Path) -> bool {
        self.0.lock().unwrap().contains(path)
    }

    /// Fails unless every one of `paths` was chosen or dropped, or is also in `also`.
    pub fn check<'a>(&self, paths: impl IntoIterator<Item = &'a Path>, also: &[PathBuf]) -> Result<()> {
        for path in paths {
            if !self.allows(path) && !also.iter().any(|other| other == path) {
                bail!("Tonality wasn't given {}. Choose it again.", path.display());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_was_chosen_is_allowed() {
        let grants = Grants::default();
        grants.allow([PathBuf::from("/home/me/Pictures/card"), PathBuf::from("/home/me/look.tonality-preset")]);
        assert!(grants.allows(Path::new("/home/me/Pictures/card")));
        assert!(grants.check([Path::new("/home/me/look.tonality-preset")], &[]).is_ok());
        assert!(!grants.allows(Path::new("/home/me/.ssh")), "never asked for");
        assert!(!grants.allows(Path::new("/home/me/Pictures/card/../../.ssh")), "climbs out of a chosen folder");
        assert!(!grants.allows(Path::new("/home/me/Pictures")), "the folder around a chosen one");
    }

    #[test]
    fn a_check_fails_on_any_path_not_chosen() {
        let grants = Grants::default();
        grants.allow([PathBuf::from("/a")]);
        assert!(grants.check([Path::new("/a"), Path::new("/b")], &[]).is_err());
        assert!(grants.check([Path::new("/a"), Path::new("/b")], &[PathBuf::from("/b")]).is_ok(), "a camera card");
        assert!(grants.check([], &[]).is_ok());
    }
}
