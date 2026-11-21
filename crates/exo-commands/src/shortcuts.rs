//! The Shortcuts sheet: a searchable table of every shortcut on the right edge of the window, over
//! a dimmed backdrop. The table is written out by hand, in Gmail's wording.

use egui::{
    Area, Color32, Context, CornerRadius, Order, Pos2, Rect, Sense, Ui, Vec2,
    text::{CCursor, CCursorRange},
};
use exo_ui::{
    BoxShadow, Family, Font, KeycapColors, Keycaps, TextStyle, focus_field, motion, palette,
    pixel_snap, rem, text_field,
};

use crate::score::command_score;

/// A section of the table: its title and `(name, keys)` rows, keys space-separated keycap tokens.
struct Section {
    title: &'static str,
    rows: &'static [(&'static str, &'static str)],
}

/// The table as a Gmail account without AI or a team sees it, so without the rows for AI features:
/// `Ask AI`, `Summarize`, the AI assistant and its `Create Empty Event`. `Send later` and compose's
/// `Remind me` are listed: Exo schedules sends and keeps reminders itself.
const SECTIONS: &[Section] = &[
    Section {
        title: "Actions",
        rows: &[("Exo Command", "⌘ K"), ("Search", "/"), ("Undo", "Z")],
    },
    Section {
        title: "Navigation",
        rows: &[
            ("Next / Previous Conversation", "J K"),
            ("Next / Previous Message", "N P"),
            ("Open", "enter"),
            ("Back", "esc"),
            ("Next Split Inbox", "tab"),
            ("Previous Split Inbox", "shift tab"),
            ("Open Label Menu", "←"),
            ("Page Down", "space"),
            ("Page Up", "shift space"),
            ("Jump to Top", "⌘ ↑"),
            ("Jump to Bottom", "⌘ ↓"),
            ("Switch Accounts", "control 1-9"),
            ("Exo Focus", "→ ← ↓ ↑"),
        ],
    },
    Section {
        title: "Conversations",
        rows: &[
            ("Mark Done (Archive)", "E"),
            ("Mark not Done", "shift E"),
            ("Remind Me (Snooze)", "H"),
            ("Star", "S"),
            ("Mark Read or Unread", "U"),
            ("Mute", "shift M"),
            ("Trash", "#"),
            ("Mark as Spam", "!"),
            ("Unsubscribe", "⌘ U"),
            ("Print", "⌘ P"),
            ("Select Conversation", "X"),
            ("Clear Selection", "esc"),
            ("Select All From Here", "mod A"),
            ("Select All", "mod shift A"),
        ],
    },
    Section {
        title: "Labels",
        rows: &[
            ("Move", "V"),
            ("Add or Remove Label", "L"),
            ("Remove Label", "Y"),
            ("Remove Label, Next", "["),
            ("Remove Label, Previous", "]"),
            ("Remove All Labels", "shift Y"),
        ],
    },
    Section {
        title: "Messages",
        rows: &[
            ("Compose", "C"),
            ("Reply All", "enter"),
            ("Reply", "R"),
            ("Forward", "F"),
            ("Open Links & Attachments", "⌘ O"),
            ("Cycle Through Links", "tab"),
            ("Expand Message", "O"),
            ("Expand/Collapse Header", "shift H"),
            ("Expand All Messages", "shift O"),
            ("Show New Messages", "shift N"),
            ("Use Snippet", "⌘ ;"),
        ],
    },
    Section {
        title: "Compose",
        rows: &[
            ("To", "⌘ shift O"),
            ("Cc", "⌘ shift C"),
            ("Bcc", "⌘ shift B"),
            ("From", "⌘ shift F"),
            ("Edit Subject", "⌘ shift S"),
            ("Attach", "⌘ shift U"),
            ("Discard Draft", "⌘ shift ,"),
            ("Instant Intro (to BCC)", "⌘ shift I"),
            ("Remind me", "⌘ shift H"),
            ("Send later", "⌘ shift L"),
            ("Use Snippet Inline", ";"),
            ("Insert Emoji", ":"),
            ("Send", "⌘ enter"),
            ("Send Instantly", "⌘ shift Z"),
            ("Send + Mark Done", "⌘ shift enter"),
        ],
    },
    Section {
        title: "Pop Out Compose",
        rows: &[
            ("Pop Out Compose", "shift C"),
            ("Reply All, Pop Out Draft", "shift enter"),
            ("Reply, Pop Out Draft", "shift R"),
            ("Forward, Pop Out Draft", "shift F"),
            ("Pop Out Draft", "⌘ shift P"),
            ("Pop In Draft", "⌘ shift P"),
            ("Pop Out Draft & Search", "⌘ /"),
            ("Toggle Focus", "⌘ D"),
        ],
    },
    Section {
        title: "Format",
        rows: &[
            ("Bold", "⌘ B"),
            ("Italics", "⌘ I"),
            ("Underline", "⌘ U"),
            ("Hyperlink", "⌘ K"),
            ("Color", "⌘ O"),
            ("Strikethrough", "⌘ shift X"),
            ("Numbers", "⌘ shift 7"),
            ("Bullets", "⌘ shift 8"),
            ("Quote", "⌘ shift 9"),
            ("Indent List", "tab"),
            ("Outdent List", "shift tab"),
            ("Increase Indent", "⌘ ]"),
            ("Decrease Indent", "⌘ ["),
        ],
    },
    Section {
        title: "Folders",
        rows: &[
            ("Go to Inbox", "G then I"),
            ("Go to Important", "G then I"),
            ("Go to Other", "G then O"),
            ("Go to Starred", "G then S"),
            ("Go to Drafts", "G then D"),
            ("Go to Sent", "G then T"),
            ("Go to Done", "G then E"),
            ("Go to Reminders", "G then H"),
            ("Go to Muted", "G then M"),
            ("Go to Snippets", "G then ;"),
            ("Go to Spam", "G then !"),
            ("Go to Trash", "G then #"),
            ("Go to All Mail", "G then A"),
            ("Go to Label …", "G then L"),
        ],
    },
    Section {
        title: "Windows",
        rows: &[
            ("New Tab", "⌘ T"),
            ("Next Tab", "⌘ shift ]"),
            ("Previous Tab", "⌘ shift ["),
            ("Close Tab", "⌘ W"),
            ("Increase Font Size", "⌘ ="),
            ("Decrease Font Size", "⌘ -"),
            ("Reset Font Size", "⌘ 0"),
            ("Find Within Page", "⌘ F"),
            ("Copy Private Link", "control /"),
        ],
    },
    Section {
        title: "Calendar",
        rows: &[
            ("Open Day", "0"),
            ("Open Week", "2"),
            ("Previous Day/Week", "–"),
            ("Next Day/Week", "="),
            ("Share Availability", "⌘ shift A"),
            ("Create Event", "B"),
        ],
    },
    Section {
        title: "Filters",
        rows: &[
            ("Unread", "shift U"),
            ("Starred", "shift S"),
            ("Important", "shift I"),
            ("No reply", "shift R"),
        ],
    },
];

