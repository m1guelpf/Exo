//! The folder drawer: the account, the system folders (Snippets among them) and the user's labels
//! in a drawer that slides out over the list, opened with ← or the header's folders button.
//!
//! Items are numbered from 0, the account, then the folders, then the labels; a nested label's
//! parent heading isn't an item.

use egui::{
    Color32, CornerRadius, Id, Painter, Pos2, Rect, Sense, Ui, Vec2, pos2, text::LayoutJob, vec2,
};
use exo_store::{Label, ListCount, ListId};
use exo_ui::{
    BoxShadow, Family, Font, KeycapColors, Keycaps, TextStyle, Theme,
    motion::{self, CubicBezier},
    paint_job,
};

use crate::lists;

/// The drawer's left edge: right of the rail.
const LEFT: f32 = 40.0;
/// `width: 30vw; min-width: 300px; max-width: 416px`.
const MIN_WIDTH: f32 = 300.0;
const MAX_WIDTH: f32 = 416.0;
/// `transition: transform 200ms ease, opacity 200ms ease`.
const SLIDE: f32 = 0.2;
/// The space above the 48px account row.
const ACCOUNT_TOP: f32 = 20.0;
/// Where the scrolling list starts, from the drawer's top.
const SCROLL_TOP: f32 = 76.0;
/// A row's height, folders, labels and headings alike, its text 16px from its top.
const ROW: f32 = 48.0;
const ROW_PADDING_TOP: f32 = 16.0;
/// Nested labels: 12px above and below the group, pulled 4px up, and 4px back after the last.
const NESTED_PADDING: f32 = 12.0;
const NESTED_PULL: f32 = 4.0;
/// The divider: 11px above its 1px line and 12px below.
const DIVIDER: f32 = 11.0 + 1.0 + 12.0;
const DIVIDER_LINE: f32 = 11.0;
/// A row's label: 12px/16px bold, 1px down.
const LABEL: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
/// The Inbox row's `Important • Other`: 12px/16px, `lighter` than the bold label, `.017em`.
const IMPORTANT_OTHER: TextStyle =
    TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0).letter_spacing(0.017 * 12.0);
/// The account avatar: a 32px circle, `margin-right: 14px`.
const AVATAR: f32 = 32.0;
const AVATAR_GAP: f32 = 14.0;
/// The avatar's initial: 18px, bold like the row it's in, centered with `dy: .35em`.
const INITIAL: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 18.0, 18.0);
/// The cursor's highlight: 6px in from either side, 2px corners, a 3px bar.
const HIGHLIGHT_INSET: f32 = 6.0;
const HIGHLIGHT_RADIUS: u8 = 2;
const BAR_WIDTH: f32 = 3.0;
/// `transition: transform 256ms cubic-bezier(0.19, 1, 0.22, 1), opacity 512ms …`.
const HIGHLIGHT_SLIDE: f32 = 0.256;
const HIGHLIGHT_FADE: f32 = 0.512;
const HIGHLIGHT_CURVE: CubicBezier = CubicBezier::new(0.19, 1.0, 0.22, 1.0);
/// Labels listed before `Load More`, unless there are fewer than ten more.
const LABELS_SHOWN: usize = 100;
/// `INBOX_COUNTS`: counts this large aren't shown.
const MAX_COUNT: usize = 1000;
/// The Inbox count's offset: 75px right while hidden, 8px left on hover, over 100ms.
const COUNT_HIDDEN: f32 = 75.0;
const COUNT_SHOWN: f32 = -8.0;
const HOVER: f32 = 0.1;

/// Where a folder of the drawer goes: a list, or the Snippets folder, which isn't one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// A list of conversations.
    List(ListId),
    /// The Snippets folder (`SH_SNIPPETS`).
    Snippets,
}

/// A line of the drawer below the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// A system folder, with its `g` key if it has one.
    Folder {
        place: Place,
        name: &'static str,
        key: Option<&'static str>,
    },
    /// A user label, showing the part of its name after its parent's when nested.
    Label {
        list: ListId,
        name: String,
        nested: bool,
    },
    /// The parent of nested labels that isn't a label itself.
    Header(String),
    /// More than 110 labels: the rest.
    LoadMore,
}

impl Line {
    /// Whether the cursor can land on it.
    fn is_item(&self) -> bool {
        !matches!(self, Self::Header(_))
    }
}

