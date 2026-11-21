//! The event form: the title over Who, When, (when creating) Repeat, the meeting link, Where,
//! About and (when creating) the calendar, with Save and Cancel below. The week view shows it as
//! a 464px modal beside the event; the right pane shows it over a day of the calendar.
//!
//! The form holds what it edits as [`Values`]; editing an event compares them with the event's
//! own to know what changed.

use std::collections::BTreeMap;

use egui::{Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use exo_store::Address;
use exo_ui::{CalendarColor, Family, Font, KeyTooltip, Menu, TextStyle};
use gmail::calendar::Event as ApiEvent;
use jiff::{ToSpan, Zoned, civil::Weekday};

use super::{
    places::Place,
    repeat::{self, Custom, CustomAction},
    when::{self, When},
    when_edit::{self, WhenEdit},
};
use crate::compose::{contacts::Index, recipients};

/// The week view's modal: 464px wide.
pub const MODAL_WIDTH: f32 = 464.0;
/// The header: `padding: 16px 20px 12px` (32px on top in the right pane), the colour square and a
/// 12px gap before the title.
const PADDING_X: f32 = 20.0;
const HEADER_TOP_MODAL: f32 = 16.0;
const HEADER_TOP_PANE: f32 = 32.0;
const HEADER_BOTTOM: f32 = 12.0;
const SQUARE: f32 = 10.0;
const TITLE_LEFT: f32 = PADDING_X + 3.0 + SQUARE + 3.0 + 12.0;
/// The title: 16px/20px nudged `top: 2px`, 2px of padding over a 2px underline, in a 24px box.
const TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 20.0);
const TITLE_HEIGHT: f32 = 24.0;
/// The fields: `padding-top: 4px`, rows 20px tall 12px apart, 20px under the last in the modal (the
/// right pane's day follows the last directly).
const BODY_TOP: f32 = 4.0;
const ROW: f32 = 20.0;
const ROW_GAP: f32 = 12.0;
const BODY_BOTTOM: f32 = 20.0;
/// A row's 16px icon and its value 12px after it.
const ICON: f32 = 16.0;
const VALUE_LEFT: f32 = PADDING_X + ICON + 12.0;
/// The fields' text: 12px/16px, the input nudged `top: 1px`, an underline 2px under it.
const SMALL: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const UNDERLINE: f32 = 2.0;
/// A guest's line under the input: 24px, 4px apart (`gap: 4px`), the first 7px under the input,
/// 322px at most; a 20px avatar 2px down, then 8px on, inside the content's 1px border and 7px of
/// padding, the swatch and 8px after it the name.
const GUEST: f32 = 24.0;
const GUEST_GAP: f32 = 4.0;
const GUESTS_TOP: f32 = 7.0;
const GUEST_MAX: f32 = 322.0;
const AVATAR: f32 = 20.0;
/// The content's border while hovered (`rgba(0, 0, 0, .09)`, Carbon `#636973`) and while the
/// keyboard is on the guest (`#aeb1dd` in both).
const GUEST_HOVER_CARBON: Color32 = Color32::from_rgb(0x63, 0x69, 0x73);
const GUEST_FOCUS: Color32 = Color32::from_rgb(0xae, 0xb1, 0xdd);
/// The optional toggle and `Remove`: 24px boxes at the content's right, the cross's box over its
/// padding and the toggle's 4px under the cross's (`margin: -4px; margin-right: -8px`); the
/// toggle's 16px icon and the cross's 8px, both `#a3a3a3`.
const GUEST_BUTTON: f32 = 24.0;
const GUEST_ICON: Color32 = Color32::from_rgb(0xa3, 0xa3, 0xa3);
/// A guest's swatch: 8px, 2px corners.
const SWATCH: f32 = 8.0;
/// The right pane lists four guests while the field isn't focused, then `+ {n} more`.
const PANE_GUESTS: usize = 4;
/// About (the compose editor): 12px/16px lines of the email font (`Family::SansEmail`), the font's
/// content area 15px from the line's top (the placeholder, painted here, is 1px lower, at `.35`);
/// the text stops short of the scrollbar's 11px gutter (`scrollbar-gutter: stable`) over
/// `padding-bottom: 2px` and a 2px border; hovered, the box 6px wider each side takes `#f5f7fa` /
/// `#363a40`.
const ABOUT_TEXT: exo_editor::BodyText = exo_editor::BodyText {
    style: TextStyle::new(Font::new(Family::SansEmail, 400), 12.0, 16.0),
    content_top: 0.0,
    content_height: 15.0,
    placeholder_opacity: 0.35 / 0.9,
};
const ABOUT_GUTTER: f32 = 11.0;
const ABOUT_HOVER_OUTSET: f32 = 6.0;
/// Who's suggestions: 12px under the input's box, in the week's modal from 14px left of it and at
/// most 420px wide with 12px of padding, in the right pane across the form less 10px each side;
/// 52px items (name over address, 12px/16px nudged `top: 1px`, between 10px paddings) from 3px down
/// (and nudged `top: 1px`), 6px under the last.
const SUGGESTION: f32 = 10.0 + 16.0 + 16.0 + 10.0;
const SUGGESTIONS_BELOW: f32 = 20.0 + 12.0;
const SUGGESTIONS_TOP: f32 = 3.0 + 1.0;
const SUGGESTIONS_BOTTOM: f32 = 6.0;
/// The calendar's list: 276px wide, at most 40vh tall before it scrolls, 12px under the input's
/// box; in the right pane from 10px inside the form, in the week's modal tethered under the input.
/// Its rows are 12px/16px between 10px paddings (two lines and more for a name that wraps,
/// `overflow-wrap: anywhere`), laid out from 3px down to 6px under the last and nudged `top: 1px`;
/// a calendar's 9px swatch (2px corners, 2px down) at the row's right, 16px clear of the name.
const CALENDAR_LIST_WIDTH: f32 = 276.0;
const CALENDAR_LIST_MAX_HEIGHT: f32 = 0.4;
const CALENDAR_ROW_PADDING: f32 = 10.0;
const CALENDAR_SWATCH: f32 = 9.0;
const CALENDAR_SWATCH_GAP: f32 = 16.0;
/// The footer: a 1px border (the modal's), `padding: 15px 36px 16px` around Classic's 32px
/// dialog-style buttons 12px apart (`top: 2.5px`).
pub const FOOTER_HEIGHT: f32 = 1.0 + 15.0 + 32.0 + 16.0;
/// The right pane's, without the border, over the app's footer.
pub const PANE_FOOTER_HEIGHT: f32 = FOOTER_HEIGHT - 1.0;
const FOOTER_X: f32 = 36.0;
const BUTTON_NUDGE: f32 = 2.5;
const BUTTON_GAP: f32 = 12.0;

/// Creating or editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Create,
    Edit,
}

/// A part of the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    Title,
    Guests,
    When,
    /// A new event's rule.
    Repeat,
    Meeting,
    Location,
    About,
    Calendar,
}

impl Field {
    /// The fields under the title, in the mode's order.
    fn rows(mode: Mode) -> &'static [Field] {
        match mode {
            Mode::Create => &[
                Field::Guests,
                Field::When,
                Field::Repeat,
                Field::Meeting,
                Field::Location,
                Field::About,
                Field::Calendar,
            ],
            Mode::Edit => &[
                Field::When,
                Field::Meeting,
                Field::Location,
                Field::Guests,
                Field::About,
            ],
        }
    }

    /// How far under the row's top its icon starts: its label's nudge (Who's and the meeting
    /// link's `top: 1px`, About's `top: 3px`, the calendar's `top: 2px`), the icon at the label's
    /// top, Meet's 3px lower in its own label.
    fn icon_top(self) -> f32 {
        match self {
            Field::Guests => 1.0,
            Field::Meeting => 1.0 + 3.0,
            Field::About => 3.0,
            Field::Calendar => 2.0,
            Field::Title | Field::When | Field::Repeat | Field::Location => 0.0,
        }
    }

    fn icon(self) -> &'static str {
        match self {
            // Repeat's label is empty (`aria-hidden`), keeping the column.
            Field::Title | Field::Repeat => "",
            Field::Guests => "legacy/DEPRECATED_guest",
            Field::When => "legacy/DEPRECATED_snooze",
            Field::Meeting => "legacy/DEPRECATED_conference",
            Field::Location => "legacy/DEPRECATED_locationPin",
            Field::About => "legacy/DEPRECATED_description",
            Field::Calendar => "legacy/DEPRECATED_calendar",
        }
    }

    fn id(self, ctx: &egui::Context) -> Id {
        exo_ui::id(ctx, ("calendar-form-field", self))
    }

    /// How far the row sits up into the gap above it (Repeat's `margin-top: -8px`, under When).
    fn pulled_up(self) -> f32 {
        if self == Field::Repeat { 8.0 } else { 0.0 }
    }
}

/// A guest on the form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guest {
    pub email: String,
    pub name: Option<String>,
    pub optional: bool,
    /// Their answer, as Calendar has it.
    pub response: Option<String>,
    /// The guest is the event's calendar (me): not shown, kept when saving.
    pub me: bool,
    pub organizer: bool,
}

/// The meeting link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Meeting {
    None,
    /// A Google Meet link, made when the event is saved.
    NewMeet,
    /// The event's own link.
    Existing(String),
}

/// What the form edits.
#[derive(Clone, Debug, PartialEq)]
pub struct Values {
    pub title: String,
    /// Start and end, once picked.
    pub when: Option<(Zoned, Zoned)>,
    pub all_day: bool,
    pub guests: Vec<Guest>,
    pub location: String,
    pub about: String,
    pub meeting: Meeting,
    /// The calendar it goes on.
    pub calendar: String,
    /// A new event's rule: none doesn't repeat.
    pub repeat: Option<exo_store::Recurrence>,
}

/// The event being edited.
#[derive(Clone, Debug)]
pub struct Edited {
    /// Its key on the calendar ([`super::event::Event::key`]).
    pub key: String,
    /// Calendar's copy (an instance, for a recurring event).
    pub event: ApiEvent,
    pub color: CalendarColor,
}

/// A dropdown open under a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dropdown {
    Meeting,
    Repeat,
}

/// The form and its own UI state.
pub struct EventForm {
    pub mode: Mode,
    pub values: Values,
    original: Values,
    pub edited: Option<Edited>,
    /// Shown in the week view (a modal) rather than the right pane.
    pub week: bool,
    guest_input: String,
    guest_error: bool,
    /// The guests' calendars' colours, and the column a hover expanded.
    pub guest_colors: super::guests::Colors,
    /// The highlighted suggestion, or (`chip`) guest.
    suggestion: usize,
    chip: Option<usize>,
    /// The guest's `Name <email>` tooltip (`CopyTooltip`), and what a click on it asked for.
    chip_tip: exo_ui::CopyTooltip,
    pub chip_clicked: Option<ChipTip>,
    dropdown: Option<(Dropdown, usize)>,
    /// A field to focus on the next frame.
    focus: Option<Field>,
    /// The field that had the keyboard last frame, and the last one that had it.
    focused: Option<Field>,
    last_focused: Field,
    /// The right pane's form has given the keyboard to the mail (a click there, ⌘D): its keys
    /// no longer work, and it says how to come back ([`paint_away_hint`]).
    pub away: bool,
    /// When's list has come over the day under the right pane's form.
    when_covered: bool,
    scroll: f32,
    /// The Save button's tooltip says to try again.
    pub failed: bool,
    /// A new event's When, typed.
    pub when: When,
    /// An event's When, edited.
    when_edit: WhenEdit,
    /// About's editor, and its height as last laid out.
    about: exo_editor::Editor,
    about_height: f32,
    /// The title's spelling, checked whether autocorrect is on or not.
    pub title_spelling: exo_editor::FieldSpelling,
    /// A right-click in About, for the app's menu.
    pub about_menu: Option<exo_editor::ContextMenu>,
    /// A word About's autocorrect pill learned, for the app's hint.
    pub about_learned: Option<String>,
    /// Where's highlighted place, and whether Escape closed its list (until the text changes).
    place: Option<usize>,
    places_closed: bool,
    /// What's typed in the calendar's field, which starts empty each time it takes the keyboard;
    /// its list's highlighted row and scroll.
    calendar_input: String,
    calendar_selected: usize,
    calendar_scroll: f32,
    calendar_scrollbar: exo_ui::Scrollbar,
    /// Save takes the keyboard on the next frame (a calendar was picked).
    focus_save: bool,
    /// Repeat's Custom dialog, while it's up.
    pub custom: Option<Custom>,
}

