//! An event's When while it's edited. Its first row has the start's date picker, the start and
//! end times (and the end's date when the event ends on another day; an all-day event's first and
//! last days instead) and `All day` with its switch; its second the time zone the times are in.
//!
//! A time is typed or picked from a list of quarter hours; a date is typed or picked from the
//! month under it.

use egui::{Color32, CornerRadius, Id, Rect, Sense, Ui, pos2, vec2};
use exo_ui::{DateInput, DatePick, DatePicker, Family, Font, Scrollbar, TextStyle, Theme};
use jiff::{
    ToSpan, Zoned,
    civil::{Date, Weekday},
    tz::TimeZone,
};

use super::timezones::{ZoneField, ZoneInput};

/// The field's two rows, 20px each and 12px apart (`gap: 12px`).
pub const HEIGHT: f32 = 20.0 + ROW_GAP + 20.0;
const ROW_GAP: f32 = 12.0;
/// The inputs: 12px/16px nudged `top: 1px` over `padding-bottom: 2px` and a 2px border,
/// transparent until focused.
const INPUT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const UNDERLINE: f32 = 2.0;
/// The separator, `All day` and the lists' items: 12px on 12px lines nudged `top: 3px`, which
/// sits them 2px under an input's text.
const SMALL_LINE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 12.0);
const SMALL_NUDGE: f32 = 2.0;
/// The start's date picker is 80px wide; the inputs size to their text (`field-sizing: content`)
/// within `min-width: 20px` and their `max-width`s (72px for a date, 52px for a time).
const START_DATE_BOX: f32 = 80.0;
const MIN_INPUT: f32 = 20.0;
const MAX_DATE: f32 = 72.0;
const MAX_TIME: f32 = 52.0;
/// Between the range's times and dates: `column-gap: 6px`.
const RANGE_GAP: f32 = 6.0;
/// Between `All day` and its switch: `column-gap: 18px`, the switch last in the row.
const ALL_DAY_GAP: f32 = 18.0;
/// The time zone input is 266px wide; its list 296px.
const ZONE_WIDTH: f32 = 266.0;
const ZONE_LIST_WIDTH: f32 = 296.0;
const ZONES: usize = 5;
/// The time lists (the start's 80px wide, the end's 160px; `max-height: 168px`): 16px left of the
/// input and 12px under its box; 32px items (`padding: 10px`) from 6px down (the first's
/// `margin-top: 3px` and every one's `top: 3px`), 6px under the last; the end's labelled 16px in
/// from each side.
const START_LIST: f32 = 80.0;
const END_LIST: f32 = 160.0;
const LIST_MAX: f32 = 168.0;
const LIST_LEFT: f32 = 16.0;
const LIST_BELOW: f32 = 12.0;
const ITEM: f32 = 32.0;
const ITEM_TOP: f32 = 6.0;
const LIST_BOTTOM: f32 = 3.0;
const ITEM_TEXT: f32 = 10.0;
const END_PADDING: f32 = 16.0;
/// `#aeb1dd` focused, `#f67b7b` wrong: Classic literals both themes keep.
const FOCUSED: Color32 = Color32::from_rgb(0xae, 0xb1, 0xdd);
const WRONG: Color32 = Color32::from_rgb(0xf6, 0x7b, 0x7b);
/// The typed time's formats, tried in this order.
const FORMATS: [&str; 5] = ["hmm", "hmma", "ha", "HHmm", "HH"];

/// A part of the field, in its tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    StartDate,
    StartTime,
    EndTime,
    EndDate,
    AllDay,
    Zone,
}

impl Part {
    fn id(self, ctx: &egui::Context) -> Id {
        exo_ui::id(ctx, ("calendar-when-edit", self as u8))
    }
}

/// A time input and its list.
#[derive(Default)]
struct TimeInput {
    text: String,
    /// The list shows, with its highlighted row and scroll.
    open: bool,
    highlighted: usize,
    scroll: f32,
    scrollbar: Scrollbar,
}

/// A change the field asks of the form.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// New times.
    Times(Zoned, Zoned),
    /// The all-day switch.
    AllDay,
    /// The times' zone: the same clock times there.
    Zone(TimeZone),
}

/// The field's state.
#[derive(Default)]
pub struct WhenEdit {
    start_date: DateInput,
    end_date: DateInput,
    start_time: TimeInput,
    end_time: TimeInput,
    zone: ZoneInput,
    /// The times the inputs were written for.
    written: Option<(Zoned, Zoned, bool)>,
    /// A part to focus on the next frame, and the one that had the keyboard last frame.
    focus: Option<Part>,
    focused: Option<Part>,
}

/// What the field shows besides itself.
pub struct Shown<'a> {
    pub start: &'a Zoned,
    pub end: &'a Zoned,
    pub all_day: bool,
    pub today: Date,
    /// The weekday weeks start on, for the months.
    pub week_start: Weekday,
}