/// The system folders in the drawer's order. Scheduled, which has no `g` key, always shows; so do
/// Reminders and Snippets; Muted once mute has been used.
fn folders(muted: bool) -> Vec<Line> {
    let folder = |list: ListId, key: Option<&'static str>| Line::Folder {
        name: lists::name(&list, &[], &[]),
        place: Place::List(list),
        key,
    };
    let mut folders = vec![
        folder(ListId::Inbox, Some("I")),
        folder(ListId::Starred, Some("S")),
        folder(ListId::Drafts, Some("D")),
        folder(ListId::Sent, Some("T")),
        folder(ListId::Done, Some("E")),
        folder(ListId::Scheduled, None),
        folder(ListId::Reminders, Some("H")),
        Line::Folder {
            place: Place::Snippets,
            name: "Snippets",
            key: Some(";"),
        },
    ];
    if muted {
        folders.push(folder(ListId::Muted, Some("M")));
    }
    folders.extend([
        folder(ListId::Spam, Some("!")),
        folder(ListId::Trash, Some("#")),
    ]);
    folders
}

/// The drawer's lines: the folders, then the user's labels by name (a leading `[` ignored), nested
/// ones under their parent, and `Load More` past 100 of them unless `all`.
pub fn lines(labels: &[Label], muted: bool, all: bool) -> Vec<Line> {
    let mut user: Vec<&Label> = lists::listed_labels(labels).collect();
    user.sort_by(|a, b| {
        crate::mailbox::compare_label_names(
            a.name.trim_start_matches('['),
            b.name.trim_start_matches('['),
        )
    });
    let shown = if all || user.len() < LABELS_SHOWN + 10 {
        user.len()
    } else {
        LABELS_SHOWN
    };
    let mut lines = folders(muted);
    let mut parent = String::new();
    for label in &user[..shown] {
        let list = ListId::Label(label.id.clone());
        match label.name.split_once('/') {
            Some((top, rest)) => {
                if parent != top {
                    parent = top.to_owned();
                    lines.push(Line::Header(parent.clone()));
                }
                lines.push(Line::Label {
                    list,
                    name: rest.to_owned(),
                    nested: true,
                });
            }
            None => {
                parent.clone_from(&label.name);
                lines.push(Line::Label {
                    list,
                    name: label.name.clone(),
                    nested: false,
                });
            }
        }
    }
    if shown < user.len() {
        lines.push(Line::LoadMore);
    }
    lines
}

/// What a key does in the drawer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// ↑: up, onto the account row too, but not when held.
    Up,
    /// K, P (and ↑ held): up, stopping at the first folder.
    UpWithin,
    /// ↓, J, N.
    Down,
    /// ⌘↑, Home, ⇧K.
    Top,
    /// ⌘↓, End, ⇧J.
    Bottom,
    /// Page Down, Space.
    PageDown,
    /// Page Up, ⇧Space.
    PageUp,
    /// Enter: open the item.
    Enter,
    /// →: close, or switch accounts on the account row.
    Right,
    /// Esc, ⌘→.
    Close,
}

/// What the drawer asks of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Go there, with the hint for having clicked it (`None` from the keyboard).
    Show(Place, Option<Hint>),
    /// Close the drawer.
    Close,
    /// Open the Accounts palette: the account row's click, Enter or →.
    SwitchAccounts,
}

/// The hint a click on a folder shows: its `g` key, or for a label, the palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hint {
    /// `Next time, hit G then {key} to go to {name} instantly!`
    GoTo {
        key: &'static str,
        name: &'static str,
    },
    /// `Next time, hit ⌘ K and enter the label to go there faster!`
    Label,
}

/// What the drawer shows besides its lines.
pub struct View<'a> {
    /// The account's address.
    pub email: &'a str,
    /// The account's color.
    pub color: Color32,
    /// The Inbox's count, shown on its row while hovered: Important's while Important and
    /// Other are on.
    pub inbox: Option<ListCount>,
    /// Important and Other are on: the Inbox's row says so.
    pub important_other: bool,
}

/// The drawer's state.
pub struct Folders {
    open: bool,
    /// The highlighted item.
    cursor: usize,
    /// The highlight jumps rather than slides to the next position (a hover moved it).
    jump: bool,
    /// The pointer moved over the drawer since the keyboard last moved the cursor: hovered
    /// folders show their keys and count.
    mousing: bool,
    /// Every label is listed (`Load More` was chosen).
    all: bool,
    scroll: f32,
    /// The scroll area's visible height, from the last frame.
    viewport: f32,
    /// The cursor moved by key: scroll it into view.
    reveal: bool,
    last_pointer: Option<Pos2>,
    /// The pointer was over the drawer last frame.
    inside: bool,
}

impl Default for Folders {
    fn default() -> Self {
        Self {
            open: false,
            cursor: 1,
            jump: false,
            mousing: false,
            all: false,
            scroll: 0.0,
            viewport: 0.0,
            reveal: false,
            last_pointer: None,
            inside: false,
        }
    }
}

impl Folders {
    /// Whether it's open: it has the keyboard, and clicks outside it close it.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Whether every label is listed.
    pub fn all(&self) -> bool {
        self.all
    }

