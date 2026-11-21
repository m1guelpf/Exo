//! Share Availability: the week view in a mode where clicks and drags on the grid pick free
//! times, beside a sidebar with the meeting's length, who it's with, the zone the times are
//! written in and the text; Insert writes it into a draft. Only `Insert free times`: the booking
//! link needs a server Exo doesn't have.

use std::collections::BTreeMap;

use egui::{Color32, CornerRadius, Id, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use exo_store::Address;
use exo_ui::{ActionKind, CalendarColor, DialogAction, Family, Font, TextStyle, Theme};
use jiff::{SignedDuration, Timestamp, Zoned, tz::TimeZone};

use super::{
    availability::{self, Person},
    event::Event,
    timezones::{self, Found},
};
use crate::compose::{contacts::Index, recipients};

/// The lengths offered, 30 minutes by default.
const DURATIONS: [i64; 4] = [15, 30, 45, 60];
/// What the slots' keys start with.
pub const SLOT: &str = "SHARE_AVAILABILITY_";

/// The sidebar's content: `padding: 32px 18px`; the heading 16px/24px with `margin-bottom:
/// 36px`; the form's rows 24px apart (`gap: 24px`).
const PADDING_TOP: f32 = 32.0;
const PADDING_X: f32 = 18.0;
const HEADING: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
const HEADING_GAP: f32 = 36.0;
const ROW_GAP: f32 = 24.0;
/// Labels: 10px/12px bold (`top: -.5px`, which Chromium's text doesn't show), `padding-bottom:
/// 8px` as blocks.
const LABEL: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 10.0, 12.0);
const LABEL_HEIGHT: f32 = 20.0;
/// Fields: 12px/16px nudged `top: 1px`, over a 2px rule 18px down, 20px tall; an input's text
/// sits 1px lower.
const FIELD: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const FIELD_HEIGHT: f32 = 20.0;
/// Who's list `margin-top: 16px`, `top: 1px`; its title and rows 24px apart; a name 16px in.
const LIST_GAP: f32 = 16.0;
const LIST_ROW: f32 = 24.0;
const NAME_LEFT: f32 = 16.0;
/// The buttons' footer: `padding: 20px 18px; gap: 12px`, the buttons 32px and `top: 1px`.
const FOOTER: f32 = 20.0 + 32.0 + 20.0;
const BUTTON_GAP: f32 = 12.0;
/// The Timezone input's list for Share Availability: four zones.
const ZONES: usize = 4;

/// Where Insert writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A draft open in compose, at its caret.
    Draft(String),
    /// A new Reply All in this conversation.
    Reply(String),
    /// A new message.
    New,
}

/// Someone the times are for (`attendees`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attendee {
    pub address: Address,
    pub person: Person,
    /// Listed under Who; recipients outside the team aren't, but count for the wording.
    pub visible: bool,
    /// Came with the draft.
    pub default: bool,
}

/// A text field of the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Who,
    Zone,
}

/// What the sidebar asks of the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareAction {
    Insert,
    Cancel,
    /// An error toast.
    Say(&'static str),
}

/// The page's state.
pub struct Share {
    pub target: Target,
    slots: BTreeMap<u64, (Zoned, Zoned)>,
    next_slot: u64,
    /// The meeting's length, in minutes.
    pub minutes: i64,
    /// The zone the times are written in.
    pub tz: TimeZone,
    pub attendees: Vec<Attendee>,
    /// The guests' calendars' colours, and the column a hover expanded.
    pub colors: super::guests::Colors,
    who: String,
    who_pick: usize,
    who_error: bool,
    /// The durations' list, open, with this row highlighted.
    durations: Option<usize>,
    /// The Timezone input's text, whether it's been typed in since a zone was picked, its list
    /// and the highlighted row.
    zone_text: String,
    zone_typed: bool,
    zones: Vec<Found>,
    zone_pick: usize,
    /// The field taking the keyboard next frame, or giving it up.
    focus: Option<Input>,
    blur: bool,
    /// The field with the keyboard last frame.
    pub typing: Option<Input>,
}

impl Share {
    /// The page for `target` and `attendees` (me first), writing times in `tz`; Who has the
    /// keyboard.
    pub fn new(target: Target, attendees: Vec<Attendee>, tz: TimeZone) -> Self {
        Self {
            target,
            slots: BTreeMap::new(),
            next_slot: 0,
            minutes: 30,
            tz,
            attendees,
            colors: super::guests::Colors::default(),
            who: String::new(),
            who_pick: 0,
            who_error: false,
            durations: None,
            zone_text: String::new(),
            zone_typed: false,
            zones: Vec::new(),
            zone_pick: 0,
            focus: Some(Input::Who),
            blur: false,
            typing: Some(Input::Who),
        }
    }

    /// Whether my calendar shows: I'm listed.
    pub fn shows_mine(&self) -> bool {
        self.attendees
            .iter()
            .any(|attendee| attendee.person.organizer && attendee.visible)
    }

    /// Escape in a field: its text goes, or with none, the field lets go of the keyboard and
    /// the page closes.
    pub fn escape(&mut self) -> bool {
        match self.typing {
            Some(Input::Who) if !self.who.is_empty() => {
                self.who.clear();
                self.who_error = false;
                false
            }
            Some(Input::Zone) if self.zone_typed => {
                self.zone_typed = false;
                self.zones.clear();
                self.zone_text.clear();
                false
            }
            _ => {
                self.blur = true;
                true
            }
        }
    }

