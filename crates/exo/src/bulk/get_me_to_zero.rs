//! Get Me To Zero: marks Done every inbox conversation older than an age the user picks, keeping
//! the unread or starred ones if asked, as one bulk action.

use egui::{Context, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use exo_ui::{
    ActionKind, Dialog, DialogAction, DialogResponse, Family, Font, Menu, TextStyle, palette,
};
use jiff::{Timestamp, ToSpan, Zoned};

/// The subheader's paragraphs: 12px/20px, nudged down 3px.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
/// A label in the time range row: 12px/16px.
const RANGE_LABEL: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// The `More options…` button: 12px/20px semibold.
const MORE: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 20.0);
/// Where things sit below the body's top (the header row's bottom).
const LEAD_TOP: f32 = 27.0;
const MOVE_TOP: f32 = 59.0;
const OLD_TOP: f32 = 111.0;
const RULE_TOP: f32 = 166.0;
const RANGE_TOP: f32 = 184.0;
const MORE_TOP: f32 = 219.0;
/// The buttons' row: `margin: 16px 0 7px` around its 28px; Classic's buttons sit 3px down in it.
const ACTIONS_ROW: f32 = 254.0;
const ACTIONS_TOP: f32 = ACTIONS_ROW + 3.0;
/// The age's row is 34px in Classic (the dropdown's `padding-top: 3px; margin-top: 1px`), each
/// "More options" row 32px; together they take the button's 20px and 44 more.
const RANGE_ROW: f32 = 34.0;
const OPTION_ROW: f32 = 32.0;
const OPTIONS_EXTRA: f32 = 44.0;
/// The body under the header, to the actions' row's margin.
const BODY_HEIGHT: f32 = ACTIONS_ROW + 28.0 + 7.0;
/// The label's 12px before the dropdown.
const LABEL_GAP: f32 = 12.0;
/// The checkbox column: `padding: 0 12px 0 4px` around the 14px box.
const CHECKBOX_LEFT: f32 = 4.0;
const CHECKBOX_COLUMN: f32 = 30.0;
/// The action buttons: 8px apart, the last 13px past the content's right edge.
const ACTION_GAP: f32 = 8.0;
const ACTIONS_OVERHANG: f32 = 13.0;
/// The open list sits 18px left of the dropdown's text and 26px below its top, under Classic's 19px
/// button.
const MENU_OFFSET: Vec2 = vec2(-18.0, 26.0);

/// An age to clean up past.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Range {
    /// In the dropdown.
    pub name: &'static str,
    days: i64,
    months: i64,
}

/// The ages, the third (a week) chosen to start with.
pub(crate) const RANGES: [Range; 5] = [
    Range {
        name: "1 day",
        days: 1,
        months: 0,
    },
    Range {
        name: "3 days",
        days: 3,
        months: 0,
    },
    Range {
        name: "1 week (most popular)",
        days: 7,
        months: 0,
    },
    Range {
        name: "2 weeks",
        days: 14,
        months: 0,
    },
    Range {
        name: "1 month",
        days: 0,
        months: 1,
    },
];

impl Range {
    /// Messages dated before this are old: today's midnight, that far back.
    pub(crate) fn before(&self, now: &Zoned) -> Timestamp {
        let midnight = now.start_of_day().unwrap_or_else(|_| now.clone());
        midnight
            .checked_sub(self.days.days().months(self.months))
            .unwrap_or(midnight)
            .timestamp()
    }
}

/// What has the keyboard in the dialog, in tab order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Focus {
    #[default]
    Range,
    MoreOptions,
    KeepUnread,
    KeepStarred,
    DoLater,
    Confirm,
}

/// The dialog's choices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct State {
    /// Into [`RANGES`].
    pub range: usize,
    /// `Keep unread emails`.
    pub keep_unread: bool,
    /// `Keep starred emails`.
    pub keep_starred: bool,
    /// The checkboxes show (`More options…` was clicked).
    more: bool,
    /// The age list is open, with this row highlighted.
    menu: Option<usize>,
    focus: Focus,
    /// The button marked `selected` (filled and glowing): `Let's go!` at first, then whichever
    /// button last had the keyboard.
    selected: Focus,
}