    /// Opens on `current`'s item (the first folder for the inbox views or a list it hasn't).
    pub fn open(&mut self, lines: &[Line], current: &Place) {
        let current = match current {
            Place::List(list) if lists::is_inbox(list) => &Place::List(ListId::Inbox),
            place => place,
        };
        let at = lines
            .iter()
            .filter(|line| line.is_item())
            .position(|line| match line {
                Line::Folder { place, .. } => place == current,
                Line::Label { list, .. } => {
                    matches!(current, Place::List(current) if current == list)
                }
                _ => false,
            });
        *self = Self {
            open: true,
            cursor: at.map_or(1, |at| at + 1),
            jump: true,
            reveal: true,
            // A pointer that was already over where the drawer opens hasn't moved over it.
            last_pointer: self.last_pointer,
            ..Self::default()
        };
    }

    /// Closes it.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Carries out `key` over `lines`.
    pub fn key(&mut self, key: Key, lines: &[Line]) -> Option<Action> {
        let last = lines.iter().filter(|line| line.is_item()).count();
        let page = (self.viewport / ROW).floor().max(1.0) as usize;
        let to = match key {
            Key::Up => self.cursor.saturating_sub(1),
            Key::UpWithin => self.cursor.saturating_sub(1).max(1).min(self.cursor),
            Key::Down => (self.cursor + 1).min(last),
            Key::Top => 1,
            Key::Bottom => last,
            Key::PageDown => (self.cursor + page).min(last),
            Key::PageUp => self.cursor.saturating_sub(page).max(1),
            Key::Enter => return self.choose(lines, self.cursor, false),
            Key::Right if self.cursor == 0 => return Some(Action::SwitchAccounts),
            Key::Right | Key::Close => return Some(Action::Close),
        };
        self.move_to(to, false);
        None
    }

    fn move_to(&mut self, cursor: usize, by_pointer: bool) {
        if !by_pointer {
            self.mousing = false;
            self.reveal = true;
        }
        self.jump = by_pointer;
        self.cursor = cursor;
    }

    /// What choosing item `index` does: opens its list, or lists every label.
    fn choose(&mut self, lines: &[Line], index: usize, clicked: bool) -> Option<Action> {
        let Some(item) = index.checked_sub(1) else {
            return Some(Action::SwitchAccounts);
        };
        let line = lines.iter().filter(|line| line.is_item()).nth(item)?;
        match line {
            // A folder without a key (Scheduled) has no hint.
            Line::Folder { place, name, key } => Some(Action::Show(
                place.clone(),
                key.filter(|_| clicked).map(|key| Hint::GoTo { key, name }),
            )),
            Line::Label { list, .. } => Some(Action::Show(
                Place::List(list.clone()),
                clicked.then_some(Hint::Label),
            )),
            Line::LoadMore => {
                self.all = true;
                None
            }
            Line::Header(_) => None,
        }
    }

