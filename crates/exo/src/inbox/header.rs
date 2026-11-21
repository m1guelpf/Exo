//! The thread list's header: the folders button, the inbox's split tabs with their counts, Add
//! Split, the split settings gear, compose and search, and the scroll shadow under it.

use egui::{Color32, CornerRadius, CursorIcon, Painter, Pos2, Rect, Sense, Ui, Vec2, pos2, vec2};
use exo_ui::{
    Family, Font, IconButton, KeyTooltip, Placement, ShortcutTooltip, TextStyle,
    motion::{self, CubicBezier},
};

/// `padding: 32px … 20px` around a 24px line of tabs.
pub(super) const HEIGHT: f32 = 76.0;
/// A tab's name: 16px/24px.
const TAB: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
/// A tab's count: 12px/24px.
const COUNT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 24.0);
/// The padding before a tab's bullet: 8px, 5px after a count.
const BULLET_PADDING: [f32; 2] = [8.0, 5.0];
/// The margin after a tab with a bullet.
const BULLET_MARGIN: f32 = 8.0;

/// What the user did in the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderAction {
    /// The folders button, or the already-selected Inbox tab: open the folder drawer.
    Folders,
    /// A split tab, by its index in [`View::tabs`](super::View::tabs).
    Tab(usize),
    /// Add Split: open the split inbox templates.
    AddSplit,
    /// The split settings gear.
    SplitSettings,
    /// Compose.
    Compose,
    /// Search.
    Search,
    /// The select-all checkmark (multi-select only): select everything, or clear.
    ToggleSelectAll,
    /// Mark the selection done (multi-select only).
    Done,
    /// Remind me for the selection (multi-select only).
    Remind,
    /// The command palette for the selection (multi-select only).
    Command,
    /// A click on the header's background, which clears the selection.
    Background,
    /// The filter tab: open the filter picker.
    Filter,
    /// The filter tab's ×: remove the filter.
    ClearFilter,
}

/// What in the header has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The folders button.
    Folders,
    /// The split tab at this index of [`View::tabs`](super::View::tabs).
    Tab(usize),
    /// A view's own tab, outside the inbox views.
    Title,
    /// The filter tab.
    Filter,
    /// Add Split.
    AddSplit,
    /// The split gear.
    SplitSettings,
    /// Compose.
    Compose,
    /// Search.
    Search,
}

impl Focus {
    /// What Enter does on it: what a click does.
    pub(super) fn action(self) -> HeaderAction {
        match self {
            Self::Folders | Self::Title => HeaderAction::Folders,
            Self::Tab(index) => HeaderAction::Tab(index),
            Self::Filter => HeaderAction::Filter,
            Self::AddSplit => HeaderAction::AddSplit,
            Self::SplitSettings => HeaderAction::SplitSettings,
            Self::Compose => HeaderAction::Compose,
            Self::Search => HeaderAction::Search,
        }
    }
}

/// What can have the keyboard in `view`'s header, in tab order: the folders button, the shown tabs
/// (or the view's own), the filter tab, Add Split, the split gear, compose and search.
pub(super) fn focusables(view: &super::View) -> Vec<Focus> {
    let mut items = vec![Focus::Folders];
    if view.inbox {
        items.extend(
            view.tabs
                .iter()
                .enumerate()
                .filter(|(_, tab)| !tab.hidden)
                .map(|(index, _)| Focus::Tab(index)),
        );
    } else {
        items.push(Focus::Title);
    }
    if view.filter.is_some() {
        items.push(Focus::Filter);
    }
    if view.inbox && view.add_split {
        items.push(Focus::AddSplit);
    }
    if view.inbox {
        items.push(Focus::SplitSettings);
    }
    items.extend([Focus::Compose, Focus::Search]);
    items
}

/// `box-shadow: 0 2px #aeb1dd`: a focused tab's, icon's or gear's underline, Classic's in both
/// themes.
const FOCUS_INK: Color32 = Color32::from_rgb(0xae, 0xb1, 0xdd);

/// Paints a focus underline: 2px under the `width`-wide box whose bottom is at `bottom`.
fn underline(painter: &Painter, left: f32, bottom: f32, width: f32) {
    painter.rect_filled(
        exo_ui::pixel_snap(Rect::from_min_size(pos2(left, bottom), vec2(width, 2.0))),
        0.0,
        FOCUS_INK,
    );
}

/// What the header shows besides the tabs.
pub(super) enum Mode {
    /// The folders button, compose and search.
    Normal,
    /// The select-all checkmark (checked when everything is) and the selection's actions, Mark
    /// Done offering Mark not Done when the selection is all `done`, Remind Me unless it's all
    /// drafts.
    Selecting {
        all: bool,
        done: bool,
        remind: Option<Remind>,
    },
}

