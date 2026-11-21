//! Move to Applications: Exo.app launched from outside an Applications folder asks to move
//! itself there before its window opens, and if so moves and opens again from there. Where it is
//! can be changed, it asks once (`Move to Applications`, `Do Not Move`), and never again once
//! declined; where it can't, it says it needs to be moved (`Move to Applications`, `Cancel`,
//! which quits).
//!
//! An app downloaded from the internet and opened where it is runs from a read-only copy macOS
//! makes of it (App Translocation): Exo looks past the copy to the app itself. Moving copies the
//! app into `/Applications` (`~/Applications` when that isn't writable), replacing one there,
//! takes the download quarantine off the copy (which would have macOS translocate it again: an
//! app moved by anything but the Finder stays quarantined), puts the original in the Trash, and
//! opens the copy once this Exo has quit.

use std::path::{Path, PathBuf};

/// The file in the state folder recording that the move was declined.
const FILE_NAME: &str = "moving.json";

/// What the file holds.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Saved {
    declined_moving: bool,
}

/// What launching should do after asking.
#[derive(Debug, PartialEq, Eq)]
pub enum Moved {
    /// Run from where Exo is.
    Stay,
    /// Exo moved, and opens again from there once this one quits: quit now.
    Relaunching,
    /// Cancelled where Exo can't run from: quit.
    Quit,
}

/// Where Exo.app is, the original when macOS runs a translocated copy, and whether it can be
/// changed there.
struct Location {
    app: PathBuf,
    writable: bool,
}

/// Offers to move Exo.app into an Applications folder, as it launches from outside one: see the
/// module docs. `state` is the state folder, which keeps a declined offer. A bare binary, or an
/// app already in an Applications folder, isn't asked about. Call on the main thread before the
/// event loop starts.
pub fn offer(state: &Path) -> Moved {
    let Some(location) = macos::location() else {
        return Moved::Stay;
    };
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    ask(state, &location, &home)
}

/// Asks where `location` isn't in an Applications folder under `home` or the system's, and moves
/// as answered.
fn ask(state: &Path, location: &Location, home: &Path) -> Moved {
    if in_applications(&location.app, home) {
        return Moved::Stay;
    }
    let path = state.join(FILE_NAME);
    let declined = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
        .is_some_and(|saved| saved.declined_moving);
    if location.writable && declined {
        return Moved::Stay;
    }
    let question = if location.writable {
        Question::Offer {
            detail: format!(
                "Would you like to move Exo to the Applications folder?{}",
                where_it_is(&location.app, home)
            ),
        }
    } else {
        Question::Require
    };
    match (macos::confirm(&question), location.writable) {
        (true, _) => match relocate(
            &location.app,
            &destination(home),
            location.writable,
            macos::trash,
        ) {
            Ok(moved) => {
                relaunch(&moved);
                Moved::Relaunching
            }
            // Exo runs on where it is.
            Err(error) => {
                log::error!("moving Exo to the Applications folder: {error}");
                Moved::Stay
            }
        },
        (false, true) => {
            let contents = serde_json::to_vec_pretty(&Saved {
                declined_moving: true,
            })
            .expect("a flag makes JSON");
            if let Err(error) =
                std::fs::create_dir_all(state).and_then(|()| std::fs::write(&path, contents))
            {
                log::warn!("writing {}: {error}", path.display());
            }
            Moved::Stay
        }
        (false, false) => Moved::Quit,
    }
}

/// The dialog: a question, its message `Move to Applications`.
enum Question {
    /// Where Exo can be changed: `detail`, `Move to Applications` (default) and `Do Not Move`
    /// (Escape).
    Offer { detail: String },
    /// Where it can't: `Exo needs to be in the Applications folder to run.`, `Move to
    /// Applications` and `Cancel`.
    Require,
}

/// Whether `app` is in `/Applications` or `~/Applications`, or a folder in either.
fn in_applications(app: &Path, home: &Path) -> bool {
    app.starts_with("/Applications") || app.starts_with(home.join("Applications"))
}

/// The detail's second line: in Downloads, on the Desktop, or none.
fn where_it_is(app: &Path, home: &Path) -> &'static str {
    if app.starts_with(home.join("Downloads")) {
        "\nIt is currently in your Downloads folder."
    } else if app.starts_with(home.join("Desktop")) {
        "\nIt is currently on your Desktop."
    } else {
        ""
    }
}

/// Where Exo moves to: `/Applications` when it can be written to, else `~/Applications`.
fn destination(home: &Path) -> PathBuf {
    let system = Path::new("/Applications");
    if writable(system) {
        system.to_owned()
    } else {
        home.join("Applications")
    }
}

/// Whether this user can change what's in `dir`.
fn writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: a NUL-terminated path.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