impl WhenEdit {
    /// Takes the keyboard to `part` on the next frame.
    pub fn focus(&mut self, part: Part) {
        self.focus = Some(part);
    }

    /// Whether a list or month is open, which Escape closes first.
    pub fn has_dropdown(&self) -> bool {
        self.start_time.open
            || self.end_time.open
            || self.start_date.month.is_some()
            || self.end_date.month.is_some()
    }

    /// Escape: closes the open list or month.
    pub fn close_dropdown(&mut self) {
        self.start_time.open = false;
        self.end_time.open = false;
        self.start_date.month = None;
        self.end_date.month = None;
    }

    /// Writes the inputs out for these times unless they were written for them already.
    fn sync(&mut self, shown: &Shown<'_>) {
        if self.written.as_ref().is_some_and(|(start, end, all_day)| {
            start == shown.start
                && end == shown.end
                && start.time_zone() == shown.start.time_zone()
                && *all_day == shown.all_day
        }) {
            return;
        }
        self.start_date = DateInput::showing(Some(shown.start.date()), shown.today);
        self.end_date = DateInput::showing(Some(shown_end_date(shown)), shown.today);
        self.start_time.text = time_text(shown.start);
        self.end_time.text = time_text(shown.end);
        self.written = Some((shown.start.clone(), shown.end.clone(), shown.all_day));
    }