    /// Paints the drawer over `page` (the window below the tab bar) and handles the mouse.
    pub fn show(
        &mut self,
        ui: &Ui,
        page: Rect,
        rem: f32,
        lines: &[Line],
        view: &View<'_>,
    ) -> Option<Action> {
        let ctx = ui.ctx();
        let id = exo_ui::id(ctx, "exo::folders");
        let shown = motion::transition(ctx, id.with("slide"), self.open, SLIDE, CubicBezier::EASE);
        if shown <= 0.0 {
            self.last_pointer = ctx.input(|input| input.pointer.hover_pos());
            return None;
        }
        let width = (0.3 * page.width()).clamp(MIN_WIDTH, MAX_WIDTH);
        let drawer = Rect::from_min_size(
            pos2(page.left() + LEFT - width * (1.0 - shown), page.top()),
            vec2(width, page.height()),
        );
        egui::Area::new(id)
            .order(egui::Order::Foreground)
            .fixed_pos(page.min)
            .show(ctx, |ui| {
                self.draw(ui, page, drawer, rem, shown, lines, view)
            })
            .inner
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one drawer, drawn from its own state; a struct would only rename them"
    )]
    fn draw(
        &mut self,
        ui: &mut Ui,
        page: Rect,
        drawer: Rect,
        rem: f32,
        shown: f32,
        lines: &[Line],
        view: &View<'_>,
    ) -> Option<Action> {
        let ctx = ui.ctx().clone();
        let id = exo_ui::id(&ctx, "exo::folders");
        let palette = exo_ui::palette(&ctx);
        let mut action = None;
        // A click anywhere else closes the drawer, and goes no further.
        if self.open && ui.allocate_rect(page, Sense::click()).clicked() {
            action = Some(Action::Close);
        }
        ui.multiply_opacity(shown);
        // The rail stays on top: the drawer slides out from under it, and its shadow doesn't
        // reach over it.
        ui.shrink_clip_rect(Rect::from_min_max(
            pos2(page.left() + LEFT, page.top()),
            page.max,
        ));
        let painter = ui.painter().clone();
        // `clip-path: inset(0 -30px -30px -30px)`: no shadow above the drawer.
        let shadow_painter = painter.with_clip_rect(Rect::from_min_max(
            pos2(page.left(), drawer.top()),
            page.max,
        ));
        // The drawer's shadow: `0 0 30px 0 rgba(0, 0, 0, .1), 0 0 .5px 0 rgba(0, 0, 0, .05)`;
        // `0 0 30px 0 #000` in Carbon.
        let shadows: &[BoxShadow] = match exo_ui::theme(&ctx) {
            Theme::Snow => &[
                BoxShadow::new([0.0, 0.0], 30.0, 0.0, Color32::from_black_alpha(26)),
                BoxShadow::new([0.0, 0.0], 0.5, 0.0, Color32::from_black_alpha(13)),
            ],
            Theme::Carbon => &[BoxShadow::new([0.0, 0.0], 30.0, 0.0, Color32::BLACK)],
        };
        for shadow in shadows {
            shadow.paint(&shadow_painter, drawer, CornerRadius::ZERO);
        }
        painter.rect_filled(drawer, 0.0, palette.background_elevated_subdued);
        // Clicks on the drawer itself don't reach the layer under it.
        let _ = ui.interact(drawer, id.with("panel"), Sense::click());

        // The pointer only moves the cursor when it really moved.
        let pointer = ctx.input(|input| input.pointer.hover_pos());
        let moved = pointer.is_some() && pointer != self.last_pointer;
        self.last_pointer = pointer;
        if moved && ui.rect_contains_pointer(drawer) {
            self.mousing = true;
        }
        // Leaving any way but over the rail closes it.
        let inside = pointer.is_some_and(|at| drawer.contains(at));
        if self.open && self.inside && !inside && pointer.is_some_and(|at| at.x > drawer.left()) {
            action = Some(Action::Close);
        }
        self.inside = inside;

        let left = drawer.left() + 3.5 * rem;
        let right = drawer.right() - 2.0 * rem;
        let list_area =
            Rect::from_min_max(pos2(drawer.left(), drawer.top() + SCROLL_TOP), drawer.max);
        self.viewport = list_area.height();

        // Lay the lines out: each one's top, from the top of the scroll content.
        let mut layout = Vec::with_capacity(lines.len() + 1);
        let mut top = 0.0;
        let mut previous_nested = false;
        let folders = lines
            .iter()
            .take_while(|line| matches!(line, Line::Folder { .. }))
            .count();
        for (i, line) in lines.iter().enumerate() {
            if i == folders {
                top += DIVIDER;
            }
            let (height, pull) = match line {
                Line::Label {
                    name, nested: true, ..
                } => {
                    let width = right - (drawer.left() + 3.5 * rem * 1.75);
                    (wrapped_height(&ctx, name, width), NESTED_PULL)
                }
                _ if previous_nested => (ROW, -NESTED_PULL),
                _ => (ROW, 0.0),
            };
            top -= pull;
            layout.push((top, height));
            top += height;
            previous_nested = matches!(line, Line::Label { nested: true, .. });
        }
        let content_height = top;

        // Scrolling: the wheel, and the cursor kept in view as it moves.
        if ui.rect_contains_pointer(list_area) {
            self.scroll -= ctx.input(|input| input.smooth_scroll_delta.y);
        }
        let item_rows: Vec<(f32, f32)> = lines
            .iter()
            .zip(&layout)
            .filter(|(line, _)| line.is_item())
            .map(|(_, row)| *row)
            .collect();
        if std::mem::take(&mut self.reveal)
            && let Some(&(row_top, height)) =
                self.cursor.checked_sub(1).and_then(|at| item_rows.get(at))
        {
            if row_top < self.scroll {
                self.scroll = row_top;
            } else if row_top + height > self.scroll + self.viewport {
                self.scroll = row_top + height - self.viewport;
            }
        }
        self.scroll = self
            .scroll
            .clamp(0.0, (content_height - self.viewport).max(0.0));
        let origin = list_area.top() - self.scroll;

        // The list, under the account row.
        let list_painter = painter.with_clip_rect(list_area);
        let highlight = self
            .cursor
            .checked_sub(1)
            .and_then(|at| item_rows.get(at))
            .copied();
        paint_highlight(
            &list_painter,
            id_for(list_painter.ctx(), "list"),
            drawer,
            origin,
            highlight,
            self.jump,
        );
        let mut index = 0;
        for (i, (line, &(row_top, height))) in lines.iter().zip(&layout).enumerate() {
            if i == folders {
                // The divider: a 1px #ededed line, #5f6165 in Carbon.
                let y = origin + row_top - DIVIDER + DIVIDER_LINE;
                let color = match exo_ui::theme(&ctx) {
                    Theme::Snow => Color32::from_rgb(0xed, 0xed, 0xed),
                    Theme::Carbon => Color32::from_rgb(0x5f, 0x61, 0x65),
                };
                list_painter.rect_filled(
                    exo_ui::pixel_snap(Rect::from_min_max(pos2(left, y), pos2(right, y + 1.0))),
                    0.0,
                    color,
                );
            }
            let row = Rect::from_min_size(
                pos2(drawer.left(), origin + row_top),
                vec2(drawer.width(), height),
            );
            if line.is_item() {
                index += 1;
            }
            if !row.intersects(list_area) {
                continue;
            }
            let item = line.is_item().then_some(index);
            let hovered = item.is_some()
                && self.open
                && pointer.is_some_and(|at| row.contains(at) && list_area.contains(at));
            if hovered
                && moved
                && let Some(item) = item
            {
                self.move_to(item, true);
            }
            let clicked = item.is_some()
                && ui
                    .interact(
                        row.intersect(list_area),
                        id.with(("folder", i)),
                        Sense::click(),
                    )
                    .clicked();
            if clicked && let Some(item) = item {
                action = self.choose(lines, item, true).or(action);
            }
            let text_top = row.top()
                + if matches!(line, Line::Label { nested: true, .. }) {
                    NESTED_PADDING
                } else {
                    ROW_PADDING_TOP
                }
                + 1.0;
            match line {
                Line::Folder { place, name, key } => {
                    let inbox = *place == Place::List(ListId::Inbox);
                    let name_width = LABEL.paint_line(
                        &list_painter,
                        pos2(left, text_top),
                        name,
                        label_color(palette),
                        Some(right - left),
                    );
                    if inbox && view.important_other {
                        paint_important_other(
                            &list_painter,
                            pos2(left + name_width, text_top),
                            palette,
                        );
                    }
                    let hover = hovered && self.mousing;
                    let shortcut = motion::transition_in(
                        &ctx,
                        id.with(("shortcut", i)),
                        hover,
                        HOVER,
                        CubicBezier::EASE,
                    );
                    let tokens = key.map(|key| ["G", "then", key]);
                    let caps = Keycaps::new(
                        &ctx,
                        tokens.as_ref().map_or(&[][..], |tokens| &tokens[..]),
                        keycap_colors(&ctx, shortcut),
                    );
                    // `float: right`, there even while transparent.
                    let shortcut_left = right - caps.width();
                    if shortcut > 0.0 && key.is_some() {
                        caps.paint(
                            &list_painter,
                            shortcut_left,
                            row.top() + ROW_PADDING_TOP - 1.0,
                        );
                    }
                    if inbox {
                        let slide = motion::transition(
                            &ctx,
                            id.with(("count", i)),
                            hover,
                            HOVER,
                            CubicBezier::EASE,
                        );
                        paint_count(
                            &list_painter,
                            &ctx,
                            row,
                            shortcut_left + COUNT_HIDDEN + (COUNT_SHOWN - COUNT_HIDDEN) * slide,
                            view.inbox,
                        );
                    }
                }
                Line::Label {
                    name, nested: true, ..
                } => {
                    let nested_left = drawer.left() + 3.5 * rem * 1.75;
                    // A nested label's name: at .5 in both themes.
                    let mut job = LayoutJob::single_section(
                        name.clone(),
                        LABEL.format(palette.ink(0.5, 0.5)),
                    );
                    job.wrap.max_width = right - nested_left;
                    paint_job(&list_painter, pos2(nested_left, text_top), job, &LABEL);
                }
                Line::Label { name, .. } | Line::Header(name) => {
                    LABEL.paint_line(
                        &list_painter,
                        pos2(left, text_top),
                        name,
                        label_color(palette),
                        Some(right - left),
                    );
                }
                Line::LoadMore => {
                    LABEL.paint_line(
                        &list_painter,
                        pos2(left, text_top),
                        "Load More",
                        palette.vbar_anchor_default,
                        None,
                    );
                }
            }
        }

        // The account, pinned above the list, and the shadow under it once the list scrolls.
        let account = Rect::from_min_size(
            pos2(drawer.left(), drawer.top() + ACCOUNT_TOP),
            vec2(drawer.width(), ROW),
        );
        paint_highlight(
            &painter,
            id_for(painter.ctx(), "account"),
            drawer,
            0.0,
            (self.cursor == 0).then_some((account.top(), ROW)),
            self.jump,
        );
        if self.open && moved && pointer.is_some_and(|at| account.contains(at)) {
            self.move_to(0, true);
        }
        if ui
            .interact(account, id.with("account"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            action = Some(Action::SwitchAccounts);
        }
        paint_account(ui, account, left, right, view);
        exo_ui::scroll_shadow(
            &painter,
            id.with("shadow"),
            pos2(drawer.left(), list_area.top()),
            drawer.width(),
            self.scroll > 0.0,
        );

        if shown < 1.0 {
            ctx.request_repaint();
        }
        action
    }
}