    /// Adds a slot from `start` to `end` and merges it with those it touches on its day.
    pub fn add(&mut self, start: Zoned, end: Zoned) {
        self.next_slot += 1;
        self.slots.insert(self.next_slot, (start, end));
        self.merge();
    }

    /// Slot `key` moved or resized to `start`–`end`: kept within its day, and not before now (the
    /// next quarter hour, if it fits that day).
    pub fn change(&mut self, key: &str, start: Zoned, end: Zoned, now: &Zoned) {
        let Some(id) = slot_id(key) else { return };
        let Some((old_start, old_end)) = self.slots.get(&id).cloned() else {
            return;
        };
        let resized = start == old_start && end != old_end;
        let (start, end) = if resized {
            (start, end)
        } else {
            let length = end.timestamp().duration_since(start.timestamp());
            let midnight = start
                .date()
                .to_zoned(start.time_zone().clone())
                .unwrap_or_else(|_| start.clone());
            let next = midnight.tomorrow().unwrap_or_else(|_| midnight.clone());
            let mut at = if start < midnight {
                midnight
            } else if end > next {
                next.checked_sub(length).unwrap_or(start)
            } else {
                start
            };
            if at < *now {
                let quarter = SignedDuration::from_mins(15).as_secs();
                let ceiling = Timestamp::from_second(
                    (now.timestamp().as_second() + quarter - 1) / quarter * quarter,
                )
                .map(|at| at.to_zoned(now.time_zone().clone()));
                let tomorrow = now
                    .date()
                    .tomorrow()
                    .ok()
                    .and_then(|day| day.to_zoned(now.time_zone().clone()).ok());
                at = match (ceiling, tomorrow) {
                    (Ok(ceiling), Some(tomorrow))
                        if ceiling.checked_add(length).is_ok_and(|end| end <= tomorrow) =>
                    {
                        ceiling
                    }
                    _ => old_start,
                };
            }
            let end = at.checked_add(length).unwrap_or_else(|_| at.clone());
            (at, end)
        };
        self.slots.insert(id, (start, end));
        self.merge();
    }

    /// The cross on slot `key`.
    pub fn remove(&mut self, key: &str) {
        if let Some(id) = slot_id(key) {
            self.slots.remove(&id);
        }
    }

    /// Slots overlapping on the same day become one, keeping the first's key.
    fn merge(&mut self) {
        let mut slots: Vec<(u64, (Zoned, Zoned))> =
            std::mem::take(&mut self.slots).into_iter().collect();
        slots.sort_by_key(|(_, (start, _))| start.timestamp());
        let mut merged: Vec<(u64, (Zoned, Zoned))> = Vec::new();
        for (id, (start, end)) in slots {
            match merged.last_mut() {
                Some((_, last)) if start <= last.1 && start.date() == last.0.date() => {
                    if end > last.1 {
                        last.1 = end;
                    }
                }
                _ => merged.push((id, (start, end))),
            }
        }
        self.slots = merged.into_iter().collect();
    }

    /// Whether a slot can be shared: as long as the meeting, and not over.
    fn valid(&self, slot: &(Zoned, Zoned), now: &Zoned) -> bool {
        self.invalid_reason(slot, now).is_none()
    }