/// The hint while the right pane's form has given the keyboard to the mail:
/// `To instantly move back here, hit` `⌘` `D` 24px under the form (`margin-top: 24px`), its
/// 24px line 2px in from the fields' left and 1px down, at `.35`, the keys in blue caps, fading
/// in over 150ms. Returns the room it takes above the day.
pub fn paint_away_hint(ui: &Ui, form_bottom_left: Pos2, away: bool) -> f32 {
    const HINT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 24.0);
    const MARGIN_TOP: f32 = 24.0;
    let ctx = ui.ctx();
    let fade = exo_ui::motion::transition_in(
        ctx,
        exo_ui::id(ctx, "calendar-form-away-hint"),
        away,
        0.15,
        exo_ui::motion::CubicBezier::EASE,
    );
    if !away {
        return 0.0;
    }
    let mut painter = ui.painter().clone();
    painter.multiply_opacity(fade);
    let palette = exo_ui::palette(ctx);
    let line = form_bottom_left + vec2(PADDING_X + 2.0, MARGIN_TOP + 1.0);
    let text = "To instantly move back here, hit";
    let width = HINT.paint_line(&painter, line, text, palette.ink(0.35, 0.35), None);
    let caps = exo_ui::Keycaps::new(
        ctx,
        &["⌘", "D"],
        exo_ui::KeycapColors::shortcuts_pane(palette),
    );
    caps.paint(&painter, line.x + width, line.y + 2.0);
    MARGIN_TOP + HINT.line_height
}

/// A click on a guest's tooltip: its text composes to them (popped out with ⇧), its icon copies
/// their address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChipTip {
    Compose(Address, bool),
    Copy(String),
}

/// What the form asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormAction {
    Save,
    Cancel,
}

/// A calendar the event can go on.
pub struct CalendarOption {
    /// The calendar account it's listed under.
    pub account: String,
    pub id: String,
    pub name: String,
    pub color: CalendarColor,
}

/// What the form shows besides itself.
pub struct Shown<'a> {
    pub calendars: &'a [CalendarOption],
    pub contacts: Option<&'a Index>,
    /// The account.
    pub me: &'a str,
    pub now: &'a Zoned,
    /// The day weeks start on, for the date pickers.
    pub week_start: Weekday,
    /// The people of the conversation open behind the form, me left out.
    pub conversation: &'a [Address],
    /// The places found for what's typed in Where.
    pub places: &'a [Place],
    /// Each guest's marker and busy status (`(busy)`/`(OOO)`), by address.
    pub chips: &'a BTreeMap<String, (super::guests::Marker, Option<&'static str>)>,
}

/// A suggestion under Who.
#[derive(Clone, Debug)]
enum Suggestion {
    Contact(Address),
    /// `Everybody in this conversation`.
    Everybody(Vec<Address>),
}

impl Suggestion {
    /// The guests it adds.
    fn guests(&self) -> Vec<Guest> {
        let addresses = match self {
            Suggestion::Contact(address) => std::slice::from_ref(address),
            Suggestion::Everybody(addresses) => addresses.as_slice(),
        };
        addresses
            .iter()
            .map(|address| Guest {
                email: address.email.clone(),
                name: address.name.clone(),
                optional: false,
                response: None,
                me: false,
                organizer: false,
            })
            .collect()
    }
}

impl EventForm {
    /// A new event's form, the title focused.
    pub fn create(values: Values, week: bool) -> Self {
        Self::new(
            Mode::Create,
            values.clone(),
            values,
            None,
            week,
            Field::Title,
        )
    }

    /// `event`'s form, `focus` focused: When's start time, as the details' time opens it.
    pub fn edit(values: Values, edited: Edited, week: bool, focus: Field) -> Self {
        let mut form = Self::new(
            Mode::Edit,
            values.clone(),
            values,
            Some(edited),
            week,
            focus,
        );
        if focus == Field::When {
            form.when_edit.focus(when_edit::Part::StartTime);
        }
        form
    }

    fn new(
        mode: Mode,
        mut values: Values,
        mut original: Values,
        edited: Option<Edited>,
        week: bool,
        focus: Field,
    ) -> Self {
        // About holds the editor's HTML, so an untouched one isn't a change.
        let mut about = if values.about.is_empty() {
            exo_editor::Editor::new()
        } else {
            exo_editor::Editor::from_html(&values.about)
        };
        about.set_body_text(ABOUT_TEXT);
        // About doesn't detect dates: `see you tomorrow` gets no `sh-date`.
        about.set_settings(exo_editor::Settings {
            skip_dates: true,
            ..exo_editor::Settings::default()
        });
        values.about = about_html(&about);
        original.about = values.about.clone();
        Self {
            mode,
            values,
            original,
            edited,
            week,
            guest_input: String::new(),
            guest_error: false,
            guest_colors: super::guests::Colors::default(),
            suggestion: 0,
            chip: None,
            chip_tip: exo_ui::CopyTooltip::default(),
            chip_clicked: None,
            dropdown: None,
            focus: Some(focus),
            focused: None,
            last_focused: focus,
            away: false,
            when_covered: false,
            scroll: 0.0,
            failed: false,
            when: When::default(),
            when_edit: WhenEdit::default(),
            about,
            about_height: ABOUT_TEXT.style.line_height,
            title_spelling: exo_editor::FieldSpelling::default(),
            about_menu: None,
            about_learned: None,
            place: None,
            places_closed: false,
            calendar_input: String::new(),
            calendar_selected: 0,
            calendar_scroll: 0.0,
            calendar_scrollbar: exo_ui::Scrollbar::default(),
            focus_save: false,
            custom: None,
        }
    }

    /// The fields that differ from the event's.
    pub fn dirty(&self) -> Vec<Field> {
        let (now, then) = (&self.values, &self.original);
        let mut dirty = Vec::new();
        let mut differs = |field: Field, changed: bool| {
            if changed {
                dirty.push(field);
            }
        };
        differs(Field::Title, now.title != then.title);
        differs(
            Field::When,
            now.when != then.when || now.all_day != then.all_day,
        );
        differs(Field::Guests, now.guests != then.guests);
        differs(Field::Location, now.location != then.location);
        differs(Field::About, now.about != then.about);
        differs(Field::Meeting, now.meeting != then.meeting);
        differs(Field::Calendar, now.calendar != then.calendar);
        differs(Field::Repeat, now.repeat != then.repeat);
        dirty
    }

    /// Nothing worth asking about before it's thrown away: an edit with no changes, or a new event
    /// without a title, guests, place or notes.
    pub fn is_empty(&self) -> bool {
        match self.mode {
            Mode::Edit => self.dirty().is_empty(),
            Mode::Create => {
                self.values.title.trim().is_empty()
                    && self.values.guests.is_empty()
                    && self.values.location.trim().is_empty()
                    && self.values.about.trim().is_empty()
            }
        }
    }

    /// Moves the keyboard to `field` (the palette's `Edit …` rows, a row of the details); an
    /// edited When to its start date, its first input.
    /// Whether the day under the right pane's form makes room for When's list: once the list has
    /// come within 10px of the day's top (`day_top`) while something's typed, as long as When has
    /// the keyboard and isn't emptied.
    pub fn when_covers(&mut self, day_top: f32) -> bool {
        let typing = self.focused == Some(Field::When) && !self.when.text.is_empty();
        if !typing {
            self.when_covered = false;
        } else if !self.when_covered {
            self.when_covered = self
                .when
                .list_bottom
                .is_some_and(|bottom| bottom + 10.0 > day_top);
        }
        self.when_covered
    }