impl Default for State {
    fn default() -> Self {
        Self {
            range: 2,
            keep_unread: false,
            keep_starred: false,
            more: false,
            menu: None,
            focus: Focus::Range,
            selected: Focus::Confirm,
        }
    }
}

/// Something done in the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Open or close the age list.
    ToggleMenu,
    /// Pick this age.
    Choose(usize),
    /// Show the checkboxes.
    MoreOptions,
    /// Toggle `Keep unread emails`.
    KeepUnread,
    /// Toggle `Keep starred emails`.
    KeepStarred,
    /// `Do this later`.
    DoLater,
    /// `Let's go!`.
    Confirm,
    /// Close it (×, the backdrop, Escape with the list closed).
    Close,
    /// Tab (1) and ⇧Tab (-1), or ↓ and ↑ in the open list.
    Move(isize),
    /// Enter or Space on what has the keyboard.
    Activate,
    /// Escape.
    Escape,
}

impl State {
    /// Applies `event`; returns what's left for the app (choices it acts on, or closing).
    pub(crate) fn handle(&mut self, event: Event) -> Option<Event> {
        match event {
            Event::ToggleMenu => {
                self.menu = if self.menu.is_some() {
                    None
                } else {
                    Some(self.range)
                }
            }
            Event::Choose(range) => {
                self.range = range;
                self.menu = None;
            }
            Event::MoreOptions => {
                self.more = true;
                self.focus = Focus::KeepUnread;
            }
            Event::KeepUnread => self.keep_unread = !self.keep_unread,
            Event::KeepStarred => self.keep_starred = !self.keep_starred,
            Event::Move(delta) => match &mut self.menu {
                Some(highlighted) => {
                    *highlighted = highlighted
                        .saturating_add_signed(delta)
                        .min(RANGES.len() - 1)
                }
                None => {
                    let order = self.order();
                    let at = order
                        .iter()
                        .position(|focus| *focus == self.focus)
                        .unwrap_or(0);
                    self.focus =
                        order[(at as isize + delta).rem_euclid(order.len() as isize) as usize];
                    if matches!(self.focus, Focus::DoLater | Focus::Confirm) {
                        self.selected = self.focus;
                    }
                }
            },
            Event::Activate => {
                if let Some(highlighted) = self.menu {
                    return self.handle(Event::Choose(highlighted));
                }
                let event = match self.focus {
                    Focus::Range => Event::ToggleMenu,
                    Focus::MoreOptions => Event::MoreOptions,
                    Focus::KeepUnread => Event::KeepUnread,
                    Focus::KeepStarred => Event::KeepStarred,
                    Focus::DoLater => Event::DoLater,
                    Focus::Confirm => Event::Confirm,
                };
                return self.handle(event);
            }
            Event::Escape if self.menu.is_some() => self.menu = None,
            Event::Escape => return Some(Event::Close),
            Event::DoLater | Event::Confirm | Event::Close => return Some(event),
        }
        None
    }

    /// The focusable controls, in tab order.
    fn order(&self) -> Vec<Focus> {
        let options: &[Focus] = if self.more {
            &[Focus::KeepUnread, Focus::KeepStarred]
        } else {
            &[Focus::MoreOptions]
        };
        [
            &[Focus::Range][..],
            options,
            &[Focus::DoLater, Focus::Confirm],
        ]
        .concat()
    }

    /// The query the bulk action runs, as the Bulk Actions dialog shows it: `before:` as a local
    /// date, `in:inbox` left out.
    pub(crate) fn display_query(&self, before: &Zoned) -> String {
        let mut parts = Vec::new();
        if self.keep_starred {
            parts.push("-is:starred".to_owned());
        }
        if self.keep_unread {
            parts.push("is:read".to_owned());
        }
        parts.push(format!("before:{}", before.strftime("%Y/%m/%d")));
        parts.join(" ")
    }
}

/// Draws the dialog over `area` while `open` (and as it fades), with `state`'s choices.
pub(crate) fn show(
    ctx: &Context,
    area: Rect,
    open: bool,
    state: &State,
) -> DialogResponse<Option<Event>> {
    let extra = if state.more { OPTIONS_EXTRA } else { 0.0 };
    Dialog::new("get-me-to-zero", "Get Me To Zero", BODY_HEIGHT + extra).show(
        ctx,
        area,
        open,
        |ui, body| draw(ui, body, state),
    )
}

