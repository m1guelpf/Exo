//! Asking before quitting: ⌘Q, the app menu's `Quit Exo` and the Dock's Quit show a native alert
//! first, since closing the window is how to put Exo away and quitting stops what it does in the
//! background. It shows while the settings menu's `Warn Before Quitting` is on, which it is to
//! start with. The alert's `Do not ask again` (macOS's own checkbox), ticked as the user quits,
//! turns the setting off; turning it on again brings the alert back. The setting is app-wide, in
//! `quit.json` in the state folder, beside `window.json`.
//!
//! macOS ending the session (logging out, restarting, shutting down) quits without asking, since
//! an alert then would cancel the logout. So does Exo quitting by itself, which goes straight to
//! AppKit's `terminate:`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

/// The alert's message.
#[cfg(target_os = "macos")]
pub(super) const MESSAGE: &str = "Are you sure you want to quit Exo?";
/// The alert's informative text.
#[cfg(target_os = "macos")]
pub(super) const INFORMATIVE_TEXT: &str =
    "Scheduled emails, reminders and notifications won't work until you open Exo again.";
/// The default button (Return).
#[cfg(target_os = "macos")]
pub(super) const QUIT: &str = "Quit";
/// The other button (Escape).
#[cfg(target_os = "macos")]
pub(super) const CANCEL: &str = "Cancel";

/// The file in the state folder.
const FILE_NAME: &str = "quit.json";

/// What the file holds.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    /// `Warn Before Quitting`.
    warn_before_quitting: bool,
}

/// `Warn Before Quitting`, Exo's own setting: whether quitting asks first. Cheap to clone; the
/// clones are one setting, which the quit alert and the settings menu share.
#[derive(Clone, Debug)]
pub struct QuitWarning(Arc<Warning>);

#[derive(Debug)]
struct Warning {
    path: PathBuf,
    on: Mutex<bool>,
}

impl QuitWarning {
    /// The setting kept in `state`, the state folder: on, unless it was turned off. A file that
    /// can't be read leaves it on.
    pub fn load(state: &Path) -> Self {
        let path = state.join(FILE_NAME);
        let saved = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Saved>(&bytes)
                .inspect_err(|error| log::warn!("ignoring {}: {error}", path.display()))
                .ok(),
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("reading {}: {error}", path.display());
                }
                None
            }
        };
        let on = saved.is_none_or(|saved| saved.warn_before_quitting);
        Self(Arc::new(Warning {
            path,
            on: Mutex::new(on),
        }))
    }

    /// Whether quitting asks first.
    pub fn is_on(&self) -> bool {
        *self.0.on.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Turns it on or off, and keeps that.
    pub fn set(&self, on: bool) {
        *self.0.on.lock().unwrap_or_else(PoisonError::into_inner) = on;
        let contents = serde_json::to_vec_pretty(&Saved {
            warn_before_quitting: on,
        })
        .expect("a flag makes JSON");
        let path = &self.0.path;
        if let Err(error) = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(path, contents))
        {
            log::warn!("writing {}: {error}", path.display());
        }
    }

    /// The alert was answered: `quit` or not, with `Do not ask again` ticked or not. Ticked with
    /// `Quit`, the setting goes off; ticked with `Cancel`, nothing changes. Whether to quit.
    pub fn answered(&self, quit: bool, dont_ask: bool) -> bool {
        if quit && dont_ask {
            self.set(false);
        }
        quit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("exo-quit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn it_warns_until_turned_off_and_keeps_that() {
        // The state folder isn't there yet on a first run.
        let dir = dir("warn");
        let warning = QuitWarning::load(&dir);
        assert!(warning.is_on());
        assert!(!warning.answered(false, true), "Cancel");
        assert!(
            warning.is_on(),
            "Cancel with the box ticked changes nothing"
        );
        assert!(warning.answered(true, false));
        assert!(
            warning.is_on(),
            "Quit with the box unticked changes nothing"
        );
        assert!(warning.answered(true, true));
        assert!(!warning.is_on(), "Quit with the box ticked turns it off");
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join(FILE_NAME)).expect("kept"))
                .expect("JSON");
        assert_eq!(saved, serde_json::json!({ "warn_before_quitting": false }));
        assert!(!QuitWarning::load(&dir).is_on(), "for good");
        warning.set(true);
        assert!(QuitWarning::load(&dir).is_on(), "until turned on");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_that_cant_be_read_warns() {
        let dir = dir("broken");
        std::fs::create_dir_all(&dir).expect("a folder");
        std::fs::write(dir.join(FILE_NAME), "not JSON").expect("written");
        assert!(QuitWarning::load(&dir).is_on());
        let _ = std::fs::remove_dir_all(dir);
    }
}
