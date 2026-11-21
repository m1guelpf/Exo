//! The Snippets folder's list: the `Private Snippets` section (with `New Private Snippet` and its
//! columns), a row per snippet (name, subject, text, and its numbers), and the cursor. It takes
//! the thread list's place; its metrics are a 1280×800 window's.

use egui::{Rect, Sense, Ui, pos2, vec2};
use exo_commands::{Command, Commands, Region};
use exo_store::Snippet;
use exo_ui::{Family, Font, HighlightState, HighlightTarget, ListHighlight, TextStyle};

use super::{Action, Column, Sort};
use crate::commands;

/// The section title and the column headers: 12px/16px bold.
const SECTION: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
/// The section's action: 14px/16px, weight 600.
const ACTION: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 14.0, 16.0);
/// A row's name: 12px/16px bold.
const NAME: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
/// A row's subject, text and numbers, and the empty section's line: 12px/16px.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// A row's numbers: 12px/16px, `letter-spacing: .017em`.
const METADATA: TextStyle =
    TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0).letter_spacing(0.017 * 12.0);
/// Rows are 36px tall.
const ROW: f32 = 36.0;
/// The first section header is 36px tall (`padding-top: 10px`), on the list's 3px.
const FIRST_HEADER: f32 = 36.0;
const LIST_TOP: f32 = 3.0;
/// The view's side padding, as the thread list has it.
const VIEW_PADDING_X: f32 = 6.0;
/// The top status: 10px, the keycap's 24px line 1px up, 10px.
const TOP_STATUS: f32 = 43.0;
/// The top status's text: 12px/16px, `top: 1px`.
const STATUS: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);

/// The folder's state.
#[derive(Default)]
pub(crate) struct Folder {
    open: bool,
    /// The highlighted snippet.
    cursor: Option<String>,
    /// The order a column's header gave it, until the folder is gone to again.
    sort: Option<Sort>,
    /// The snippet open read-only in the pane.
    viewing: Option<String>,
    /// Which row the pointer gives the cursor, as in the thread list.
    hover: crate::inbox::RowHover,
    /// The snippet the right pane shows.
    pane: Option<String>,
    /// When the pointer last moved the cursor, until the right pane has followed.
    hovered_at: Option<f64>,
}

impl Folder {
    /// The folder becomes the list.
    pub fn open(&mut self) {
        self.open = true;
    }

    /// Another list replaces it (the view closed first).
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Snippet `id` opens read-only, the cursor on its row.
    pub fn view(&mut self, id: &str) {
        self.open = true;
        self.set_cursor(Some(id.to_owned()));
        self.viewing = Some(id.to_owned());
    }

    /// The pane shows the folder's tips again.
    pub fn close_view(&mut self) {
        self.viewing = None;
    }

    /// The snippet open read-only.
    pub fn viewing(&self) -> Option<&str> {
        self.viewing.as_deref()
    }

    /// Whether it's the list.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The highlighted snippet.
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// J / K over `ids` (the rows, in order), stopping at the ends.
    pub fn move_cursor(&mut self, ids: &[String], delta: isize) {
        let at = self
            .cursor
            .as_ref()
            .and_then(|cursor| ids.iter().position(|id| id == cursor))
            .unwrap_or(0);
        self.set_cursor(
            ids.get(
                at.saturating_add_signed(delta)
                    .min(ids.len().saturating_sub(1)),
            )
            .cloned(),
        );
    }

    /// The cursor goes to `cursor` from a key or the app: the right pane follows at once.
    fn set_cursor(&mut self, cursor: Option<String>) {
        self.cursor = cursor;
        self.hovered_at = None;
    }

    /// The pointer moved the cursor to `id`, which highlights the row at once: the right pane
    /// follows once it has stayed there ([`crate::SETTLED_HOVER`]).
    fn hover_to(&mut self, id: String, now: f64) {
        if self.cursor.as_ref() != Some(&id) {
            self.cursor = Some(id);
            self.hovered_at = Some(now);
        }
    }

    /// The snippet the right pane shows at `now`: the cursor's, unless a hover put it there too
    /// recently, when the one before.
    pub fn pane(&mut self, ctx: &egui::Context, now: f64) -> Option<&str> {
        match self.hovered_at {
            Some(at) if now - at < crate::SETTLED_HOVER => {
                ctx.request_repaint_after_secs((at + crate::SETTLED_HOVER - now) as f32)
            }
            _ => {
                self.hovered_at = None;
                self.pane.clone_from(&self.cursor);
            }
        }
        self.pane.as_deref()
    }

