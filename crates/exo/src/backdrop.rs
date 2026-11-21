//! The day's photo behind Inbox Zero and the full-screen alerts: open landscape photos
//! (`assets/inbox-zero`), one a day, each with its own scrims and placement (`photos.json`).

use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};

use egui::{Color32, Mesh, Painter, Rect, Shape, Vec2, pos2, vec2};
use jiff::Zoned;
use serde::Deserialize;

use exo_commands::{Command, Commands, Region};

use crate::photo::Photo;

/// What the palette does with the photo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `Download Inbox Zero Image`: the photo to the download folder.
    Download,
    /// `Inbox Zero: Disable/Enable Animated Images`: the Reduce Motion setting.
    ToggleAnimatedImages,
}

/// Registers `Download Inbox Zero Image` at Inbox Zero (`zero`), in the list and its header,
/// and `Inbox Zero: Disable Animated Images` (`Enable` once `reduced`) everywhere.
pub(crate) fn register(
    commands: &mut Commands<crate::commands::Action>,
    zero: bool,
    reduced: bool,
) {
    let act = crate::commands::Action::Backdrop;
    if zero {
        const LIST: &[Region] = &[Region::ThreadList, Region::ThreadListHeader];
        commands.add(
            Command::new(
                "DOWNLOAD_INBOX_ZERO",
                "Download Inbox Zero Image",
                act(Action::Download),
            )
            .regions(LIST)
            .icon("data/download"),
        );
    }
    let title = if reduced {
        "Inbox Zero: Enable Animated Images"
    } else {
        "Inbox Zero: Disable Animated Images"
    };
    commands.add(
        Command::new(
            "TOGGLE_REDUCED_MOTION",
            title,
            act(Action::ToggleAnimatedImages),
        )
        .aliases([
            "Reduce Motion",
            "Allow Motion",
            "Disable Motion",
            "Enable Motion",
        ])
        .default_score(0.0)
        .icon("data/settings"),
    );
}

/// One photo of the rotation, as `photos.json` lists it.
#[derive(Debug, Deserialize)]
struct Entry {
    file: String,
    /// The top scrim's opacity in percent, instead of the default scrims.
    top_scrim: Option<f32>,
    /// The bottom scrim's, likewise.
    bottom_scrim: Option<f32>,
    /// Where the photo sits, as CSS's `object-position` takes it.
    position: Option<String>,
}

static PHOTOS: LazyLock<Vec<Entry>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../assets/inbox-zero/photos.json"))
        .expect("photos.json is well-formed")
});

/// The day's photo: it changes once a day, at 2 AM local time, and is the same for everyone in a
/// time zone.
fn index(now: &Zoned) -> usize {
    let minutes =
        now.timestamp().as_second().div_euclid(60) + i64::from(now.offset().seconds()) / 60;
    let day = (minutes - 120).div_euclid(60 * 24) + 160;
    day.rem_euclid(PHOTOS.len() as i64) as usize
}

/// Where the photos are: `Exo.app`'s `Contents/Resources/inbox-zero` (`fastlane build`), else
/// the checkout's `assets/inbox-zero`.
fn photos_dir() -> PathBuf {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("Resources/inbox-zero")));
    bundled.filter(|dir| dir.is_dir()).unwrap_or_else(|| {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/inbox-zero"
        ))
    })
}

/// The backdrop: the day's photo, decoding or decoded.
pub struct Backdrop {
    entry: &'static Entry,
    photo: Photo,
    /// The day it's the photo for ([`index`]).
    day: usize,
}

impl Backdrop {
    /// Starts decoding the photo for `now`.
    pub fn new(ctx: &egui::Context, now: &Zoned) -> Self {
        let day = index(now);
        Self::of(ctx, &PHOTOS[day], day)
    }

    /// The photo named `file` until the day turns, for a screenshot of a given photo.
    pub fn named(ctx: &egui::Context, file: &str, now: &Zoned) -> Option<Self> {
        PHOTOS
            .iter()
            .find(|entry| entry.file == file)
            .map(|entry| Self::of(ctx, entry, index(now)))
    }