    /// Why a slot can't be shared, as its block's tooltip says: too short before over.
    fn invalid_reason(&self, (start, end): &(Zoned, Zoned), now: &Zoned) -> Option<&'static str> {
        if end.timestamp().duration_since(start.timestamp())
            < SignedDuration::from_mins(self.minutes)
        {
            Some("This slot is shorter than the event duration.")
        } else if end < now {
            Some("This slot is in the past.")
        } else {
            None
        }
    }

    /// The slots that can be shared.
    pub fn valid_slots(&self, now: &Zoned) -> Vec<(Zoned, Zoned)> {
        self.slots
            .values()
            .filter(|slot| self.valid(slot, now))
            .cloned()
            .collect()
    }

    /// The slots as the grid draws them, in `color` (my primary calendar's).
    pub fn events(&self, color: CalendarColor, now: &Zoned) -> Vec<Event> {
        self.slots
            .iter()
            .map(|(id, slot)| {
                let title = format!(
                    "{} – {}",
                    slot.0.strftime("%-I:%M"),
                    slot.1.strftime("%-I:%M")
                );
                let reason = self.invalid_reason(slot, now);
                Event {
                    share: Some(reason.is_none()),
                    tooltip: reason,
                    ..Event::draft(
                        format!("{SLOT}{id}"),
                        title,
                        slot.0.clone(),
                        slot.1.clone(),
                        false,
                        color,
                    )
                }
            })
            .collect()
    }

    /// The text Insert writes, for the valid slots.
    pub fn text(&self, now: &Zoned) -> String {
        let people: Vec<Person> = self
            .attendees
            .iter()
            .map(|attendee| attendee.person)
            .collect();
        availability::text(&self.valid_slots(now), self.minutes, &people, &self.tz, now)
    }

    /// Insert can go: a slot can be shared.
    pub fn can_insert(&self, now: &Zoned) -> bool {
        self.slots.values().any(|slot| self.valid(slot, now))
    }

    /// The earliest slot's start, which the zones' offsets are for, or now.
    fn relevant(&self, now: &Zoned) -> Timestamp {
        self.slots
            .values()
            .map(|(start, _)| start.timestamp())
            .min()
            .unwrap_or(now.timestamp())
    }

    /// Paints the sidebar over `sidebar` (its left border included). `me` is the account, whose
    /// colour is `color`; `contacts` suggests who to add.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        sidebar: Rect,
        now: &Zoned,
        color: CalendarColor,
        contacts: Option<&Index>,
    ) -> Vec<ShareAction> {
        let ctx = ui.ctx().clone();
        let palette = exo_ui::palette(&ctx);
        let carbon = palette.theme == Theme::Carbon;
        let mut actions = Vec::new();
        // `#fff`, Carbon `#18191a`, with the sidebar's `border-left: 1px solid rgba(0,0,0,.09)`.
        ui.painter().rect_filled(
            sidebar,
            0.0,
            if carbon {
                Color32::from_rgb(0x18, 0x19, 0x1a)
            } else {
                Color32::WHITE
            },
        );
        ui.painter().vline(
            sidebar.left() + 0.5,
            sidebar.y_range(),
            Stroke::new(1.0, Color32::from_black_alpha(23)),
        );
        let left = sidebar.left() + 1.0 + PADDING_X;
        let right = sidebar.right() - PADDING_X;
        let width = right - left;
        let painter = ui.painter().clone();
        let mut top = sidebar.top() + PADDING_TOP;
        HEADING.paint_line(
            &painter,
            pos2(left, top),
            "Share Availability",
            palette.ink(0.85, 0.85),
            None,
        );
        top += HEADING.line_height + HEADING_GAP;
        let label = |text: &str, top: f32| {
            LABEL.paint_line(
                &painter,
                pos2(left, top),
                text,
                palette.ink(0.35, 0.35),
                None,
            );
        };
        // A field's rule: `rgba(0,0,0,.085)`, Carbon `hsla(0,0%,100%,.5)`; `#aeb1dd` with the
        // keyboard, `#f67b7b` in error.
        let rule = |box_top: f32, focused: bool, error: bool| {
            let color = if error {
                Color32::from_rgb(0xf6, 0x7b, 0x7b)
            } else if focused {
                Color32::from_rgb(0xae, 0xb1, 0xdd)
            } else if carbon {
                Color32::from_white_alpha(128)
            } else {
                Color32::from_black_alpha(22)
            };
            painter.rect_filled(
                Rect::from_min_max(pos2(left, box_top + 18.0), pos2(right, box_top + 20.0)),
                0.0,
                color,
            );
        };

        // Duration: a dropdown, `30 mins`.
        label("Duration", top);
        let button = Rect::from_min_size(pos2(left, top + LABEL_HEIGHT), vec2(width, FIELD_HEIGHT));
        let opened = ui
            .interact(
                button,
                exo_ui::id(ui.ctx(), "share-duration"),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        FIELD.paint_line(
            &painter,
            pos2(left, button.top() + 1.0),
            &format!("{} mins", self.minutes),
            palette.ink(0.9, 0.9),
            None,
        );
        rule(button.top(), self.durations.is_some(), false);
        if opened.clicked() {
            self.durations = if self.durations.is_some() {
                None
            } else {
                DURATIONS
                    .iter()
                    .position(|minutes| *minutes == self.minutes)
            };
        }
        top = button.bottom() + ROW_GAP;

        // Who: the contact input, then the listed attendees.
        label("Who", top);
        let who_box = Rect::from_min_size(
            pos2(left, top + LABEL_HEIGHT + 1.0),
            vec2(width, FIELD_HEIGHT),
        );
        let who_id = exo_ui::id(&ctx, "share-who");
        let had_focus = ui.memory(|memory| memory.has_focus(who_id));
        let keys = ui.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Tab),
            )
        });
        if self.who.is_empty() {
            FIELD.paint_line(
                &painter,
                who_box.min + vec2(0.0, 1.0),
                "Enter name or email",
                palette.ink(0.35, 0.35),
                None,
            );
        }
        let response = self.field(ui, who_id, Input::Who, who_box);
        rule(who_box.top(), response.has_focus(), self.who_error);
        if response.changed() {
            self.who_pick = 0;
            if recipients::is_valid(self.who.trim()) {
                self.who_error = false;
            }
        }
        let suggestions: Vec<Address> = match contacts {
            Some(book) if !self.who.trim().is_empty() => {
                book.top(self.who.trim(), 3).into_iter().cloned().collect()
            }
            _ => Vec::new(),
        };
        let mut add = None;
        if had_focus && !self.who.is_empty() {
            let (up, down, enter, tab) = keys;
            if up {
                self.who_pick = self.who_pick.saturating_sub(1);
            } else if down {
                self.who_pick = (self.who_pick + 1).min(suggestions.len().saturating_sub(1));
            }
            if enter || tab {
                if let Some(pick) = suggestions.get(self.who_pick).or(suggestions.first()) {
                    add = Some(pick.clone());
                } else if recipients::is_valid(self.who.trim()) {
                    add = Some(recipients::parse(self.who.trim()));
                } else {
                    self.who_error = true;
                }
                // Enter and Tab stay in the field.
                self.focus = Some(Input::Who);
            }
        }
        if response.has_focus() && !suggestions.is_empty() {
            let (hovered, picked) =
                suggestion_list(ui, who_box, &suggestions, &self.who, self.who_pick);
            self.who_pick = hovered.unwrap_or(self.who_pick);
            if let Some(index) = picked {
                add = Some(suggestions[index].clone());
            }
        }
        if let Some(address) = add {
            self.add_attendee(address, &right_domain(&self.attendees));
            self.who.clear();
            self.who_pick = 0;
        }
        // The list's box (`top: 1px` aside): under the input's, its title and each row 20px tall.
        let list_top = who_box.bottom() - 1.0 + LIST_GAP;
        let mut row_top = list_top + 1.0;
        FIELD.paint_line(
            &painter,
            pos2(left, row_top),
            "Attendees",
            palette.ink(0.35, 0.35),
            None,
        );
        let mut removed = None;
        for (index, attendee) in self
            .attendees
            .iter()
            .enumerate()
            .filter(|(_, attendee)| attendee.visible)
        {
            row_top += LIST_ROW;
            let name = attendee
                .address
                .name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| attendee.address.email.clone());
            let name_width = FIELD.width(&ctx, &name).min(width - NAME_LEFT - 16.0);
            // Hovered, a pill: 3px up, `padding: 2px 7px 2px 23px` in a 1px border, 8px out to
            // the left; the cross 8px after the name.
            let pill = Rect::from_min_max(
                pos2(left - 8.0, row_top - 3.0),
                pos2(
                    left + NAME_LEFT + name_width + 8.0 + 8.0 + 8.0,
                    row_top + 19.0,
                ),
            );
            let hovered = ui.rect_contains_pointer(pill);
            if hovered {
                // `#e6e8f0` in `#aeb1dd`, Carbon `#555b64` in `#636973`.
                let (fill, border) = if carbon {
                    (
                        Color32::from_rgb(0x55, 0x5b, 0x64),
                        Color32::from_rgb(0x63, 0x69, 0x73),
                    )
                } else {
                    (
                        Color32::from_rgb(0xe6, 0xe8, 0xf0),
                        Color32::from_rgb(0xae, 0xb1, 0xdd),
                    )
                };
                ui.painter().rect(
                    exo_ui::pixel_snap(pill),
                    CornerRadius::same(4),
                    fill,
                    Stroke::new(1.0, border),
                    egui::StrokeKind::Inside,
                );
            }
            // The swatch: 8px, 4px down, my calendar's colour, or a guest's once their calendar
            // shows.
            let swatch = if attendee.person.organizer {
                Some(color)
            } else {
                self.colors.of(&attendee.address.email)
            };
            if let Some(swatch) = swatch {
                ui.painter().rect_filled(
                    Rect::from_min_size(pos2(left, row_top + 4.0), Vec2::splat(8.0)),
                    CornerRadius::same(1),
                    swatch.swatch(palette.theme),
                );
            }
            FIELD.paint_line(
                &painter,
                pos2(left + NAME_LEFT, row_top + 1.0),
                &name,
                palette.ink(0.85, 0.85),
                Some(name_width),
            );
            if hovered {
                let cross = Rect::from_min_size(
                    pos2(left + NAME_LEFT + name_width + 8.0, row_top + 4.0),
                    Vec2::splat(8.0),
                );
                // The cross: `rgba(0,0,0,.3)`, Carbon white.
                exo_ui::paint_icon(
                    ui,
                    "legacy/DEPRECATED_closeV3",
                    cross,
                    palette.ink(0.3, 0.3),
                );
                if ui
                    .interact(
                        cross.expand(4.0),
                        exo_ui::id(ui.ctx(), ("share-remove", index)),
                        Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    removed = Some(index);
                }
            }
        }
        if let Some(index) = removed {
            // The last one listed stays.
            if self
                .attendees
                .iter()
                .filter(|attendee| attendee.visible)
                .count()
                <= 1
            {
                actions.push(ShareAction::Say(
                    "Include at least one attendee to share availability.",
                ));
            } else if self.attendees[index].default && !self.attendees[index].person.internal {
                self.attendees[index].visible = false;
            } else {
                self.attendees.remove(index);
            }
        }
        top = row_top - 1.0 + FIELD_HEIGHT + ROW_GAP;

        // Timezone: `GMT−05:00       New York`, typed to find another.
        label("Timezone", top);
        let zone_box = Rect::from_min_size(
            pos2(left, top + LABEL_HEIGHT + 1.0),
            vec2(width, FIELD_HEIGHT),
        );
        let zone_id = exo_ui::id(&ctx, "share-zone");
        let relevant = self.relevant(now);
        let current = timezones::current(self.tz.iana_name().unwrap_or("UTC"), relevant);
        if !self.zone_typed {
            self.zone_text = current.as_ref().map(Found::text).unwrap_or_default();
        }
        let had_zone_focus = ui.memory(|memory| memory.has_focus(zone_id));
        let response = self.field(ui, zone_id, Input::Zone, zone_box);
        rule(zone_box.top(), response.has_focus(), false);
        if response.gained_focus() {
            // Taking the keyboard: what was typed, else this zone over the common ones.
            self.zones = if self.zone_typed {
                timezones::search(&self.zone_text, relevant, ZONES)
            } else {
                timezones::suggested(current.as_ref(), relevant, ZONES)
            };
            self.zone_pick = 0;
        }
        if response.changed() {
            self.zone_typed = true;
            self.zones = timezones::search(&self.zone_text, relevant, ZONES);
            self.zone_pick = 0;
        }
        let mut picked_zone = None;
        if had_zone_focus && !self.zones.is_empty() {
            let (up, down, enter, tab) = keys;
            if up {
                self.zone_pick = self.zone_pick.saturating_sub(1);
            } else if down {
                self.zone_pick = (self.zone_pick + 1).min(self.zones.len() - 1);
            } else if enter || tab {
                picked_zone = self.zones.get(self.zone_pick).cloned();
            }
        }
        if response.has_focus() && !self.zones.is_empty() {
            let list = Rect::from_min_size(
                pos2(zone_box.left() - 7.0, zone_box.bottom() + 12.0),
                vec2(276.0, self.zones.len() as f32 * 32.0 + 9.0),
            );
            let (hovered, picked) = zone_list(
                ui,
                exo_ui::id(ui.ctx(), "share-zone-list"),
                list,
                &self.zones,
                self.zone_pick,
                &ZoneRows::SIDEBAR,
            );
            self.zone_pick = hovered.unwrap_or(self.zone_pick);
            if let Some(index) = picked {
                picked_zone = self.zones.get(index).cloned();
            }
        }
        if response.lost_focus() && picked_zone.is_none() {
            self.zones.clear();
            // Leaving it: an emptied field takes the zone back.
            if self.zone_text.trim().is_empty() {
                self.zone_typed = false;
            }
        }
        if let Some(found) = picked_zone {
            if let Ok(tz) = TimeZone::get(&found.iana) {
                self.tz = tz;
            }
            self.zone_typed = false;
            self.zones.clear();
            self.blur = true;
        }
        top = zone_box.bottom() - 1.0 + ROW_GAP;

        // Preview: the text as it'll go in, kept as written (`white-space: pre-wrap`).
        let text = self.text(now);
        if !text.is_empty() {
            LABEL.paint_line(
                &painter,
                pos2(left, top),
                "Preview",
                palette.ink(0.35, 0.35),
                None,
            );
            let mut job =
                egui::text::LayoutJob::single_section(text, FIELD.format(palette.ink(0.55, 0.55)));
            job.wrap.max_width = width;
            job.wrap.break_anywhere = false;
            exo_ui::paint_job(
                &painter,
                pos2(left, top + LABEL.line_height + 1.0),
                job,
                &FIELD,
            );
        }

        // The footer: Insert, disabled until a slot can be shared, and Cancel.
        let buttons_top = sidebar.bottom() - FOOTER + 20.0 + 1.0;
        // A disabled Insert is unfilled in Carbon too.
        let insert = DialogAction::new("Insert", ActionKind::Primary)
            .enabled(self.can_insert(now))
            .disabled_fill(Color32::TRANSPARENT)
            .dark(carbon);
        let insert_rect = Rect::from_min_size(
            pos2(left, buttons_top),
            vec2(insert.width(&ctx), exo_ui::DIALOG_ACTION_HEIGHT),
        );
        if insert
            .show(ui, exo_ui::id(ui.ctx(), "share-insert"), insert_rect)
            .clicked()
        {
            actions.push(ShareAction::Insert);
        }
        let cancel = DialogAction::new("Cancel", ActionKind::Plain).dark(carbon);
        let cancel_rect = Rect::from_min_size(
            pos2(insert_rect.right() + BUTTON_GAP, buttons_top),
            vec2(cancel.width(&ctx), exo_ui::DIALOG_ACTION_HEIGHT),
        );
        if cancel
            .show(ui, exo_ui::id(ui.ctx(), "share-cancel"), cancel_rect)
            .clicked()
        {
            actions.push(ShareAction::Cancel);
        }

        // The durations' list, centered under its button, over the rest.
        if let Some(highlighted) = self.durations {
            let (hovered, picked, rect) = duration_list(ui, button, highlighted);
            self.durations = Some(hovered.unwrap_or(highlighted));
            if let Some(picked) = picked {
                self.minutes = DURATIONS[picked];
                self.durations = None;
            } else if ui.input(|input| input.pointer.any_pressed())
                && !ui.rect_contains_pointer(rect)
                && !opened.hovered()
            {
                self.durations = None;
            }
        }
        self.typing = [(who_id, Input::Who), (zone_id, Input::Zone)]
            .into_iter()
            .find(|(id, _)| ui.memory(|memory| memory.has_focus(*id)))
            .map(|(_, input)| input);
        actions
    }

    /// One of the text fields, on the line box at `rect`'s top; takes or lets go of the
    /// keyboard as asked.
    fn field(&mut self, ui: &mut Ui, id: Id, input: Input, rect: Rect) -> egui::Response {
        let palette = exo_ui::palette(ui.ctx());
        if self.focus == Some(input) {
            self.focus = None;
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        let line = Rect::from_min_size(
            rect.min + vec2(0.0, 1.0),
            vec2(rect.width(), FIELD.line_height),
        );
        let text = match input {
            Input::Who => &mut self.who,
            Input::Zone => &mut self.zone_text,
        };
        let color = if input == Input::Who && self.who_error {
            Color32::from_rgb(0xf6, 0x7b, 0x7b)
        } else {
            palette.ink(0.9, 0.9)
        };
        let length = text.chars().count();
        let response = exo_ui::text_field(
            ui,
            id,
            text,
            line,
            &FIELD,
            color,
            palette.text_selection_default,
        );
        // The Timezone input selects its text when it takes the keyboard.
        if input == Input::Zone && response.gained_focus() {
            exo_ui::focus_field(
                ui.ctx(),
                id,
                egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(length),
                ),
            );
        }
        if self.blur && self.typing == Some(input) {
            response.surrender_focus();
        }
        if self.typing == Some(input) || self.typing.is_none() {
            self.blur = false;
        }
        response
    }

    /// Adds `address` under Who: listed again if they were, else added; me first, as the
    /// organizer.
    fn add_attendee(&mut self, address: Address, team: &Option<String>) {
        if let Some(existing) = self
            .attendees
            .iter_mut()
            .find(|attendee| attendee.address.email.eq_ignore_ascii_case(&address.email))
        {
            existing.visible = true;
            return;
        }
        let internal = team.as_deref().is_some_and(|domain| {
            address
                .email
                .rsplit_once('@')
                .is_some_and(|(_, theirs)| theirs.eq_ignore_ascii_case(domain))
        });
        self.attendees.push(Attendee {
            address,
            person: Person {
                internal,
                organizer: false,
            },
            visible: true,
            default: false,
        });
    }
}