fn id_for(ctx: &egui::Context, highlight: &str) -> Id {
    exo_ui::id(ctx, ("exo::folders::highlight", highlight))
}

/// A nested label's height: its wrapped lines with 12px above and below.
fn wrapped_height(ctx: &egui::Context, name: &str, width: f32) -> f32 {
    let mut job = LayoutJob::single_section(name.to_owned(), LABEL.format(Color32::PLACEHOLDER));
    job.wrap.max_width = width;
    let rows = ctx
        .fonts_mut(|fonts| fonts.layout_job(job))
        .rows
        .len()
        .max(1);
    rows as f32 * LABEL.line_height + 2.0 * NESTED_PADDING
}

/// The cursor's highlight on the row at `target` (its top and height in content that starts
/// at `origin` on screen, so scrolling doesn't slide it), sliding from where it was, or fading
/// out with no target.
fn paint_highlight(
    painter: &Painter,
    id: Id,
    drawer: Rect,
    origin: f32,
    target: Option<(f32, f32)>,
    jump: bool,
) {
    let ctx = painter.ctx();
    let now = ctx.input(|input| input.time);
    #[derive(Clone, Copy)]
    struct Slide {
        from: (f32, f32),
        to: (f32, f32),
        started: f64,
    }
    let opacity = motion::transition(
        ctx,
        id.with("shown"),
        target.is_some(),
        if jump { 0.0 } else { HIGHLIGHT_FADE },
        HIGHLIGHT_CURVE,
    );
    let previous = ctx.data(|data| data.get_temp::<Slide>(id));
    let at = |slide: &Slide| {
        let t = ((now - slide.started) as f32 / HIGHLIGHT_SLIDE).clamp(0.0, 1.0);
        let eased = HIGHLIGHT_CURVE.at(t);
        (
            (
                slide.from.0 + (slide.to.0 - slide.from.0) * eased,
                slide.from.1 + (slide.to.1 - slide.from.1) * eased,
            ),
            t < 1.0,
        )
    };
    let slide = match (previous, target) {
        (Some(slide), Some(to)) if slide.to == to => slide,
        (Some(slide), Some(to)) if !jump => Slide {
            from: at(&slide).0,
            to,
            started: now,
        },
        (_, Some(to)) => Slide {
            from: to,
            to,
            started: now,
        },
        (Some(slide), None) => slide,
        (None, None) => return,
    };
    ctx.data_mut(|data| data.insert_temp(id, slide));
    let ((top, height), moving) = at(&slide);
    if opacity <= 0.0 {
        return;
    }
    let palette = exo_ui::palette(ctx);
    let rect = exo_ui::pixel_snap(Rect::from_min_size(
        pos2(drawer.left() + HIGHLIGHT_INSET, origin + top),
        vec2(drawer.width() - 2.0 * HIGHLIGHT_INSET, height),
    ));
    painter.rect_filled(
        rect,
        CornerRadius::same(HIGHLIGHT_RADIUS),
        palette.background_brand_subdued.gamma_multiply(opacity),
    );
    // `::before`, shown while the drawer has the keyboard, which it always has while open.
    let bar = Rect::from_min_size(rect.min, vec2(BAR_WIDTH, rect.height()));
    let corners = CornerRadius {
        nw: HIGHLIGHT_RADIUS,
        sw: HIGHLIGHT_RADIUS,
        ne: 0,
        se: 0,
    };
    painter.rect_filled(
        bar,
        corners,
        palette.border_base_active.gamma_multiply(opacity),
    );
    if moving || (opacity < 1.0 && target.is_some()) {
        ctx.request_repaint();
    }
}