const INPUT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
const TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);
const NAME: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const ROW_HEIGHT: f32 = 24.0;
/// The column starts 80px below the top, then 4px of padding.
const COLUMN_TOP: f32 = 80.0;
/// The sheet fades in and out over 200ms, `ease`.
const FADE: f32 = 0.2;

/// The sheet's state between frames.
#[derive(Default)]
pub(crate) struct Shortcuts {
    open: bool,
    query: String,
    scroll: f32,
    /// Focus the search field and select its text next frame.
    focus: bool,
    /// The account's `Send + Mark Done`: `Send` goes, and `Send + Mark Done` is `⌘ enter`.
    send_and_done: bool,
}

impl Shortcuts {
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the sheet as new: it always opens unsearched and unscrolled.
    pub(crate) fn open(&mut self) {
        *self = Self {
            open: true,
            focus: true,
            send_and_done: self.send_and_done,
            ..Self::default()
        };
    }

    pub(crate) fn set_send_and_done(&mut self, on: bool) {
        self.send_and_done = on;
    }

    /// `section`'s rows as shown: with Send + Mark Done on, `Send` is hidden and `Send + Mark
    /// Done` is on `⌘ enter`.
    fn rows(&self, section: &Section) -> Vec<(&'static str, &'static str)> {
        section
            .rows
            .iter()
            .filter(|(name, _)| !(self.send_and_done && *name == "Send"))
            .map(|&(name, keys)| {
                if self.send_and_done && name == "Send + Mark Done" {
                    (name, "⌘ enter")
                } else {
                    (name, keys)
                }
            })
            .collect()
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
    }