    /// The order a column's header asked for.
    pub fn sort(&self) -> Option<Sort> {
        self.sort
    }

    /// A column's header: the section sorts by it, most first, or the other way when it already
    /// does.
    pub fn sort_by(&mut self, column: Column) {
        let ascending = self
            .sort
            .is_some_and(|sort| sort.column == column && !sort.ascending);
        self.sort = Some(Sort { column, ascending });
    }

    /// Back to the default order.
    pub fn unsort(&mut self) {
        self.sort = None;
    }

    /// Keeps the cursor on a row that's there: the first, when it isn't.
    fn settle(&mut self, ids: &[String]) {
        if self
            .cursor
            .as_ref()
            .is_none_or(|cursor| !ids.contains(cursor))
        {
            self.set_cursor(ids.first().cloned());
        }
    }
}

/// The folder's keys, the thread list's over snippets: J / K, Enter to edit, C for a
/// new one (`Create Snippet`'s place), and `Discard Snippet`.
pub(super) fn register(commands: &mut Commands<commands::Action>, count: usize) {
    const LIST: &[Region] = &[Region::ThreadList];
    let act = commands::Action::Snippets;
    // Shift moves too: the folder selects nothing.
    commands.add(
        Command::shortcut("SNIPPET_NEXT", act(Action::Move(1)))
            .keys(["j", "down", "shift+j", "shift+down"])
            .regions(LIST),
    );
    commands.add(
        Command::shortcut("SNIPPET_PREVIOUS", act(Action::Move(-1)))
            .keys(["k", "up", "shift+k", "shift+up"])
            .regions(LIST),
    );
    commands.add(
        Command::shortcut("SNIPPET_CREATE", act(Action::Create))
            .key("c")
            .regions(LIST),
    );
    // Enter opens the highlighted snippet, and never the thread list's row underneath.
    commands.add(
        Command::shortcut("SNIPPET_OPEN", act(Action::Open(None)))
            .key("enter")
            .regions(LIST),
    );
    if count > 0 {
        commands.add(
            Command::new("EDIT_SNIPPET", "Edit Snippet", act(Action::Edit(None)))
                .default_score(6.2)
                .regions(LIST),
        );
        commands.add(
            Command::new("DISCARD_SNIPPET", "Discard Snippet", act(Action::Discard))
                .keys(["mod+shift+,", "mod+shift+.", "#"])
                .key_code(["⌘", "shift", ","])
                .aliases(["Delete Snippet"])
                .regions(LIST),
        );
    }
}

/// The read-only view's keys: Escape back to the folder, J / K to the next and previous
/// snippets, `Edit Snippet` and `Discard Snippet`.
pub(super) fn register_view(commands: &mut Commands<commands::Action>) {
    const VIEW: &[Region] = &[Region::ReadOnlySnippet];
    let act = commands::Action::Snippets;
    commands.add(
        Command::shortcut("SNIPPET_VIEW_BACK", act(Action::Back))
            .key("escape")
            .regions(VIEW),
    );
    commands.add(
        Command::shortcut("SNIPPET_VIEW_NEXT", act(Action::Move(1)))
            .keys(["j", "down"])
            .regions(VIEW),
    );
    commands.add(
        Command::shortcut("SNIPPET_VIEW_PREVIOUS", act(Action::Move(-1)))
            .keys(["k", "up"])
            .regions(VIEW),
    );
    commands.add(
        Command::new("EDIT_SNIPPET", "Edit Snippet", act(Action::Edit(None)))
            .default_score(6.2)
            .regions(VIEW),
    );
    commands.add(
        Command::new("DISCARD_SNIPPET", "Discard Snippet", act(Action::Discard))
            .keys(["mod+shift+,", "mod+shift+."])
            .key_code(["⌘", "shift", ","])
            .aliases(["Delete Snippet"])
            .regions(VIEW),
    );
}

/// The Replied number's tooltip, 8px under `anchor` on the tooltips' hover timing, the numbers'
/// thousands grouped.
fn replies_tooltip(ui: &Ui, anchor: &egui::Response, replies: u32, sends: u32) {
    let text = format!(
        "{} out of {} messages sent with\nthis snippet were replied to.",
        grouped(replies),
        grouped(sends)
    );
    let within = egui::Rangef::new(ui.clip_rect().left(), ui.ctx().content_rect().right());
    exo_ui::TextTooltip {
        text: &text,
        max_width: f32::INFINITY,
        learn_more: false,
    }
    .show_below(ui, anchor, 8.0, within);
}