    /// Paints the field with its first input's text at `at`'s top left, `width` wide, and
    /// returns the changes asked for and whether it has the keyboard.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        at: egui::Pos2,
        width: f32,
        shown: &Shown<'_>,
    ) -> (Vec<Change>, bool) {
        self.sync(shown);
        let ctx = ui.ctx().clone();
        let palette = exo_ui::palette(&ctx);
        let ink = palette.ink(0.9, 0.85);
        let focus = if ui.is_sizing_pass() || !ui.is_enabled() {
            None
        } else {
            self.focus.take()
        };
        if let Some(part) = focus {
            let id = part.id(&ctx);
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        let (enter, tab, shift) = ui.input(|input| {
            (
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Tab),
                input.modifiers.shift,
            )
        });
        let tabbed_from = [
            Part::StartDate,
            Part::StartTime,
            Part::EndTime,
            Part::EndDate,
            Part::AllDay,
            Part::Zone,
        ]
        .into_iter()
        .find(|part| tab && ui.memory(|memory| memory.focused()) == Some(part.id(&ctx)));
        let mut changes = Vec::new();
        let mut focused = None;
        let mut next = None;
        let same_day = shown.start.date() == shown.end.date();

        let mut x = at.x;
        let start_date = self.date_input(
            ui,
            Part::StartDate,
            at.x,
            at.y,
            shown,
            focus == Some(Part::StartDate),
        );
        x += START_DATE_BOX;
        if start_date.has_focus {
            focused = Some(Part::StartDate);
        }
        if let Some(date) = start_date.picked {
            // The start moves to that day, the end with it.
            if let Some(change) = moved_start(shown, date) {
                changes.push(change);
            }
            next = next.or(start_date.advance.then_some(Part::StartDate));
        }
        if shown.all_day {
            x = separator(
                ui,
                x - START_DATE_BOX + start_date.width + RANGE_GAP,
                at.y,
                ink,
            );
            let end_date = self.date_input(
                ui,
                Part::EndDate,
                x,
                at.y,
                shown,
                focus == Some(Part::EndDate),
            );
            if end_date.has_focus {
                focused = Some(Part::EndDate);
            }
            if let Some(date) = end_date.picked {
                // The last day, so the end is the day after it.
                if let Some(end) = date
                    .tomorrow()
                    .ok()
                    .and_then(|day| day.to_zoned(shown.start.time_zone().clone()).ok())
                {
                    changes.push(Change::Times(shown.start.clone(), end));
                }
                next = next.or(end_date.advance.then_some(Part::EndDate));
            }
        } else {
            for part in [Part::StartTime, Part::EndTime] {
                let time = self.time_input(ui, part, x, at.y, shown, focus == Some(part));
                if time.has_focus {
                    focused = Some(part);
                }
                if let Some(at) = time.picked {
                    changes.push(match part {
                        // The end keeps the event's length.
                        Part::StartTime => Change::Times(
                            at.clone(),
                            at.checked_add(
                                shown
                                    .end
                                    .timestamp()
                                    .duration_since(shown.start.timestamp()),
                            )
                            .unwrap_or_else(|_| shown.end.clone()),
                        ),
                        _ => Change::Times(shown.start.clone(), at),
                    });
                    next = next.or(time.advance.then_some(part));
                }
                x += time.width;
                if part == Part::StartTime {
                    x = separator(ui, x + RANGE_GAP, at.y, ink);
                } else {
                    x += RANGE_GAP;
                }
            }
            if !same_day {
                let end_date = self.date_input(
                    ui,
                    Part::EndDate,
                    x,
                    at.y,
                    shown,
                    focus == Some(Part::EndDate),
                );
                if end_date.has_focus {
                    focused = Some(Part::EndDate);
                }
                if let Some(date) = end_date.picked {
                    // The end on that day at its time, at least 15 minutes after the start.
                    let end = date
                        .to_datetime(shown.end.time())
                        .to_zoned(shown.end.time_zone().clone())
                        .ok();
                    let end = end.map(|end| {
                        if end < *shown.start {
                            shown.start.checked_add(15.minutes()).unwrap_or(end)
                        } else {
                            end
                        }
                    });
                    if let Some(end) = end {
                        changes.push(Change::Times(shown.start.clone(), end));
                    }
                    next = next.or(end_date.advance.then_some(Part::EndDate));
                }
            }
        }

        // `All day` and its switch, at the row's end.
        let right = at.x + width;
        let switch = Rect::from_min_size(
            pos2(right - exo_ui::SLIDER_SIZE.x, at.y - 1.0),
            exo_ui::SLIDER_SIZE,
        );
        let label_width = SMALL_LINE.width(&ctx, "All day");
        let label = Rect::from_min_size(
            pos2(
                switch.left() - ALL_DAY_GAP - label_width,
                at.y + SMALL_NUDGE,
            ),
            vec2(label_width, SMALL_LINE.line_height),
        );
        let toggle = ui
            .interact(
                Rect::from_min_max(label.min, switch.max),
                Part::AllDay.id(&ctx),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if focus == Some(Part::AllDay) {
            toggle.request_focus();
        }
        SMALL_LINE.paint_line(ui.painter(), label.min, "All day", ink, None);
        if toggle.has_focus() {
            // Focused: its border, 2px under its text.
            ui.painter().rect_filled(
                Rect::from_min_size(
                    pos2(label.left(), label.bottom() + 2.0),
                    vec2(label.width(), UNDERLINE),
                ),
                0.0,
                FOCUSED,
            );
            focused = Some(Part::AllDay);
        }
        exo_ui::paint_slider(
            ui.painter(),
            switch,
            shown.all_day,
            false,
            Part::AllDay.id(&ctx),
            exo_ui::SliderColors::CLASSIC,
        );
        if toggle.clicked()
            || (toggle.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Space)))
        {
            changes.push(Change::AllDay);
        }

        // The time zone, on the second row.
        let zone_line = Rect::from_min_size(
            pos2(at.x, at.y + 20.0 + ROW_GAP),
            vec2(ZONE_WIDTH, INPUT.line_height),
        );
        let iana = shown.start.time_zone().iana_name().map(str::to_owned);
        let field = ZoneField {
            id: Part::Zone.id(&ctx),
            line: zone_line,
            zone: iana.as_deref(),
            now: shown.start.timestamp(),
            limit: ZONES,
            ink,
            placeholder: palette.text_base_accent,
            always_ruled: false,
            blur_on_pick: false,
            list_width: ZONE_LIST_WIDTH,
            rows: super::share::ZoneRows::EDIT_EVENT,
        };
        let zone = self.zone.show(ui, &field);
        if zone.focused {
            focused = Some(Part::Zone);
        }
        if let Some(tz) = zone.picked.and_then(|iana| TimeZone::get(&iana).ok()) {
            changes.push(Change::Zone(tz));
        }

        // A part taken with Enter hands the keyboard on, as Tab does from any part but the last
        // (which egui hands to the form's next field).
        if let Some(part) = next.filter(|_| enter) {
            self.focus = Some(self.after(part, shown));
        }
        match tabbed_from {
            Some(part) if shift => self.focus = self.before(part, shown),
            Some(part) if part != Part::Zone => self.focus = Some(self.after(part, shown)),
            _ => {}
        }
        self.focused = focused;
        (changes, focused.is_some())
    }

    /// The part after `part` in the tab order; past the last, the zone keeps it.
    fn after(&self, part: Part, shown: &Shown<'_>) -> Part {
        let order = order(shown);
        order
            .iter()
            .position(|each| *each == part)
            .and_then(|at| order.get(at + 1))
            .copied()
            .unwrap_or(Part::Zone)
    }

    /// The part before `part`; none before the first (egui hands the keyboard to the title).
    fn before(&self, part: Part, shown: &Shown<'_>) -> Option<Part> {
        let order = order(shown);
        order
            .iter()
            .position(|each| *each == part)
            .and_then(|at| at.checked_sub(1))
            .map(|at| order[at])
    }

    /// A date input at `x`, its text's line `top`: the start's (`Part::StartDate`) or the
    /// end's, its month while it has the keyboard.
    fn date_input(
        &mut self,
        ui: &mut Ui,
        part: Part,
        x: f32,
        top: f32,
        shown: &Shown<'_>,
        focus: bool,
    ) -> Taken<Date> {
        let ctx = ui.ctx().clone();
        let palette = exo_ui::palette(&ctx);
        let id = part.id(&ctx);
        let (min, current) = match part {
            Part::StartDate => (None, shown.start.date()),
            _ => (Some(shown.start.date()), shown_end_date(shown)),
        };
        let input = if part == Part::StartDate {
            &mut self.start_date
        } else {
            &mut self.end_date
        };
        let width = INPUT.width(&ctx, &input.text).clamp(MIN_INPUT, MAX_DATE);
        let line = Rect::from_min_size(pos2(x, top), vec2(width, INPUT.line_height));
        let keys = ui.input(|input| {
            (
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Tab),
                input.key_pressed(egui::Key::ArrowDown),
            )
        });
        let had_focus = ui.memory(|memory| memory.has_focus(id));
        let before = input.text.clone();
        let response = exo_ui::text_field(
            ui,
            id,
            &mut input.text,
            line,
            &INPUT,
            palette.ink(0.9, 0.85),
            palette.text_selection_default,
        );
        let mut taken = Taken {
            width,
            has_focus: response.has_focus(),
            picked: None,
            advance: false,
        };
        if response.gained_focus() || focus {
            // Its text selected, and the month opens.
            exo_ui::focus_field(
                &ctx,
                id,
                egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(input.text.chars().count()),
                ),
            );
            input.month = Some(input.parsed.unwrap_or(current).first_of_month());
        }
        if response.clicked() && input.month.is_none() {
            input.month = Some(input.parsed.unwrap_or(current).first_of_month());
        }
        if input.text != before {
            input.typed(current, shown.today, min, false);
        }
        if had_focus {
            if keys.2 && input.month.is_none() {
                input.month = Some(input.parsed.unwrap_or(current).first_of_month());
            } else if (keys.0 || keys.1) && !input.error {
                // Enter or Tab takes what's typed.
                taken.picked = input.parsed.filter(|date| *date != current);
                taken.advance = true;
                input.month = None;
                if taken.picked.is_none() {
                    *input = DateInput::showing(Some(current), shown.today);
                }
            }
        }
        if input.error || response.has_focus() {
            let color = if input.error { WRONG } else { FOCUSED };
            ui.painter().rect_filled(
                Rect::from_min_size(
                    pos2(line.left(), line.bottom() + 2.0),
                    vec2(width, UNDERLINE),
                ),
                0.0,
                color,
            );
        }
        if let Some(mut month) = input.month.filter(|_| response.has_focus() || had_focus) {
            // Tethered to the input's box: its line, padding and border.
            let picker = DatePicker {
                id: id.with("month"),
                month: &mut month,
                selected: input.parsed,
                today: shown.today,
                min,
                week_start: shown.week_start,
            };
            match picker.show(
                &ctx,
                pos2(line.left(), line.bottom() + 2.0 + UNDERLINE) + exo_ui::DATE_PICKER_OFFSET,
            ) {
                Some(DatePick::Picked(day)) => {
                    // The month closes, and the next part takes the keyboard.
                    input.month = None;
                    taken.picked = (day != current).then_some(day);
                    taken.advance = true;
                    self.focus = Some(self.after(part, shown));
                    let input = if part == Part::StartDate {
                        &mut self.start_date
                    } else {
                        &mut self.end_date
                    };
                    *input = DateInput::showing(Some(day), shown.today);
                }
                Some(DatePick::MonthChanged) => {
                    input.month = Some(month);
                    ui.memory_mut(|memory| memory.request_focus(id));
                }
                None => {}
            }
        }
        let input = if part == Part::StartDate {
            &mut self.start_date
        } else {
            &mut self.end_date
        };
        if response.lost_focus() && !ui.memory(|memory| memory.has_focus(id)) {
            // Leaving it closes the month; what's typed and not taken goes back.
            input.month = None;
            if taken.picked.is_none() && !input.error {
                *input = DateInput::showing(Some(current), shown.today);
            }
        }
        taken
    }

    /// A time input at `x`: the start's or the end's, with its list while it has the keyboard.
    fn time_input(
        &mut self,
        ui: &mut Ui,
        part: Part,
        x: f32,
        top: f32,
        shown: &Shown<'_>,
        focus: bool,
    ) -> Taken<Zoned> {
        let ctx = ui.ctx().clone();
        let palette = exo_ui::palette(&ctx);
        let id = part.id(&ctx);
        let current = if part == Part::StartTime {
            shown.start
        } else {
            shown.end
        };
        let options = time_options(part, shown);
        let time = if part == Part::StartTime {
            &mut self.start_time
        } else {
            &mut self.end_time
        };
        let width = INPUT.width(&ctx, &time.text).clamp(MIN_INPUT, MAX_TIME);
        let line = Rect::from_min_size(pos2(x, top), vec2(width, INPUT.line_height));
        let keys = ui.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Tab),
            )
        });
        let had_focus = ui.memory(|memory| memory.has_focus(id));
        let before = time.text.clone();
        let response = exo_ui::text_field(
            ui,
            id,
            &mut time.text,
            line,
            &INPUT,
            palette.ink(0.9, 0.85),
            palette.text_selection_default,
        );
        let mut taken = Taken {
            width,
            has_focus: response.has_focus(),
            picked: None,
            advance: false,
        };
        // What's typed filters the list to the times starting with it, unless it's the current
        // time's.
        let typed = time.text != time_text(current);
        let listed: Vec<&(String, String, Zoned)> = options
            .iter()
            .filter(|(label, _, _)| !typed || label.starts_with(&time.text.to_uppercase()))
            .collect();
        if response.gained_focus() || focus {
            // Its text selected, the list open on the current time at its top.
            exo_ui::focus_field(
                &ctx,
                id,
                egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(time.text.chars().count()),
                ),
            );
            time.open = true;
            time.highlighted = listed
                .iter()
                .position(|(_, _, at)| at == current)
                .unwrap_or(0);
            time.scroll = scroll_to(time.highlighted, listed.len());
        }
        if time.text != before {
            time.open = true;
            time.highlighted = 0;
            time.scroll = 0.0;
        }
        if had_focus && time.open && !listed.is_empty() {
            if keys.0 {
                time.highlighted = time.highlighted.saturating_sub(1);
            } else if keys.1 {
                time.highlighted = (time.highlighted + 1).min(listed.len() - 1);
            }
            if keys.0 || keys.1 {
                // Keep it in view.
                let item = ITEM_TOP + time.highlighted as f32 * ITEM;
                let view = list_height(listed.len());
                time.scroll = time.scroll.clamp(item + ITEM - view, item);
            }
        }
        let valid = valid_time(&time.text);
        if had_focus && (keys.2 || keys.3) {
            // Enter or Tab: the highlighted time, else what's typed when it reads as one.
            let picked = if time.open && !listed.is_empty() {
                listed.get(time.highlighted).map(|(_, _, at)| at.clone())
            } else {
                parse_time(&time.text, shown.start, part == Part::EndTime)
            };
            if let Some(at) = picked {
                taken.picked = Some(at);
                taken.advance = true;
                time.open = false;
            }
        }
        if !valid || response.has_focus() {
            let color = if valid { FOCUSED } else { WRONG };
            ui.painter().rect_filled(
                Rect::from_min_size(
                    pos2(line.left(), line.bottom() + 2.0),
                    vec2(width, UNDERLINE),
                ),
                0.0,
                color,
            );
        }
        if time.open && response.has_focus() && !listed.is_empty() {
            let list_width = if part == Part::StartTime {
                START_LIST
            } else {
                END_LIST
            };
            let rect = Rect::from_min_size(
                pos2(
                    line.left() - LIST_LEFT,
                    line.bottom() + 2.0 + UNDERLINE + LIST_BELOW,
                ),
                vec2(list_width, list_height(listed.len())),
            );
            if let Some(index) = paint_list(ui, id.with("list"), rect, part, &listed, time) {
                taken.picked = Some(listed[index].2.clone());
                time.open = false;
                self.focus = Some(self.after(part, shown));
            }
        }
        let time = if part == Part::StartTime {
            &mut self.start_time
        } else {
            &mut self.end_time
        };
        if response.lost_focus() {
            time.open = false;
            // Leaving it: what's typed and differs is taken when it reads as a time.
            if taken.picked.is_none() && time.text != time_text(current) {
                taken.picked = parse_time(&time.text, shown.start, part == Part::EndTime);
            }
        }
        if let Some(at) = &taken.picked {
            time.text = time_text(at);
        }
        taken
    }
}

