//! Where the window opens: where it was left, at the size it had, maximized or in full screen if it
//! was, kept in `window.json` beside the account list. A place on a display that's gone is dropped
//! for the default.

use std::path::{Path, PathBuf};

use fastframe_shell::window::Geometry;

/// The file in the state folder.
const FILE_NAME: &str = "window.json";

/// What the file holds: the normal window's inner size and position in points, and whether it
/// was maximized or in full screen.
#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Saved {
    size: Option<[f32; 2]>,
    position: Option<[f32; 2]>,
    maximized: bool,
    full_screen: bool,
}

/// The window's place, as saved and as it changes.
#[derive(Clone, Debug)]
pub struct Placement {
    path: PathBuf,
    geometry: Geometry,
    full_screen: bool,
}

impl Placement {
    /// The place saved in `dir`, the state folder. With none, or one that can't be read, the
    /// window opens at its default place.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(FILE_NAME);
        let saved = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                log::warn!("ignoring {}: {error}", path.display());
                Saved::default()
            }),
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("reading {}: {error}", path.display());
                }
                Saved::default()
            }
        };
        let geometry = Geometry {
            size: saved.size,
            position: saved.position,
            maximized: saved.maximized,
        };
        Self {
            path,
            geometry,
            full_screen: saved.full_screen,
        }
    }

    /// `builder`, which has the default place, opening the window where it was instead: at its
    /// size and position (unless no screen shows that position any more), maximized if it was,
    /// in full screen if it was.
    pub fn apply(&self, builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
        let builder = self.geometry.apply(builder);
        if self.full_screen {
            builder.with_fullscreen(true)
        } else {
            builder
        }
    }

    /// Takes in the window's place as it is now. Whether it changed, to save it.
    pub(super) fn remember(&mut self, ctx: &egui::Context) -> bool {
        // A full-screen or minimized window keeps the normal place behind it.
        let moved = self.geometry.remember(ctx);
        let full_screen = ctx
            .input(|input| input.viewport().fullscreen)
            .unwrap_or(self.full_screen);
        let toggled = std::mem::replace(&mut self.full_screen, full_screen) != full_screen;
        moved || toggled
    }

    /// Whether the window was left in full screen.
    pub(super) fn full_screen(&self) -> bool {
        self.full_screen
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// The file's contents.
    pub(super) fn contents(&self) -> Vec<u8> {
        let Geometry {
            size,
            position,
            maximized,
        } = self.geometry;
        let saved = Saved {
            size,
            position,
            maximized,
            full_screen: self.full_screen,
        };
        serde_json::to_vec_pretty(&saved).expect("numbers and flags make JSON")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("exo-placement-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        dir
    }

    #[test]
    fn the_window_opens_where_it_was_left() {
        let dir = dir("saved");
        let saved = Saved {
            size: Some([1100.0, 700.0]),
            position: Some([300.0, 120.0]),
            maximized: true,
            full_screen: true,
        };
        std::fs::write(
            dir.join(FILE_NAME),
            serde_json::to_vec(&saved).expect("JSON"),
        )
        .expect("written");
        let placement = Placement::load(&dir);
        assert_eq!(
            serde_json::from_slice::<Saved>(&placement.contents()).expect("JSON"),
            saved,
            "saved as loaded"
        );

        let builder = placement.apply(
            egui::ViewportBuilder::default()
                .with_inner_size([1280.0, 800.0])
                .with_position([22.0, 22.0]),
        );
        assert_eq!(builder.inner_size, Some(egui::vec2(1100.0, 700.0)));
        assert_eq!(builder.position, Some(egui::pos2(300.0, 120.0)));
        assert_eq!(
            (builder.maximized, builder.fullscreen),
            (Some(true), Some(true))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn without_a_readable_file_the_window_opens_at_its_default_place() {
        let dir = dir("corrupt");
        let default = || {
            egui::ViewportBuilder::default()
                .with_inner_size([1280.0, 800.0])
                .with_position([22.0, 22.0])
        };
        for contents in [
            None,
            Some("{\"size\": [\"wide\"]"),
            Some("{\"size\": [1e9, 1e9], \"position\": [1e9, 0]}"),
        ] {
            if let Some(contents) = contents {
                std::fs::write(dir.join(FILE_NAME), contents).expect("written");
            }
            let builder = Placement::load(&dir).apply(default());
            assert_eq!(
                builder.inner_size,
                Some(egui::vec2(1280.0, 800.0)),
                "{contents:?}"
            );
            assert_eq!(
                builder.position,
                Some(egui::pos2(22.0, 22.0)),
                "{contents:?}"
            );
            assert_eq!(builder.fullscreen, None, "{contents:?}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