/// The team's domain, from the organizer's address.
fn right_domain(attendees: &[Attendee]) -> Option<String> {
    attendees
        .iter()
        .find(|attendee| attendee.person.organizer)
        .and_then(|me| crate::splits::team_domain(&me.address.email))
}

/// Slot `key`'s id.
fn slot_id(key: &str) -> Option<u64> {
    key.strip_prefix(SLOT)?.parse().ok()
}

/// A dropdown's box at `rect` over everything (a contact list's 2px corners and two shadows, or
/// the Timezone list's 4px and one), `rows` rows `row` tall from `pad` down, each painted by
/// `paint` (the highlighted one filled first). Returns the row under a moving pointer and the
/// row pressed.
#[expect(
    clippy::too_many_arguments,
    reason = "a list's box, its rows and how to paint them"
)]
fn dropdown(
    ui: &Ui,
    id: Id,
    rect: Rect,
    timezone: Option<Color32>,
    rows: usize,
    row: f32,
    pad: f32,
    highlighted: usize,
    mut paint: impl FnMut(&egui::Painter, Rect, usize),
) -> (Option<usize>, Option<usize>) {
    let ctx = ui.ctx().clone();
    let palette = exo_ui::palette(&ctx);
    let colors = exo_ui::ContactListColors::new(palette, false);
    let (pointer, pressed, moving) = ctx.input(|input| {
        (
            input.pointer.hover_pos(),
            input.pointer.primary_pressed(),
            input.pointer.is_moving(),
        )
    });
    let (mut hovered, mut picked) = (None, None);
    egui::Area::new(id)
        .order(egui::Order::Tooltip)
        .fixed_pos(rect.min)
        .constrain(false)
        .show(&ctx, |area| {
            let painter = area.painter().clone();
            if let Some(carbon) = timezone {
                // The Timezone list: `0 2px 8px rgba(0,0,0,.1)`, 4px corners.
                let snapped = exo_ui::pixel_snap(rect);
                exo_ui::BoxShadow::new([0.0, 2.0], 8.0, 0.0, Color32::from_black_alpha(26)).paint(
                    &painter,
                    snapped,
                    CornerRadius::same(4),
                );
                painter.rect_filled(
                    snapped,
                    CornerRadius::same(4),
                    if palette.theme == Theme::Carbon {
                        carbon
                    } else {
                        Color32::WHITE
                    },
                );
            } else {
                exo_ui::paint_contact_list_box(&painter, rect, colors.background);
            }
            let _ = area.allocate_rect(rect, Sense::click());
            for index in 0..rows {
                let line = Rect::from_min_size(
                    pos2(rect.left(), rect.top() + pad + index as f32 * row),
                    vec2(rect.width(), row),
                );
                if index == highlighted {
                    // The Timezone list's `#f2f4fa`, else `hsl(220, 45%, 97%)`; Carbon `#464b53`.
                    let fill = if timezone.is_some() && palette.theme == Theme::Snow {
                        Color32::from_rgb(0xf2, 0xf4, 0xfa)
                    } else {
                        colors.selected
                    };
                    painter.rect_filled(exo_ui::pixel_snap(line), 0.0, fill);
                }
                paint(&painter, line, index);
                if pointer.is_some_and(|at| line.contains(at)) {
                    if moving {
                        hovered = Some(index);
                    }
                    if pressed {
                        picked = Some(index);
                    }
                }
            }
        });
    (hovered, picked)
}