    /// Gives the keyboard to the mail: the field that had it lets go.
    pub fn give_up_keyboard(&mut self, ctx: &egui::Context) {
        self.away = true;
        if self.focused.is_some() {
            ctx.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
    }

    /// Takes the keyboard back: to the field that had it last.
    pub fn take_keyboard(&mut self) {
        self.focus(self.last_focused);
    }

    /// Puts the keyboard on `field`, the form's again if it had given it up.
    pub fn focus(&mut self, field: Field) {
        self.away = false;
        self.focus = Some(field);
        if field == Field::When && self.mode == Mode::Edit {
            self.when_edit.focus(when_edit::Part::StartDate);
        }
    }

    /// Whether a dropdown or suggestions are open, or When or the calendar's field holds text,
    /// which Escape takes first.
    pub fn has_dropdown(&self) -> bool {
        self.dropdown.is_some()
            || (!self.guest_input.trim().is_empty() && self.focused == Some(Field::Guests))
            || (self.typing_when() && !self.when.text.is_empty())
            || (!self.calendar_input.is_empty() && self.focused == Some(Field::Calendar))
            || (self.mode == Mode::Edit && self.when_edit.has_dropdown())
            || self.location_query().is_some()
    }

    /// What Where asks places for while it has the keyboard and its list isn't closed.
    pub fn location_query(&self) -> Option<&str> {
        let text = self.values.location.trim();
        (self.focused == Some(Field::Location) && !self.places_closed && !text.is_empty())
            .then_some(text)
    }

    /// Escape: closes the dropdown or the suggestions; in When and the calendar's field, clears
    /// the text.
    pub fn close_dropdown(&mut self) {
        if self.typing_when() && !self.when.text.is_empty() {
            self.when.clear();
            self.when_edited();
            return;
        }
        if self.focused == Some(Field::Calendar) && !self.calendar_input.is_empty() {
            // The field keeps the keyboard, which egui's Escape takes.
            self.calendar_input.clear();
            self.focus = Some(Field::Calendar);
            return;
        }
        self.dropdown = None;
        self.guest_input.clear();
        self.when_edit.close_dropdown();
        self.places_closed = true;
        self.place = None;
    }

    /// A new event's When has the keyboard.
    fn typing_when(&self) -> bool {
        self.mode == Mode::Create && self.focused == Some(Field::When)
    }

    /// When's text changed: in the right pane the draft's time goes until a suggestion is taken;
    /// the week view keeps it.
    fn when_edited(&mut self) {
        if !self.week {
            self.values.when = None;
            self.when.zone = None;
            self.when.mark_written(None, self.values.all_day);
        }
    }

    /// The suggestion the right pane's day previews: the highlighted one while When lists some.
    pub fn when_preview(&self) -> Option<&when::Choice> {
        if self.week || !self.typing_when() {
            return None;
        }
        self.when.choices().get(self.when.selected)
    }

    /// Takes When's suggestion `index` and moves to the next field.
    fn commit_when(&mut self, index: usize, now: &Zoned) {
        if let Some(choice) = self.when.commit(index, self.values.all_day, now) {
            self.values.when = Some((choice.start, choice.end));
            self.values.all_day = choice.all_day;
            self.focus = self.next(Field::When);
        }
    }

    /// The draft's times as the calendar shows them, and the time its block shows when it was
    /// typed in another zone: in the right pane, the highlighted suggestion while When lists
    /// some.
    pub fn draft_when(&self, now: &Zoned) -> Option<(Zoned, Zoned, Option<String>)> {
        if let Some(choice) = self.when_preview() {
            return Some((
                choice.start.clone(),
                choice.end.clone(),
                choice
                    .zone
                    .as_ref()
                    .map(|zone| when::zone_block_time(zone, now)),
            ));
        }
        let (start, end) = self.values.when.clone()?;
        let tz = now.time_zone();
        // An edited event's times put in another zone are labelled in it.
        let zone = match self.mode {
            Mode::Create => self.when.zone.clone(),
            Mode::Edit => (start.time_zone() != tz && !self.values.all_day).then(|| {
                crate::time_suggest::ZoneTime {
                    label: String::new(),
                    tz: Some(start.time_zone().clone()),
                    start: start
                        .datetime()
                        .to_zoned(tz.clone())
                        .map_or(start.timestamp(), |at| at.timestamp()),
                    end: end
                        .datetime()
                        .to_zoned(tz.clone())
                        .map_or(end.timestamp(), |at| at.timestamp()),
                }
            }),
        };
        Some((
            start.with_time_zone(tz.clone()),
            end.with_time_zone(tz.clone()),
            zone.as_ref().map(|zone| when::zone_block_time(zone, now)),
        ))
    }

    /// What's typed in Who and not yet a guest, as guests, and whether it all reads as
    /// addresses.
    pub fn typed_guests(&self) -> (Vec<Guest>, bool) {
        let typed: Vec<Address> = recipients::split_pasted(self.guest_input.trim())
            .iter()
            .map(|text| recipients::parse(text))
            .collect();
        let valid = typed
            .iter()
            .all(|address| recipients::is_valid(&address.email));
        (
            typed
                .into_iter()
                .map(|address| Guest {
                    email: address.email,
                    name: address.name,
                    optional: false,
                    response: None,
                    me: false,
                    organizer: false,
                })
                .collect(),
            valid,
        )
    }

    /// Takes what's typed in Who as guests, if it's valid; marks the field otherwise. Returns
    /// whether it was.
    pub fn commit_guests(&mut self) -> bool {
        if self.guest_input.trim().is_empty() {
            return true;
        }
        let (typed, valid) = self.typed_guests();
        if !valid || self.values.guests.len() >= 200 {
            self.guest_error = true;
            return false;
        }
        for guest in typed {
            if !self
                .values
                .guests
                .iter()
                .any(|known| known.email.eq_ignore_ascii_case(&guest.email))
            {
                self.values.guests.push(guest);
            }
        }
        self.guest_input.clear();
        self.guest_error = false;
        true
    }

    /// Marks Who as holding a bad address and focuses it.
    pub fn mark_guests_invalid(&mut self) {
        self.guest_error = true;
        self.focus = Some(Field::Guests);
    }

    /// The form's height: the header and the rows, without the footer.
    pub fn content_height(&self, pane: bool) -> f32 {
        let header = if pane {
            HEADER_TOP_PANE
        } else {
            HEADER_TOP_MODAL
        } + TITLE_HEIGHT
            + HEADER_BOTTOM;
        let rows: f32 = Field::rows(self.mode)
            .iter()
            .map(|field| self.row_height(*field, pane) - field.pulled_up())
            .sum::<f32>()
            + ROW_GAP * (Field::rows(self.mode).len() - 1) as f32;
        header + BODY_TOP + rows + if pane { 0.0 } else { BODY_BOTTOM }
    }

    fn row_height(&self, field: Field, pane: bool) -> f32 {
        match field {
            Field::Guests => {
                let lines =
                    self.shown_guests(pane).len() + usize::from(self.hidden_guests(pane) > 0);
                ROW + if lines == 0 {
                    0.0
                } else {
                    GUESTS_TOP + lines as f32 * (GUEST + GUEST_GAP) - GUEST_GAP
                }
            }
            Field::When
                if self.mode == Mode::Create
                    && self.when.zone.is_some()
                    && self.values.when.is_some() =>
            {
                ROW + when::ZONE_LINE
            }
            Field::When if self.mode == Mode::Edit && self.values.when.is_some() => {
                when_edit::HEIGHT
            }
            // The editor's lines, its padding and border (as last laid out).
            Field::About => self.about_height + 2.0 + UNDERLINE,
            _ => ROW,
        }
    }

    fn visible_guests(&self) -> Vec<(usize, &Guest)> {
        self.values
            .guests
            .iter()
            .enumerate()
            .filter(|(_, guest)| !guest.me)
            .collect()
    }

    fn shown_guests(&self, pane: bool) -> Vec<(usize, &Guest)> {
        let mut visible = self.visible_guests();
        if pane && self.focused != Some(Field::Guests) {
            visible.truncate(PANE_GUESTS);
        }
        visible
    }

    fn hidden_guests(&self, pane: bool) -> usize {
        self.visible_guests().len() - self.shown_guests(pane).len()
    }

    /// Paints the form's header and rows in `rect` (`pane` in the right pane).
    pub fn show(&mut self, ui: &mut Ui, rect: Rect, shown: &Shown<'_>, pane: bool) {
        self.when.list_bottom = None;
        let ctx = ui.ctx().clone();
        let palette = exo_ui::palette(&ctx);
        // A new area's first pass sizes it with its widgets disabled, which drops their focus.
        let focus = if ui.is_sizing_pass() || !ui.is_enabled() {
            None
        } else {
            self.focus.take()
        };
        let mut focused = None;

        // Header: the title, after the colour square's room (the square itself stays empty: no
        // way of opening the form gives it a colour).
        let top = rect.top()
            + if pane {
                HEADER_TOP_PANE
            } else {
                HEADER_TOP_MODAL
            };
        let title_line = Rect::from_min_max(
            pos2(rect.left() + TITLE_LEFT, top + 2.0),
            pos2(rect.right() - PADDING_X, top + 2.0 + TITLE.line_height),
        );
        let title = input(
            ui,
            Field::Title.id(ui.ctx()),
            &mut self.values.title,
            title_line,
            &TITLE,
            "Event title",
            false,
            focus == Some(Field::Title),
            Some(&mut self.title_spelling),
        );
        if title.has_focus() {
            focused = Some(Field::Title);
        }
        if title.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            self.focus = Some(Field::rows(self.mode)[0]);
        }
        let header_bottom = top + TITLE_HEIGHT + HEADER_BOTTOM;

        // The rows, scrolling under the header when they don't fit.
        let body = Rect::from_min_max(pos2(rect.left(), header_bottom), rect.max);
        let content: f32 = self.content_height(pane) - (header_bottom - rect.top());
        if ui.rect_contains_pointer(body) {
            self.scroll -= ui.input(|input| input.smooth_scroll_delta.y);
        }
        self.scroll = self.scroll.clamp(0.0, (content - body.height()).max(0.0));
        let mut y = body.top() + BODY_TOP - self.scroll;
        let mut overlay = None;
        // A focused input's underline hangs 1px under its row (the box starts 1px down), which
        // the right pane's last row ends the form on.
        let clip = ui
            .clip_rect()
            .intersect(body.with_max_y(body.bottom() + 1.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        child.set_clip_rect(clip);
        for &field in Field::rows(self.mode) {
            y -= field.pulled_up();
            let height = self.row_height(field, pane);
            let row = Rect::from_min_max(
                pos2(rect.left() + PADDING_X, y),
                pos2(rect.right() - PADDING_X, y + height),
            );
            // The row's icon: `rgba(0,0,0,.35)` / `hsla(0,0%,100%,.35)`.
            let label = Rect::from_min_size(row.min, Vec2::splat(ICON));
            let icon = exo_ui::icon_rect(child.ctx(), field.icon(), label);
            let icon = icon.translate(vec2(0.0, row.top() + field.icon_top() - icon.top()));
            exo_ui::paint_icon(&child, field.icon(), icon, palette.ink(0.35, 0.35));
            let value = Rect::from_min_max(
                pos2(rect.left() + VALUE_LEFT, y + 1.0),
                pos2(row.right(), y + 1.0 + SMALL.line_height),
            );
            let has_focus = match field {
                Field::Guests => self.guests_row(
                    &mut child,
                    value,
                    shown,
                    pane,
                    focus == Some(field),
                    &mut overlay,
                    rect,
                ),
                Field::When => self.when_row(
                    &mut child,
                    value,
                    shown,
                    focus == Some(field),
                    &mut overlay,
                    rect,
                ),
                Field::Repeat => {
                    self.repeat_row(&mut child, value, shown, focus == Some(field), &mut overlay)
                }
                Field::Meeting => {
                    self.meeting_row(&mut child, value, focus == Some(field), &mut overlay)
                }
                Field::Location => self.location_row(
                    &mut child,
                    value,
                    shown,
                    focus == Some(field),
                    &mut overlay,
                    rect,
                    pane,
                ),
                Field::About => self.about_row(
                    &mut child,
                    Rect::from_min_max(row.min + vec2(VALUE_LEFT - PADDING_X, 0.0), row.max),
                    focus == Some(field),
                ),
                Field::Calendar => self.calendar_row(
                    &mut child,
                    value,
                    shown,
                    focus == Some(field),
                    &mut overlay,
                    rect,
                    pane,
                ),
                Field::Title => false,
            };
            if has_focus {
                focused = Some(field);
            }
            y += height + ROW_GAP;
        }
        if self.focused == Some(Field::Guests) && focused != Some(Field::Guests) {
            // Leaving Who takes what's typed.
            self.commit_guests();
            self.chip = None;
        }
        self.focused = focused;
        if let Some(field) = focused {
            self.last_focused = field;
        }
        if let Some(paint) = overlay {
            self.paint_overlay(ui, paint, shown);
        }
    }

    /// Paints Save and Cancel in `footer`, its top border unless in the right pane; Save glows
    /// while it has the keyboard (`:focus`). Returns what was clicked.
    pub fn footer(&mut self, ui: &Ui, footer: Rect, pane: bool) -> Option<FormAction> {
        let ctx = ui.ctx().clone();
        let palette = exo_ui::palette(&ctx);
        if !pane {
            ui.painter().hline(
                footer.x_range(),
                footer.top() + 0.5,
                Stroke::new(1.0, palette.border_base_default),
            );
        }
        let top = footer.top() + if pane { 0.0 } else { 1.0 } + 15.0 + BUTTON_NUDGE;
        let enabled = self.mode == Mode::Create || !self.dirty().is_empty();
        let carbon = palette.theme == exo_ui::Theme::Carbon;
        let mut x = footer.left() + FOOTER_X;
        let mut clicked = None;
        for (label, action) in [("Save", FormAction::Save), ("Cancel", FormAction::Cancel)] {
            let save = action == FormAction::Save;
            let id = exo_ui::id(ui.ctx(), ("calendar-form-button", label));
            // Save and Cancel: Classic's uppercase dialog buttons; a disabled Save is unfilled in
            // Carbon too (`background: none`).
            let button = exo_ui::DialogAction::new(
                label,
                if save {
                    exo_ui::ActionKind::Primary
                } else {
                    exo_ui::ActionKind::Plain
                },
            )
            .enabled(!save || enabled)
            .focused(save && enabled && ctx.memory(|memory| memory.has_focus(id)))
            .disabled_fill(Color32::TRANSPARENT)
            .dark(carbon);
            let rect = Rect::from_min_size(
                pos2(x, top),
                vec2(button.width(&ctx), exo_ui::DIALOG_ACTION_HEIGHT),
            );
            x = rect.right() + BUTTON_GAP;
            let response = button.show(ui, id, rect);
            // Once the pointer is up: a click elsewhere takes the keyboard from what has it.
            if save && self.focus_save && !ui.input(|input| input.pointer.any_down()) {
                self.focus_save = false;
                response.request_focus();
            }
            let (tip, keys): (&str, &[&str]) = match (action, self.mode) {
                (FormAction::Save, _) if self.failed => ("Please try to save again.", &[]),
                (FormAction::Save, _) => ("Save Event", &["⌘", "enter"]),
                (FormAction::Cancel, Mode::Create) => ("Cancel Event", &["⌘", "shift", ","]),
                (FormAction::Cancel, Mode::Edit) => ("Discard changes", &["⌘", "shift", ","]),
            };
            // Shown while the button has the keyboard; 16px over the button.
            KeyTooltip { text: tip, keys }.show_at_focused(
                ui,
                &response,
                exo_ui::Placement::Above(-16.0),
                response.has_focus(),
                None,
            );
            if response.clicked() {
                clicked = Some(action);
            }
        }
        clicked
    }

    /// Who: the input, the guests under it, suggestions while typing. Returns whether it has the
    /// keyboard.
    #[allow(clippy::too_many_arguments)] // The row's place, what it shows and what it opens over the `form`.
    fn guests_row(
        &mut self,
        ui: &mut Ui,
        value: Rect,
        shown: &Shown<'_>,
        pane: bool,
        focus: bool,
        overlay: &mut Option<Overlay>,
        form: Rect,
    ) -> bool {
        let palette = exo_ui::palette(ui.ctx());
        let id = Field::Guests.id(ui.ctx());
        let (up, down, enter, tab, backspace) = ui.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Tab),
                input.key_pressed(egui::Key::Backspace),
            )
        });
        let had_focus = ui.memory(|memory| memory.has_focus(id));
        let before = self.guest_input.clone();
        let response = input(
            ui,
            id,
            &mut self.guest_input,
            value,
            &SMALL,
            "Enter name or email",
            self.guest_error,
            focus,
            None,
        );
        if self.guest_input != before {
            self.guest_error = false;
            self.suggestion = 0;
            self.chip = None;
        }
        let suggestions: Vec<Suggestion> = match shown.contacts {
            Some(book) if !self.guest_input.trim().is_empty() => {
                let limit = if self.mode == Mode::Edit { 2 } else { 3 };
                book.top(self.guest_input.trim(), limit + self.values.guests.len())
                    .into_iter()
                    .filter(|contact| {
                        !contact.email.eq_ignore_ascii_case(shown.me)
                            && !self
                                .values
                                .guests
                                .iter()
                                .any(|guest| guest.email.eq_ignore_ascii_case(&contact.email))
                    })
                    .take(limit)
                    .cloned()
                    .map(Suggestion::Contact)
                    .collect()
            }
            // An empty field of a new event's first suggests the conversation's people, up to
            // 200.
            _ if self.guest_input.is_empty()
                && self.values.guests.is_empty()
                && self.mode == Mode::Create
                && (1..=200).contains(&shown.conversation.len()) =>
            {
                vec![Suggestion::Everybody(shown.conversation.to_vec())]
            }
            _ => Vec::new(),
        };
        let guests: Vec<usize> = self
            .visible_guests()
            .iter()
            .map(|(index, _)| *index)
            .collect();
        if had_focus {
            if let Some(chip) = self.chip {
                // ↑/↓ between the guests and the input; Backspace removes, Enter edits.
                if up {
                    self.chip = Some(chip.saturating_sub(1));
                } else if down {
                    self.chip = (chip + 1 < guests.len()).then_some(chip + 1);
                } else if backspace && self.guest_input.is_empty() {
                    if let Some(&index) = guests.get(chip) {
                        self.values.guests.remove(index);
                    }
                    self.chip = None;
                } else if enter && let Some(&index) = guests.get(chip) {
                    let guest = self.values.guests.remove(index);
                    self.guest_input = guest.email;
                    self.chip = None;
                }
            } else if !suggestions.is_empty() {
                if up {
                    self.suggestion = self.suggestion.saturating_sub(1);
                } else if down {
                    self.suggestion = (self.suggestion + 1).min(suggestions.len() - 1);
                } else if enter || tab {
                    let pick = &suggestions[self.suggestion.min(suggestions.len() - 1)];
                    self.values.guests.extend(pick.guests());
                    self.guest_input.clear();
                    self.focus = Some(Field::Guests);
                }
            } else if up && self.guest_input.is_empty() && !guests.is_empty() {
                self.chip = Some(guests.len() - 1);
            } else if enter || tab {
                if self.guest_input.trim().is_empty() {
                    // An empty submit moves on.
                    if enter {
                        self.focus = self.next(Field::Guests);
                    }
                } else if self.commit_guests() {
                    self.focus = Some(Field::Guests);
                }
            }
        }
        if response.has_focus() && !suggestions.is_empty() && self.chip.is_none() {
            *overlay = Some(Overlay::Suggestions {
                below: value,
                items: suggestions,
                form,
                pane,
            });
        }

        let shown_guests: Vec<(usize, Guest)> = self
            .shown_guests(pane)
            .into_iter()
            .map(|(index, guest)| (index, guest.clone()))
            .collect();
        // From the input's 20px box (`value` is its 16px line).
        let mut top = value.top() + ROW + GUESTS_TOP;
        let mut remove = None;
        let mut toggle = None;
        let mut tip = None;
        let carbon = exo_ui::theme(ui.ctx()) == exo_ui::Theme::Carbon;
        for (position, (index, guest)) in shown_guests.iter().enumerate() {
            let line = Rect::from_min_size(
                pos2(value.left(), top),
                vec2(value.width().min(GUEST_MAX), GUEST),
            );
            let hit = ui.interact(
                line,
                exo_ui::id(ui.ctx(), ("calendar-form-guest", index)),
                Sense::click(),
            );
            let hovered = ui.rect_contains_pointer(line);
            let highlighted = self.chip == Some(position) && response.has_focus();
            let content =
                Rect::from_min_max(pos2(line.left() + AVATAR + 8.0, line.top()), line.max);
            let border = if highlighted {
                Some(GUEST_FOCUS)
            } else if ui.rect_contains_pointer(content) {
                Some(if carbon {
                    GUEST_HOVER_CARBON
                } else {
                    Color32::from_black_alpha(23)
                })
            } else {
                None
            };
            if let Some(border) = border {
                ui.painter().rect_stroke(
                    exo_ui::pixel_snap(content),
                    CornerRadius::same(4),
                    Stroke::new(1.0, border),
                    egui::StrokeKind::Inside,
                );
            }
            // A click puts the keyboard on the guest.
            if hit.clicked() {
                self.chip = Some(position);
                ui.memory_mut(|memory| memory.request_focus(id));
            }
            let avatar =
                Rect::from_min_size(pos2(line.left(), line.top() + 2.0), Vec2::splat(AVATAR));
            ui.painter()
                .circle_filled(avatar.center(), AVATAR / 2.0, palette.icon_base_subdued);
            let initial = guest
                .name
                .as_deref()
                .unwrap_or(&guest.email)
                .chars()
                .next()
                .map(|first| first.to_uppercase().to_string())
                .unwrap_or_default();
            let width = SMALL.width(ui.ctx(), &initial);
            SMALL.paint_line(
                ui.painter(),
                pos2(avatar.center().x - width / 2.0, avatar.top() + 2.0),
                &initial,
                palette.icon_base_inverse,
                None,
            );
            let text = guest.name.clone().unwrap_or_else(|| guest.email.clone());
            // Their calendar's swatch, or its spinner, inside the content's border and padding.
            let swatch_at = content.left() + 8.0;
            let (marker, busy) = shown
                .chips
                .get(&guest.email)
                .copied()
                .unwrap_or((super::guests::Marker::Empty, None));
            super::guests::paint_marker(
                ui,
                Rect::from_min_size(pos2(swatch_at, line.center().y - 4.0), Vec2::splat(SWATCH)),
                marker,
            );
            let name_at = pos2(swatch_at + SWATCH + 8.0, line.top() + 5.0);
            let toggle_box = Rect::from_min_size(
                pos2(content.right() - 2.0 * GUEST_BUTTON + 4.0, content.top()),
                Vec2::splat(GUEST_BUTTON),
            );
            let cross_box = Rect::from_min_size(
                pos2(content.right() - GUEST_BUTTON, content.top()),
                Vec2::splat(GUEST_BUTTON),
            );
            // `(optional)`, else `(busy)` / `(OOO)`, at `.35` 4px after the name, which gives way
            // to it (`flex-shrink: 0`) and is cut before the toggle.
            let note = if guest.optional {
                Some("(optional)")
            } else {
                busy
            };
            let note_width = note.map_or(0.0, |note| 4.0 + SMALL.width(ui.ctx(), note));
            let used = SMALL.paint_line(
                ui.painter(),
                name_at,
                &text,
                palette.text_base_default,
                Some((toggle_box.left() - name_at.x - note_width).max(0.0)),
            );
            if let Some(note) = note {
                SMALL.paint_line(
                    ui.painter(),
                    name_at + vec2(used + 4.0, 0.0),
                    note,
                    palette.text_base_subdued,
                    None,
                );
            }
            // The swatch, name and note carry `Name <email>`'s tooltip.
            let target = Rect::from_min_max(
                pos2(swatch_at, line.top() + 4.0),
                pos2(
                    name_at.x + used + note_width,
                    line.top() + 4.0 + SMALL.line_height,
                ),
            );
            if ui.rect_contains_pointer(target) {
                let shown = guest.name.as_ref().map_or_else(
                    || guest.email.clone(),
                    |name| format!("{name} <{}>", guest.email),
                );
                tip = Some(exo_ui::CopyTip {
                    target,
                    shown,
                    copied: guest.email.clone(),
                });
            }
            if hovered {
                // Required guests show the filled user, optional ones the outline.
                let (icon, label) = if guest.optional {
                    ("origin/User", "Mark Required")
                } else {
                    ("origin/UserFill", "Mark Optional")
                };
                let marked = exo_ui::IconButton::new(icon)
                    .colors(GUEST_ICON, GUEST_ICON)
                    .show(
                        ui,
                        exo_ui::id(ui.ctx(), ("calendar-form-guest-optional", index)),
                        toggle_box,
                        Rect::from_center_size(toggle_box.center(), Vec2::splat(16.0)),
                    );
                KeyTooltip {
                    text: label,
                    keys: &[],
                }
                .show_below(ui, &marked, 8.0);
                if marked.clicked() {
                    toggle = Some(*index);
                }
                // The guest's cross.
                let removed = exo_ui::IconButton::new("legacy/DEPRECATED_closeV2")
                    .colors(GUEST_ICON, GUEST_ICON)
                    .show(
                        ui,
                        exo_ui::id(ui.ctx(), ("calendar-form-guest-remove", index)),
                        cross_box,
                        Rect::from_center_size(cross_box.center(), Vec2::splat(8.0)),
                    );
                KeyTooltip {
                    text: "Remove",
                    keys: &[],
                }
                .show_below(ui, &removed, 8.0);
                if removed.clicked() {
                    remove = Some(*index);
                }
            }
            top += GUEST + GUEST_GAP;
        }
        if let Some((click, tip)) =
            self.chip_tip
                .show(ui, exo_ui::id(ui.ctx(), "calendar-form-guest-tip"), tip)
        {
            self.chip_clicked = Some(match click {
                exo_ui::CopyClick::Text => {
                    let guest = shown_guests
                        .iter()
                        .find(|(_, guest)| guest.email == tip.copied)
                        .map(|(_, guest)| guest);
                    ChipTip::Compose(
                        Address {
                            name: guest.and_then(|guest| guest.name.clone()),
                            email: tip.copied,
                        },
                        ui.input(|input| input.modifiers.shift_only()),
                    )
                }
                exo_ui::CopyClick::Icon => ChipTip::Copy(tip.copied),
            });
        }
        let hidden = self.hidden_guests(pane);
        if hidden > 0 {
            SMALL.paint_line(
                ui.painter(),
                pos2(value.left(), top + 4.0),
                &format!("+ {hidden} more"),
                palette.text_base_subdued,
                None,
            );
        }
        if let Some(index) = remove {
            self.values.guests.remove(index);
        }
        if let Some(index) = toggle {
            self.values.guests[index].optional = !self.values.guests[index].optional;
        }
        response.has_focus()
    }

    /// When: a new event's typed time with its suggestions, listed across the `form`; editing an
    /// event, its dates, times and zone ([`WhenEdit`]).
    fn when_row(
        &mut self,
        ui: &mut Ui,
        value: Rect,
        shown: &Shown<'_>,
        focus: bool,
        overlay: &mut Option<Overlay>,
        form: Rect,
    ) -> bool {
        if self.mode == Mode::Edit {
            return self.edited_when(ui, value, shown);
        }
        let palette = exo_ui::palette(ui.ctx());
        let id = Field::When.id(ui.ctx());
        if focus {
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        self.when
            .sync(self.values.when.as_ref(), self.values.all_day, shown.now);
        let had_focus = ui.memory(|memory| memory.has_focus(id));
        let (up, down, enter, tab) = ui.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Tab),
            )
        });
        let before = self.when.text.clone();
        let response = input(
            ui,
            id,
            &mut self.when.text,
            value,
            &SMALL,
            "tomorrow 2 pm 30 min new york",
            self.when.error,
            false,
            None,
        );
        if response.gained_focus() {
            self.when.focused(self.values.when.is_some(), shown.now);
        }
        if self.when.text != before {
            self.when.edited(shown.now);
            self.when_edited();
        }
        if had_focus && !self.when.choices().is_empty() {
            if up {
                self.when.step(-1);
            } else if down {
                self.when.step(1);
            } else if enter || tab {
                self.commit_when(self.when.selected, shown.now);
            }
        } else if had_focus && enter {
            // Nothing to take: the field keeps the keyboard, which the input gave up.
            self.focus = Some(Field::When);
        }
        if let Some(line) =
            self.when
                .local_line(self.values.when.as_ref(), self.values.all_day, shown.now)
        {
            // The time here, under the field, in Classic's .35.
            SMALL.paint_line(
                ui.painter(),
                pos2(value.left(), value.top() + when::ZONE_LINE_TOP),
                &line,
                palette.ink(0.35, 0.35),
                Some(value.width()),
            );
        }
        if response.has_focus() && !self.when.choices().is_empty() {
            *overlay = Some(Overlay::When { below: value, form });
        }
        response.has_focus()
    }

    /// An edited event's When, its changes taken.
    fn edited_when(&mut self, ui: &mut Ui, value: Rect, shown: &Shown<'_>) -> bool {
        let Some((start, end)) = self.values.when.clone() else {
            return false;
        };
        let edit = when_edit::Shown {
            start: &start,
            end: &end,
            all_day: self.values.all_day,
            today: shown.now.date(),
            week_start: shown.week_start,
        };
        let (changes, focused) = self.when_edit.show(ui, value.min, value.width(), &edit);
        for change in changes {
            match change {
                when_edit::Change::Times(start, end) => self.values.when = Some((start, end)),
                when_edit::Change::AllDay => self.toggle_all_day(),
                // A new zone: the same clock times in it.
                when_edit::Change::Zone(tz) => {
                    if let Some((start, end)) = &self.values.when
                        && let (Ok(start), Ok(end)) = (
                            start.datetime().to_zoned(tz.clone()),
                            end.datetime().to_zoned(tz),
                        )
                    {
                        self.values.when = Some((start, end));
                    }
                }
            }
        }
        focused
    }

    /// Makes the event all day, over the start's days, or takes it back to a time: half an hour
    /// from 9:00 on its start's day.
    pub fn toggle_all_day(&mut self) {
        let Some((start, end)) = self.values.when.clone() else {
            return;
        };
        self.values.all_day = !self.values.all_day;
        let tz = start.time_zone().clone();
        self.values.when = if self.values.all_day {
            let first = start.date().to_zoned(tz.clone()).ok();
            let last = end
                .checked_sub(1.nanosecond())
                .ok()
                .map(|end| end.date())
                .unwrap_or(start.date());
            let after = last.tomorrow().ok().and_then(|day| day.to_zoned(tz).ok());
            first.zip(after).or(Some((start, end)))
        } else {
            let from = start.date().at(9, 0, 0, 0).to_zoned(tz).ok();
            from.and_then(|from| from.checked_add(30.minutes()).ok().map(|to| (from, to)))
                .or(Some((start, end)))
        };
    }

    /// Where: the input, and the places found for it while it has the keyboard. ↓ and ↑ move
    /// through them, Enter takes the highlighted one, else moves on.
    #[allow(clippy::too_many_arguments)] // The row's place, what it shows and what it opens over the `form`.
    fn location_row(
        &mut self,
        ui: &mut Ui,
        value: Rect,
        shown: &Shown<'_>,
        focus: bool,
        overlay: &mut Option<Overlay>,
        form: Rect,
        pane: bool,
    ) -> bool {
        let id = Field::Location.id(ui.ctx());
        let had_focus = ui.memory(|memory| memory.has_focus(id));
        let (up, down, enter) = ui.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::Enter),
            )
        });
        let before = self.values.location.clone();
        let response = input(
            ui,
            id,
            &mut self.values.location,
            value,
            &SMALL,
            "Enter a location",
            false,
            focus,
            None,
        );
        if self.values.location != before {
            self.place = None;
            self.places_closed = false;
        }
        let listed = !self.places_closed
            && !self.values.location.trim().is_empty()
            && !shown.places.is_empty();
        if had_focus && listed {
            if down {
                self.place = Some(
                    self.place
                        .map_or(0, |place| (place + 1).min(shown.places.len() - 1)),
                );
            } else if up {
                self.place = self.place.and_then(|place| place.checked_sub(1));
            }
        }
        if had_focus && enter {
            match self
                .place
                .and_then(|place| shown.places.get(place))
                .filter(|_| listed)
            {
                Some(place) => self.pick_place(place),
                None => self.focus = self.next(Field::Location),
            }
        }
        if response.has_focus() && listed {
            *overlay = Some(Overlay::Places {
                below: value,
                items: shown.places.to_vec(),
                form,
                pane,
            });
        }
        response.has_focus()
    }

    /// Takes `place` as Where, its list closed until the text changes.
    fn pick_place(&mut self, place: &Place) {
        self.values.location = place.location();
        self.place = None;
        self.places_closed = true;
        self.focus = Some(Field::Location);
    }

    /// The meeting link: `Add a meeting link`, or the link (or its state) with a ×.
    fn meeting_row(
        &mut self,
        ui: &mut Ui,
        value: Rect,
        focus: bool,
        overlay: &mut Option<Overlay>,
    ) -> bool {
        let palette = exo_ui::palette(ui.ctx());
        let id = Field::Meeting.id(ui.ctx());
        let response = ui
            .interact(value, id, Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if focus {
            response.request_focus();
        }
        let keys = ui.input(|input| {
            (
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Backspace) || input.key_pressed(egui::Key::Delete),
            )
        });
        match self.values.meeting.clone() {
            Meeting::None => {
                SMALL.paint_line(
                    ui.painter(),
                    value.min,
                    "Add a meeting link",
                    palette.text_base_accent,
                    None,
                );
                if response.clicked() || (response.has_focus() && keys.0) {
                    self.dropdown = if self
                        .dropdown
                        .is_some_and(|(open, _)| open == Dropdown::Meeting)
                    {
                        None
                    } else {
                        Some((Dropdown::Meeting, 0))
                    };
                }
            }
            meeting => {
                // The link's pill: `left: -5px; padding: 2px 4px`.
                let text = match &meeting {
                    Meeting::Existing(link) => link
                        .trim_start_matches("https://")
                        .trim_start_matches("http://")
                        .to_owned(),
                    _ => "Link will be added when saved".to_owned(),
                };
                let width = SMALL.width(ui.ctx(), &text);
                let pill = Rect::from_min_size(
                    pos2(value.left() - 5.0, value.top() - 2.0),
                    vec2(width + 8.0 + 20.0, SMALL.line_height + 4.0),
                );
                if response.hovered() || response.has_focus() {
                    ui.painter().rect_filled(
                        exo_ui::pixel_snap(pill),
                        CornerRadius::same(4),
                        palette.fill_hover_default,
                    );
                }
                SMALL.paint_line(
                    ui.painter(),
                    pos2(value.left() - 1.0, value.top()),
                    &text,
                    palette.text_base_default,
                    None,
                );
                let cross = Rect::from_min_size(
                    pos2(pill.right() - 18.0, pill.top() + 2.0),
                    Vec2::splat(16.0),
                );
                let close = "legacy/DEPRECATED_closeV3";
                let removed = exo_ui::IconButton::new(close)
                    .show(
                        ui,
                        exo_ui::id(ui.ctx(), "calendar-form-meeting-remove"),
                        cross,
                        exo_ui::icon_rect(ui.ctx(), close, cross),
                    )
                    .clicked();
                if removed || (response.has_focus() && keys.1) {
                    self.values.meeting = Meeting::None;
                }
            }
        }
        if response.has_focus() {
            underline(ui, value, palette.border_brand_default);
        }
        if let Some((Dropdown::Meeting, highlighted)) = self.dropdown {
            *overlay = Some(Overlay::Menu {
                below: value,
                which: Dropdown::Meeting,
                items: vec!["Google Meet".to_owned()],
                highlighted,
            });
        }
        response.has_focus()
    }

    /// Repeat: the rule's name, the presets for the event's start when clicked, `Custom` opening
    /// its dialog.
    fn repeat_row(
        &mut self,
        ui: &mut Ui,
        value: Rect,
        shown: &Shown<'_>,
        focus: bool,
        overlay: &mut Option<Overlay>,
    ) -> bool {
        let palette = exo_ui::palette(ui.ctx());
        let anchor = self.anchor(shown.now);
        let text = repeat::label(self.values.repeat.as_ref(), anchor);
        let width = SMALL.width(ui.ctx(), &text);
        let trigger = Rect::from_min_size(value.min, vec2(width, value.height()));
        let response = ui
            .interact(trigger, Field::Repeat.id(ui.ctx()), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        // A click focuses the trigger, as the button takes focus, so its list's keys work.
        if focus || response.clicked() {
            response.request_focus();
        }
        // A placeholder while it doesn't repeat: `.35`.
        let ink = if self.values.repeat.is_none() {
            palette.ink(0.35, 0.35)
        } else {
            palette.text_base_default
        };
        SMALL.paint_line(ui.painter(), value.min, &text, ink, None);
        if response.has_focus() {
            // Focused: Classic's `#aeb1dd`, 2px under the trigger's 2px padding.
            ui.painter().rect_filled(
                Rect::from_min_size(
                    pos2(trigger.left(), trigger.top() + SMALL.line_height + 2.0),
                    vec2(width, 2.0),
                ),
                0.0,
                Color32::from_rgb(0xae, 0xb1, 0xdd),
            );
        }
        // Enter on the focused trigger also clicks it (egui's); it opens the list, and with the
        // list open it's the list's.
        let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
        if (response.clicked() && !enter)
            || (response.has_focus() && enter && self.dropdown.is_none())
        {
            self.dropdown = if self
                .dropdown
                .is_some_and(|(open, _)| open == Dropdown::Repeat)
            {
                None
            } else {
                Some((Dropdown::Repeat, 0))
            };
        }
        if response.has_focus() && self.dropdown.is_some() {
            // ↑ and ↓ move through the open list, not to the next field (egui's focus moves).
            let filter = egui::EventFilter {
                vertical_arrows: true,
                ..Default::default()
            };
            let id = Field::Repeat.id(ui.ctx());
            ui.memory_mut(|memory| memory.set_focus_lock_filter(id, filter));
        }
        if let Some((Dropdown::Repeat, highlighted)) = self.dropdown {
            let items = repeat::presets(anchor)
                .into_iter()
                .map(|(_, name)| name)
                .collect();
            *overlay = Some(Overlay::Menu {
                below: value,
                which: Dropdown::Repeat,
                items,
                highlighted,
            });
        }
        response.has_focus()
    }

    /// The day a rule repeats from: the event's start, else today.
    fn anchor(&self, now: &Zoned) -> jiff::civil::Date {
        self.values
            .when
            .as_ref()
            .map_or_else(|| now.date(), |(start, _)| start.date())
    }

    /// Paints Repeat's Custom dialog over `page` while it's up, and takes its rule.
    pub fn show_custom(
        &mut self,
        ctx: &egui::Context,
        page: Rect,
        now: &Zoned,
        week_start: Weekday,
    ) {
        let Some(custom) = &mut self.custom else {
            return;
        };
        match custom.show(ctx, page, now.date(), week_start) {
            Some(CustomAction::Done(rule)) => {
                self.values.repeat = Some(rule);
                self.custom = None;
                self.focus = Some(Field::Repeat);
            }
            Some(CustomAction::Cancel) => {
                self.custom = None;
                self.focus = Some(Field::Repeat);
            }
            None => {}
        }
    }

    /// ⌘Enter in the Custom dialog: Done, when it can.
    pub fn finish_custom(&mut self) {
        if let Some(rule) = self.custom.as_ref().and_then(Custom::done) {
            self.values.repeat = Some(rule);
            self.custom = None;
            self.focus = Some(Field::Repeat);
        }
    }

    /// Escape in the Custom dialog: its open list, else the dialog.
    pub fn escape_custom(&mut self) {
        if let Some(custom) = &mut self.custom
            && !custom.escape()
        {
            self.custom = None;
            self.focus = Some(Field::Repeat);
        }
    }

    /// The calendar it goes on: a field showing the calendar's name, which empties as it takes the
    /// keyboard and lists the writable calendars by account under it, filtered by what's typed.
    /// ↑/↓ move through them round the ends, Enter or a click takes one and moves on to Save;
    /// leaving empties it.
    #[allow(clippy::too_many_arguments)] // The row's place, what it shows and what it opens over the `form`.
    fn calendar_row(
        &mut self,
        ui: &mut Ui,
        value: Rect,
        shown: &Shown<'_>,
        focus: bool,
        overlay: &mut Option<Overlay>,
        form: Rect,
        pane: bool,
    ) -> bool {
        let id = Field::Calendar.id(ui.ctx());
        let had_focus = ui.memory(|memory| memory.has_focus(id));
        let (up, down, enter) = ui.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::Enter),
            )
        });
        if had_focus {
            let rows = calendar_rows(shown.calendars, &self.calendar_input);
            let at = calendar_highlight(&rows, self.calendar_selected);
            if up || down {
                self.calendar_selected = calendar_step(&rows, at, up);
                self.calendar_keep_in_view(&rows, ui.ctx());
            } else if enter {
                if let Some(CalendarRow::Calendar(calendar)) = rows.get(at) {
                    let id = calendar.id.clone();
                    self.pick_calendar(id);
                } else {
                    // Nothing to take: the field keeps the keyboard (egui's Enter lets it go).
                    self.focus = Some(Field::Calendar);
                }
            }
        }
        // Without the keyboard it shows the calendar's name; with it (or taking it back), what's
        // typed.
        let typing = had_focus || focus;
        let mut text = if typing {
            std::mem::take(&mut self.calendar_input)
        } else {
            calendar_label(shown.calendars, &self.values.calendar)
        };
        let before = text.clone();
        let response = input(
            ui,
            id,
            &mut text,
            value,
            &SMALL,
            "Enter calendar name",
            false,
            focus,
            None,
        );
        if typing {
            if text != before {
                self.calendar_selected = 0;
                self.calendar_scroll = 0.0;
            }
            self.calendar_input = text;
        }
        if response.has_focus() && !had_focus {
            // Taking the keyboard: from the top, the first calendar highlighted.
            self.calendar_selected = 0;
            self.calendar_scroll = 0.0;
        } else if had_focus && !response.has_focus() && self.focus != Some(Field::Calendar) {
            self.calendar_input.clear();
        }
        if response.has_focus() && !shown.calendars.is_empty() {
            *overlay = Some(Overlay::Calendars {
                below: value,
                form,
                pane,
            });
        }
        response.has_focus()
    }

    /// Takes calendar `id`: the field empties and Save takes the keyboard.
    fn pick_calendar(&mut self, id: String) {
        self.values.calendar = id;
        self.calendar_input.clear();
        self.focus_save = true;
    }

    /// Scrolls the calendar list to show its highlighted row: the top for the first row, else as
    /// little as it takes.
    fn calendar_keep_in_view(&mut self, rows: &[CalendarRow<'_>], ctx: &egui::Context) {
        let (top, bottom) = calendar_row_span(ctx, rows, self.calendar_selected);
        let (_, viewport) = calendar_list_heights(ctx, rows);
        if self.calendar_selected == 0 {
            self.calendar_scroll = 0.0;
        } else if top < self.calendar_scroll {
            self.calendar_scroll = top;
        } else if bottom > self.calendar_scroll + viewport {
            self.calendar_scroll = bottom - viewport;
        }
    }

    /// About: the compose editor in `rect`, its border focused, the box hovered while it isn't.
    /// Returns whether it has the keyboard.
    fn about_row(&mut self, ui: &mut Ui, rect: Rect, focus: bool) -> bool {
        let palette = exo_ui::palette(ui.ctx());
        let text = Rect::from_min_max(
            rect.min,
            pos2(rect.right() - ABOUT_GUTTER, rect.bottom() - 2.0 - UNDERLINE),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(text)
                .id_salt("calendar-form-about"),
        );
        let focused = self.focused == Some(Field::About);
        if !focused && ui.rect_contains_pointer(rect) {
            let hover = if palette.theme == exo_ui::Theme::Carbon {
                Color32::from_rgb(0x36, 0x3a, 0x40)
            } else {
                Color32::from_rgb(0xf5, 0xf7, 0xfa)
            };
            ui.painter()
                .rect_filled(rect.expand2(vec2(ABOUT_HOVER_OUTSET, 0.0)), 0.0, hover);
        }
        // The placeholder sits 1px under where the text starts.
        if self.about.is_empty() {
            ABOUT_TEXT.style.paint_line(
                ui.painter(),
                text.min + vec2(0.0, 1.0),
                "Enter agenda or notes",
                palette.ink(0.35, 0.35),
                Some(text.width()),
            );
        }
        let style = exo_editor::EditorStyle {
            placeholder: None,
            min_height: ABOUT_TEXT.style.line_height,
        };
        let edited = self.about.ui(&mut child, &style);
        self.about_menu = edited.context_menu.or(self.about_menu.take());
        self.about_learned = edited.learned.or(self.about_learned.take());
        if focus {
            edited.response.request_focus();
        }
        self.about_height = edited.response.rect.height();
        if edited.changed {
            self.values.about = about_html(&self.about);
        }
        let focused = edited.response.has_focus() || focus;
        if focused {
            // The box's focused border, Classic's `#aeb1dd`.
            ui.painter().rect_filled(
                Rect::from_min_size(
                    pos2(rect.left(), text.top() + self.about_height + 2.0),
                    vec2(rect.width(), UNDERLINE),
                ),
                0.0,
                Color32::from_rgb(0xae, 0xb1, 0xdd),
            );
        }
        focused
    }

    /// About's editor.
    pub fn about(&self) -> &exo_editor::Editor {
        &self.about
    }

    /// About's editor, for the formatting keys.
    pub fn about_mut(&mut self) -> &mut exo_editor::Editor {
        &mut self.about
    }

    /// Takes About's HTML again after an edit made outside the form (a spelling correction).
    pub fn sync_about(&mut self) {
        self.values.about = about_html(&self.about);
    }

    /// About has the keyboard: its formatting keys apply, and Escape is the editor's when it
    /// has something open.
    pub fn about_focused(&self) -> bool {
        self.focused == Some(Field::About)
    }

    /// The field after `field` in the mode's order.
    fn next(&self, field: Field) -> Option<Field> {
        let rows = Field::rows(self.mode);
        rows.iter()
            .position(|row| *row == field)
            .and_then(|at| rows.get(at + 1))
            .copied()
    }

    /// Paints a dropdown or the suggestions over everything, and takes their picks.
    fn paint_overlay(&mut self, ui: &Ui, overlay: Overlay, shown: &Shown<'_>) {
        let ctx = ui.ctx().clone();
        let palette = exo_ui::palette(&ctx);
        let keys = ui.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::Enter),
            )
        });
        match overlay {
            Overlay::Suggestions {
                below,
                items,
                form,
                pane,
            } => {
                let (left, right) = if pane {
                    (form.left() + 10.0, form.right() - 10.0)
                } else {
                    (
                        below.left() - 14.0,
                        (below.left() - 14.0 + 420.0).min(form.right() - 10.0),
                    )
                };
                let top = below.top() + SUGGESTIONS_BELOW;
                let list = Rect::from_min_max(
                    pos2(left, top),
                    pos2(
                        right,
                        top + SUGGESTIONS_TOP - 1.0
                            + items.len() as f32 * SUGGESTION
                            + SUGGESTIONS_BOTTOM,
                    ),
                );
                // `padding: 10px calc(2rem - 10px)`, the modal's 12px.
                let padding = if pane {
                    2.0 * exo_ui::rem(ctx.content_rect().width(), false) - 10.0
                } else {
                    12.0
                };
                let (background, selected) = suggestion_fills(palette, false);
                // The text `.5`, what matches `.9`.
                let inks = (palette.ink(0.9, 0.9), palette.ink(0.5, 0.5));
                let query = self.guest_input.trim().to_lowercase();
                egui::Area::new(exo_ui::id(&ctx, "calendar-form-suggestions"))
                    .order(egui::Order::Tooltip)
                    .fixed_pos(list.min)
                    .constrain(false)
                    .show(&ctx, |area| {
                        let painter = area.painter().clone();
                        let snapped = exo_ui::pixel_snap(list);
                        exo_ui::BoxShadow::new([0.0, 2.0], 8.0, 0.0, Color32::from_black_alpha(26))
                            .paint(&painter, snapped, CornerRadius::same(4));
                        painter.rect_filled(snapped, CornerRadius::same(4), background);
                        for (index, item) in items.iter().enumerate() {
                            let row = Rect::from_min_size(
                                pos2(
                                    list.left(),
                                    list.top() + SUGGESTIONS_TOP + index as f32 * SUGGESTION,
                                ),
                                vec2(list.width(), SUGGESTION),
                            );
                            let hit = area
                                .interact(
                                    row,
                                    exo_ui::id(area.ctx(), ("calendar-form-suggestion", index)),
                                    Sense::hover(),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if hit.hovered() {
                                self.suggestion = index;
                            }
                            if index == self.suggestion {
                                painter.rect_filled(row, 0.0, selected);
                            }
                            let (name_at, email_at) = (
                                pos2(row.left() + padding, row.top() + 11.0),
                                pos2(row.left() + padding, row.top() + 27.0),
                            );
                            let room = row.width() - 2.0 * padding;
                            match item {
                                Suggestion::Contact(contact) => {
                                    let name = contact
                                        .name
                                        .clone()
                                        .unwrap_or_else(|| contact.email.clone());
                                    paint_matched(&painter, name_at, &name, &query, inks, room);
                                    paint_matched(
                                        &painter,
                                        email_at,
                                        &contact.email,
                                        &query,
                                        inks,
                                        room,
                                    );
                                }
                                // Its name all in the matched ink, the addresses after it.
                                Suggestion::Everybody(people) => {
                                    SMALL.paint_line(
                                        &painter,
                                        name_at,
                                        "Everybody in this conversation",
                                        inks.0,
                                        Some(room),
                                    );
                                    let emails = people
                                        .iter()
                                        .map(|person| person.email.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    SMALL.paint_line(
                                        &painter,
                                        email_at,
                                        &emails,
                                        inks.1,
                                        Some(room),
                                    );
                                }
                            }
                            // Picked on the press, while the input still has the keyboard.
                            if hit.hovered() && area.input(|input| input.pointer.primary_pressed())
                            {
                                self.values.guests.extend(item.guests());
                                self.guest_input.clear();
                                self.focus = Some(Field::Guests);
                            }
                        }
                    });
            }
            Overlay::Places {
                below,
                items,
                form,
                pane,
            } => {
                // Where's list: in the week's modal 4px under the input's box from its left,
                // 420px wide; in the right pane as Who's.
                let (left, right, top) = if pane {
                    (
                        form.left() + 10.0,
                        form.right() - 10.0,
                        below.top() + SUGGESTIONS_BELOW,
                    )
                } else {
                    (below.left(), below.left() + 420.0, below.top() + 20.0 + 4.0)
                };
                let list = Rect::from_min_max(
                    pos2(left, top),
                    pos2(
                        right,
                        top + SUGGESTIONS_TOP - 1.0
                            + items.len() as f32 * SUGGESTION
                            + SUGGESTIONS_BOTTOM,
                    ),
                );
                let padding = 2.0 * exo_ui::rem(ctx.content_rect().width(), false) - 10.0;
                let (background, selected) = suggestion_fills(palette, !pane);
                // The name `.35`, what matches `.9`; the address `.5`.
                let (dim, matched, address) = (
                    palette.ink(0.35, 0.35),
                    palette.ink(0.9, 0.9),
                    palette.ink(0.5, 0.5),
                );
                let query = self.values.location.trim().to_lowercase();
                let (pointer, pressed) =
                    ctx.input(|input| (input.pointer.hover_pos(), input.pointer.primary_pressed()));
                let mut picked = None;
                egui::Area::new(exo_ui::id(&ctx, "calendar-form-places"))
                    .order(egui::Order::Tooltip)
                    .fixed_pos(list.min)
                    .constrain(false)
                    .show(&ctx, |area| {
                        let painter = area.painter().clone();
                        let snapped = exo_ui::pixel_snap(list);
                        exo_ui::BoxShadow::new([0.0, 2.0], 8.0, 0.0, Color32::from_black_alpha(26))
                            .paint(&painter, snapped, CornerRadius::same(4));
                        painter.rect_filled(snapped, CornerRadius::same(4), background);
                        let _ = area.allocate_rect(list, Sense::hover());
                        for (index, place) in items.iter().enumerate() {
                            let row = Rect::from_min_size(
                                pos2(
                                    list.left(),
                                    list.top() + SUGGESTIONS_TOP + index as f32 * SUGGESTION,
                                ),
                                vec2(list.width(), SUGGESTION),
                            );
                            let hovered = pointer.is_some_and(|at| row.contains(at));
                            if hovered {
                                self.place = Some(index);
                            }
                            if self.place == Some(index) {
                                painter.rect_filled(row, 0.0, selected);
                            }
                            let room = row.width() - 2.0 * padding;
                            paint_occurrences(
                                &painter,
                                pos2(row.left() + padding, row.top() + 11.0),
                                &place.name,
                                &query,
                                (matched, dim),
                                room,
                            );
                            SMALL.paint_line(
                                &painter,
                                pos2(row.left() + padding, row.top() + 27.0),
                                &place.address,
                                address,
                                Some(room),
                            );
                            // Picked on the press, while the input still has the keyboard.
                            if hovered && pressed {
                                picked = Some(place.clone());
                            }
                        }
                    });
                if let Some(place) = picked {
                    self.pick_place(&place);
                }
            }
            Overlay::Menu {
                below,
                which,
                items,
                highlighted,
            } => {
                let labels: Vec<&str> = items.iter().map(String::as_str).collect();
                let menu = Menu {
                    items: &labels,
                    highlighted,
                };
                let id = exo_ui::id(&ctx, ("calendar-form-menu", which as u8));
                // Repeat's list is padded `4px 0`, 18px left of the trigger and 8px under it.
                let menu = if which == Dropdown::Repeat {
                    menu.show_compact(
                        ui,
                        id,
                        pos2(below.left() - 18.0, below.bottom() + 11.0),
                        16.0,
                        0.0,
                    )
                } else {
                    menu.show(ui, id, pos2(below.left() - 8.0, below.bottom() + 6.0))
                };
                let mut highlighted = menu.hovered.unwrap_or(highlighted);
                if keys.0 {
                    highlighted = highlighted.saturating_sub(1);
                } else if keys.1 {
                    highlighted = (highlighted + 1).min(labels.len().saturating_sub(1));
                }
                self.dropdown = Some((which, highlighted));
                let picked = menu.clicked.or((keys.2
                    && self.focused.is_some_and(|field| field != Field::Title))
                .then_some(highlighted));
                if let Some(picked) = picked {
                    self.dropdown = None;
                    match which {
                        Dropdown::Meeting => self.values.meeting = Meeting::NewMeet,
                        Dropdown::Repeat => {
                            let anchor = self.anchor(shown.now);
                            match repeat::presets(anchor)
                                .get(picked)
                                .map(|(preset, _)| *preset)
                            {
                                Some(repeat::Preset::Custom) => {
                                    self.custom =
                                        Some(Custom::new(anchor, self.values.repeat.as_ref()))
                                }
                                Some(preset) => self.values.repeat = preset.rule(),
                                None => {}
                            }
                        }
                    }
                } else if ui.input(|input| input.pointer.any_released())
                    && !ui.rect_contains_pointer(menu.rect)
                    && !ui.rect_contains_pointer(below)
                {
                    self.dropdown = None;
                }
            }
            Overlay::When { below, form } => {
                if let Some(index) = self.when.paint_list(ui, below, form, !self.week, shown.now) {
                    self.commit_when(index, shown.now);
                }
            }
            Overlay::Calendars { below, form, pane } => {
                self.paint_calendars(&ctx, below, form, pane, shown.calendars)
            }
        }
    }

    /// The calendar's list under its input `below`: hovering a calendar highlights it, pressing one
    /// takes it, the wheel scrolls the list.
    fn paint_calendars(
        &mut self,
        ctx: &egui::Context,
        below: Rect,
        form: Rect,
        pane: bool,
        calendars: &[CalendarOption],
    ) {
        let rows = calendar_rows(calendars, &self.calendar_input);
        if rows.is_empty() {
            return;
        }
        let palette = exo_ui::palette(ctx);
        let lines = calendar_row_lines(ctx, &rows);
        let (content, height) = calendar_list_heights(ctx, &rows);
        let window = ctx.content_rect();
        // 12px under the input's 20px box; in the modal it turns over to 12px above the input
        // when it would leave the window, and stays inside the window.
        let mut top = below.top() + SUGGESTIONS_BELOW;
        let left = if pane {
            form.left() + 10.0
        } else {
            below.left()
        };
        if !pane {
            let above = below.top() - 12.0 - height;
            if top + height > window.bottom() && above >= window.top() {
                top = above;
            }
            top = top.min(window.bottom() - height).max(window.top());
        }
        let list = Rect::from_min_size(pos2(left, top), vec2(CALENDAR_LIST_WIDTH, height));
        self.calendar_scroll = self.calendar_scroll.clamp(0.0, content - height);
        let (background, selected) = suggestion_fills(palette, !pane);
        // Calendars at `.9`, account headers at `.35` in Classic's normal weight.
        let (ink, header_ink) = (palette.ink(0.9, 0.9), palette.ink(0.35, 0.35));
        let padding = calendar_list_padding(ctx);
        let (pointer, moved, pressed) = ctx.input(|input| {
            (
                input.pointer.hover_pos(),
                input.pointer.delta() != Vec2::ZERO,
                input.pointer.primary_pressed(),
            )
        });
        let highlight = calendar_highlight(&rows, self.calendar_selected);
        let mut hovered = None;
        let mut picked = None;
        egui::Area::new(exo_ui::id(ctx, "calendar-form-calendars"))
            .order(egui::Order::Tooltip)
            .fixed_pos(list.min)
            .constrain(false)
            .show(ctx, |area| {
                let snapped = exo_ui::pixel_snap(list);
                exo_ui::BoxShadow::new([0.0, 2.0], 8.0, 0.0, Color32::from_black_alpha(26)).paint(
                    area.painter(),
                    snapped,
                    CornerRadius::same(4),
                );
                area.painter()
                    .rect_filled(snapped, CornerRadius::same(4), background);
                // Not `hovered()`, which a press elsewhere's widget clears on the frame it takes one.
                let _ = area.allocate_rect(list, Sense::hover());
                let over = area.rect_contains_pointer(list);
                if over {
                    let wheel =
                        area.input_mut(|input| std::mem::take(&mut input.smooth_scroll_delta.y));
                    self.calendar_scroll =
                        (self.calendar_scroll - wheel).clamp(0.0, content - height);
                }
                let painter = area.painter().with_clip_rect(list);
                // Rows are nudged 1px down (`position: relative; top: 1px`) from the 3px margin.
                let mut y = list.top() + SUGGESTIONS_TOP - self.calendar_scroll;
                for (index, (row, lines)) in rows.iter().zip(&lines).enumerate() {
                    let rect = Rect::from_min_size(
                        pos2(list.left(), y),
                        vec2(
                            list.width(),
                            2.0 * CALENDAR_ROW_PADDING + lines.len() as f32 * SMALL.line_height,
                        ),
                    );
                    y = rect.bottom();
                    let under = over && pointer.is_some_and(|at| rect.contains(at));
                    let color = match row {
                        CalendarRow::Header(_) => header_ink,
                        CalendarRow::Calendar(calendar) => {
                            if under && moved {
                                hovered = Some(index);
                            }
                            if under && pressed {
                                picked = Some(calendar.id.clone());
                            }
                            if index == highlight {
                                painter.rect_filled(rect, 0.0, selected);
                            }
                            // The swatch: the colour itself, in both themes.
                            let swatch = Rect::from_min_size(
                                pos2(
                                    rect.right() - padding - CALENDAR_SWATCH,
                                    rect.top() + CALENDAR_ROW_PADDING + 2.0,
                                ),
                                Vec2::splat(CALENDAR_SWATCH),
                            );
                            painter.rect_filled(
                                swatch,
                                CornerRadius::same(2),
                                calendar.color.base(),
                            );
                            ink
                        }
                    };
                    for (line, text) in lines.iter().enumerate() {
                        SMALL.paint_line(
                            &painter,
                            pos2(
                                rect.left() + padding,
                                rect.top() + CALENDAR_ROW_PADDING + line as f32 * SMALL.line_height,
                            ),
                            text,
                            color,
                            None,
                        );
                    }
                }
                self.calendar_scrollbar.show(
                    area,
                    exo_ui::id(ctx, "calendar-form-calendars-scrollbar"),
                    list,
                    self.calendar_scroll,
                    content,
                );
            });
        if let Some(index) = hovered.filter(|index| *index != highlight) {
            self.calendar_selected = index;
            self.calendar_keep_in_view(&rows, ctx);
        }
        if let Some(id) = picked {
            self.pick_calendar(id);
        }
    }
}