/// The Inbox row's `Important • Other` after its name: 12px after it, black or white at .5, its
/// dot at .25 with 1px either side.
fn paint_important_other(painter: &Painter, at: Pos2, palette: &exo_ui::Palette) {
    let (color, dot) = (
        IMPORTANT_OTHER.format(palette.ink(0.5, 0.5)),
        IMPORTANT_OTHER.format(palette.ink(0.25, 0.25)),
    );
    // The dot's leading space collapses into the one before it, its trailing one swallows
    // `Other`'s; its margins go either side.
    let mut job = LayoutJob::default();
    job.append("Important ", 0.0, color.clone());
    job.append("• ", 1.0, dot);
    job.append("Other", 1.0, color);
    paint_job(painter, pos2(at.x + 12.0, at.y), job, &IMPORTANT_OTHER);
}

/// A folder's `G then {key}` caps (its `then` in the subdued text color), `opacity` of the way
/// faded in.
fn keycap_colors(ctx: &egui::Context, opacity: f32) -> KeycapColors {
    let palette = exo_ui::palette(ctx);
    let colors = KeycapColors::base(palette);
    KeycapColors {
        background: colors.background.gamma_multiply(opacity),
        text: colors.text.gamma_multiply(opacity),
        divider: palette.text_base_subdued.gamma_multiply(opacity),
    }
}