/// Who's suggestions: 24px wider than the field and centered under it, 8px down, padded 11px;
/// each row 50px, a 13px name over a 12px address 16px in, the typed text darker. Returns the
/// row under a moving pointer and the row pressed.
fn suggestion_list(
    ui: &Ui,
    field: Rect,
    suggestions: &[Address],
    query: &str,
    highlighted: usize,
) -> (Option<usize>, Option<usize>) {
    const NAME: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 13.0, 16.0);
    let colors = exo_ui::ContactListColors::new(exo_ui::palette(ui.ctx()), false);
    let rect = Rect::from_min_size(
        pos2(
            field.center().x - (field.width() + 24.0) / 2.0,
            field.bottom() + 8.0,
        ),
        vec2(field.width() + 24.0, suggestions.len() as f32 * 50.0 + 22.0),
    );
    dropdown(
        ui,
        exo_ui::id(ui.ctx(), "share-who-suggestions"),
        rect,
        None,
        suggestions.len(),
        50.0,
        11.0,
        highlighted,
        |painter, row, index| {
            let contact = &suggestions[index];
            let name = contact.name.as_deref().unwrap_or(&contact.email);
            exo_ui::paint_matched(
                painter,
                pos2(row.left() + 16.0, row.top() + 9.0),
                name,
                query,
                &NAME,
                colors.name.0,
                colors.name.1,
                row.width() - 32.0,
            );
            exo_ui::paint_matched(
                painter,
                pos2(row.left() + 16.0, row.top() + 29.0),
                &contact.email,
                query,
                &FIELD,
                colors.email.0,
                colors.email.1,
                row.width() - 32.0,
            );
        },
    )
}