/// The parts shown, in tab order.
fn order(shown: &Shown<'_>) -> &'static [Part] {
    if shown.all_day {
        &[Part::StartDate, Part::EndDate, Part::AllDay, Part::Zone]
    } else if shown.start.date() == shown.end.date() {
        &[
            Part::StartDate,
            Part::StartTime,
            Part::EndTime,
            Part::AllDay,
            Part::Zone,
        ]
    } else {
        &[
            Part::StartDate,
            Part::StartTime,
            Part::EndTime,
            Part::EndDate,
            Part::AllDay,
            Part::Zone,
        ]
    }
}

/// What a part took this frame.
struct Taken<T> {
    /// Its input's width.
    width: f32,
    has_focus: bool,
    picked: Option<T>,
    /// It was taken with Enter or Tab (not by leaving it), so the keyboard moves on.
    advance: bool,
}

/// The day an all-day event's end date shows (its last day; Calendar's end is the day after),
/// or a timed one's end day.
fn shown_end_date(shown: &Shown<'_>) -> Date {
    if shown.all_day {
        shown.end.date().yesterday().unwrap_or(shown.end.date())
    } else {
        shown.end.date()
    }
}

/// The start moved to `date` at its time, the end keeping the event's length; an all-day
/// event's end keeping its number of days.
fn moved_start(shown: &Shown<'_>, date: Date) -> Option<Change> {
    let tz = shown.start.time_zone().clone();
    let start = date.to_datetime(shown.start.time()).to_zoned(tz).ok()?;
    let end = if shown.all_day {
        let days = shown.start.date().until(shown.end.date()).ok()?.get_days();
        start.checked_add(i64::from(days).days()).ok()?
    } else {
        start
            .checked_add(
                shown
                    .end
                    .timestamp()
                    .duration_since(shown.start.timestamp()),
            )
            .ok()?
    };
    Some(Change::Times(start, end))
}

/// The `–` between two inputs, `RANGE_GAP` before the next; returns where that starts.
fn separator(ui: &Ui, x: f32, top: f32, ink: Color32) -> f32 {
    x + SMALL_LINE.paint_line(ui.painter(), pos2(x, top + SMALL_NUDGE), "–", ink, None) + RANGE_GAP
}

/// A time as the inputs show it: `9:00 AM`, in the time's own zone.
fn time_text(at: &Zoned) -> String {
    at.strftime("%-I:%M %p").to_string()
}

/// A list's options: label, duration label and time. The start's are its day's quarter hours;
/// the end's, on the start's day, the 24 hours after the start labelled with the length
/// (`30 mins`, `1 hour`, `1.5 hours`), else its own day's quarter hours.
fn time_options(part: Part, shown: &Shown<'_>) -> Vec<(String, String, Zoned)> {
    let quarters = |day: Date, tz: &TimeZone| -> Vec<(String, String, Zoned)> {
        (0..96)
            .filter_map(|quarter| {
                day.at((quarter / 4) as i8, ((quarter % 4) * 15) as i8, 0, 0)
                    .to_zoned(tz.clone())
                    .ok()
            })
            .map(|at| (time_text(&at), String::new(), at))
            .collect()
    };
    match part {
        Part::StartTime => quarters(shown.start.date(), shown.start.time_zone()),
        _ if shown.start.date() == shown.end.date() => (1..=96)
            .filter_map(|quarter| shown.start.checked_add((15 * quarter).minutes()).ok())
            .map(|at| {
                let minutes = at
                    .timestamp()
                    .duration_since(shown.start.timestamp())
                    .as_mins();
                let length = if minutes < 60 {
                    format!("{minutes} mins")
                } else {
                    format!(
                        "{} hour{}",
                        minutes as f64 / 60.0,
                        if minutes == 60 { "" } else { "s" }
                    )
                };
                (time_text(&at), length, at)
            })
            .collect(),
        _ => quarters(shown.end.date(), shown.end.time_zone()),
    }
}

/// The list's height for `count` items, at most `max-height`.
fn list_height(count: usize) -> f32 {
    (ITEM_TOP + count as f32 * ITEM + LIST_BOTTOM).min(LIST_MAX)
}

/// The scroll that puts item `index` at the list's top.
fn scroll_to(index: usize, count: usize) -> f32 {
    let content = ITEM_TOP + count as f32 * ITEM + LIST_BOTTOM;
    (ITEM_TOP + index as f32 * ITEM).min((content - LIST_MAX).max(0.0))
}

/// Paints a time list in `rect`, scrolled and highlighted as `time` has it; the pointer
/// highlights and picks, on the press (the input keeps the keyboard until the release). Returns
/// the item picked.
fn paint_list(
    ui: &Ui,
    id: Id,
    rect: Rect,
    part: Part,
    listed: &[&(String, String, Zoned)],
    time: &mut TimeInput,
) -> Option<usize> {
    let ctx = ui.ctx().clone();
    let palette = exo_ui::palette(&ctx);
    let carbon = palette.theme == Theme::Carbon;
    // `#fff`, Carbon `#27292d` inside the form, `0 2px 8px rgba(0,0,0,.1)`, 4px corners; the
    // selected item `#f2f4fa` / `#464b53`; items `.9`, the durations `.4`.
    let fill = if carbon {
        Color32::from_rgb(0x27, 0x29, 0x2d)
    } else {
        Color32::WHITE
    };
    let selected = if carbon {
        Color32::from_rgb(0x46, 0x4b, 0x53)
    } else {
        Color32::from_rgb(0xf2, 0xf4, 0xfa)
    };
    let (text, duration) = (palette.ink(0.9, 0.9), palette.ink(0.4, 0.4));
    let content = ITEM_TOP + listed.len() as f32 * ITEM + LIST_BOTTOM;
    let mut clicked = None;
    egui::Area::new(id)
        .order(egui::Order::Tooltip)
        .fixed_pos(rect.min)
        .constrain(false)
        .show(&ctx, |area| {
            let rect = exo_ui::pixel_snap(rect);
            let response = area.allocate_rect(rect, Sense::hover());
            if response.hovered() {
                time.scroll -= area.input(|input| input.smooth_scroll_delta.y);
            }
            time.scroll = time.scroll.clamp(0.0, (content - rect.height()).max(0.0));
            let painter = area.painter().with_clip_rect(rect);
            exo_ui::BoxShadow::new([0.0, 2.0], 8.0, 0.0, Color32::from_black_alpha(26)).paint(
                area.painter(),
                rect,
                CornerRadius::same(4),
            );
            area.painter()
                .rect_filled(rect, CornerRadius::same(4), fill);
            let (pointer, moving, pressed) = ctx.input(|input| {
                (
                    input.pointer.hover_pos(),
                    input.pointer.is_moving(),
                    input.pointer.primary_pressed(),
                )
            });
            for (index, (label, length, _)) in listed.iter().enumerate() {
                let item = Rect::from_min_size(
                    pos2(
                        rect.left(),
                        rect.top() + ITEM_TOP + index as f32 * ITEM - time.scroll,
                    ),
                    vec2(rect.width(), ITEM),
                );
                if item.bottom() < rect.top() || item.top() > rect.bottom() {
                    continue;
                }
                let hovered = pointer.is_some_and(|at| item.contains(at) && rect.contains(at));
                if hovered && moving {
                    time.highlighted = index;
                }
                if index == time.highlighted {
                    painter.rect_filled(item, 0.0, selected);
                }
                let line = item.top() + ITEM_TEXT;
                if part == Part::StartTime {
                    let width = SMALL_LINE.width(&ctx, label);
                    SMALL_LINE.paint_line(
                        &painter,
                        pos2(item.center().x - width / 2.0, line),
                        label,
                        text,
                        None,
                    );
                } else {
                    SMALL_LINE.paint_line(
                        &painter,
                        pos2(item.left() + END_PADDING, line),
                        label,
                        text,
                        None,
                    );
                    let width = SMALL_LINE.width(&ctx, length);
                    SMALL_LINE.paint_line(
                        &painter,
                        pos2(item.right() - END_PADDING - width, line),
                        length,
                        duration,
                        None,
                    );
                }
                if hovered && pressed {
                    clicked = Some(index);
                }
            }
            time.scrollbar
                .show(area, id.with("scrollbar"), rect, time.scroll, content);
        });
    clicked
}

/// A typed time as one of [`FORMATS`] reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Read {
    hour: u8,
    minute: u8,
    /// `h` (1–12) rather than `HH`, and whether it said pm.
    twelve: bool,
    pm: Option<bool>,
}