/// The Inbox's count, its right edge at `right`: 75px past the keys' left edge, or clear of
/// them by 8px while hovered. The row clips it.
fn paint_count(
    painter: &Painter,
    ctx: &egui::Context,
    row: Rect,
    right: f32,
    count: Option<ListCount>,
) {
    let Some(count) = count.filter(|count| (1..MAX_COUNT).contains(&count.threads)) else {
        return;
    };
    let text = if count.exact {
        count.threads.to_string()
    } else {
        format!("{}+", count.threads)
    };
    // Black at .35, white at .45.
    let color = exo_ui::palette(ctx).ink(0.35, 0.45);
    let width = LABEL.single_line(painter, &text, color, None).size().x;
    let clip = painter.with_clip_rect(painter.clip_rect().intersect(row));
    LABEL.paint_line(
        &clip,
        pos2(right - width, row.top() + ROW_PADDING_TOP + 1.0),
        &text,
        color,
        None,
    );
}

/// The account row: avatar, address and the switch-account caret.
fn paint_account(ui: &Ui, row: Rect, left: f32, right: f32, view: &View<'_>) {
    let (painter, ctx) = (ui.painter(), ui.ctx());
    let palette = exo_ui::palette(ctx);
    // The avatar, initial or photo, on whole pixels.
    let avatar = exo_ui::pixel_snap(Rect::from_min_size(
        pos2(left, row.center().y - AVATAR / 2.0),
        Vec2::splat(AVATAR),
    ));
    match crate::profile_pictures::photo(ctx, view.email) {
        // A 3px ring in the account's color.
        Some(photo) => exo_ui::paint_photo_avatar(painter, avatar, &photo, 3.0, view.color, 1.0),
        // The initial: `#fff`, `#27292d` in Carbon; `dy: .35em`.
        None => exo_ui::paint_initial_avatar(
            painter,
            avatar,
            view.color,
            &crate::accounts::initial(view.email),
            &INITIAL,
            palette.text_base_inverse,
            0.35 * 18.0,
        ),
    }
    // The caret: 6×11 at the row's right edge, centered and raised half a pixel.
    let caret = Rect::from_min_size(pos2(right - 6.0, row.center().y - 5.75), vec2(6.0, 11.0));
    let text_left = left + AVATAR + AVATAR_GAP;
    LABEL.paint_line(
        painter,
        pos2(text_left, row.center().y - 8.0),
        view.email,
        label_color(palette),
        Some((caret.left() - 4.0 - text_left).max(0.0)),
    );
    let name = "legacy/DEPRECATED_rightCaret";
    exo_ui::paint_icon(
        ui,
        name,
        exo_ui::icon_rect(ui.ctx(), name, caret),
        palette.icon_base_default,
    );
}

/// A row's label color: black at .75, white at .75.
fn label_color(palette: &exo_ui::Palette) -> Color32 {
    palette.ink(0.75, 0.75)
}

#[cfg(test)]
mod tests {
    use super::*;
    use exo_store::LabelKind;

    fn label(id: &str, name: &str) -> Label {
        Label {
            id: id.into(),
            name: name.into(),
            kind: LabelKind::User,
            text_color: None,
            background_color: None,
            hidden: false,
        }
    }