/// How a Timezone input's list lays out its rows: their height, the padding over the first and
/// under the last, where each row's 12px/16px text sits, and the list's Carbon fill.
pub(super) struct ZoneRows {
    pub row: f32,
    pub pad: f32,
    pub bottom: f32,
    pub text_top: f32,
    pub carbon: Color32,
}

impl ZoneRows {
    /// Share Availability's and Calendar Settings': 32px rows from 4px down, on `#27292d`.
    pub const SIDEBAR: Self = Self {
        row: 32.0,
        pad: 4.0,
        bottom: 5.0,
        text_top: 8.0,
        carbon: Color32::from_rgb(0x27, 0x29, 0x2d),
    };
    /// The event form's When (12px lines nudged `top: 3px`): 28px rows from 6px down, on the
    /// form's `#27292d`.
    pub const EDIT_EVENT: Self = Self {
        row: 28.0,
        pad: 6.0,
        bottom: 3.0,
        text_top: 6.0,
        carbon: Color32::from_rgb(0x27, 0x29, 0x2d),
    };
}

/// A Timezone input's list at `rect` (Share Availability's 276px from 7px left of the field and
/// 12px under it; Calendar Settings' 286px from 16px left and 4px under; the form's 296px):
/// `rows` of a zone's offset (80px wide), name and, on the right, abbreviation (left out when
/// it's an offset). Returns the row under a moving pointer and the row pressed.
pub(super) fn zone_list(
    ui: &Ui,
    id: Id,
    rect: Rect,
    zones: &[Found],
    highlighted: usize,
    rows: &ZoneRows,
) -> (Option<usize>, Option<usize>) {
    let ctx = ui.ctx().clone();
    let palette = exo_ui::palette(&ctx);
    dropdown(
        ui,
        id,
        rect,
        Some(rows.carbon),
        zones.len(),
        rows.row,
        rows.pad,
        highlighted,
        |painter, row, index| {
            let zone = &zones[index];
            let line = row.top() + rows.text_top;
            FIELD.paint_line(
                painter,
                pos2(row.left() + 15.0, line),
                &zone.offset,
                palette.ink(0.4, 0.4),
                None,
            );
            FIELD.paint_line(
                painter,
                pos2(row.left() + 95.0, line),
                &zone.name,
                palette.ink(0.9, 0.9),
                Some(row.width() - 95.0 - 60.0),
            );
            if !zone.abbreviation.chars().any(|ch| ch.is_ascii_digit()) {
                let width = FIELD.width(&ctx, &zone.abbreviation);
                FIELD.paint_line(
                    painter,
                    pos2(row.right() - 15.0 - width, line),
                    &zone.abbreviation,
                    palette.ink(0.9, 0.9),
                    None,
                );
            }
        },
    )
}