/// The selection's Remind Me button.
pub(super) struct Remind {
    /// Every selected thread has a reminder: the bolder clock.
    pub set: bool,
    /// One thread's reminder, as its tooltip's status line says it.
    pub status: Option<String>,
}

/// Paints the header for `view` in `rect` (the list view's width, [`HEIGHT`] tall) over the list's
/// top, with the scroll shadow once the list has `scrolled`, and `focus` showing what has the
/// keyboard; at Inbox Zero (`zero`) it's transparent.
#[expect(
    clippy::too_many_arguments,
    reason = "one header, drawn from the list's state; a struct would only rename them"
)]
pub(super) fn show(
    ui: &Ui,
    rect: Rect,
    rem: f32,
    compact: bool,
    view: &super::View,
    mode: &Mode,
    scrolled: bool,
    zero: bool,
    focus: Option<Focus>,
) -> Option<HeaderAction> {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter();
    if !zero {
        painter.rect_filled(rect, 0.0, palette.background_base_default);
    }
    let background = ui.interact(
        rect,
        ui.id().with("thread-list-header-background"),
        Sense::click(),
    );
    let id = ui.id().with("thread-list-header");
    // After the header: its 1px line overlaps the header's last row of pixels.
    exo_ui::scroll_shadow(
        painter,
        id.with("shadow"),
        rect.left_bottom(),
        rect.width(),
        scrolled,
    );
    let mut action = None;
    // Over Inbox Zero's photo the glyphs are white: the folders' lines at .3 (.5 hovered), search,
    // compose and the split gear at .8 (1 hovered).
    let glyphs = |icon: IconButton<'static>| {
        if zero {
            icon.colors(white(0.8), white(1.0))
        } else {
            icon
        }
    };
    let lines = zero.then(|| (white(0.3), white(0.5)));
    // Each with its tooltip under it (`placement: "bottom"`, `padding: 8px 0` less the tether's
    // offset).
    let button = |hit: Rect,
                  icon_rect: Rect,
                  icon: &str,
                  colors: Option<(Color32, Color32)>,
                  clicked: HeaderAction,
                  tooltip: KeyTooltip<'_>,
                  placement: Placement| {
        let button = IconButton::new(icon);
        let button = if let Some((rest, hover)) = colors {
            button.colors(rest, hover)
        } else {
            button
        };
        let response = button.show(ui, id.with(icon), hit, icon_rect);
        tooltip.show_at(ui, &response, placement);
        response.clicked().then_some(clicked)
    };
    // Compose and search with the keyboard: the icon in its hover color, a 14px underline under it
    // (6px, 5px), and the tooltip as on hover.
    let icon = |hit: Rect,
                icon_rect: Rect,
                icon: &'static str,
                clicked: HeaderAction,
                tooltip: KeyTooltip<'_>,
                focused: bool,
                below_icon: f32| {
        let mut button = glyphs(IconButton::new(icon));
        if focused {
            button = button.colors(palette.icon_base_hover, palette.icon_base_hover);
            underline(
                painter,
                icon_rect.left(),
                icon_rect.bottom() + below_icon,
                14.0,
            );
        }
        let response = button.show(ui, id.with(icon), hit, icon_rect);
        ShortcutTooltip {
            rows: std::slice::from_ref(&tooltip),
            note: None,
        }
        .show_below(ui, &response, 8.0, focused, egui::Rangef::EVERYTHING);
        response.clicked().then_some(clicked)
    };
    let below = |gap: f32| Placement::Below { gap, shift: 0.0 };

    match mode {
        Mode::Normal => {
            // Folders: `left: calc(2.5rem - 25px); top: 36px`, the 16×12 lines.
            let hamburger = pos2(
                rect.left() + if compact { 9.0 } else { 2.5 * rem - 25.0 },
                rect.top() + 36.0,
            );
            // `Folders` (offset `2 0`): `Show folders` / `Hide folders` belong to the pinned folder
            // drawer, behind a flag.
            let folders = KeyTooltip {
                text: "Folders",
                keys: &[],
            };
            let glyph_box = Rect::from_min_size(hamburger, vec2(16.0, 12.0));
            // Classic's `padding: 10px` around the lines: the tooltip sits 6px under that.
            let hit = Rect::from_min_size(hamburger - vec2(10.0, 10.0), vec2(36.0, 32.0));
            if focus == Some(Focus::Folders) {
                // Focused, its lines are `#bec1e4`.
                let ink = Color32::from_rgb(0xbe, 0xc1, 0xe4);
                let response = IconButton::new("legacy/DEPRECATED_hamburgerMenu")
                    .colors(ink, ink)
                    .show(
                        ui,
                        id.with("legacy/DEPRECATED_hamburgerMenu"),
                        hit,
                        glyph_box,
                    );
                ShortcutTooltip {
                    rows: std::slice::from_ref(&folders),
                    note: None,
                }
                .show_below(ui, &response, 6.0, true, egui::Rangef::EVERYTHING);
                action = action.or(response.clicked().then_some(HeaderAction::Folders));
            } else {
                action = action.or(button(
                    hit,
                    glyph_box,
                    "legacy/DEPRECATED_hamburgerMenu",
                    lines,
                    HeaderAction::Folders,
                    folders,
                    below(6.0),
                ));
            }

            // Search and compose float right, 14px glyphs 35px down: search's right edge
            // `2rem - 7px` in, compose's box `2rem + 15px` in with 6px of padding on its right.
            // Hidden while the settings menu is open at Inbox Zero.
            if !(zero && view.settings_open) {
                let search = pos2(rect.right() - 2.0 * rem + 5.0 - 14.0, rect.top() + 35.0);
                let tooltip = KeyTooltip {
                    text: "Search",
                    keys: &["/"],
                };
                let glyph = |at: egui::Pos2| Rect::from_min_size(at, Vec2::splat(14.0));
                // Their clickable boxes, which center the tooltips: search's pads the glyph
                // `10px 10px 10px 6px`, compose's `10px 6px 10px 10px`.
                action = action.or(icon(
                    Rect::from_min_size(search - vec2(6.0, 10.0), vec2(30.0, 34.0)),
                    glyph(search),
                    "legacy/DEPRECATED_headerSearch-14",
                    HeaderAction::Search,
                    tooltip,
                    focus == Some(Focus::Search),
                    5.0,
                ));
                let compose = pos2(rect.right() - 2.0 * rem - 20.0 - 15.0, rect.top() + 35.0);
                let tooltip = KeyTooltip {
                    text: "Compose",
                    keys: &["C"],
                };
                action = action.or(icon(
                    Rect::from_min_size(compose - vec2(10.0, 10.0), vec2(30.0, 34.0)),
                    glyph(compose),
                    "legacy/DEPRECATED_headerCompose-14",
                    HeaderAction::Compose,
                    tooltip,
                    focus == Some(Focus::Compose),
                    6.0,
                ));
            }
        }
        Mode::Selecting { all, done, remind } => {
            // The checkmark: `left: calc(2.5rem - 11px); top: 46px; margin: -10px` against the list
            // view's padding box.
            let view_left = rect.left() - 6.0;
            let at = pos2(
                view_left + if compact { 15.0 } else { 2.5 * rem - 21.0 },
                rect.top() + 36.0,
            );
            let hit = Rect::from_min_size(at - Vec2::splat(10.0), vec2(35.0, 34.0));
            let response = ui
                .interact(hit, id.with("select-all"), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            // The tooltip's top left goes to the bottom left of the checkmark's 15×14 box, 16px
            // under it and 8px across.
            let rows = [
                KeyTooltip {
                    text: "Select All",
                    keys: &["⌘", "shift", "A"],
                },
                KeyTooltip {
                    text: "Select All From Here",
                    keys: &["⌘", "A"],
                },
            ];
            ShortcutTooltip {
                rows: &rows,
                note: None,
            }
            .show_at(
                ui,
                &response,
                Placement::BelowFrom {
                    gap: 6.0,
                    inset: 10.0,
                },
            );
            super::row::paint_checkmark(painter, at, *all, palette.icon_base_default);
            if response.clicked() {
                action = Some(HeaderAction::ToggleSelectAll);
            }
            // The selection's actions: `padding-right: calc(2rem - 5px)`, `margin: -4px -4px 0 0`
            // under the header's 32px top padding, buttons 2px down in their clickables.
            let mut left = rect.right() - 2.0 * rem + 5.0 + 4.0;
            // Their tooltips sit 10px right of centre.
            let done_tooltip = if *done {
                KeyTooltip {
                    text: "Mark not Done",
                    keys: &["shift", "E"],
                }
            } else {
                KeyTooltip {
                    text: "Mark Done",
                    keys: &["E"],
                }
            };
            let placement = Placement::Below {
                gap: 8.0,
                shift: 10.0,
            };
            // As a row's actions: 24px boxes 4px apart, the 14px glyphs 5px in, the 16×14 check at
            // its box's left, 5px down.
            let mut place = |inset: Vec2, size: Vec2| {
                let hit =
                    Rect::from_min_size(pos2(left - 24.0, rect.top() + 30.0), Vec2::splat(24.0));
                left = hit.left() - 4.0;
                (hit, Rect::from_min_size(hit.min + inset, size))
            };
            let (hit, glyph) = place(vec2(5.0, 5.0), Vec2::splat(14.0));
            action = action.or(button(
                hit,
                glyph,
                "legacy/DEPRECATED_threadActionsCommand",
                None,
                HeaderAction::Command,
                KeyTooltip {
                    text: "Exo Command",
                    keys: &["⌘", "K"],
                },
                placement,
            ));
            if let Some(remind) = remind {
                let (hit, glyph) = place(vec2(5.0, 5.0), Vec2::splat(14.0));
                let icon = if remind.set {
                    super::row::REMIND_SET_ICON
                } else {
                    super::row::REMIND_ICON
                };
                let mut button = IconButton::new(icon);
                if remind.set {
                    // With a reminder set, the clock's own colors.
                    button = button.colors(
                        Color32::from_rgb(0xe0, 0xbf, 0xe0),
                        Color32::from_rgb(0xcd, 0x7a, 0xcf),
                    );
                }
                let response = button.show(ui, id.with(icon), hit, glyph);
                let tooltip = KeyTooltip {
                    text: "Remind Me",
                    keys: &["H"],
                };
                match &remind.status {
                    Some(status) => tooltip.show_at_with_status(
                        ui,
                        &response,
                        placement,
                        status,
                        egui::Rangef::EVERYTHING,
                    ),
                    None => tooltip.show_at(ui, &response, placement),
                }
                if response.clicked() {
                    action = Some(HeaderAction::Remind);
                }
            }
            let (hit, glyph) = place(vec2(0.0, 5.0), vec2(16.0, 14.0));
            action = action.or(button(
                hit,
                glyph,
                "legacy/DEPRECATED_threadActionsDone",
                None,
                HeaderAction::Done,
                done_tooltip,
                placement,
            ));
        }
    }

    // The tabs: an inbox view's split tabs, or the view's own, selected.
    if !view.inbox {
        let (x, top) = (
            rect.left() + if compact { 44.0 } else { 3.5 * rem },
            rect.top() + 32.0,
        );
        // The selected tab's name.
        let label_width = TAB.paint_line(
            painter,
            pos2(x, top),
            &view.title,
            palette.ink(0.8, 0.85),
            None,
        );
        let tab = Rect::from_min_size(pos2(x, top), vec2(label_width, 24.0));
        if focus == Some(Focus::Title) {
            underline(painter, tab.left(), tab.bottom(), tab.width());
        }
        if ui
            .interact(tab, id.with("inbox-tab"), Sense::click())
            .clicked()
        {
            action = Some(HeaderAction::Folders);
        }
        if let Some(filter) = view.filter {
            let (_, clicked) = filter_tab(
                ui,
                id.with("filter"),
                pos2(tab.right() + 1.5 * rem, top),
                filter,
                focus == Some(Focus::Filter),
            );
            action = action.or(clicked);
        }
        // Add Split and the split gear belong to the inbox views.
        return action.or_else(|| background.clicked().then_some(HeaderAction::Background));
    }
    // Tooltips only with more than one tab to tell apart.
    let several = view.tabs.iter().filter(|tab| !tab.hidden).count() > 1;
    let placed = layout(ui, rect, rem, compact, view);
    for (index, (folder, at)) in view.tabs.iter().zip(&placed.tabs).enumerate() {
        let Some((at, cut)) = *at else { continue };
        let (_, clicked) = tab(
            ui,
            id.with(("tab", &folder.list)),
            at,
            folder,
            index == view.selected,
            cut,
            several,
            focus == Some(Focus::Tab(index)),
            zero,
        );
        if clicked {
            action = Some(HeaderAction::Tab(index));
        }
    }
    if let (Some(filter), Some(at)) = (view.filter, placed.filter) {
        let (_, clicked) = filter_tab(
            ui,
            id.with("filter"),
            at,
            filter,
            focus == Some(Focus::Filter),
        );
        action = action.or(clicked);
    }

    if let Some(at) = placed.add_split {
        let (end, top) = (at.x, at.y);
        let add_label = TAB.single_line(painter, "Add Split", Color32::PLACEHOLDER, None);
        let add = Rect::from_min_size(pos2(end, top), vec2(10.5 + 6.0 + add_label.size().x, 24.0));
        let add_response = ui
            .interact(add, id.with("add-split"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        // Add Split's icon and label; focused as hovered. Over the photo they're white, the button
        // at .85.
        let lit = add_response.hovered() || focus == Some(Focus::AddSplit);
        let color = ink(
            palette,
            zero,
            if lit { 0.6 } else { 0.4 },
            if lit { 0.6 } else { 0.4 },
        );
        let color = if zero {
            color.gamma_multiply(0.85)
        } else {
            color
        };
        let (icon_color, label_color) = (color, color);
        plus(
            painter,
            exo_ui::pixel_snap(Rect::from_min_size(
                pos2(end, top + 4.75),
                Vec2::splat(10.5),
            ))
            .min,
            icon_color,
        );
        TAB.paint_line(
            painter,
            pos2(end + 16.5, top),
            "Add Split",
            label_color,
            None,
        );
        if add_response.clicked() {
            action = Some(HeaderAction::AddSplit);
        }
    }

    // The split settings gear shows while the header is hovered or has the keyboard
    // (`transition: opacity 150ms`).
    let gear = placed.gear + vec2(0.0, 2.0);
    let gear_hit = Rect::from_min_size(gear - Vec2::splat(8.0), Vec2::splat(32.0));
    let gear_focused = focus == Some(Focus::SplitSettings);
    let visible = motion::transition(
        ui.ctx(),
        id.with("gear"),
        ui.rect_contains_pointer(rect) || focus.is_some(),
        0.15,
        CubicBezier::EASE,
    );
    if visible > 0.0 {
        let response = ui
            .interact(gear_hit, id.with("gear-hit"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let color = match (zero, response.hovered()) {
            (true, hovered) => white(if hovered { 1.0 } else { 0.8 }),
            (false, true) => palette.icon_base_hover,
            (false, false) => palette.icon_base_default,
        };
        // The gear, 14px, 1px down.
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_settings",
            exo_ui::pixel_snap(Rect::from_min_size(
                gear + vec2(0.0, 1.0),
                Vec2::splat(14.0),
            )),
            color.gamma_multiply(visible),
        );
        if gear_focused {
            // The gear's focus underline: 16px wide, 4px under the 16px icon box, which starts 1px
            // above the gear's clickable's padding.
            underline(painter, gear.x, gear.y - 1.0 + 16.0 + 4.0, 16.0);
        }
        // Offset `3 0`.
        ShortcutTooltip {
            rows: &[KeyTooltip {
                text: "Split Inbox Settings",
                keys: &[],
            }],
            note: None,
        }
        .show_below(ui, &response, 5.0, gear_focused, egui::Rangef::EVERYTHING);
        if response.clicked() {
            action = Some(HeaderAction::SplitSettings);
        }
    }
    action.or_else(|| background.clicked().then_some(HeaderAction::Background))
}

/// The header's line of tabs: 24px tall.
const LINE: f32 = 24.0;

/// Where the header's row of tabs puts its items: tabs, the filter tab, Add Split and the split
/// gear, left to right from `3.5rem` in, wrapping onto another 24px line when one doesn't fit
/// with its margin before `8rem` from the right (`flex-wrap: wrap`).
pub(super) struct Layout {
    /// Each tab's place; none once it has gone.
    tabs: Vec<Option<PlacedTab>>,
    filter: Option<Pos2>,
    add_split: Option<Pos2>,
    /// The gear's line box.
    gear: Pos2,
    /// Lines the row takes.
    pub(super) lines: usize,
}

/// Lays out the inbox header's row for `view` in `rect` (the header's).
pub(super) fn layout(ui: &Ui, rect: Rect, rem: f32, compact: bool, view: &super::View) -> Layout {
    let id = ui.id().with("thread-list-header");
    let left = rect.left() + if compact { 44.0 } else { 3.5 * rem };
    let right = rect.right() - 8.0 * rem;
    let mut flow = Flow {
        left,
        right,
        x: left,
        top: rect.top() + 32.0,
        lines: 1,
    };
    let mut tabs = Vec::with_capacity(view.tabs.len());
    for folder in &view.tabs {
        let (opacity, collapsed) = hiding(ui, id.with(("tab-hidden", &folder.list)), folder.hidden);
        let shown = 1.0 - collapsed;
        if shown <= 0.0 {
            tabs.push(None);
            continue;
        }
        // `margin-right: 1.5rem` after a tab, 8px before a bullet's partner; a hidden tab's
        // `max-width` falls from the whole label's (`100%`).
        let margin = if folder.bullet {
            BULLET_MARGIN
        } else {
            1.5 * rem
        };
        let cut = folder.hidden.then_some(((right - left) * shown, opacity));
        let width = tab_width(ui, id.with(("tab", &folder.list)), folder);
        tabs.push(Some((
            flow.place(cut.map_or(width, |(max, _)| width.min(max)), margin * shown),
            cut,
        )));
    }
    let filter = view
        .filter
        .map(|name| flow.place(filter_width(ui, name), 1.5 * rem));
    let add_split = view
        .add_split
        .then(|| flow.place(10.5 + 6.0 + TAB.width(ui.ctx(), "Add Split"), 1.5 * rem));
    let gear = flow.place(16.0, 0.0);
    Layout {
        tabs,
        filter,
        add_split,
        gear,
        lines: flow.lines,
    }
}

/// A tab's line box, and while it hides, the width it's cut to and its opacity.
type PlacedTab = (Pos2, Option<(f32, f32)>);

/// `flex-wrap: wrap` for one row of items.
struct Flow {
    left: f32,
    right: f32,
    x: f32,
    top: f32,
    lines: usize,
}

impl Flow {
    /// Where an item `width` wide with `margin` after it goes: next on the line, or first on a
    /// new one when it doesn't fit.
    fn place(&mut self, width: f32, margin: f32) -> Pos2 {
        if self.x > self.left && self.x + width + margin > self.right {
            (self.x, self.top, self.lines) = (self.left, self.top + LINE, self.lines + 1);
        }
        let at = pos2(self.x, self.top);
        self.x += width + margin;
        at
    }
}

/// The header's height for `lines` lines of tabs: `padding: 32px … 20px` around them.
pub(super) fn height(lines: usize) -> f32 {
    32.0 + LINE * lines as f32 + 20.0
}

/// The header's height on screen with `lines` lines of tabs: it grows at once, and shrinks over
/// 250ms (`ease`).
pub(super) fn height_shown(ui: &Ui, lines: usize) -> f32 {
    #[derive(Clone, Copy)]
    struct Shrink {
        from: f32,
        to: f32,
        since: f64,
    }
    let id = ui.id().with("thread-list-header").with("height");
    let ctx = ui.ctx();
    let now = ctx.input(|input| input.time);
    let target = height(lines);
    let at = |shrink: &Shrink| {
        shrink.from
            + (shrink.to - shrink.from)
                * CubicBezier::EASE.at((((now - shrink.since) / 0.25) as f32).clamp(0.0, 1.0))
    };
    let mut shrink = ctx
        .data(|data| data.get_temp::<Shrink>(id))
        .unwrap_or(Shrink {
            from: target,
            to: target,
            since: now,
        });
    if shrink.to != target {
        let current = at(&shrink);
        shrink = if target > current {
            Shrink {
                from: target,
                to: target,
                since: now,
            }
        } else {
            Shrink {
                from: current,
                to: target,
                since: now,
            }
        };
    }
    ctx.data_mut(|data| data.insert_temp(id, shrink));
    let shown = at(&shrink);
    if shown != target {
        ctx.request_repaint();
    }
    shown
}

/// A tab's width: its name, its count's box and its bullet.
fn tab_width(ui: &Ui, id: egui::Id, folder: &crate::splits::Tab) -> f32 {
    let (label, count_width, _) = tab_parts(ui, id, folder);
    let count = count_width.map_or(0.0, |width| 5.0 + width);
    let bullet = if folder.bullet {
        BULLET_PADDING[usize::from(count_width.is_some())] + TAB.width(ui.ctx(), "•")
    } else {
        0.0
    };
    label + count + bullet
}

/// A tab's name's width, its count's box's (`width 100ms 250ms ease-in-out` after it changes),
/// and its count with whether it's incomplete.
fn tab_parts(
    ui: &Ui,
    id: egui::Id,
    folder: &crate::splits::Tab,
) -> (f32, Option<f32>, Option<(String, bool)>) {
    let label = TAB.width(ui.ctx(), &folder.name);
    let count = folder
        .count
        .filter(|count| (1..crate::splits::INBOX_COUNTS).contains(&count.threads))
        .map(|count| (count.threads.to_string(), !count.exact));
    // A count's box: 8px a character, the `+` one.
    let chars = count.as_ref().map_or(0, |(digits, incomplete)| {
        digits.len() + usize::from(*incomplete)
    });
    let width = motion::transition_value(
        ui.ctx(),
        id.with("count-width"),
        8.0 * chars as f32,
        0.1,
        0.25,
        CubicBezier::EASE_IN_OUT,
    );
    (label, count.is_some().then_some(width), count)
}

/// The filter tab's width: its 22px box.
fn filter_width(ui: &Ui, name: &str) -> f32 {
    1.0 + 8.0 + FILTER.width(ui.ctx(), name) + 8.0 + 8.0 + 8.0 + 1.0
}

/// A tab that hides while empty as it hides: its opacity (going over 250ms) and how far it has
/// collapsed (0–1; its `max-width` and margin go after 50ms, over 250ms), on
/// `cubic-bezier(0.19, 1, 0.22, 1)`. It comes back at once, the transition being the hidden
/// state's; one hidden from the start doesn't animate.
fn hiding(ui: &Ui, id: egui::Id, hidden: bool) -> (f32, f32) {
    const CURVE: CubicBezier = CubicBezier::new(0.19, 1.0, 0.22, 1.0);
    let elapsed = 0.3
        * ui.ctx()
            .animate_bool_with_time(id, hidden, if hidden { 0.3 } else { 0.0 });
    (
        1.0 - CURVE.at((elapsed / 0.25).min(1.0)),
        CURVE.at(((elapsed - 0.05) / 0.25).clamp(0.0, 1.0)),
    )
}

/// Paints a split tab (`Folder`) with its line box at `at`: the name, its count (12px, 5px after
/// the name, `top: 1px`, in a box 8px wide per character that widens 250ms after the count
/// does), a `+` while the count is incomplete, and its bullet. The selected tab is in the text
/// color and its count subdued; the others are in the accent color, the name subdued while
/// hovered. A tab collapsing as it hides is cut to `cut`'s width at its opacity. With `tooltip`, hovering it says
/// what it is ([`crate::splits::describe`]). Returns the tab's box and whether it was clicked.
#[expect(
    clippy::too_many_arguments,
    reason = "one tab of the header's row, drawn from its state"
)]
fn tab(
    ui: &Ui,
    id: egui::Id,
    at: egui::Pos2,
    folder: &crate::splits::Tab,
    selected: bool,
    cut: Option<(f32, f32)>,
    tooltip: bool,
    focused: bool,
    zero: bool,
) -> (Rect, bool) {
    let palette = exo_ui::palette(ui.ctx());
    let mut painter = ui.painter().clone();
    let (label, count_width, count) = tab_parts(ui, id, folder);
    let width = tab_width(ui, id, folder);
    let bullet = folder.bullet.then(|| {
        at.x + label
            + count_width.map_or(0.0, |width| 5.0 + width)
            + BULLET_PADDING[usize::from(count.is_some())]
    });
    let (digits, incomplete) = count.map_or((None, false), |(digits, incomplete)| {
        (Some(digits), incomplete)
    });
    let rect = Rect::from_min_size(at, vec2(cut.map_or(width, |(max, _)| width.min(max)), 24.0));
    if let Some((_, opacity)) = cut {
        // `overflow: hidden`.
        painter.set_clip_rect(painter.clip_rect().intersect(rect));
        painter.multiply_opacity(opacity);
    }
    // A tab is clickable; a hidden one takes no pointer.
    let response = ui.interact(
        rect,
        id,
        if folder.hidden {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    let hovered = response.hovered() && !folder.hidden;
    if hovered {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if tooltip && !folder.hidden {
        let (text, keys) = folder.key();
        exo_ui::DescribedTooltip {
            description: folder.description.as_deref(),
            row: KeyTooltip { text, keys },
        }
        .show_below(ui, &response, 8.0, focused);
    }
    // The name's and the count's colors: selected, hovered or neither.
    let name_color = match (selected, hovered) {
        (true, _) => ink(palette, zero, 0.8, 0.85),
        (false, true) => ink(palette, zero, 0.6, 0.6),
        (false, false) => ink(palette, zero, 0.4, 0.4),
    };
    let count_color = if selected {
        ink(palette, zero, 0.35, 0.35)
    } else {
        ink(palette, zero, 0.25, 0.25)
    };
    TAB.paint_line(&painter, at, &folder.name, name_color, None);
    if focused {
        // Focused, the underline is under the name alone.
        underline(&painter, at.x, at.y + 24.0, label);
    }
    if let Some(digits) = &digits {
        let count_at = pos2(at.x + label + 5.0, at.y + 1.0);
        let digits_width = COUNT.paint_line(&painter, count_at, digits, count_color, None);
        if incomplete {
            // The count's `+`: 1px down, no margin.
            COUNT.paint_line(
                &painter,
                count_at + vec2(digits_width, 1.0),
                "+",
                count_color,
                None,
            );
        }
    }
    if let Some(x) = bullet {
        // The bullet.
        TAB.paint_line(
            &painter,
            pos2(x, at.y),
            "•",
            ink(palette, zero, 0.3, 0.3),
            None,
        );
    }
    (rect, response.clicked() && !folder.hidden)
}

/// White at `alpha`, over Inbox Zero's photo.
fn white(alpha: f32) -> Color32 {
    Color32::from_white_alpha((alpha * 255.0).round() as u8)
}

/// [`exo_ui::Palette::ink`], or over Inbox Zero's photo (`zero`) white at Carbon's opacity: the
/// tabs over the photo repeat Carbon's colors (a name is white at .4).
fn ink(palette: &exo_ui::Palette, zero: bool, snow: f32, carbon: f32) -> Color32 {
    if zero {
        white(carbon)
    } else {
        palette.ink(snow, carbon)
    }
}

/// The filter tab's name: 12px/20px bold, nudged up 2px.
const FILTER: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);

/// Paints the filter tab with its left edge at `at.x` on the tabs' line at `at.y`: a 22px rounded
/// box (`height: 22px; margin-top: 1px; padding: 0 8px; border-radius: 6px`), the filter's name,
/// and an 8px × after 8px. Returns its box and what was clicked.
fn filter_tab(
    ui: &Ui,
    id: egui::Id,
    at: egui::Pos2,
    name: &str,
    focused: bool,
) -> (Rect, Option<HeaderAction>) {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter();
    let label = FILTER
        .single_line(painter, name, Color32::PLACEHOLDER, None)
        .size()
        .x;
    let tab = Rect::from_min_size(
        pos2(at.x, at.y + 1.0),
        vec2(1.0 + 8.0 + label + 8.0 + 8.0 + 8.0 + 1.0, 22.0),
    );
    let close = Rect::from_min_size(
        pos2(tab.left() + 9.0 + label + 8.0, tab.top() + 7.0),
        Vec2::splat(8.0),
    );
    let close_response = ui
        .interact(close.expand(8.0), id.with("close"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let response = ui
        .interact(tab, id, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    // A 1px `#dee1e6` border, white at .3 in Carbon; hovered, the unread actions color. Focused,
    // its border, name and × are `#aeb1dd`.
    let border = if focused {
        FOCUS_INK
    } else if response.hovered() || close_response.hovered() {
        palette.actions_unread
    } else {
        match exo_ui::theme(ui.ctx()) {
            exo_ui::Theme::Snow => Color32::from_rgb(0xde, 0xe1, 0xe6),
            exo_ui::Theme::Carbon => Color32::from_white_alpha(77),
        }
    };
    painter.rect_stroke(
        tab,
        CornerRadius::same(6),
        egui::Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    // Inside the border, `top: -2px`, on the baseline of the tab's own 24px strut (3px down).
    FILTER.paint_line(
        painter,
        pos2(tab.left() + 9.0, tab.top() + 2.0),
        name,
        if focused {
            FOCUS_INK
        } else {
            palette.link_text_primary
        },
        None,
    );
    // `Icons.close()`: two 1/12-wide diagonals of a 12px box, drawn 8px wide.
    let color = if focused {
        FOCUS_INK
    } else if close_response.hovered() {
        palette.text_base_default
    } else {
        palette.icon_base_default
    };
    let stroke = egui::Stroke::new(8.0 / 12.0, color);
    let inset = 0.5 * 8.0 / 12.0;
    painter.line_segment(
        [
            close.min + Vec2::splat(inset),
            close.max - Vec2::splat(inset),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pos2(close.right() - inset, close.top() + inset),
            pos2(close.left() + inset, close.bottom() - inset),
        ],
        stroke,
    );
    let clicked = if close_response.clicked() {
        Some(HeaderAction::ClearFilter)
    } else {
        response.clicked().then_some(HeaderAction::Filter)
    };
    (tab, clicked)
}

/// The Add Split `+`: 1.05px bars in an 11px viewBox drawn 10.5px wide at `at`.
fn plus(painter: &Painter, at: egui::Pos2, color: Color32) {
    let scale = 10.5 / 11.0;
    let bar = |min: [f32; 2], max: [f32; 2]| {
        Rect::from_min_max(
            at + vec2(min[0], min[1]) * scale,
            at + vec2(max[0], max[1]) * scale,
        )
    };
    painter.rect_filled(bar([0.0, 4.725], [10.5, 5.775]), 0.0, color);
    painter.rect_filled(bar([4.725, 0.0], [5.775, 4.725]), 0.0, color);
    painter.rect_filled(bar([4.725, 5.775], [5.775, 10.5]), 0.0, color);
}