    fn names(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| match line {
                Line::Folder { name, .. } => (*name).to_owned(),
                Line::Label {
                    name, nested: true, ..
                } => format!("  {name}"),
                Line::Label { name, .. } => name.clone(),
                Line::Header(name) => format!("# {name}"),
                Line::LoadMore => "Load More".into(),
            })
            .collect()
    }

    #[test]
    fn folders_then_labels_nested_under_their_parents() {
        let labels = [
            label("L1", "Projects/Atlas"),
            label("L2", "[News]"),
            label("L3", "Clients"),
            label("L4", "Clients/Calloway"),
            label("L5", "Input/Needs Action"),
            label("L6", "[Superhuman]/AI/Pitch"),
            label("L7", "[Exo]/Muted"),
        ];
        assert_eq!(
            names(&lines(&labels, false, false)),
            [
                "Inbox",
                "Starred",
                "Drafts",
                "Sent",
                "Done",
                "Scheduled",
                "Reminders",
                "Snippets",
                "Spam",
                "Trash",
                "Clients",
                "  Calloway",
                "# Input",
                "  Needs Action",
                "# Projects",
                "  Atlas"
            ]
        );
        assert!(
            names(&lines(&labels, true, false)).contains(&"Muted".to_owned()),
            "once mute is used"
        );
    }

    #[test]
    fn long_label_lists_load_more() {
        let many: Vec<Label> = (0..120)
            .map(|i| label(&format!("L{i}"), &format!("Label {i}")))
            .collect();
        let short = lines(&many, false, false);
        assert_eq!(short.last(), Some(&Line::LoadMore));
        assert_eq!(
            short
                .iter()
                .filter(|line| matches!(line, Line::Label { .. }))
                .count(),
            100
        );
        assert_eq!(
            lines(&many[..105], false, false)
                .iter()
                .filter(|line| matches!(line, Line::Label { .. }))
                .count(),
            105,
            "fewer than ten more show at once"
        );

        let mut folders = Folders::default();
        folders.open(&short, &Place::List(ListId::Inbox));
        folders.key(Key::Bottom, &short);
        assert_eq!(folders.key(Key::Enter, &short), None);
        assert!(folders.all());
    }

    #[test]
    fn opens_on_the_current_view() {
        let labels = [label("L1", "Clients"), label("L2", "Clients/Calloway")];
        let lines = lines(&labels, false, false);
        let mut folders = Folders::default();
        folders.open(&lines, &Place::List(ListId::Important));
        assert_eq!(folders.cursor, 1, "the inbox views open on Inbox");
        folders.open(&lines, &Place::List(ListId::Label("L2".into())));
        assert_eq!(folders.cursor, 12);
        assert_eq!(
            folders.key(Key::Enter, &lines),
            Some(Action::Show(Place::List(ListId::Label("L2".into())), None))
        );
        folders.open(&lines, &Place::Snippets);
        assert_eq!(folders.cursor, 8, "the Snippets folder opens on Snippets");
        assert_eq!(
            folders.key(Key::Enter, &lines),
            Some(Action::Show(Place::Snippets, None))
        );
        folders.open(&lines, &Place::List(ListId::All));
        assert_eq!(folders.cursor, 1, "All Mail isn't listed");
    }

    #[test]
    fn keys_walk_the_account_folders_and_labels() {
        let lines = lines(&[label("L1", "Family")], false, false);
        let mut folders = Folders::default();
        folders.open(&lines, &Place::List(ListId::Inbox));
        folders.key(Key::UpWithin, &lines);
        assert_eq!(folders.cursor, 1, "K stops at the first folder");
        folders.key(Key::Up, &lines);
        assert_eq!(folders.cursor, 0, "↑ reaches the account");
        assert_eq!(
            folders.key(Key::Right, &lines),
            Some(Action::SwitchAccounts),
            "→ on the account opens the Accounts palette"
        );
        assert_eq!(
            folders.key(Key::Enter, &lines),
            Some(Action::SwitchAccounts),
            "and so does Enter"
        );
        folders.key(Key::Down, &lines);
        folders.key(Key::Down, &lines);
        assert_eq!(
            folders.key(Key::Enter, &lines),
            Some(Action::Show(Place::List(ListId::Starred), None))
        );
        folders.key(Key::Bottom, &lines);
        assert_eq!(folders.cursor, 11);
        folders.key(Key::Down, &lines);
        assert_eq!(folders.cursor, 11, "stops at the last label");
        folders.key(Key::Top, &lines);
        assert_eq!(folders.cursor, 1);
        assert_eq!(folders.key(Key::Right, &lines), Some(Action::Close));
        assert_eq!(folders.key(Key::Close, &lines), Some(Action::Close));
    }

    #[test]
    fn clicks_hint_at_the_keyboard() {
        let lines = lines(&[label("L1", "Family")], false, false);
        let mut folders = Folders::default();
        assert_eq!(
            folders.choose(&lines, 4, true),
            Some(Action::Show(
                Place::List(ListId::Sent),
                Some(Hint::GoTo {
                    key: "T",
                    name: "Sent"
                })
            ))
        );
        assert_eq!(
            folders.choose(&lines, 6, true),
            Some(Action::Show(Place::List(ListId::Scheduled), None)),
            "Scheduled has no key to hint at"
        );
        assert_eq!(
            folders.choose(&lines, 7, true),
            Some(Action::Show(
                Place::List(ListId::Reminders),
                Some(Hint::GoTo {
                    key: "H",
                    name: "Reminders"
                })
            ))
        );
        assert_eq!(
            folders.choose(&lines, 8, true),
            Some(Action::Show(
                Place::Snippets,
                Some(Hint::GoTo {
                    key: ";",
                    name: "Snippets"
                })
            ))
        );
        assert_eq!(
            folders.choose(&lines, 11, true),
            Some(Action::Show(
                Place::List(ListId::Label("L1".into())),
                Some(Hint::Label)
            ))
        );
    }
}