/// The durations' list: 272px, centered 8px under the button, 28px rows from 4px down, 12px at
/// `.7`. Returns the row under a moving pointer and the row pressed.
fn duration_list(
    ui: &Ui,
    button: Rect,
    highlighted: usize,
) -> (Option<usize>, Option<usize>, Rect) {
    const ITEM: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 28.0);
    let palette = exo_ui::palette(ui.ctx());
    let rect = Rect::from_min_size(
        pos2(button.center().x - 136.0, button.bottom() + 8.0),
        vec2(272.0, DURATIONS.len() as f32 * 28.0 + 8.0),
    );
    let (hovered, picked) = dropdown(
        ui,
        exo_ui::id(ui.ctx(), "share-durations"),
        rect,
        None,
        DURATIONS.len(),
        28.0,
        4.0,
        highlighted,
        |painter, row, index| {
            ITEM.paint_line(
                painter,
                pos2(row.left() + 16.0, row.top()),
                &format!("{} mins", DURATIONS[index]),
                palette.ink(0.7, 0.7),
                None,
            );
        },
    );
    (hovered, picked, rect)
}

impl crate::Mail {
    /// Share Availability: the week view on today, picking free times for `target`. Who lists me
    /// and the draft's (or the reply's) teammates; outside recipients aren't listed but count
    /// for the wording.
    pub(crate) fn calendar_share(&mut self, target: Target, now: &Zoned) {
        let me = self.account.email().to_owned();
        let from = self
            .snapshot
            .aliases
            .iter()
            .find(|alias| alias.primary)
            .map(|alias| Address {
                name: alias.name.clone(),
                email: alias.email.clone(),
            })
            .unwrap_or(Address {
                name: None,
                email: me.clone(),
            });
        let team = crate::splits::team_domain(&me);
        let (organizer, recipients) = self.compose_reply_fields(&target);
        let organizer = organizer.unwrap_or(from);
        let mut attendees = vec![Attendee {
            address: organizer,
            person: Person {
                internal: true,
                organizer: true,
            },
            visible: true,
            default: true,
        }];
        for address in recipients {
            if address.email.eq_ignore_ascii_case(&me)
                || crate::format::names::is_role_based(&address.email)
                || attendees
                    .iter()
                    .any(|attendee| attendee.address.email.eq_ignore_ascii_case(&address.email))
            {
                continue;
            }
            let internal = team.as_deref().is_some_and(|domain| {
                address
                    .email
                    .rsplit_once('@')
                    .is_some_and(|(_, theirs)| theirs.eq_ignore_ascii_case(domain))
            });
            attendees.push(Attendee {
                address,
                person: Person {
                    internal,
                    organizer: false,
                },
                visible: internal,
                default: true,
            });
        }
        self.calendar.details = None;
        self.calendar_open_week(now.date());
        if let Some(week) = &mut self.calendar.week {
            week.share = Some(Share::new(target, attendees, now.time_zone().clone()));
        }
    }

