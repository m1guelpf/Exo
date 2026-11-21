//! How the window opens: on screen where it was left, or minimized into the Dock when macOS
//! opened Exo as a login item (Open at Login).

use std::sync::OnceLock;

/// How the window opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Opening {
    /// On screen, where it was left.
    #[default]
    Shown,
    /// Minimized into the Dock without showing first, at its normal place; a full screen it was
    /// left in waits for it to come back.
    Minimized,
}

impl Opening {
    /// This launch's: minimized when macOS launched Exo as a login item. Ask first as the window is
    /// made (eframe's `window_builder`), while macOS is handing Exo the event it launched it with;
    /// that answer holds for the run.
    pub fn launch() -> Self {
        static LAUNCH: OnceLock<Opening> = OnceLock::new();
        *LAUNCH.get_or_init(|| {
            #[cfg(target_os = "macos")]
            if super::macos::launched_at_login() {
                return Self::Minimized;
            }
            Self::Shown
        })
    }

    /// `builder`, already at the window's place, for this opening: a window opening minimized
    /// doesn't open in full screen, which has a Space of its own and no way into the Dock, and
    /// which AppKit would otherwise keep trying to enter (winit retries a full screen it was built
    /// with until it takes).
    pub fn apply(self, builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
        match self {
            Self::Shown => builder,
            Self::Minimized => builder.with_fullscreen(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::Placement;

    #[test]
    fn a_window_opening_minimized_keeps_its_place_but_not_its_full_screen() {
        let dir = std::env::temp_dir().join(format!("exo-opening-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let saved = r#"{"size": [1100, 700], "position": [300, 120], "maximized": true, "full_screen": true}"#;
        std::fs::write(dir.join("window.json"), saved).expect("written");
        let placement = Placement::load(&dir);
        let builder = |opening: Opening| {
            opening.apply(
                placement.apply(
                    egui::ViewportBuilder::default()
                        .with_inner_size([1280.0, 800.0])
                        .with_position([22.0, 22.0]),
                ),
            )
        };

        let minimized = builder(Opening::Minimized);
        assert_eq!(
            (minimized.inner_size, minimized.position),
            (
                Some(egui::vec2(1100.0, 700.0)),
                Some(egui::pos2(300.0, 120.0))
            )
        );
        assert_eq!(
            (minimized.maximized, minimized.fullscreen),
            (Some(true), Some(false))
        );
        assert!(
            placement.full_screen(),
            "the full screen waits for the window"
        );
        assert_eq!(builder(Opening::Shown).fullscreen, Some(true));
        let _ = std::fs::remove_dir_all(dir);
    }
}