/// The folder's top status at `at` (its text's left, the status's top): `Hit C to create a
/// Snippet!`, after `You currently have no Snippets.` when there are `none`.
fn top_status(ui: &Ui, painter: &egui::Painter, at: egui::Pos2, none: bool) {
    let palette = exo_ui::palette(ui.ctx());
    // `rgba(0,0,0,.5)` (white in Carbon).
    let ink = palette.ink(0.5, 0.5);
    let before = if none {
        "You currently have no Snippets. Hit"
    } else {
        "Hit"
    };
    let line = at.y + 10.0;
    let mut x = at.x + STATUS.paint_line(painter, pos2(at.x, line), before, ink, None);
    // The keycap, 1px up in its line.
    let key = exo_ui::Keycaps::new(
        ui.ctx(),
        &["C"],
        exo_ui::KeycapColors::shortcuts_pane(palette),
    );
    key.paint(painter, x, at.y + 8.0);
    x += key.width();
    STATUS.paint_line(painter, pos2(x, line), " to create a Snippet!", ink, None);
}

/// `count` with thousands separated by commas (`1,500`), as en-US writes them.
fn grouped(count: u32) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// A count, shortened: `999`, `1k`, `1.5k`, `150k`, `1M`.
fn statistic(count: u32) -> String {
    let short = |value: f64, unit: &str| {
        let rounded = if value < 10.0 {
            (value * 10.0).round() / 10.0
        } else {
            value.round()
        };
        format!("{rounded}{unit}")
    };
    match count {
        0..1000 => count.to_string(),
        1000..1_000_000 => short(f64::from(count) / 1000.0, "k"),
        _ => short(f64::from(count) / 1_000_000.0, "M"),
    }
}

