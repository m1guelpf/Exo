//! View Raw Email Data (`|`, the palette's row): the conversation as Gmail sends it
//! ([`exo_store::Store::thread_source`]) in a read-only text area over the whole page, its text
//! selected once it's in, until Escape.
//!
//! The text area: `12px monospace` (Courier), black or white at 90% on `#fff` or `#18191a`, no
//! padding, soft-wrapped (`pre-wrap`, `break-word`), under the page's selection color.

use std::{
    collections::HashMap,
    sync::mpsc::{Receiver, Sender, channel},
};

use egui::{Area, Color32, Context, Order, Pos2, Rect, Sense, Vec2};
use exo_commands::{Command, Commands, Region};
use exo_ui::{Family, Font, TextStyle, palette};

use crate::{Mail, commands};

/// `font: 12px monospace`, whose `normal` line is 14px tall.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Monospace, 400), 12.0, 14.0);
/// Where `|` opens it, and where the palette lists it from.
const REGIONS: &[Region] = &[
    Region::ThreadList,
    Region::ThreadMessagesHeader,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
];

/// What the viewer's commands ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// `|`, the palette's `View Raw Email Data`.
    Open,
    /// Escape.
    Close,
    /// ⌘A.
    SelectAll,
    /// ⌘C.
    Copy,
    /// A key that moves through the text.
    Move(Move),
}

/// How a key moves through the text. Each leaves the text unselected, as moving the caret does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    /// ↓ / ↑: by a row.
    Lines(i32),
    /// Page Down / Page Up: by a screenful.
    Pages(i32),
    /// ⌘↑, Home.
    Top,
    /// ⌘↓, End.
    Bottom,
}

/// A conversation's source as it arrived: its id, then the text.
type Arrived = (String, String);

/// The viewer's state between frames.
pub(crate) struct RawSource {
    open: Option<Viewer>,
    arrived: (Sender<Arrived>, Receiver<Arrived>),
}

impl Default for RawSource {
    fn default() -> Self {
        Self {
            open: None,
            arrived: channel(),
        }
    }
}

/// The open viewer.
struct Viewer {
    thread: String,
    /// Empty until the source arrives.
    text: String,
    /// The text's rows when laid out `width` px wide.
    rows: Vec<Row>,
    width: f32,
    /// The page's height as last shown, a screenful for Page Down.
    height: f32,
    scroll: f32,
    /// Everything is selected (the text area's `select()`); anything else is nothing.
    selected: bool,
}

/// One row of the text: a byte range, and whether a line break ends it (which a selection
/// paints too).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    start: usize,
    end: usize,
    breaks: bool,
}

/// Registers `|` and the palette's row for conversation `thread`, and the viewer's keys while it's
/// open.
pub(crate) fn register(
    commands: &mut Commands<commands::Action>,
    thread: Option<&str>,
    open: bool,
) {
    let act = commands::Action::RawSource;
    if open {
        const VIEWER: &[Region] = &[Region::ThreadSourceViewer];
        let key =
            |id: &'static str, action: Action| Command::shortcut(id, act(action)).regions(VIEWER);
        commands.add(key("CLOSE_SOURCE", Action::Close).key("esc"));
        commands.add(key("SELECT_SOURCE", Action::SelectAll).key("mod+a"));
        commands.add(key("COPY_SOURCE", Action::Copy).key("mod+c"));
        commands.add(
            key("SOURCE_DOWN", Action::Move(Move::Lines(1)))
                .key("down")
                .repeat(act(Action::Move(Move::Lines(1)))),
        );
        commands.add(
            key("SOURCE_UP", Action::Move(Move::Lines(-1)))
                .key("up")
                .repeat(act(Action::Move(Move::Lines(-1)))),
        );
        commands.add(
            key("SOURCE_PAGE_DOWN", Action::Move(Move::Pages(1)))
                .key("pagedown")
                .repeat(act(Action::Move(Move::Pages(1)))),
        );
        commands.add(
            key("SOURCE_PAGE_UP", Action::Move(Move::Pages(-1)))
                .key("pageup")
                .repeat(act(Action::Move(Move::Pages(-1)))),
        );
        commands.add(key("SOURCE_TOP", Action::Move(Move::Top)).keys(["mod+up", "home"]));
        commands.add(key("SOURCE_BOTTOM", Action::Move(Move::Bottom)).keys(["mod+down", "end"]));
        return;
    }
    if thread.is_some() {
        commands.add(
            Command::new(
                "VIEW_RAW_EMAIL_DATA",
                "View Raw Email Data",
                act(Action::Open),
            )
            .key("|")
            .key_code(["|"])
            .default_score(0.0)
            .icon("data/description")
            .regions(REGIONS),
        );
    }
}