fn draw(ui: &mut Ui, body: Rect, state: &State) -> Option<Event> {
    let tokens = palette(ui.ctx());
    let painter = ui.painter().clone();
    let (left, top) = (body.left(), body.top());
    let mut event = None;

    // The subheader: three paragraphs (`rgba(0, 0, 0, .85)`), the lead with a sparkle.
    let line = |y: f32, text: &str| {
        TEXT.paint_line(
            &painter,
            pos2(left, top + y),
            text,
            tokens.ink(0.85, 0.85),
            None,
        )
    };
    line(LEAD_TOP, "Let's get you to Inbox Zero! ✨");
    line(
        MOVE_TOP,
        "We'll move old emails from your Inbox to the Done folder.",
    );
    line(
        MOVE_TOP + 20.0,
        "You can always find them again with search.",
    );
    line(
        OLD_TOP,
        "What counts as old? Most people move emails older than 1 week.",
    );
    line(OLD_TOP + 20.0, "Be bold, and say goodbye to the old!");
    painter.hline(
        body.x_range(),
        top + RULE_TOP + 0.5,
        Stroke::new(1.0, tokens.border_base_default),
    );

    // `Clean up emails older than:` and the age dropdown, both `rgba(0, 0, 0, .7)`
    // (the label and the dropdown's button). The value's focus underline
    // (`box-shadow: 0 2px #aeb1dd`) never shows in Classic: its `padding-top: 3px` puts the value
    // at the bottom of the button, which clips it (`overflow: hidden`).
    let ink = tokens.ink(0.7, 0.7);
    let label = "Clean up emails older than:";
    let label_width = RANGE_LABEL.paint_line(
        &painter,
        pos2(left, top + RANGE_TOP + 5.0),
        label,
        ink,
        None,
    );
    let text_left = left + label_width + LABEL_GAP + 1.0;
    let button = exo_ui::paint_dialog_dropdown(
        ui,
        pos2(text_left, top + RANGE_TOP + 5.0),
        RANGES[state.range].name,
    );
    let response = ui
        .interact(button, exo_ui::id(ui.ctx(), "gmtz-range"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        event = Some(Event::ToggleMenu);
    }
    if let Some(highlighted) = state.menu {
        let names: Vec<&str> = RANGES.iter().map(|range| range.name).collect();
        let list = Menu {
            items: &names,
            highlighted,
        }
        .show_in_dialog(
            ui,
            exo_ui::id(ui.ctx(), "gmtz-range-list"),
            pos2(text_left, button.top()) + MENU_OFFSET,
        );
        if let Some(clicked) = list.clicked {
            event = Some(Event::Choose(clicked));
        } else if let Some(hovered) = list.hovered.filter(|hovered| *hovered != highlighted) {
            event = Some(Event::Move(hovered as isize - highlighted as isize));
        } else if ui.input(|input| input.pointer.any_released())
            && !response.hovered()
            && !ui.rect_contains_pointer(list.rect)
        {
            event = Some(Event::ToggleMenu);
        }
    }

    // `More options…`, or the two checkboxes it shows.
    if state.more {
        let rows = [
            ("Keep unread emails", state.keep_unread, Event::KeepUnread),
            (
                "Keep starred emails",
                state.keep_starred,
                Event::KeepStarred,
            ),
        ];
        for (i, (text, checked, toggle)) in rows.into_iter().enumerate() {
            let row_top = top + RANGE_TOP + RANGE_ROW + OPTION_ROW * i as f32;
            exo_ui::paint_checkbox(
                &painter,
                pos2(left + CHECKBOX_LEFT, row_top + 4.0),
                checked,
                tokens,
            );
            let width = TEXT.paint_line(
                &painter,
                pos2(left + CHECKBOX_COLUMN, row_top + 3.0),
                text,
                ink,
                None,
            );
            let hit = Rect::from_min_size(
                pos2(left, row_top),
                vec2(CHECKBOX_COLUMN + width, OPTION_ROW),
            );
            if ui
                .interact(
                    hit,
                    exo_ui::id(ui.ctx(), ("gmtz-option", i)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                event = Some(toggle);
            }
        }
    } else {
        // `More options…`: `rgba(0, 0, 0, .35)`, `.7` hovered or focused.
        let width = MORE.width(ui.ctx(), "More options…");
        let hit = Rect::from_min_size(pos2(left, top + MORE_TOP), vec2(width, 20.0));
        let response = ui
            .interact(hit, exo_ui::id(ui.ctx(), "gmtz-more"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let more_ink = if response.hovered() || state.focus == Focus::MoreOptions {
            ink
        } else {
            tokens.ink(0.35, 0.35)
        };
        MORE.paint_line(&painter, hit.min, "More options…", more_ink, None);
        if response.clicked() {
            event = Some(Event::MoreOptions);
        }
    }

    // `Do this later` and `Let's go! ✨`, right-aligned; the confirm button is always filled, and
    // the selected one glows (the secondary never fills here).
    let actions_top = top + ACTIONS_TOP + if state.more { OPTIONS_EXTRA } else { 0.0 };
    let confirm = DialogAction::new("Let's go! ✨", ActionKind::Primary)
        .focused(state.selected == Focus::Confirm);
    let later = DialogAction::new("Do this later", ActionKind::Plain)
        .unfilled()
        .focused(state.selected == Focus::DoLater);
    let confirm_rect = Rect::from_min_size(
        pos2(
            body.right() + ACTIONS_OVERHANG - confirm.width(ui.ctx()),
            actions_top,
        ),
        vec2(confirm.width(ui.ctx()), 32.0),
    );
    let later_rect = Rect::from_min_size(
        pos2(
            confirm_rect.left() - ACTION_GAP - later.width(ui.ctx()),
            actions_top,
        ),
        vec2(later.width(ui.ctx()), 32.0),
    );
    if confirm
        .show(ui, exo_ui::id(ui.ctx(), "gmtz-confirm"), confirm_rect)
        .clicked()
    {
        event = Some(Event::Confirm);
    }
    if later
        .show(ui, exo_ui::id(ui.ctx(), "gmtz-later"), later_rect)
        .clicked()
    {
        event = Some(Event::DoLater);
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_count_back_from_midnight() {
        let now: Zoned = "2026-11-17T09:00:00[America/New_York]".parse().unwrap();
        let at = |range: usize| {
            RANGES[range]
                .before(&now)
                .to_zoned(now.time_zone().clone())
                .to_string()
        };
        assert_eq!(at(0), "2026-11-16T00:00:00-05:00[America/New_York]");
        assert_eq!(at(2), "2026-11-10T00:00:00-05:00[America/New_York]");
        assert_eq!(at(4), "2026-10-17T00:00:00-04:00[America/New_York]");
    }

    #[test]
    fn the_query_follows_the_options() {
        let now: Zoned = "2026-11-17T09:00:00[America/New_York]".parse().unwrap();
        let mut state = State::default();
        let before = RANGES[state.range]
            .before(&now)
            .to_zoned(now.time_zone().clone());
        assert_eq!(state.display_query(&before), "before:2026/11/10");
        state.keep_unread = true;
        state.keep_starred = true;
        assert_eq!(
            state.display_query(&before),
            "-is:starred is:read before:2026/11/10"
        );
    }

    #[test]
    fn tab_skips_the_hidden_checkboxes_and_enter_uses_what_has_focus() {
        let mut state = State::default();
        assert_eq!(
            state.handle(Event::Activate),
            None,
            "Enter on the age opens its list"
        );
        assert_eq!(state.menu, Some(2));
        state.handle(Event::Move(1));
        state.handle(Event::Activate);
        assert_eq!((state.range, state.menu), (3, None));
        state.handle(Event::Move(1));
        state.handle(Event::Activate);
        assert!(state.more, "More options…");
        state.handle(Event::Activate);
        assert!(state.keep_unread);
        state.handle(Event::Move(2));
        assert_eq!(state.handle(Event::Activate), Some(Event::DoLater));
        assert_eq!(state.handle(Event::Escape), Some(Event::Close));
    }
}