/// Something drawn over the form.
enum Overlay {
    Suggestions {
        below: Rect,
        items: Vec<Suggestion>,
        form: Rect,
        pane: bool,
    },
    /// Where's places, under its input in the `form`.
    Places {
        below: Rect,
        items: Vec<Place>,
        form: Rect,
        pane: bool,
    },
    Menu {
        below: Rect,
        which: Dropdown,
        items: Vec<String>,
        highlighted: usize,
    },
    /// When's suggestions, under its input in the `form`.
    When { below: Rect, form: Rect },
    /// The calendar's list, under its input in the `form`.
    Calendars { below: Rect, form: Rect, pane: bool },
}

/// A row of the calendar's list.
#[derive(Clone, Copy)]
enum CalendarRow<'a> {
    /// An account's address over its calendars, while more than one account's
    /// are listed.
    Header(&'a str),
    Calendar(&'a CalendarOption),
}

/// The calendar's list for what's `typed`: with nothing typed, every calendar as `calendars`
/// lists them (the mail account's first); else those whose name matches (`command-score` above
/// 0), best first. Each account's calendars follow its address, in the order the accounts first
/// come, once there's more than one account.
fn calendar_rows<'a>(calendars: &'a [CalendarOption], typed: &str) -> Vec<CalendarRow<'a>> {
    let mut matching: Vec<(f64, &CalendarOption)> = calendars
        .iter()
        .map(|calendar| {
            (
                if typed.is_empty() {
                    1.0
                } else {
                    exo_commands::command_score(&calendar.name, typed)
                },
                calendar,
            )
        })
        .filter(|(score, _)| *score > 0.0)
        .collect();
    matching.sort_by(|(a, _), (b, _)| b.total_cmp(a));
    let mut accounts: Vec<(&str, Vec<&CalendarOption>)> = Vec::new();
    for (_, calendar) in matching {
        match accounts
            .iter_mut()
            .find(|(account, _)| *account == calendar.account)
        {
            Some((_, listed)) => listed.push(calendar),
            None => accounts.push((&calendar.account, vec![calendar])),
        }
    }
    let headed = accounts.len() > 1;
    accounts
        .into_iter()
        .flat_map(|(account, listed)| {
            headed
                .then_some(CalendarRow::Header(account))
                .into_iter()
                .chain(listed.into_iter().map(CalendarRow::Calendar))
        })
        .collect()
}