/// `text`'s rows `width` px wide, `pre-wrap` and `break-word`: a line breaks after the last space
/// that fits (spaces at a row's end hang past it), else anywhere.
fn wrap(text: &str, width: f32, advance: &mut impl FnMut(char) -> f32) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut line_start = 0;
    for line in text.split('\n') {
        let mut start = line_start;
        let mut x = 0.0;
        // The byte after the last run of spaces in this row, and the width up to it.
        let mut after_space: Option<(usize, f32)> = None;
        for (at, ch) in line.char_indices().map(|(at, ch)| (line_start + at, ch)) {
            let w = advance(ch);
            if ch == ' ' {
                x += w;
                after_space = Some((at + 1, x));
                continue;
            }
            if x + w > width && at > start {
                match after_space.filter(|(next, _)| *next > start) {
                    Some((next, before)) => {
                        rows.push(Row {
                            start,
                            end: next,
                            breaks: false,
                        });
                        start = next;
                        x -= before;
                    }
                    None => {
                        rows.push(Row {
                            start,
                            end: at,
                            breaks: false,
                        });
                        start = at;
                        x = 0.0;
                    }
                }
                after_space = None;
            }
            x += w;
        }
        let end = line_start + line.len();
        rows.push(Row {
            start,
            end,
            breaks: end < text.len(),
        });
        line_start = end + 1;
    }
    rows
}

impl Mail {
    /// Carries out a raw source viewer command.
    pub(crate) fn raw_source_run(&mut self, action: Action) {
        match action {
            Action::Open => self.raw_source_open(),
            Action::Close => self.raw_source.open = None,
            Action::SelectAll => {
                if let Some(viewer) = &mut self.raw_source.open {
                    viewer.selected = true;
                }
            }
            Action::Copy => {
                if let Some(viewer) = self
                    .raw_source
                    .open
                    .as_ref()
                    .filter(|viewer| viewer.selected && !viewer.text.is_empty())
                {
                    self.ctx.copy_text(viewer.text.clone());
                }
            }
            Action::Move(to) => {
                if let Some(viewer) = &mut self.raw_source.open {
                    viewer.selected = false;
                    viewer.scroll = match to {
                        Move::Lines(lines) => viewer.scroll + lines as f32 * TEXT.line_height,
                        Move::Pages(pages) => viewer.scroll + pages as f32 * viewer.height,
                        Move::Top => 0.0,
                        Move::Bottom => f32::INFINITY,
                    };
                }
            }
        }
    }

    /// Opens the viewer on the open thread, or the list's highlighted conversation, empty until
    /// Gmail answers.
    fn raw_source_open(&mut self) {
        let Some(thread) = self.raw_source_thread() else {
            return;
        };
        self.raw_source.open = Some(Viewer {
            thread: thread.clone(),
            text: String::new(),
            rows: Vec::new(),
            width: -1.0,
            height: 0.0,
            scroll: 0.0,
            selected: false,
        });
        let (sender, ctx) = (self.raw_source.arrived.0.clone(), self.ctx.clone());
        self.account.loader().run(
            move |store| async move { (store.thread_source(&thread).await, thread) },
            move |(source, thread)| match source {
                Ok(source) => {
                    let _ = sender.send((thread, source));
                    ctx.request_repaint();
                }
                Err(error) => log::info!("reading the source of {thread}: {error}"),
            },
        );
    }

    /// The conversation `|` is for: the open thread's, or the list's highlighted one.
    pub(crate) fn raw_source_thread(&self) -> Option<String> {
        match &self.thread {
            Some(thread) => Some(thread.id().to_owned()),
            None => self
                .inbox
                .cursor()
                .filter(|thread| !thread.draft_only)
                .map(|thread| thread.id.clone()),
        }
    }

    /// Whether the viewer is open.
    pub(crate) fn raw_source_open_now(&self) -> bool {
        self.raw_source.open.is_some()
    }

    /// The raw source viewer's text while it's open (empty until Gmail answers), and whether
    /// it's selected. For tests.
    pub fn raw_source(&self) -> Option<(&str, bool)> {
        self.raw_source
            .open
            .as_ref()
            .map(|viewer| (viewer.text.as_str(), viewer.selected))
    }