/// A typed time, its spaces and colons dropped, read in `format`: each token takes what it can
/// greedily, and nothing may be left over.
fn read(text: &str, format: &str) -> Option<Read> {
    let mut rest = text;
    let mut read = Read {
        hour: 0,
        minute: 0,
        twelve: false,
        pm: None,
    };
    let mut tokens = format;
    while !tokens.is_empty() {
        if let Some(after) = tokens.strip_prefix("HH") {
            // Two digits, 0–23.
            let (value, left) = digits(rest, 2)?;
            (value <= 23).then_some(())?;
            read.hour = value;
            (rest, tokens) = (left, after);
        } else if let Some(after) = tokens.strip_prefix("mm") {
            let (value, left) = digits(rest, 2)?;
            (value <= 59).then_some(())?;
            read.minute = value;
            (rest, tokens) = (left, after);
        } else if let Some(after) = tokens.strip_prefix('h') {
            // `/^(1[0-2]|0?\d)/`, then 1–12.
            let length = match rest.as_bytes() {
                [b'1', b'0'..=b'2', ..] | [b'0', b'0'..=b'9', ..] => 2,
                [b'0'..=b'9', ..] => 1,
                _ => return None,
            };
            let value: u8 = rest[..length].parse().ok()?;
            (1..=12).contains(&value).then_some(())?;
            (read.hour, read.twelve) = (value, true);
            (rest, tokens) = (&rest[length..], after);
        } else {
            // `a`: `/^(a|p)\.?\s?m\.?/i`, else `/^(a|p)/i`.
            let after = tokens.strip_prefix('a')?;
            let lower = rest.to_ascii_lowercase();
            read.pm = Some(match lower.as_bytes().first() {
                Some(b'a') => false,
                Some(b'p') => true,
                _ => return None,
            });
            let long = [".m.", ".m", "m.", "m"]
                .iter()
                .find(|suffix| lower[1..].starts_with(**suffix))
                .map_or(0, |suffix| suffix.len());
            (rest, tokens) = (&rest[1 + long..], after);
        }
    }
    rest.is_empty().then_some(read)
}

