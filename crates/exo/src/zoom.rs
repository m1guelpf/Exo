//! Zoom: ⌘= (or the keypad's +) zooms in, ⌘- out and ⌘0 back to actual size, with no other
//! modifier held, before anything else sees the keys; View's zoom items send the same keys.
//! Chromium's zoom levels, one a press from -3 to 3, each 1.2 times the last, so 57.9% to
//! 172.8%. The whole app zooms, every account, except the tab strip ([`crate::chrome`]). egui's
//! own zoom keys (0.1 a press, as far as 500%) are off.
//!
//! The zoom lasts across launches, in `zoom.json` in the state folder.

use std::path::{Path, PathBuf};

use egui::{Event, Key};

/// As far as zooming goes either way.
const MAX_LEVEL: i32 = 3;
/// Each level's factor over the one below (Chromium's `kTextSizeMultiplierRatio`).
const RATIO: f32 = 1.2;
/// The file in the state folder.
const FILE_NAME: &str = "zoom.json";

/// What the file holds.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    /// The zoom level, 0 for actual size.
    level: i32,
}

/// The app's zoom.
#[derive(Debug, Default)]
pub struct Zoom {
    level: i32,
    /// Where it's kept; none for a zoom that lasts the run (screenshots and tests).
    path: Option<PathBuf>,
}

impl Zoom {
    /// The zoom kept in `state`, the state folder: actual size, unless it was zoomed. A file that
    /// can't be read reads as actual size.
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
        let level = saved.map_or(0, |saved| saved.level.clamp(-MAX_LEVEL, MAX_LEVEL));
        Self {
            level,
            path: Some(path),
        }
    }

    /// Turns egui's own zoom keys off in `ctx`, for these.
    pub(crate) fn install(ctx: &egui::Context) {
        ctx.options_mut(|options| options.zoom_with_keyboard = false);
    }

    /// Takes this frame's zoom keys out of `ctx`'s input and zooms `ctx` to the level they leave.
    /// Call before anything else reads the frame's keys.
    pub(crate) fn frame(&mut self, ctx: &egui::Context) {
        let steps: Vec<i32> = ctx.input_mut(|input| {
            let mut steps = Vec::new();
            input.events.retain(|event| match zoom_key(event) {
                Some(step) => {
                    steps.push(step);
                    false
                }
                None => true,
            });
            steps
        });
        let level = steps
            .into_iter()
            .fold(self.level, |level, step| match step {
                0 => 0,
                step => (level + step).clamp(-MAX_LEVEL, MAX_LEVEL),
            });
        if level != self.level {
            self.level = level;
            self.save();
        }
        let factor = factor(self.level);
        if ctx.zoom_factor() != factor {
            ctx.set_zoom_factor(factor);
        }
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let contents =
            serde_json::to_vec_pretty(&Saved { level: self.level }).expect("a number makes JSON");
        if let Err(error) = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(path, contents))
        {
            log::warn!("writing {}: {error}", path.display());
        }
    }
}

/// The zoom factor of `level`: 1.2 to its power.
fn factor(level: i32) -> f32 {
    RATIO.powi(level)
}

/// What a zoom key does to the level: +1, -1, or 0 for actual size. ⌘ alone, on a press or its
/// repeats: `=` or `+` in, `-` out, `0` back. Its release is let through, as nothing reads it.
fn zoom_key(event: &Event) -> Option<i32> {
    let Event::Key {
        key,
        pressed: true,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    // On the Mac ⌘ is `command` and ⌃ `ctrl`; elsewhere `command` is ⌃ itself.
    if !modifiers.command
        || modifiers.shift
        || modifiers.alt
        || (cfg!(target_os = "macos") && modifiers.ctrl)
    {
        return None;
    }
    match key {
        Key::Equals | Key::Plus => Some(1),
        Key::Minus => Some(-1),
        Key::Num0 => Some(0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use egui::Modifiers;

    use super::*;

    const COMMAND: Modifiers = Modifiers {
        mac_cmd: cfg!(target_os = "macos"),
        command: true,
        ..Modifiers::NONE
    };

    fn press(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn chromiums_seven_levels() {
        let factors: Vec<String> = (-MAX_LEVEL..=MAX_LEVEL)
            .map(|level| format!("{:.1}%", factor(level) * 100.0))
            .collect();
        assert_eq!(
            factors,
            [
                "57.9%", "69.4%", "83.3%", "100.0%", "120.0%", "144.0%", "172.8%"
            ]
        );
    }

    #[test]
    fn only_command_alone_zooms() {
        assert_eq!(zoom_key(&press(Key::Equals, COMMAND)), Some(1));
        assert_eq!(
            zoom_key(&press(Key::Plus, COMMAND)),
            Some(1),
            "the keypad's +"
        );
        assert_eq!(zoom_key(&press(Key::Minus, COMMAND)), Some(-1));
        assert_eq!(zoom_key(&press(Key::Num0, COMMAND)), Some(0));
        for extra in [Modifiers::SHIFT, Modifiers::ALT, Modifiers::CTRL] {
            assert_eq!(
                zoom_key(&press(Key::Equals, COMMAND | extra)),
                None,
                "{extra:?}"
            );
        }
        assert_eq!(zoom_key(&press(Key::Equals, Modifiers::NONE)), None);
        let release = Event::Key {
            key: Key::Equals,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: COMMAND,
        };
        assert_eq!(zoom_key(&release), None);
    }

    #[test]
    fn zooming_stops_three_levels_out_and_comes_back_at_once() {
        let ctx = egui::Context::default();
        Zoom::install(&ctx);
        let mut zoom = Zoom::default();
        // The keys' frame, and the next, which the new zoom applies from.
        let mut frame = |keys: &[Key]| {
            for events in [
                keys.iter().map(|key| press(*key, COMMAND)).collect(),
                Vec::new(),
            ] {
                ctx.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| zoom.frame(ui.ctx()),
                )
                .drop_without_applying_deltas();
            }
            ctx.zoom_factor()
        };
        assert_eq!(frame(&[Key::Equals; 5]), factor(3));
        assert_eq!(frame(&[Key::Minus]), factor(2));
        assert_eq!(frame(&[Key::Num0]), 1.0);
        assert_eq!(frame(&[Key::Minus; 4]), factor(-3));
    }

    #[test]
    fn the_zoom_is_kept() {
        let dir = std::env::temp_dir().join(format!("exo-zoom-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Zoom::load(&dir).level, 0, "none kept yet");
        let mut zoom = Zoom::load(&dir);
        zoom.level = 2;
        zoom.save();
        assert_eq!(Zoom::load(&dir).level, 2);
        std::fs::write(dir.join(FILE_NAME), r#"{"level": 9}"#).expect("written");
        assert_eq!(Zoom::load(&dir).level, MAX_LEVEL, "as far as it goes");
        let _ = std::fs::remove_dir_all(dir);
    }
}