    /// Draws the sheet (while open or fading out) over `area`; a click on the backdrop or ×
    /// closes it.
    pub(crate) fn show(&mut self, ctx: &Context, area: Rect) {
        let id = exo_ui::id(ctx, "exo_commands::shortcuts");
        let opacity = motion::transition(
            ctx,
            id.with("fade"),
            self.open,
            FADE,
            motion::CubicBezier::EASE,
        );
        if opacity <= 0.0 {
            return;
        }
        let dismissed = Area::new(id)
            .order(Order::Foreground)
            .fixed_pos(area.min)
            .show(ctx, |ui| {
                ui.set_opacity(opacity);
                self.draw(ui, area)
            })
            .inner;
        if dismissed {
            self.close();
        }
    }

    /// Paints the sheet; returns whether a click dismissed it.
    fn draw(&mut self, ui: &mut Ui, area: Rect) -> bool {
        let ctx = ui.ctx().clone();
        let tokens = palette(&ctx);
        let rem = rem(area.width(), false);
        let width = exo_ui::right_pane_width(area.width());
        let panel = Rect::from_min_max(Pos2::new(area.right() - width, area.top()), area.max);
        // Boxes paint on whole pixels; text stays where layout put it.
        let snapped = pixel_snap(panel);

        let backdrop = ui.allocate_rect(area, Sense::click());
        let panel_response = ui.allocate_rect(panel, Sense::click());
        let painter = ui.painter().clone();
        painter.rect_filled(area, 0.0, Color32::from_black_alpha(128));
        BoxShadow::new([0.0, 0.0], 30.0, 0.0, Color32::from_black_alpha(38)).paint(
            &painter,
            snapped,
            CornerRadius::ZERO,
        );
        painter.rect_filled(snapped, 0.0, tokens.background_base_default);

        // Search field: a 28px row with the brand bar on the panel's left edge.
        let header_top = panel.top() + 28.0;
        let bar = Rect::from_min_size(Pos2::new(panel.left(), header_top), Vec2::new(3.0, 28.0));
        painter.rect_filled(pixel_snap(bar), 0.0, tokens.border_base_active);
        let input_left = panel.left() + 3.0 + 2.0 * rem;
        self.input(
            ui,
            Pos2::new(input_left, header_top + 4.0),
            panel.right() - 2.0 * rem - input_left,
        );
        if panel_response.clicked() {
            self.focus = true;
        }

        // ×: 12px, 36px from the top, 2rem from the right, with a 12px hit margin.
        let close_rect = Rect::from_min_size(
            Pos2::new(panel.right() - 2.0 * rem - 12.0, panel.top() + 36.0),
            Vec2::splat(12.0),
        );
        let close = ui.interact(
            close_rect.expand(12.0),
            exo_ui::id(ui.ctx(), "exo_commands::shortcuts_close"),
            Sense::click(),
        );
        let close_color = motion::mix(
            tokens.icon_base_default,
            tokens.icon_base_hover,
            motion::transition(
                &ctx,
                close.id,
                close.hovered(),
                0.15,
                motion::CubicBezier::EASE,
            ),
        );
        exo_ui::paint_icon(ui, "legacy/close", close_rect, close_color);

        // The column.
        let column =
            Rect::from_min_max(Pos2::new(panel.left(), panel.top() + COLUMN_TOP), panel.max);
        let sections = self.visible_sections();
        let content_height: f32 = 4.0
            + sections
                .iter()
                .map(|(section, _)| {
                    20.0 + 8.0 + ROW_HEIGHT * self.rows(section).len() as f32 + 20.0
                })
                .sum::<f32>();
        let wheel = if ui.rect_contains_pointer(panel) {
            ui.input(|input| input.smooth_scroll_delta.y)
        } else {
            0.0
        };
        // Searching doesn't scroll, but the column can't stay scrolled past its end.
        self.scroll = (self.scroll - wheel).clamp(0.0, (content_height - column.height()).max(0.0));
        let mut column_ui = ui.new_child(egui::UiBuilder::new());
        column_ui.set_clip_rect(column);
        let column_painter = column_ui.painter().clone();
        let mut y = column.top() + 4.0 - self.scroll;
        let name_left = panel.left() + 2.0 * rem;
        let keys_left = name_left + (158.0_f32).max((width - 2.0 * rem) / 2.0);
        let (keycaps, highlighted_keycaps) = (
            KeycapColors::shortcuts_pane(tokens),
            KeycapColors::shortcuts_pane_highlighted(tokens),
        );
        for (section, matches) in &sections {
            TITLE.paint_line(
                &column_painter,
                Pos2::new(name_left, y + 3.0),
                section.title,
                tokens.text_base_default,
                None,
            );
            y += 20.0 + 8.0;
            for (&(name, keys), &matched) in self.rows(section).iter().zip(matches) {
                let row =
                    Rect::from_min_size(Pos2::new(panel.left(), y), Vec2::new(width, ROW_HEIGHT));
                let highlight = motion::transition(
                    &ctx,
                    exo_ui::id(&ctx, ("exo_commands::shortcut_row", section.title, name)),
                    matched,
                    0.3,
                    motion::CubicBezier::EASE,
                );
                if highlight > 0.0 {
                    column_painter.rect_filled(
                        pixel_snap(row),
                        0.0,
                        tokens.fill_active_default.gamma_multiply(highlight),
                    );
                }
                NAME.paint_line(
                    &column_painter,
                    Pos2::new(name_left, y + 3.0),
                    name,
                    tokens.text_base_default,
                    None,
                );
                let colors = KeycapColors {
                    background: motion::mix(
                        keycaps.background,
                        highlighted_keycaps.background,
                        highlight,
                    ),
                    ..keycaps
                };
                let tokens: Vec<&str> = keys.split(' ').collect();
                Keycaps::new(&ctx, &tokens, colors).paint(&column_painter, keys_left, y + 3.0);
                y += ROW_HEIGHT;
            }
            y += 20.0;
        }

        // The header's shadow once the column has scrolled.
        exo_ui::scroll_shadow(
            &painter,
            exo_ui::id(painter.ctx(), "exo_commands::shortcuts_shadow"),
            column.min,
            width,
            self.scroll > 0.0,
        );

        close.clicked() || backdrop.clicked()
    }