    fn of(ctx: &egui::Context, entry: &'static Entry, day: usize) -> Self {
        Self {
            entry,
            photo: Photo::load_blurred(ctx, photos_dir().join(&entry.file)),
            day,
        }
    }

    /// Moves on to `now`'s photo once the day has turned (at 2 AM).
    pub fn refresh(&mut self, ctx: &egui::Context, now: &Zoned) {
        if index(now) != self.day {
            *self = Self::new(ctx, now);
        }
    }

    /// Blocks until the photo is decoded, for a screen that shows only once it can draw it.
    pub fn wait(&mut self) {
        self.photo.wait();
    }

    /// The photo is decoded. For screenshots and tests.
    pub fn is_loaded(&self) -> bool {
        self.photo.is_loaded()
    }

    /// The photo's file, for `Download Inbox Zero Image`.
    pub fn path(&self) -> PathBuf {
        photos_dir().join(&self.entry.file)
    }

    /// The photo's file name.
    pub fn file(&self) -> &Path {
        Path::new(&self.entry.file)
    }

    /// Paints the photo over `window`, covering it (`object-fit: cover`) where its position puts
    /// it.
    pub fn paint_photo(&self, painter: &Painter, window: Rect) {
        let at = position(self.entry.position.as_deref());
        self.photo.paint(painter, window, |free| free * at);
    }

    /// Paints Inbox Zero's backdrop over `window`: the photo and its scrims, the bottom one
    /// reaching further while the settings menu is open; with the menu open, the photo blurred
    /// under black at .5 (`background: rgba(0,0,0,.5); backdrop-filter: blur(8px)`, at once).
    pub fn paint(&self, painter: &Painter, window: Rect, settings_open: bool) {
        if settings_open {
            let at = position(self.entry.position.as_deref());
            self.photo.paint_blurred(painter, window, |free| free * at);
        } else {
            self.paint_photo(painter, window);
        }
        let black = |opacity: f32| Color32::from_black_alpha((opacity * 255.0).round() as u8);
        match (self.entry.top_scrim, self.entry.bottom_scrim) {
            // `linear-gradient(to bottom, rgba(0,0,0,.5) 0, rgba(0,0,0,.05) 176px, transparent
            // 216px), linear-gradient(to top, rgba(0,0,0,.6) 0, rgba(0,0,0,.06) 176px,
            // transparent 216px)`.
            (None, None) => {
                scrim(
                    painter,
                    window,
                    Edge::Top,
                    &[
                        (0.0, black(0.5)),
                        (176.0, black(0.05)),
                        (216.0, Color32::TRANSPARENT),
                    ],
                );
                scrim(
                    painter,
                    window,
                    Edge::Bottom,
                    &[
                        (0.0, black(0.6)),
                        (176.0, black(0.06)),
                        (216.0, Color32::TRANSPARENT),
                    ],
                );
            }
            // A photo's own scrims, 50% at the top and 60% at the bottom unless it says:
            // `rgba(0,0,0,{o}%) 0px, rgba(0,0,0,{o/10}%) 264px, transparent 324px`, the bottom one
            // to 636px with the settings open.
            (top, bottom) => {
                for (edge, opacity, extent) in [
                    (Edge::Top, top.unwrap_or(50.0), 264.0),
                    (
                        Edge::Bottom,
                        bottom.unwrap_or(60.0),
                        if settings_open { 636.0 } else { 264.0 },
                    ),
                ] {
                    let opacity = opacity / 100.0;
                    scrim(
                        painter,
                        window,
                        edge,
                        &[
                            (0.0, black(opacity)),
                            (extent, black(opacity / 10.0)),
                            (extent + 60.0, Color32::TRANSPARENT),
                        ],
                    );
                }
            }
        }
        if settings_open {
            painter.rect_filled(window, 0.0, black(0.5));
        }
    }
}