impl crate::Mail {
    /// Draws the folder in `rect` (the content pane): the thread list's header titled
    /// `Snippets`, then the section and its rows. Returns what was asked for.
    pub(crate) fn show_snippets(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        rem: f32,
        window_width: f32,
        now: f64,
    ) {
        let palette = exo_ui::palette(ui.ctx());
        ui.painter()
            .rect_filled(rect, 0.0, palette.background_base_default);
        self.snippet_replies_arrived();
        let settings = self.account.settings();
        let sort = self.snippets.folder.sort();
        let snippets: Vec<Snippet> =
            super::ordered(&settings.snippets, &settings.snippets_last_used, sort)
                .into_iter()
                .cloned()
                .collect();
        let ids: Vec<String> = snippets.iter().map(|snippet| snippet.id.clone()).collect();
        self.snippets.folder.settle(&ids);
        let view = rect.shrink2(vec2(VIEW_PADDING_X, 0.0));
        let list_top = view.top() + crate::inbox::HEADER_HEIGHT;
        let compact = window_width <= 1152.0;
        // `padding: … 1rem … 3.5rem` (44px narrow).
        let left = view.left() + if compact { 44.0 } else { 3.5 * rem };
        let content_width = view.right() - rem - left;
        // The numbers' columns, from the view's right: Author 92px (62px under 1024px),
        // Replied 60, Opened 60, Sends 50, each 8px after the last.
        let author = view.right() - rem - if window_width < 1024.0 { 62.0 } else { 92.0 };
        let replied = author - 8.0 - 60.0;
        let opened = replied - 8.0 - 60.0;
        let sends = opened - 8.0 - 50.0;
        let painter = ui.painter_at(Rect::from_min_max(pos2(rect.left(), list_top), rect.max));
        let mut actions = Vec::new();
        // The section title and the column headers.
        let heading = palette.ink(0.4, 0.4);

        // The top status, over the rows while a draft is popped out.
        let mut header = list_top + LIST_TOP;
        if self.compose.popped().is_some() {
            top_status(ui, &painter, pos2(left, header), snippets.is_empty());
            header += TOP_STATUS;
        }

        // The section header: `flex: 1 1 22.5%` for the title, the action after it.
        // On its 16px line, `top: .5px`.
        SECTION.paint_line(
            &painter,
            pos2(left, header + 10.0),
            "Private Snippets",
            heading,
            None,
        );
        let action_left = left + 0.225 * content_width;
        // Classic's `text-transform: uppercase`, on the container's 16px line
        // (`top: .5px`); the span's 17px box is what takes the click.
        let width = ACTION.paint_line(
            &painter,
            pos2(action_left, header + 10.0),
            "NEW PRIVATE SNIPPET",
            palette.link_text_primary,
            None,
        );
        let button = ui
            .interact(
                Rect::from_min_size(pos2(action_left, header + 8.5), vec2(width, 17.0)),
                ui.id().with("new-private-snippet"),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if button.clicked() {
            actions.push(Action::New(String::new()));
        }
        // The headers `Sends `, `Opened `, `Replied ` sort the section (not while it's empty),
        // the sorted one with its arrow; the private section's `Author` doesn't.
        for (column, title, sorts) in [
            (sends, "Sends ", Some(Column::Sends)),
            (opened, "Opened ", Some(Column::Opened)),
            (replied, "Replied ", Some(Column::Replied)),
            (author, "Author", None),
        ] {
            let width =
                SECTION.paint_line(&painter, pos2(column, header + 10.0), title, heading, None);
            let Some(by) = sorts else { continue };
            if let Some(sorted) = sort.filter(|sort| sort.column == by) {
                // `DEPRECATED_downArrow` (12px box), its bottom 4px over the 16px line's; legacy
                // themes turn it over for ascending.
                let arrow =
                    Rect::from_min_size(pos2(column + width, header + 10.0), vec2(12.0, 12.0));
                let turn = if sorted.ascending {
                    std::f32::consts::PI
                } else {
                    0.0
                };
                exo_ui::paint_icon_rotated(ui, "legacy/DEPRECATED_downArrow", arrow, heading, turn);
            }
            if !snippets.is_empty() {
                let target = Rect::from_min_size(
                    pos2(column, header + 10.0),
                    vec2(width, SECTION.line_height),
                );
                if ui
                    .interact(
                        target,
                        ui.id().with(("snippet-sort", title)),
                        Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    actions.push(Action::Sort(by));
                }
            }
        }

        let rows_top = header + FIRST_HEADER;
        if snippets.is_empty() {
            TEXT.paint_line(
                &painter,
                pos2(left, rows_top + 10.0),
                "You currently have no Snippets.",
                palette.text_base_default,
                None,
            );
        }
        let cursor = self
            .snippets
            .folder
            .cursor()
            .and_then(|cursor| ids.iter().position(|id| id == cursor));
        let focused = self.commands.region() == Region::ThreadList;
        let highlight = cursor.map(|at| HighlightTarget {
            top: at as f32 * ROW,
            height: ROW,
        });
        ListHighlight::new(ui.id().with("snippet-highlight"))
            .paint(
                &painter,
                pos2(view.left(), rows_top),
                view.width(),
                highlight,
                HighlightState {
                    focused,
                    ..HighlightState::default()
                },
            )
            .paint(&painter);
        let name_right = left + 0.225 * content_width;
        let center_width = crate::inbox::center_width(view.width(), rem, compact);
        let mut hovered = None;
        for (at, snippet) in snippets.iter().enumerate() {
            let top = rows_top + at as f32 * ROW;
            let line = top + 10.0;
            // The name, cut with an ellipsis 2rem before the subject.
            NAME.paint_line(
                &painter,
                pos2(left, line),
                crate::compose::snippets::name(snippet),
                palette.ink(0.85, 0.85),
                Some(name_right - left - 2.0 * rem),
            );
            // The subject whole, then the text cut as the thread list's rows are, to the list's
            // middle column, `…` after it when cut; a snippet's
            // thread, in no inbox, counts as done, which takes the Done check's 24px.
            // The text stops 12px before the Sends column's 8px.
            let end = sends - 8.0 - 12.0;
            let mut x = name_right;
            if !snippet.subject.is_empty() {
                x += TEXT.paint_line(
                    &painter,
                    pos2(x, line),
                    &snippet.subject,
                    palette.text_base_default,
                    Some((end - x).max(0.0)),
                ) + rem;
            }
            if let Some((text, cut)) = crate::format::row::snippet(
                &crate::compose::snippets::preview(snippet),
                &[],
                center_width,
                &[],
                &snippet.subject,
                true,
            ) {
                let text = if cut { format!("{text}…") } else { text };
                TEXT.paint_line(
                    &painter,
                    pos2(x, line),
                    &text,
                    palette.text_base_subdued,
                    Some((end - x).max(0.0)),
                );
            }
            // As Exo counts them; opens need read statuses, which Exo hasn't got: `--`.
            let analytics = &snippet.analytics;
            let mut replies = None;
            for (column, value) in [
                (sends, statistic(analytics.sends)),
                (opened, "--".to_owned()),
                (
                    replied,
                    super::analytics::percentage(analytics.replies, analytics.sends),
                ),
                (author, "Me".to_owned()),
            ] {
                let width = METADATA.paint_line(
                    &painter,
                    pos2(column, line),
                    &value,
                    palette.text_base_subdued,
                    None,
                );
                if column == replied {
                    replies = Some(Rect::from_min_size(
                        pos2(column, line),
                        vec2(width, TEXT.line_height),
                    ));
                }
            }
            let row = Rect::from_min_size(pos2(view.left(), top), vec2(view.width(), ROW));
            let response = ui
                .interact(
                    row.intersect(painter.clip_rect()),
                    ui.id().with(("snippet-row", &snippet.id)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if ui.rect_contains_pointer(row.intersect(painter.clip_rect())) {
                hovered = Some(snippet.id.clone());
            }
            if response.clicked() {
                actions.push(Action::Open(Some(snippet.id.clone())));
            }
            if let Some(anchor) = replies {
                let anchor = ui.interact(
                    anchor.intersect(painter.clip_rect()),
                    ui.id().with(("snippet-replies", &snippet.id)),
                    Sense::hover(),
                );
                replies_tooltip(ui, &anchor, analytics.replies, analytics.sends);
            }
        }
        // A row the pointer moves onto takes the cursor, as in the thread list.
        let folder = &mut self.snippets.folder;
        if let Some(id) = folder.hover.update(
            ui,
            hovered,
            folder.cursor.as_deref(),
            now,
            painter.clip_rect(),
        ) {
            folder.hover_to(id, now);
        }

        match crate::inbox::show_header(
            ui,
            Rect::from_min_size(view.min, vec2(view.width(), crate::inbox::HEADER_HEIGHT)),
            rem,
            compact,
            "Snippets",
        ) {
            Some(crate::inbox::HeaderAction::Folders) => {
                self.run(commands::Action::ToggleFolders, now)
            }
            Some(crate::inbox::HeaderAction::Compose) => {
                self.compose_new(crate::compose::Fields::default(), None)
            }
            Some(crate::inbox::HeaderAction::Search) => {
                self.run(commands::Action::Search(crate::search::Action::Toggle), now)
            }
            _ => {}
        }
        for action in actions {
            self.snippets_run(action, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statistics_shorten_to_thousands_and_millions() {
        assert_eq!(statistic(999), "999");
        assert_eq!(statistic(1000), "1k");
        assert_eq!(statistic(1500), "1.5k");
        assert_eq!(statistic(150_000), "150k");
        assert_eq!(statistic(1_000_000), "1M");
    }

    #[test]
    fn counts_group_their_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1500), "1,500");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn a_header_sorts_most_first_then_the_other_way() {
        let mut folder = Folder::default();
        folder.sort_by(Column::Sends);
        assert_eq!(
            folder.sort(),
            Some(Sort {
                column: Column::Sends,
                ascending: false
            })
        );
        folder.sort_by(Column::Sends);
        assert_eq!(
            folder.sort(),
            Some(Sort {
                column: Column::Sends,
                ascending: true
            })
        );
        folder.sort_by(Column::Replied);
        assert_eq!(
            folder.sort(),
            Some(Sort {
                column: Column::Replied,
                ascending: false
            }),
            "another column starts most first"
        );
    }

    #[test]
    fn the_right_pane_follows_a_hover_once_it_settles_and_a_key_at_once() {
        let ctx = egui::Context::default();
        let ids = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let mut folder = Folder::default();
        folder.settle(&ids);
        assert_eq!(folder.pane(&ctx, 0.0), Some("a"));
        folder.hover_to("b".to_owned(), 1.0);
        assert_eq!(folder.cursor(), Some("b"), "the highlight moves at once");
        assert_eq!(
            folder.pane(&ctx, 1.05),
            Some("a"),
            "the pane waits for the hover to settle"
        );
        assert_eq!(folder.pane(&ctx, 1.1), Some("b"));
        folder.move_cursor(&ids, 1);
        assert_eq!(folder.pane(&ctx, 1.11), Some("c"), "a key moves it at once");
    }

    #[test]
    fn the_cursor_stays_on_a_row() {
        let ids = vec!["a".to_owned(), "b".to_owned()];
        let mut folder = Folder::default();
        folder.settle(&ids);
        assert_eq!(folder.cursor(), Some("a"));
        folder.move_cursor(&ids, 5);
        assert_eq!(folder.cursor(), Some("b"), "stops at the end");
        folder.settle(&ids[..1]);
        assert_eq!(folder.cursor(), Some("a"), "a discarded row's cursor moves");
    }
}