/// Copies `app` into `folder` (made if missing), replacing what's there by its name (into the
/// Trash, by `trash`), takes the download quarantine off the copy, and with `remove` puts `app`
/// in the Trash. Returns the copy.
fn relocate(
    app: &Path,
    folder: &Path,
    remove: bool,
    trash: impl Fn(&Path) -> std::io::Result<()>,
) -> std::io::Result<PathBuf> {
    let name = app
        .file_name()
        .ok_or_else(|| std::io::Error::other("the app has no name"))?;
    let moved = folder.join(name);
    std::fs::create_dir_all(folder)?;
    if moved.exists() {
        trash(&moved)?;
    }
    // `ditto` keeps what an app bundle needs: symlinks, extended attributes, its signature.
    run(std::process::Command::new("/usr/bin/ditto")
        .arg(app)
        .arg(&moved))?;
    run(std::process::Command::new("/usr/bin/xattr")
        .args(["-d", "-r", "com.apple.quarantine"])
        .arg(&moved))
    .or_else(|error| {
        // Nothing to take off.
        if has_quarantine(&moved) {
            Err(error)
        } else {
            Ok(())
        }
    })?;
    if remove {
        trash(app)?;
    }
    Ok(moved)
}

fn run(command: &mut std::process::Command) -> std::io::Result<()> {
    let output = command.output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "{command:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

/// Whether anything in `app` still carries the download quarantine.
fn has_quarantine(app: &Path) -> bool {
    std::process::Command::new("/usr/bin/xattr")
        .args(["-r", "-p", "com.apple.quarantine"])
        .arg(app)
        .output()
        .is_ok_and(|output| !output.stdout.is_empty())
}

/// Opens `app` once this process has quit (and let go of the one-Exo lock).
fn relaunch(app: &Path) {
    let script =
        "while /bin/kill -0 \"$1\" >/dev/null 2>&1; do /bin/sleep 0.1; done; /usr/bin/open \"$2\"";
    let spawned = std::process::Command::new("/bin/sh")
        .args(["-c", script, "sh"])
        .arg(std::process::id().to_string())
        .arg(app)
        .spawn();
    if let Err(error) = spawned {
        log::error!("opening Exo again from the Applications folder: {error}");
    }
}

mod macos {
    use std::ffi::c_void;
    use std::path::Path;

    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication};
    use objc2_foundation::{NSBundle, NSFileManager, NSString, NSURL, ns_string};

    use super::{Location, Question};

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        /// `SecTranslocateIsTranslocatedURL` (`Security/SecTranslocate.h`).
        fn SecTranslocateIsTranslocatedURL(
            path: *const c_void,
            translocated: *mut bool,
            error: *mut *const c_void,
        ) -> u8;
        /// `SecTranslocateCreateOriginalPathForURL`: a +1 `CFURLRef`, or null.
        fn SecTranslocateCreateOriginalPathForURL(
            translocated: *const c_void,
            error: *mut *const c_void,
        ) -> *mut NSURL;
    }

    /// Where Exo.app is: past its translocated copy, if macOS runs one. `None` for a bare binary.
    pub(super) fn location() -> Option<Location> {
        let bundle = NSBundle::mainBundle();
        bundle.bundleIdentifier()?;
        let url = bundle.bundleURL();
        let original = original(&url).unwrap_or(url);
        let app = original.to_file_path()?;
        let writable = app.parent().is_some_and(super::writable) && super::writable(&app);
        Some(Location { app, writable })
    }

    /// The app a translocated copy at `url` was made from; `None` when `url` isn't one.
    fn original(url: &NSURL) -> Option<Retained<NSURL>> {
        let url = std::ptr::from_ref(url).cast::<c_void>();
        let mut translocated = false;
        // SAFETY: an `NSURL` is a `CFURLRef` (toll-free bridged); the errors aren't asked for.
        let asked = unsafe {
            SecTranslocateIsTranslocatedURL(url, &mut translocated, std::ptr::null_mut())
        };
        if asked == 0 || !translocated {
            return None;
        }
        // SAFETY: as above; the original comes back retained, which `from_raw` takes over.
        unsafe {
            Retained::from_raw(SecTranslocateCreateOriginalPathForURL(
                url,
                std::ptr::null_mut(),
            ))
        }
    }

    /// Asks `question`, in front; whether to move.
    pub(super) fn confirm(question: &Question) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let app = NSApplication::sharedApplication(mtm);
        // `app.focus()`. `activate` is macOS 14's, and Exo runs on 13.
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        let alert = NSAlert::new(mtm);
        // AppKit's informational alert, with the app's icon.
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(ns_string!("Move to Applications"));
        alert.addButtonWithTitle(ns_string!("Move to Applications"));
        match question {
            Question::Offer { detail } => {
                alert.setInformativeText(&NSString::from_str(detail));
                // Escape declines.
                let declining = alert.addButtonWithTitle(ns_string!("Do Not Move"));
                declining.setKeyEquivalent(ns_string!("\u{1b}"));
            }
            Question::Require => {
                alert.setInformativeText(ns_string!(
                    "Exo needs to be in the Applications folder to run."
                ));
                alert.addButtonWithTitle(ns_string!("Cancel"));
            }
        }
        alert.runModal() == NSAlertFirstButtonReturn
    }

    /// Puts `path` in the Trash.
    pub(super) fn trash(path: &Path) -> std::io::Result<()> {
        let url =
            NSURL::from_file_path(path).ok_or_else(|| std::io::Error::other("not a file path"))?;
        NSFileManager::defaultManager()
            .trashItemAtURL_resultingItemURL_error(&url, None)
            .map_err(|error| std::io::Error::other(error.localizedDescription().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/Users/maya")
    }

    #[test]
    fn applications_folders_and_their_subfolders_need_no_move() {
        for app in [
            "/Applications/Exo.app",
            "/Applications/Mail/Exo.app",
            "/Users/maya/Applications/Exo.app",
        ] {
            assert!(in_applications(Path::new(app), &home()), "{app}");
        }
        for app in [
            "/Users/maya/Downloads/Exo.app",
            "/Applications Old/Exo.app",
            "/Users/maya/Desktop/Exo.app",
            "/Volumes/Exo/Exo.app",
        ] {
            assert!(!in_applications(Path::new(app), &home()), "{app}");
        }
    }

    #[test]
    fn the_detail_says_where_exo_is() {
        assert_eq!(
            where_it_is(Path::new("/Users/maya/Downloads/Exo.app"), &home()),
            "\nIt is currently in your Downloads folder."
        );
        assert_eq!(
            where_it_is(Path::new("/Users/maya/Downloads/Exo 2/Exo.app"), &home()),
            "\nIt is currently in your Downloads folder."
        );
        assert_eq!(
            where_it_is(Path::new("/Users/maya/Desktop/Exo.app"), &home()),
            "\nIt is currently on your Desktop."
        );
        assert_eq!(where_it_is(Path::new("/Volumes/Exo/Exo.app"), &home()), "");
    }

    /// A folder for one test, empty.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("exo-move-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        dir
    }

    /// A downloaded app: a bundle with a link and a quarantined file.
    fn downloaded(dir: &Path) -> PathBuf {
        let app = dir.join("Downloads/Exo.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).expect("a bundle");
        std::fs::write(app.join("Contents/MacOS/Exo"), b"#!/bin/sh\n").expect("an executable");
        std::os::unix::fs::symlink("MacOS/Exo", app.join("Contents/Current")).expect("a link");
        let quarantine = "0083;6740a000;Safari;";
        run(std::process::Command::new("/usr/bin/xattr")
            .args(["-w", "com.apple.quarantine", quarantine])
            .arg(app.join("Contents/MacOS/Exo")))
        .expect("quarantined");
        assert!(has_quarantine(&app));
        app
    }

    #[test]
    fn moving_copies_the_app_without_its_quarantine_and_trashes_the_original() {
        let dir = scratch("move");
        let app = downloaded(&dir);
        let trashed = std::cell::RefCell::new(Vec::new());
        let trash = |path: &Path| {
            trashed.borrow_mut().push(path.to_owned());
            std::fs::remove_dir_all(path)
        };
        let folder = dir.join("Applications");
        std::fs::create_dir_all(folder.join("Exo.app")).expect("an older Exo");
        let moved = relocate(&app, &folder, true, trash).expect("moved");
        assert_eq!(moved, folder.join("Exo.app"));
        assert_eq!(
            std::fs::read(moved.join("Contents/MacOS/Exo")).expect("copied"),
            b"#!/bin/sh\n"
        );
        assert_eq!(
            std::fs::read_link(moved.join("Contents/Current")).expect("a link still"),
            Path::new("MacOS/Exo")
        );
        assert!(
            !has_quarantine(&moved),
            "not quarantined, so not translocated"
        );
        assert_eq!(
            *trashed.borrow(),
            [folder.join("Exo.app"), app.clone()],
            "the older Exo, then the download"
        );
        assert!(!app.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_app_that_cant_be_changed_where_it_is_is_copied_and_left() {
        let dir = scratch("copy");
        let app = downloaded(&dir);
        let moved = relocate(&app, &dir.join("Applications"), false, |_| {
            Err(std::io::Error::other("not asked"))
        })
        .expect("copied");
        assert!(moved.join("Contents/MacOS/Exo").exists() && app.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_declined_offer_isnt_made_again_and_an_app_in_applications_isnt_asked_about() {
        let dir = scratch("declined");
        let location = |app: &str, writable| Location {
            app: PathBuf::from(app),
            writable,
        };
        assert_eq!(
            ask(&dir, &location("/Applications/Exo.app", true), &home()),
            Moved::Stay
        );
        std::fs::write(dir.join(FILE_NAME), r#"{"declined_moving": true}"#).expect("declined");
        assert_eq!(
            ask(
                &dir,
                &location("/Users/maya/Downloads/Exo.app", true),
                &home()
            ),
            Moved::Stay,
            "asked once"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