impl crate::Mail {
    /// Runs a palette command about the photo.
    pub(crate) fn backdrop_run(&mut self, action: Action) {
        match action {
            // Saved like an attachment, with its notification.
            Action::Download => match std::fs::read(self.backdrop.path()) {
                Ok(bytes) => self
                    .attachment_files
                    .save_file(self.backdrop.file().display().to_string(), bytes),
                Err(error) => log::warn!("reading the Inbox Zero photo: {error}"),
            },
            // Exo's photos are all stills, so the setting has nothing to stop yet; its hint says
            // what it did.
            Action::ToggleAnimatedImages => {
                let reduce = !self.account.settings().reduce_motion;
                self.account
                    .update_settings(|settings| settings.reduce_motion = reduce);
                self.commands
                    .hint(exo_commands::Hint::new().text(if reduce {
                        "Animated Images Disabled"
                    } else {
                        "Animated Images Enabled"
                    }));
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Edge {
    Top,
    Bottom,
}

/// A `linear-gradient` from `edge` into `window` through `stops` (distance from the edge, color),
/// transparent past the last. A mesh interpolates premultiplied colors linearly, as CSS does.
fn scrim(painter: &Painter, window: Rect, edge: Edge, stops: &[(f32, Color32)]) {
    let y = |distance: f32| match edge {
        Edge::Top => window.top() + distance,
        Edge::Bottom => window.bottom() - distance,
    };
    let mut mesh = Mesh::default();
    for (i, &(distance, color)) in stops.iter().enumerate() {
        let (left, right) = (
            pos2(window.left(), y(distance)),
            pos2(window.right(), y(distance)),
        );
        for point in [left, right] {
            mesh.colored_vertex(point, color);
        }
        if i > 0 {
            let base = (2 * i - 2) as u32;
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 2, base + 3);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// Where `object-position` puts a covering image, as fractions of how much larger than the
/// window it is: `left X% top Y%` or `left X% bottom Y%`. Anything else CSS would throw out as
/// invalid (`left: 50% top 44%`, with its stray colon), leaving the default `50% 50%`.
fn position(css: Option<&str>) -> Vec2 {
    let center = vec2(0.5, 0.5);
    let Some(css) = css else { return center };
    let percent = |token: &str| {
        token
            .strip_suffix('%')?
            .parse::<f32>()
            .ok()
            .map(|value| value / 100.0)
    };
    match css.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["left", x, "top", y] => percent(x)
            .zip(percent(y))
            .map_or(center, |(x, y)| vec2(x, y)),
        ["left", x, "bottom", y] => percent(x)
            .zip(percent(y))
            .map_or(center, |(x, y)| vec2(x, 1.0 - y)),
        _ => center,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_photo_is_in_the_assets_with_its_placement_understood() {
        assert!((30..=50).contains(&PHOTOS.len()), "{} photos", PHOTOS.len());
        for entry in PHOTOS.iter() {
            assert!(
                photos_dir().join(&entry.file).is_file(),
                "{} is missing",
                entry.file
            );
            if let Some(css) = &entry.position {
                let understood = css.starts_with("left ") && css.split_whitespace().count() == 4;
                assert!(understood || css.contains(':'), "{css}");
            }
        }
    }

    #[test]
    fn the_photo_changes_at_two_in_the_morning_local_time() {
        let at = |time: &str| index(&time.parse::<Zoned>().expect("a time"));
        let today = at("2026-11-17T09:00:00-05:00[America/New_York]");
        assert_eq!(at("2026-11-17T02:00:00-05:00[America/New_York]"), today);
        assert_eq!(at("2026-11-18T01:59:00-05:00[America/New_York]"), today);
        assert_eq!(
            at("2026-11-18T02:00:00-05:00[America/New_York]"),
            (today + 1) % PHOTOS.len()
        );
        // The same local time elsewhere is the same photo.
        assert_eq!(at("2026-11-17T09:00:00+09:00[Asia/Tokyo]"), today);
    }

    #[test]
    fn object_position_reads_as_css_does() {
        assert_eq!(position(None), vec2(0.5, 0.5));
        assert_eq!(position(Some("left 50% top 40%")), vec2(0.5, 0.4));
        assert_eq!(
            position(Some("left 52% bottom 41%")),
            vec2(0.52, 1.0 - 0.41)
        );
        assert_eq!(position(Some("left: 50% top 44%")), vec2(0.5, 0.5));
    }
}