    /// Draws the viewer over `page`, if it's open, and handles its pointer.
    pub(crate) fn show_raw_source(&mut self, ctx: &Context, page: Rect) {
        while let Ok((thread, source)) = self.raw_source.arrived.1.try_recv() {
            if let Some(viewer) = self
                .raw_source
                .open
                .as_mut()
                .filter(|viewer| viewer.thread == thread)
            {
                viewer.text = source;
                viewer.width = -1.0;
                // Once the text is in, the text area selects it.
                viewer.selected = true;
            }
        }
        let Some(viewer) = self.raw_source.open.as_mut() else {
            return;
        };
        let tokens = palette(ctx);
        let (background, ink) = match tokens.theme {
            exo_ui::Theme::Snow => (Color32::WHITE, Color32::from_black_alpha(230)),
            exo_ui::Theme::Carbon => (
                Color32::from_rgb(0x18, 0x19, 0x1a),
                Color32::from_white_alpha(230),
            ),
        };
        if viewer.width != page.width() {
            let font = TEXT.format(Color32::WHITE).font_id;
            let mut widths: HashMap<char, f32> = HashMap::new();
            let mut advance = |ch: char| {
                *widths
                    .entry(ch)
                    .or_insert_with(|| ctx.fonts_mut(|fonts| fonts.glyph_width(&font, ch)))
            };
            viewer.rows = wrap(&viewer.text, page.width(), &mut advance);
            viewer.width = page.width();
        }
        viewer.height = page.height();
        Area::new(exo_ui::id(ctx, "exo::raw_source"))
            .order(Order::Middle)
            .fixed_pos(page.min)
            .show(ctx, |ui| {
                let response = ui.allocate_rect(page, Sense::click_and_drag());
                if response.hovered() {
                    viewer.scroll -= ui.input(|input| input.smooth_scroll_delta.y);
                }
                if response.clicked() || response.drag_started() {
                    viewer.selected = false;
                }
                let content = viewer.rows.len() as f32 * TEXT.line_height;
                viewer.scroll = viewer.scroll.clamp(0.0, (content - page.height()).max(0.0));
                let painter = ui.painter_at(page);
                painter.rect_filled(page, 0.0, background);
                let first = (viewer.scroll / TEXT.line_height).floor().max(0.0) as usize;
                let shown = (page.height() / TEXT.line_height).ceil() as usize + 1;
                for (at, row) in viewer.rows.iter().enumerate().skip(first).take(shown) {
                    let top = page.top() + at as f32 * TEXT.line_height - viewer.scroll;
                    let text = &viewer.text[row.start..row.end];
                    if viewer.selected {
                        // The line break is selected too, a space wide; the box lands on whole px.
                        let space = TEXT.width(ctx, " ");
                        let width = TEXT.width(ctx, text) + if row.breaks { space } else { 0.0 };
                        painter.rect_filled(
                            Rect::from_min_size(
                                Pos2::new(page.left(), top),
                                Vec2::new(width.round(), TEXT.line_height),
                            ),
                            0.0,
                            tokens.text_selection_default,
                        );
                    }
                    TEXT.paint_line(&painter, Pos2::new(page.left(), top), text, ink, None);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str, columns: usize) -> Vec<&str> {
        wrap(text, columns as f32 * 7.2 + 0.1, &mut |_| 7.2)
            .iter()
            .map(|row| &text[row.start..row.end])
            .collect()
    }

    #[test]
    fn lines_break_at_their_newlines() {
        assert_eq!(
            rows("{\n    \"id\": \"t1\"\n}", 40),
            ["{", "    \"id\": \"t1\"", "}"]
        );
        let breaks: Vec<bool> = wrap("a\nb", 100.0, &mut |_| 7.2)
            .iter()
            .map(|row| row.breaks)
            .collect();
        assert_eq!(
            breaks,
            [true, false],
            "the last line has no break to select"
        );
    }

    #[test]
    fn a_long_line_wraps_after_its_last_space_that_fits_and_spaces_hang() {
        assert_eq!(
            rows("\"value\": \"from mail by mx\"", 12),
            ["\"value\": ", "\"from mail ", "by mx\""]
        );
        assert_eq!(
            rows("ab   cd", 3),
            ["ab   ", "cd"],
            "spaces past the edge stay on their row"
        );
    }

    #[test]
    fn a_word_too_long_for_a_row_breaks_anywhere() {
        assert_eq!(
            rows("    \"data\": \"PGh0bWw+PGJvZHk+SGk8L2JvZHk+\"", 16),
            ["    \"data\": ", "\"PGh0bWw+PGJvZHk", "+SGk8L2JvZHk+\""]
        );
    }
}