    /// What `Share Availability` from the mail views is for: the open or highlighted
    /// conversation's draft open in compose, else a Reply All in it; from the week, a new
    /// message.
    pub(super) fn calendar_share_target(&self) -> Target {
        if self.calendar.week.is_some() {
            return Target::New;
        }
        let thread = self
            .thread
            .as_ref()
            .map(|view| view.id().to_owned())
            .or_else(|| self.inbox.cursor().map(|row| row.id.clone()));
        match thread {
            Some(thread) => self
                .compose_thread_draft(&thread)
                .map_or(Target::Reply(thread), Target::Draft),
            None => Target::New,
        }
    }

    /// What the sidebar asked for.
    pub(super) fn calendar_share_action(&mut self, action: ShareAction, now: &Zoned) {
        match action {
            ShareAction::Insert => self.calendar_share_insert(now),
            ShareAction::Cancel => self.calendar.week = None,
            ShareAction::Say(text) => self
                .commands
                .toast(exo_commands::Toast::new(text).kind(exo_ui::ToastKind::Error)),
        }
    }

    /// Insert: the text, its lines as `<br>`s, into the draft, and the week view gone.
    pub(super) fn calendar_share_insert(&mut self, now: &Zoned) {
        let Some(share) = self
            .calendar
            .week
            .as_ref()
            .and_then(|week| week.share.as_ref())
        else {
            return;
        };
        if !share.can_insert(now) {
            return;
        }
        let html = format!("{} <br>", share.text(now).replace('\n', "<br>"));
        let target = share.target.clone();
        self.calendar.week = None;
        self.compose_insert_availability(&target, &html);
    }

    /// A press on the grid while sharing: a slot, a slot moved, sized or removed.
    pub(super) fn calendar_share_grid(
        &mut self,
        action: super::grid::GridAction,
        now: &Zoned,
    ) -> bool {
        use super::grid::GridAction;
        let Some(share) = self
            .calendar
            .week
            .as_mut()
            .and_then(|week| week.share.as_mut())
        else {
            return false;
        };
        match action {
            GridAction::Slot {
                start,
                end,
                all_day: false,
            } => share.add(start, end),
            GridAction::Change { key, start, end } if key.starts_with(SLOT) => {
                share.change(&key, start, end, now)
            }
            GridAction::Remove(key) => share.remove(&key),
            GridAction::Select { key, .. } if key.starts_with(SLOT) => {}
            GridAction::Past => self.commands.toast(
                exo_commands::Toast::new("Slots cannot be added in the past.")
                    .kind(exo_ui::ToastKind::Error),
            ),
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    fn at(day: i8, hour: i8, minute: i8) -> Zoned {
        date(2026, 11, day)
            .at(hour, minute, 0, 0)
            .to_zoned(TimeZone::get("America/New_York").expect("zone"))
            .expect("exists")
    }

    fn share() -> Share {
        let me = Attendee {
            address: Address {
                name: Some("Maya Okafor".into()),
                email: "maya@brightmoor.co".into(),
            },
            person: Person {
                internal: true,
                organizer: true,
            },
            visible: true,
            default: true,
        };
        Share::new(
            Target::New,
            vec![me],
            TimeZone::get("America/New_York").expect("zone"),
        )
    }

    fn spans(share: &Share) -> Vec<String> {
        share
            .slots
            .values()
            .map(|(start, end)| format!("{}–{}", start.strftime("%d %H:%M"), end.strftime("%H:%M")))
            .collect()
    }

    #[test]
    fn slots_touching_on_a_day_merge() {
        let mut share = share();
        share.add(at(18, 13, 0), at(18, 13, 30));
        share.add(at(18, 13, 30), at(18, 14, 0));
        share.add(at(19, 9, 0), at(19, 9, 30));
        assert_eq!(spans(&share), ["18 13:00–14:00", "19 09:00–09:30"]);
    }

    #[test]
    fn short_and_past_slots_dont_count() {
        let now = at(17, 9, 0);
        let mut share = share();
        share.add(at(18, 13, 0), at(18, 13, 15));
        assert!(!share.can_insert(&now), "shorter than the meeting");
        let first = |share: &Share, now: &Zoned| {
            share
                .events(CalendarColor::BLUE, now)
                .first()
                .map(|event| (event.share, event.tooltip))
        };
        assert_eq!(
            first(&share, &now),
            Some((
                Some(false),
                Some("This slot is shorter than the event duration.")
            ))
        );
        share.minutes = 15;
        assert!(share.can_insert(&now));
        assert_eq!(
            first(&share, &at(18, 14, 0)),
            Some((Some(false), Some("This slot is in the past.")))
        );
        assert_eq!(
            share.text(&now),
            "Does  tomorrow (Nov 18) work? I can do 1:00–1:15 pm EST."
        );
    }

    #[test]
    fn moved_slots_stay_on_their_day_and_after_now() {
        let now = at(17, 9, 5);
        let mut share = share();
        share.add(at(18, 13, 0), at(18, 14, 0));
        let key = format!("{SLOT}1");
        // Past midnight, it stops at the day's end.
        share.change(&key, at(18, 23, 30), at(19, 0, 30), &now);
        assert_eq!(spans(&share), ["18 23:00–00:00"]);
        // Before now, it starts at the next quarter hour.
        share.change(&key, at(17, 7, 0), at(17, 8, 0), &now);
        assert_eq!(spans(&share), ["17 09:15–10:15"]);
    }

    #[test]
    fn escape_clears_a_field_before_closing() {
        let mut share = share();
        share.who = "pri".into();
        assert!(!share.escape());
        assert!(share.who.is_empty());
        assert!(share.escape(), "an empty field closes the page");
    }
}