/// Up to `max` leading digits.
fn digits(text: &str, max: usize) -> Option<(u8, &str)> {
    let length = text
        .bytes()
        .take(max)
        .take_while(u8::is_ascii_digit)
        .count();
    (length > 0).then_some(())?;
    Some((text[..length].parse().ok()?, &text[length..]))
}

/// Drops spaces and colons.
fn trimmed(text: &str) -> String {
    text.trim()
        .chars()
        .filter(|ch| *ch != ' ' && *ch != ':')
        .collect()
}

/// Whether `text` reads as a time in one of the formats.
fn valid_time(text: &str) -> bool {
    let text = trimmed(text);
    FORMATS.iter().any(|format| read(&text, format).is_some())
}

/// `text` as a time on `reference`'s day: the first format it reads in; a 12-hour time without
/// am or pm takes the reference's half of the day. An end before the reference is the next
/// day's.
fn parse_time(text: &str, reference: &Zoned, end: bool) -> Option<Zoned> {
    let text = trimmed(text);
    let read = FORMATS.iter().find_map(|format| read(&text, format))?;
    let hour = if read.twelve {
        // The hour in the half of the day its am or pm (or the reference) says.
        match (read.pm.unwrap_or(reference.hour() >= 12), read.hour) {
            (true, 12) => 12,
            (true, hour) => hour + 12,
            (false, 12) => 0,
            (false, hour) => hour,
        }
    } else {
        read.hour
    };
    let minute = read.minute;
    let at = reference
        .date()
        .at(hour as i8, minute as i8, 0, 0)
        .to_zoned(reference.time_zone().clone())
        .ok()?;
    if end && at < *reference {
        return at.checked_add(1.day()).ok();
    }
    Some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Zoned {
        format!("{text}[America/New_York]").parse().expect("time")
    }

    fn typed(text: &str, reference: &str) -> Option<String> {
        parse_time(text, &at(reference), false).map(|at| at.strftime("%-I:%M %p").to_string())
    }

    #[test]
    fn typed_times_read_in_the_first_format_that_fits() {
        assert_eq!(typed("930", "2026-11-18T09:00"), Some("9:30 AM".into()));
        assert_eq!(typed("2p", "2026-11-18T09:00"), Some("2:00 PM".into()));
        assert_eq!(typed("1:30 pm", "2026-11-18T09:00"), Some("1:30 PM".into()));
        assert_eq!(
            typed("9", "2026-11-18T09:00"),
            Some("9:00 AM".into()),
            "`HH`"
        );
        assert_eq!(
            typed("100", "2026-11-18T09:00"),
            Some("10:00 AM".into()),
            "the hour takes `10` greedily"
        );
        assert_eq!(
            typed("21", "2026-11-18T09:00"),
            Some("2:01 AM".into()),
            "`hmm` comes first"
        );
        assert_eq!(
            typed("15", "2026-11-18T09:00"),
            Some("1:05 AM".into()),
            "read once (rereading `1:05 AM` would make it 10:05)"
        );
        assert_eq!(
            typed("715", "2026-11-18T15:00"),
            Some("7:15 PM".into()),
            "the reference's half of the day"
        );
        assert_eq!(
            typed("12", "2026-11-18T09:00"),
            Some("12:00 PM".into()),
            "`HH` (the list takes it first)"
        );
        assert_eq!(typed("soon", "2026-11-18T09:00"), None);
        assert!(
            valid_time("9:00 AM")
                && valid_time("1400")
                && !valid_time("9:00 AMX")
                && !valid_time("")
        );
    }

    #[test]
    fn an_end_before_the_start_is_the_next_day() {
        let end = parse_time("1a", &at("2026-11-18T22:00"), true).expect("end");
        assert_eq!(end, at("2026-11-19T01:00"));
    }

    #[test]
    fn the_ends_list_labels_lengths() {
        let (start, end) = (at("2026-11-18T09:00"), at("2026-11-18T11:00"));
        let shown = Shown {
            start: &start,
            end: &end,
            all_day: false,
            today: start.date(),
            week_start: Weekday::Sunday,
        };
        let options = time_options(Part::EndTime, &shown);
        let labels: Vec<(&str, &str)> = options
            .iter()
            .take(6)
            .map(|(time, length, _)| (time.as_str(), length.as_str()))
            .collect();
        assert_eq!(
            labels,
            [
                ("9:15 AM", "15 mins"),
                ("9:30 AM", "30 mins"),
                ("9:45 AM", "45 mins"),
                ("10:00 AM", "1 hour"),
                ("10:15 AM", "1.25 hours"),
                ("10:30 AM", "1.5 hours")
            ]
        );
        assert_eq!(time_options(Part::StartTime, &shown).len(), 96);
        assert_eq!(scroll_to(36, 96), 6.0 + 36.0 * 32.0, "9:00 AM at the top");
    }
}