    /// The search field: what's typed is lower-cased as it's typed.
    fn input(&mut self, ui: &mut Ui, line_top_left: Pos2, width: f32) {
        let ctx = ui.ctx().clone();
        let tokens = palette(&ctx);
        let id = exo_ui::id(&ctx, "exo_commands::shortcuts_input");
        if self.query.is_empty() {
            INPUT.paint_line(
                ui.painter(),
                line_top_left,
                "Search",
                tokens.text_base_subdued,
                None,
            );
        }
        let line = Rect::from_min_size(line_top_left, Vec2::new(width, INPUT.line_height));
        // The search field's selection colour.
        text_field(
            ui,
            id,
            &mut self.query,
            line,
            &INPUT,
            tokens.text_base_default,
            Color32::from_rgb(0xb4, 0xd5, 0xfe),
        );
        self.query = self.query.to_lowercase();
        if std::mem::take(&mut self.focus) {
            focus_field(
                &ctx,
                id,
                CCursorRange::two(CCursor::new(0), CCursor::new(self.query.chars().count())),
            );
        }
    }

    /// The sections to show and which of their rows match: all of them for an empty query, else
    /// those with a matching row or a matching title, listed whole.
    fn visible_sections(&self) -> Vec<(&'static Section, Vec<bool>)> {
        SECTIONS
            .iter()
            .filter_map(|section| {
                let matches: Vec<bool> = self
                    .rows(section)
                    .iter()
                    .map(|(name, _)| {
                        !self.query.is_empty() && command_score(name, &self.query) > 0.0
                    })
                    .collect();
                let shown = self.query.is_empty()
                    || matches.contains(&true)
                    || command_score(section.title, &self.query) > 0.0;
                shown.then_some((section, matches))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;

    use super::*;

    #[test]
    fn send_and_mark_done_takes_command_enter_when_on() {
        let compose = SECTIONS
            .iter()
            .find(|section| section.title == "Compose")
            .expect("Compose");
        let mut sheet = Shortcuts::default();
        sheet.set_send_and_done(true);
        sheet.open();
        let rows = sheet.rows(compose);
        assert!(
            !rows.iter().any(|(name, _)| *name == "Send"),
            "Send is hidden"
        );
        assert!(rows.contains(&("Send + Mark Done", "⌘ enter")));
        assert!(
            Shortcuts::default()
                .rows(compose)
                .contains(&("Send + Mark Done", "⌘ shift enter"))
        );
    }

    #[test]
    fn search_shows_whole_sections_with_matches() {
        let sheet = Shortcuts {
            query: "mark".to_owned(),
            ..Shortcuts::default()
        };
        let sections = sheet.visible_sections();
        let titles: Vec<_> = sections.iter().map(|(section, _)| section.title).collect();
        // "mark" finds Conversations and Compose (Labels' "Remove All Labels" doesn't contain an m
        // before a, r, k in order).
        assert_eq!(titles, ["Conversations", "Compose"]);
        let (conversations, matches) = &sections[0];
        let highlighted: Vec<_> = conversations
            .rows
            .iter()
            .zip(matches)
            .filter(|(_, matched)| **matched)
            .map(|(row, _)| row.0)
            .collect();
        assert_eq!(
            highlighted,
            [
                "Mark Done (Archive)",
                "Mark not Done",
                "Mark Read or Unread",
                "Mark as Spam"
            ]
        );
    }

    #[test]
    fn the_column_never_stays_scrolled_past_its_end() {
        let area = Rect::from_min_max(Pos2::new(0.0, 41.0), Pos2::new(1280.0, 800.0));
        let mut harness = Harness::builder()
            .with_size([1280.0, 800.0])
            .build_ui_state(
                |ui, sheet: &mut Shortcuts| sheet.show(ui.ctx(), area),
                Shortcuts::default(),
            );
        exo_ui::install(&harness.ctx).expect("the system fonts load");
        // Partway down the whole table, then a search narrows it to two sections, with the pointer
        // off the sheet.
        let sheet = harness.state_mut();
        sheet.open();
        sheet.scroll = 800.0;
        sheet.query = "mark".to_owned();
        harness.run_steps(1);
        // The two sections' end: 117px down at 800px tall.
        assert!(
            harness.state().scroll < 150.0,
            "scrolled to {}",
            harness.state().scroll
        );
    }
}