/// The highlighted row: `row` when it's a calendar, else the first calendar.
fn calendar_highlight(rows: &[CalendarRow<'_>], row: usize) -> usize {
    if matches!(rows.get(row), Some(CalendarRow::Calendar(_))) {
        row
    } else {
        rows.iter()
            .position(|row| matches!(row, CalendarRow::Calendar(_)))
            .unwrap_or(0)
    }
}

/// The calendar ↑ (`up`) or ↓ takes the highlight to from `row`, round the ends, past the
/// headers.
fn calendar_step(rows: &[CalendarRow<'_>], row: usize, up: bool) -> usize {
    let count = rows.len();
    (1..=count)
        .map(|step| {
            if up {
                (row + count - step) % count
            } else {
                (row + step) % count
            }
        })
        .find(|at| *at == row || matches!(rows[*at], CalendarRow::Calendar(_)))
        .unwrap_or(row)
}

/// The field's text without the keyboard: the calendar's name, then its account's address once
/// calendars of more than one account are listed, unless it's named as its account
/// (`{name} · {account}`).
fn calendar_label(calendars: &[CalendarOption], id: &str) -> String {
    let grouped = calendars
        .iter()
        .any(|calendar| calendar.account != calendars[0].account);
    match calendars.iter().find(|calendar| calendar.id == id) {
        Some(calendar) if grouped && calendar.name != calendar.account => {
            format!("{} · {}", calendar.name, calendar.account)
        }
        Some(calendar) => calendar.name.clone(),
        None => id.to_owned(),
    }
}

/// The list's rows' side padding, `calc(2rem - 10px)`.
fn calendar_list_padding(ctx: &egui::Context) -> f32 {
    2.0 * exo_ui::rem(ctx.content_rect().width(), false) - 10.0
}

/// Each row's lines: a header's address, a calendar's name wrapped in the room the swatch leaves.
fn calendar_row_lines(ctx: &egui::Context, rows: &[CalendarRow<'_>]) -> Vec<Vec<String>> {
    let room = CALENDAR_LIST_WIDTH
        - 2.0 * calendar_list_padding(ctx)
        - CALENDAR_SWATCH_GAP
        - CALENDAR_SWATCH;
    rows.iter()
        .map(|row| match row {
            CalendarRow::Header(account) => vec![(*account).to_owned()],
            CalendarRow::Calendar(calendar) => SMALL.wrap(ctx, &calendar.name, room),
        })
        .collect()
}

/// Each row's height.
fn calendar_row_heights(ctx: &egui::Context, rows: &[CalendarRow<'_>]) -> Vec<f32> {
    calendar_row_lines(ctx, rows)
        .iter()
        .map(|lines| 2.0 * CALENDAR_ROW_PADDING + lines.len() as f32 * SMALL.line_height)
        .collect()
}

/// Where row `row` is drawn in the list's content: under the 3px margin, nudged 1px down.
fn calendar_row_span(ctx: &egui::Context, rows: &[CalendarRow<'_>], row: usize) -> (f32, f32) {
    let heights = calendar_row_heights(ctx, rows);
    let top = SUGGESTIONS_TOP + heights.iter().take(row).sum::<f32>();
    (top, top + heights.get(row).copied().unwrap_or(0.0))
}

/// The list's content height, and the height it shows (`max-height: 40vh`).
fn calendar_list_heights(ctx: &egui::Context, rows: &[CalendarRow<'_>]) -> (f32, f32) {
    let content = SUGGESTIONS_TOP - 1.0
        + calendar_row_heights(ctx, rows).iter().sum::<f32>()
        + SUGGESTIONS_BOTTOM;
    (
        content,
        content.min(CALENDAR_LIST_MAX_HEIGHT * ctx.content_rect().height()),
    )
}

/// The suggestion lists' fills and their highlighted row's: Classic's literals. One `tethered`
/// under its input in the week's modal is outside the form, on its own Carbon `#2f3237`.
fn suggestion_fills(palette: &exo_ui::Palette, tethered: bool) -> (Color32, Color32) {
    match palette.theme {
        exo_ui::Theme::Snow => (Color32::WHITE, Color32::from_rgb(0xf2, 0xf4, 0xfa)),
        exo_ui::Theme::Carbon if tethered => (
            Color32::from_rgb(0x2f, 0x32, 0x37),
            Color32::from_rgb(0x46, 0x4b, 0x53),
        ),
        exo_ui::Theme::Carbon => (
            Color32::from_rgb(0x27, 0x29, 0x2d),
            Color32::from_rgb(0x46, 0x4b, 0x53),
        ),
    }
}

/// `text` with every occurrence of `query` in `inks.0`, the rest in `inks.1`, within `room`.
fn paint_occurrences(
    painter: &egui::Painter,
    at: Pos2,
    text: &str,
    query: &str,
    inks: (Color32, Color32),
    room: f32,
) {
    let lower = text.to_lowercase();
    let mut x = 0.0;
    let mut from = 0;
    // Lower-casing keeps these strings' lengths for the letters places have; else no matches.
    let found = if query.is_empty() || lower.len() != text.len() {
        Vec::new()
    } else {
        lower.match_indices(query).map(|(at, _)| at).collect()
    };
    for start in found {
        if start < from {
            continue;
        }
        let (Some(before), Some(matched)) =
            (text.get(from..start), text.get(start..start + query.len()))
        else {
            break;
        };
        for (part, ink) in [(before, inks.1), (matched, inks.0)] {
            x += SMALL.paint_line(
                painter,
                pos2(at.x + x, at.y),
                part,
                ink,
                Some((room - x).max(0.0)),
            );
        }
        from = start + query.len();
    }
    SMALL.paint_line(
        painter,
        pos2(at.x + x, at.y),
        text.get(from..).unwrap_or_default(),
        inks.1,
        Some((room - x).max(0.0)),
    );
}

/// `text` with the characters matching `query` at its start in `inks.0`, the rest in `inks.1`,
/// within `room`.
fn paint_matched(
    painter: &egui::Painter,
    at: Pos2,
    text: &str,
    query: &str,
    inks: (Color32, Color32),
    room: f32,
) {
    let lower = text.to_lowercase();
    let matched = if !query.is_empty() && lower.starts_with(query) {
        query.len().min(text.len())
    } else {
        0
    };
    let split = (0..=matched)
        .rev()
        .find(|at| text.is_char_boundary(*at))
        .unwrap_or(0);
    let used = SMALL.paint_line(painter, at, &text[..split], inks.0, Some(room));
    SMALL.paint_line(
        painter,
        pos2(at.x + used, at.y),
        &text[split..],
        inks.1,
        Some((room - used).max(0.0)),
    );
}

/// A single-line input on `line` with its placeholder and, while focused (or `error`), its
/// underline. `focus` takes the keyboard to it; with `spelling`, it's spellchecked.
#[allow(clippy::too_many_arguments)] // Each is one of the input's own properties.
fn input(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    line: Rect,
    style: &TextStyle,
    placeholder: &str,
    error: bool,
    focus: bool,
    spelling: Option<&mut exo_editor::FieldSpelling>,
) -> egui::Response {
    let palette = exo_ui::palette(ui.ctx());
    if focus {
        ui.memory_mut(|memory| memory.request_focus(id));
    }
    if text.is_empty() {
        style.paint_line(
            ui.painter(),
            line.min,
            placeholder,
            palette.text_base_accent,
            Some(line.width()),
        );
    }
    // The form's inputs are `rgba(0, 0, 0, .9)`, Carbon's `hsla(0, 0%, 100%, .85)`.
    let ink = palette.ink(0.9, 0.85);
    let response = match spelling {
        Some(spelling) => spelling.show(
            ui,
            id,
            text,
            line,
            style,
            ink,
            palette.text_selection_default,
        ),
        None => exo_ui::text_field(
            ui,
            id,
            text,
            line,
            style,
            ink,
            palette.text_selection_default,
        ),
    };
    if error {
        underline(ui, line, palette.border_critical_default);
    } else if response.has_focus() {
        underline(ui, line, palette.border_brand_default);
    }
    response
}

/// The editor's HTML, or nothing for an empty one.
fn about_html(editor: &exo_editor::Editor) -> String {
    if editor.is_empty() {
        String::new()
    } else {
        editor.html()
    }
}

/// The 2px underline under an input's `line`.
fn underline(ui: &Ui, line: Rect, color: Color32) {
    let bottom = line.bottom() + 2.0;
    ui.painter().rect_filled(
        Rect::from_min_max(
            pos2(line.left(), bottom),
            pos2(line.right(), bottom + UNDERLINE),
        ),
        0.0,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::tz::TimeZone;

    fn at(text: &str) -> Zoned {
        text.parse::<jiff::civil::DateTime>()
            .expect("time")
            .to_zoned(TimeZone::get("America/New_York").expect("zone"))
            .expect("exists")
    }

    fn values() -> Values {
        Values {
            title: String::new(),
            when: None,
            all_day: false,
            guests: Vec::new(),
            location: String::new(),
            about: String::new(),
            meeting: Meeting::None,
            calendar: "me".into(),
            repeat: None,
        }
    }

    #[test]
    fn the_day_makes_room_once_when_s_list_reaches_it_until_when_is_left_or_emptied() {
        let mut form = EventForm::create(values(), false);
        form.focused = Some(Field::When);
        form.when.text = "fri".into();
        form.when.list_bottom = Some(300.0);
        assert!(!form.when_covers(320.0), "10px short");
        form.when.list_bottom = Some(312.0);
        assert!(form.when_covers(320.0));
        form.when.list_bottom = Some(200.0);
        assert!(form.when_covers(360.0), "it stays while typing");
        form.focused = Some(Field::Title);
        assert!(!form.when_covers(360.0), "leaving When ends it");
        form.focused = Some(Field::When);
        form.when.text.clear();
        assert!(!form.when_covers(360.0));
    }

    #[test]
    fn typed_guests_need_addresses() {
        let mut form = EventForm::create(values(), true);
        form.guest_input = "lena@brightmoor.co, Jordan <jordan@calloway.com>".into();
        assert!(form.commit_guests());
        assert_eq!(
            form.values
                .guests
                .iter()
                .map(|guest| guest.email.as_str())
                .collect::<Vec<_>>(),
            ["lena@brightmoor.co", "jordan@calloway.com"]
        );
        form.guest_input = "not an address".into();
        assert!(!form.commit_guests());
        assert!(form.guest_error);
        assert!(!form.is_empty(), "guests make it worth asking");
    }

    #[test]
    fn an_edit_knows_what_changed() {
        let mut values = values();
        values.when = Some((at("2026-11-17T15:00"), at("2026-11-17T16:00")));
        let edited = Edited {
            key: "k".into(),
            event: ApiEvent::default(),
            color: CalendarColor::BLUE,
        };
        let mut form = EventForm::edit(values, edited, true, Field::Title);
        assert!(form.is_empty());
        form.values.title = "Design crit".into();
        form.toggle_all_day();
        assert_eq!(form.dirty(), [Field::Title, Field::When]);
        assert_eq!(
            form.values.when,
            Some((at("2026-11-17T00:00"), at("2026-11-18T00:00")))
        );
    }

    /// The writable calendars of `fixtures/calendar-accounts.json`, as the form lists them.
    fn calendars() -> Vec<CalendarOption> {
        let option = |account: &str, id: &str, name: &str| CalendarOption {
            account: account.into(),
            id: id.into(),
            name: name.into(),
            color: CalendarColor::BLUE,
        };
        vec![
            option(
                "maya@brightmoor.co",
                "maya@brightmoor.co",
                "maya@brightmoor.co",
            ),
            option("maya@brightmoor.co", "team", "Brightmoor Team"),
            option(
                "maya.lin@example.com",
                "maya.lin@example.com",
                "maya.lin@example.com",
            ),
        ]
    }

    fn listed(rows: &[CalendarRow<'_>]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                CalendarRow::Header(account) => format!("# {account}"),
                CalendarRow::Calendar(calendar) => calendar.name.clone(),
            })
            .collect()
    }

    #[test]
    fn the_calendar_list_heads_each_account_once_there_are_two_and_ranks_what_is_typed() {
        let calendars = calendars();
        assert_eq!(
            listed(&calendar_rows(&calendars, "")),
            [
                "# maya@brightmoor.co",
                "maya@brightmoor.co",
                "Brightmoor Team",
                "# maya.lin@example.com",
                "maya.lin@example.com"
            ]
        );
        assert_eq!(
            listed(&calendar_rows(&calendars[..2], "")),
            ["maya@brightmoor.co", "Brightmoor Team"],
            "one account's: no headers"
        );
        assert_eq!(
            listed(&calendar_rows(&calendars, "team")),
            ["Brightmoor Team"]
        );
        // Best first, each account's calendars together under it, the accounts in the order
        // their best come.
        assert_eq!(
            listed(&calendar_rows(&calendars, "e")),
            [
                "# maya.lin@example.com",
                "maya.lin@example.com",
                "# maya@brightmoor.co",
                "Brightmoor Team"
            ]
        );
        assert_eq!(
            listed(&calendar_rows(&calendars, "bri")),
            ["Brightmoor Team", "maya@brightmoor.co"],
            "the better match first"
        );
        assert!(calendar_rows(&calendars, "zzz").is_empty());
    }

    #[test]
    fn the_calendar_list_highlight_skips_headers_and_goes_round() {
        let calendars = calendars();
        let rows = calendar_rows(&calendars, "");
        assert_eq!(
            calendar_highlight(&rows, 0),
            1,
            "the first calendar, not its header"
        );
        assert_eq!(calendar_highlight(&rows, 9), 1);
        assert_eq!(calendar_highlight(&rows, 2), 2);
        assert_eq!(calendar_step(&rows, 2, false), 4, "past maya.lin's header");
        assert_eq!(
            calendar_step(&rows, 4, false),
            1,
            "round the end, past the first header"
        );
        assert_eq!(calendar_step(&rows, 1, true), 4);
    }

    #[test]
    fn the_calendar_field_names_its_account_once_there_are_two() {
        let calendars = calendars();
        assert_eq!(
            calendar_label(&calendars, "team"),
            "Brightmoor Team · maya@brightmoor.co"
        );
        assert_eq!(
            calendar_label(&calendars, "maya.lin@example.com"),
            "maya.lin@example.com",
            "named as its account"
        );
        assert_eq!(calendar_label(&calendars[..2], "team"), "Brightmoor Team");
        assert_eq!(calendar_label(&calendars, "gone"), "gone");
    }
}
