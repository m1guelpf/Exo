//! Where attachments download to: one folder for the whole app rather than one in an account's
//! settings, kept in `downloads.json` beside the account list. Unset, it's the user's Downloads
//! folder. The app's [`egui::Context`] carries it, so every account's mail sees a change at
//! once.

use std::path::{Path, PathBuf};

use egui::{Context, Id};

/// The file in the state folder.
const FILE_NAME: &str = "downloads.json";

/// What the file holds: electron-store's key.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Saved {
    downloads_location: Option<PathBuf>,
}

/// The location and where it's kept, in the context's data.
#[derive(Clone, Debug, Default)]
struct Location {
    /// `downloads.json`; `None` keeps a change for this run only (screenshots and tests).
    file: Option<PathBuf>,
    /// The folder chosen; `None` for the Downloads folder.
    folder: Option<PathBuf>,
}

fn id() -> Id {
    Id::new("exo::downloads")
}

/// Why the folder couldn't change (`Could not change download folder`).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The folder is inside Exo's own app bundle.
    #[error("{0} is inside the app")]
    InsideApp(PathBuf),
    /// The choice couldn't be saved.
    #[error("saving the download folder: {0}")]
    Save(#[from] std::io::Error),
}

/// Reads the folder saved in `state` (the state folder) into `ctx`'s app. A file that can't be
/// read, or a folder inside the app, is the Downloads folder.
pub fn install(ctx: &Context, state: &Path) {
    let file = state.join(FILE_NAME);
    let saved = match std::fs::read(&file) {
        Ok(bytes) => serde_json::from_slice::<Saved>(&bytes).unwrap_or_else(|error| {
            log::warn!("ignoring {}: {error}", file.display());
            Saved::default()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Saved::default(),
        Err(error) => {
            log::warn!("reading {}: {error}", file.display());
            Saved::default()
        }
    };
    let folder = saved
        .downloads_location
        .filter(|folder| !inside_app(folder));
    ctx.data_mut(|data| {
        data.insert_temp(
            id(),
            Location {
                file: Some(file),
                folder,
            },
        )
    });
}

/// The user's Downloads folder.
pub fn downloads_folder() -> Option<PathBuf> {
    home().map(|home| home.join("Downloads"))
}

/// The user's Desktop.
pub fn desktop_folder() -> Option<PathBuf> {
    home().map(|home| home.join("Desktop"))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The folder chosen, or `None` for the Downloads folder.
pub fn chosen(ctx: &Context) -> Option<PathBuf> {
    ctx.data(|data| data.get_temp::<Location>(id()))
        .and_then(|location| location.folder)
}

/// Where downloads go now.
pub fn folder(ctx: &Context) -> Option<PathBuf> {
    chosen(ctx).or_else(downloads_folder)
}

/// Makes `folder` where downloads go, and saves it.
///
/// # Errors
/// [`Error::InsideApp`] for a folder inside Exo itself, and [`Error::Save`] when the choice can't
/// be written; either way nothing changes.
pub fn choose(ctx: &Context, folder: PathBuf) -> Result<(), Error> {
    let folder = std::path::absolute(&folder).unwrap_or(folder);
    if inside_app(&folder) {
        return Err(Error::InsideApp(folder));
    }
    let mut location = ctx
        .data(|data| data.get_temp::<Location>(id()))
        .unwrap_or_default();
    if let Some(file) = &location.file {
        let saved = Saved {
            downloads_location: Some(folder.clone()),
        };
        let json = serde_json::to_vec_pretty(&saved).expect("the location always serializes");
        let partial = file.with_extension("json.partial");
        std::fs::write(&partial, json)?;
        std::fs::rename(&partial, file)?;
    }
    location.folder = Some(folder);
    ctx.data_mut(|data| data.insert_temp(id(), location));
    Ok(())
}

/// Whether `folder` is Exo's app bundle or inside it: downloads must never overwrite the app.
fn inside_app(folder: &Path) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let bundle = exe.ancestors().find(|ancestor| {
        ancestor
            .extension()
            .is_some_and(|extension| extension == "app")
    });
    let install = exe.parent();
    [bundle, install]
        .into_iter()
        .flatten()
        .any(|root| folder.starts_with(root))
}

/// The folder the user picks in a folder chooser that starts at `current` (Settings' `Other…`:
/// folders only, with New Folder, its button `Select`); `None` when they cancel. Runs the panel
/// modally.
#[cfg(target_os = "macos")]
pub fn pick(current: Option<&Path>) -> Option<PathBuf> {
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{NSString, NSURL};

    let Some(main_thread) = objc2::MainThreadMarker::new() else {
        log::warn!("the folder chooser opens on the main thread only");
        return None;
    };
    let panel = NSOpenPanel::openPanel(main_thread);
    panel.setCanChooseFiles(false);
    panel.setCanChooseDirectories(true);
    panel.setCanCreateDirectories(true);
    panel.setAllowsMultipleSelection(false);
    panel.setPrompt(Some(&NSString::from_str("Select")));
    if let Some(url) = current.and_then(NSURL::from_directory_path) {
        panel.setDirectoryURL(Some(&url));
    }
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    panel.URLs().iter().find_map(|url| url.to_file_path())
}

/// No folder chooser off macOS yet.
#[cfg(not(target_os = "macos"))]
pub fn pick(_current: Option<&Path>) -> Option<PathBuf> {
    log::warn!("no folder chooser on this platform");
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "exo-downloads-location-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("a temp folder");
        dir
    }

    #[test]
    fn a_choice_lasts_and_reaches_every_account() {
        let dir = state();
        let ctx = Context::default();
        install(&ctx, &dir);
        assert_eq!(
            folder(&ctx),
            downloads_folder(),
            "the Downloads folder until one's chosen"
        );
        let desktop = desktop_folder().expect("a home folder");
        choose(&ctx, desktop.clone()).expect("saves");
        assert_eq!(folder(&ctx), Some(desktop.clone()));
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join(FILE_NAME)).expect("written"))
                .expect("JSON");
        assert_eq!(
            json["downloadsLocation"],
            desktop.to_string_lossy().as_ref(),
            "electron-store's key"
        );
        let relaunched = Context::default();
        install(&relaunched, &dir);
        assert_eq!(
            chosen(&relaunched),
            Some(desktop),
            "read back at the next launch"
        );
        std::fs::remove_dir_all(dir).expect("cleans up");
    }

    #[test]
    fn the_app_itself_is_refused() {
        let ctx = Context::default();
        let exe = std::env::current_exe().expect("the test binary");
        let inside = exe.parent().expect("its folder").join("downloads");
        assert!(matches!(choose(&ctx, inside), Err(Error::InsideApp(_))));
        assert_eq!(chosen(&ctx), None, "nothing changed");
    }
}
