//! The time grid: a gutter of hour labels beside a column per day, 44px an hour, an all-day row
//! above, the events laid out in their columns, and the line at the current time.

use egui::{Color32, Painter, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use exo_ui::{Family, Font, Scrollbar, TextStyle};
use jiff::{SignedDuration, ToSpan, Zoned, civil::Date, tz::TimeZone};

use super::{
    block::{self, Look},
    event::Event,
    layout::{self, HOUR_HEIGHT},
};

/// A day's height.
const DAY_HEIGHT: f32 = 24.0 * HOUR_HEIGHT;
/// A time-axis row: 15 minutes.
const SLOT: f32 = HOUR_HEIGHT / 4.0;
/// The hour labels: 12px/12px, right-aligned, `top: 7px`.
const AXIS_TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 12.0);
const AXIS_TEXT_TOP: f32 = 7.0;
/// The time axis's rows: `padding-right: 4px; border-right: 1px`.
const AXIS_PADDING_RIGHT: f32 = 4.0;
/// The time zones' header: `padding: 0 5px 12px 12px`, in the right pane `padding-top: 12px;
/// padding-bottom: 2px` and a bottom border.
const TIMEZONE_PADDING_RIGHT: f32 = 5.0;
const PANE_TIMEZONE_HEIGHT: f32 = 12.0 + 12.0 + 2.0 + 1.0;
/// A day's events: `width: calc(100% - 4px)`.
const EVENTS_INSET: f32 = 4.0;
/// The all-day row's height for one, two, and three or more rows, and the right pane's
/// `margin-top: -3px`.
const ALL_DAY_HEIGHTS: [f32; 3] = [30.0, 54.0, 66.0];
const PANE_ALL_DAY_OVERLAP: f32 = 3.0;
/// All-day rows: `padding-top: 5px`, blocks 23px tall with a 1px margin.
const ALL_DAY_PADDING_TOP: f32 = 5.0;
const ALL_DAY_BLOCK: f32 = 23.0;
const ALL_DAY_ROW: f32 = 24.0;
/// The now line: 2px tall, red; in the week view an 8px bulb at its left end (`top: -3px;
/// left: -4.5px`).
const NOW_LINE: Color32 = Color32::from_rgb(0xff, 0x56, 0x5e);
const NOW_BULB: f32 = 8.0;
/// The gutter: 64px, 60px in the right pane; with a secondary time zone 93px, or 111px when it's
/// off the hour from this one.
const WEEK_GUTTER: f32 = 64.0;
const PANE_GUTTER: f32 = 60.0;
const ZONES_GUTTER: f32 = 93.0;
const MINUTE_ZONES_GUTTER: f32 = 111.0;
/// The hour and time zone labels: each zone's at least 36px wide (the first 54px off the hour),
/// right-aligned, 4px apart.
const ZONE_LABEL: f32 = 36.0;
const ZONE_GAP: f32 = 4.0;
/// A weekday header: `padding-top: 28px; padding-bottom: 15px`, a 60px title 22px tall
/// with `margin-bottom: 2px`, 14px/22px nudged `padding-top: 1px`.
const DAY_HEADER: f32 = 28.0 + 22.0 + 2.0 + 15.0;
const DAY_TITLE_TOP: f32 = 28.0;
const DAY_TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 14.0, 22.0);
const DAY_TITLE_WIDTH: f32 = 60.0;
/// The weekday headers' names, Sunday first (`Thur`, not `Thu`).
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thur", "Fri", "Sat"];
/// The time zone labels: `top: 3px` outside the right pane, above `padding-bottom: 12px`.
const TIMEZONE_TEXT_TOP: f32 = 3.0;
const TIMEZONE_PADDING_BOTTOM: f32 = 12.0;
/// The add-event line: a 3px border tiled with a pill `rgb(191,191,191)` 3×1.5px every 4.5px.
const INDICATOR: Color32 = Color32::from_rgb(191, 191, 191);

/// The most a block I can't change moves when dragged.
const RESTRICTED_DRAG: f32 = 11.0;
/// While dragging this close to the top or bottom, the grid scrolls a quarter hour every 15 ms.
const AUTO_SCROLL_EDGE: f32 = 20.0;
const AUTO_SCROLL_STEP: f32 = SLOT / 0.015;
/// A block's bottom edge, which resizes it (6px for exactly half an hour, 4px under it).
const RESIZE_EDGE: f32 = 8.0;
/// The key of the event the form makes (`POTENTIAL_EVENT`).
pub const DRAFT: &str = "POTENTIAL_EVENT";
/// What Find Time's free slots' keys start with.
pub const FREE_SLOT: &str = "FIND_TIME_";

/// What the grid shows.
pub struct Grid<'a> {
    /// The days, one column each.
    pub days: &'a [Date],
    /// Events to draw (declined ones already left out).
    pub events: &'a [Event],
    /// The current time, for the line and for past events.
    pub now: &'a Zoned,
    pub tz: &'a TimeZone,
    /// The iCalendar UID of the invite being read, whose block glows.
    pub invite: Option<&'a str>,
    /// The event whose details are open, which glows too.
    pub selected: Option<&'a str>,
    /// The event the form makes or changes, drawn over the others.
    pub draft: Option<&'a Event>,
    /// An event being edited, which the draft stands in for.
    pub hidden: Option<&'a str>,
    /// The form is open: the others in the muted palette, and a click on one does nothing.
    pub form: bool,
    /// The details are up: a click only closes them.
    pub details: bool,
    /// Find Time is on: the hours outside work shaded, the empty grid inert.
    pub finding: bool,
    /// Guests' calendars overlaid as columns: each guest's events, in their colour
    /// ([`super::guests::guest_events`]).
    pub guests: &'a [(String, Vec<Event>)],
    /// The column a hover expanded: a guest's address, or mine otherwise.
    pub expanded: Option<&'a str>,
    /// My events take a column beside the guests'.
    pub mine: bool,
    /// Share Availability is picking times: a click on empty time makes a slot this many minutes
    /// long, and none can start before now.
    pub sharing: Option<i64>,
    /// Secondary time zones, whose hours and names the week's gutter shows before this one's.
    pub zones: &'a [TimeZone],
}

/// The grid's scroll and pointer state, kept across frames.
#[derive(Default)]
pub struct GridView {
    scroll: f32,
    /// The all-day row count the initial scroll was made for.
    scrolled_for: Option<usize>,
    /// The hour an event the grid opened for ends at.
    reveal: Option<i8>,
    /// An event whose details open once the grid draws it (after a calendar notification's
    /// click).
    select: Option<String>,
    scrollbar: Scrollbar,
    /// What's being dragged.
    drag: Option<Drag>,
    /// Where the draft was drawn last, which the form is placed beside.
    pub draft_anchor: Option<Rect>,
    /// When the grid last scrolled itself during a drag.
    scrolled_at: Option<f64>,
    /// The guests' column under the pointer this frame (its day and calendar, `None` for
    /// mine), and hover intent on the columns.
    column_under: Option<(Date, Option<String>)>,
    column_intent: exo_ui::HoverIntent<(Date, Option<String>)>,
}

/// A drag in the grid.
#[derive(Clone, Debug)]
enum Drag {
    /// Drafting a new event on day `day`: from the quarter hour it started on to the one under
    /// the pointer, in minutes.
    New { day: usize, from: i32, to: i32 },
    /// Drafting an all-day event across the days `from` to `to`.
    NewAllDay { from: usize, to: usize },
    /// Moving event `key` (or with `resize`, its end) to `start`–`end`; `grab` is where the
    /// pointer took it, after its start.
    Event {
        key: String,
        resize: bool,
        all_day: bool,
        grab: SignedDuration,
        start: Zoned,
        end: Zoned,
        original: (Zoned, Zoned),
    },
}

/// Where the grid is: the right pane's one day, or the week view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Variant {
    /// The right pane's: no weekday headers, a white grid between hairlines.
    Pane,
    /// The week view's: weekday headers, the now line's bulb.
    Week,
}

/// What the pointer did in the grid.
#[derive(Clone, Debug, PartialEq)]
pub enum GridAction {
    /// Clicked event `key`; its details go beside `anchor`: the block's day column at the
    /// block's height, or an all-day block itself.
    Select { key: String, anchor: Rect },
    /// Clicked empty grid or all-day row, outside the events.
    Background,
    /// Started dragging an event I can't change.
    Restricted,
    /// Clicked or dragged out empty time: a new event there, or the draft moved there.
    Slot {
        start: Zoned,
        end: Zoned,
        all_day: bool,
    },
    /// Moved or resized event `key` (the draft's too, [`DRAFT`]) to `start`–`end`.
    Change {
        key: String,
        start: Zoned,
        end: Zoned,
    },
    /// Double-clicked event `key`, which I can change: its form.
    Open(String),
    /// Started picking a time before now while sharing availability.
    Past,
    /// The cross on shared slot `key`.
    Remove(String),
    /// A time zone's name in the gutter: Calendar Settings, at the secondary time zone.
    ZoneSettings,
    /// The pointer rested on a guests' column: it expands, a guest's address or `None` for mine.
    Expand(Option<String>),
}

/// How far a block I can't change follows the pointer: half the way, at most 11px, along `axis`;
/// back in place when let go.
fn jiggle(
    ui: &Ui,
    response: &egui::Response,
    event: &Event,
    axis: Vec2,
    actions: &mut Vec<GridAction>,
) -> Vec2 {
    if event.editable {
        return Vec2::ZERO;
    }
    if response.drag_started() {
        actions.push(GridAction::Restricted);
    }
    if !response.dragged() {
        return Vec2::ZERO;
    }
    let moved = ui
        .input(|input| {
            input
                .pointer
                .press_origin()
                .zip(input.pointer.interact_pos())
                .map(|(from, to)| to - from)
        })
        .unwrap_or_default();
    axis * (moved.dot(axis) / 2.0).clamp(-RESTRICTED_DRAG, RESTRICTED_DRAG)
}

/// The parts of the grid's box.
struct Frame {
    variant: Variant,
    gutter: Rect,
    headers: Rect,
    all_day: Rect,
    bodies: Rect,
    columns: Rect,
}

impl GridView {
    /// Scrolls to the start again the next time the grid shows (a new mount).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Whether something is being dragged.
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Escape while dragging: the drag is dropped where it started.
    pub fn cancel_drag(&mut self) {
        self.drag = None;
    }

    /// Once the grid has scrolled to the morning, scrolls to the bottom if an event ending at
    /// `hour` would end below the view (an invite's day).
    pub fn reveal(&mut self, hour: i8) {
        self.reveal = Some(hour);
    }

    /// Opens event `key`'s details once the grid draws it.
    pub fn select(&mut self, key: String) {
        self.select = Some(key);
    }

    /// Draws the grid in `rect`. Returns what the pointer did.
    pub fn show(
        &mut self,
        ui: &Ui,
        rect: Rect,
        rem: f32,
        grid: &Grid<'_>,
        variant: Variant,
    ) -> Vec<GridAction> {
        self.draft_anchor = None;
        let dragged = self.dragged(grid);
        let rows = all_day_rows(grid, dragged.as_ref());
        // With guests overlaid, as many rows as the fullest column of a day has events.
        let attendee_rows = (!grid.guests.is_empty()).then(|| {
            super::guests::all_day_columns(
                &shown(grid, dragged.as_ref()),
                grid.guests,
                grid.expanded,
                grid.days,
            )
        });
        let row_count = attendee_rows.as_ref().map_or(rows.len(), |days| {
            days.iter()
                .flatten()
                .map(|(.., events)| events.len())
                .max()
                .unwrap_or(0)
        });
        let all_day_height = ALL_DAY_HEIGHTS[row_count.clamp(1, 3) - 1];
        let frame = match variant {
            Variant::Pane => {
                // `padding: 0 2rem 0 calc(2rem - 26px)`.
                let content = Rect::from_min_max(
                    pos2(rect.left() + 2.0 * rem - 26.0, rect.top()),
                    pos2(rect.right() - 2.0 * rem, rect.bottom()),
                );
                let headers = Rect::from_min_size(
                    content.min,
                    vec2(
                        content.width(),
                        PANE_TIMEZONE_HEIGHT.max(all_day_height - PANE_ALL_DAY_OVERLAP),
                    ),
                );
                let bodies =
                    Rect::from_min_max(pos2(content.left(), headers.bottom()), content.max);
                Frame {
                    variant,
                    gutter: Rect::from_min_max(
                        content.min,
                        pos2(content.left() + PANE_GUTTER, content.bottom()),
                    ),
                    headers,
                    all_day: Rect::from_min_max(
                        pos2(
                            content.left() + PANE_GUTTER,
                            headers.bottom() - all_day_height,
                        ),
                        pos2(content.right(), headers.bottom()),
                    ),
                    bodies,
                    columns: Rect::from_min_max(
                        pos2(content.left() + PANE_GUTTER, bodies.top()),
                        bodies.max,
                    ),
                }
            }
            Variant::Week => {
                let headers =
                    Rect::from_min_size(rect.min, vec2(rect.width(), DAY_HEADER + all_day_height));
                let bodies = Rect::from_min_max(pos2(rect.left(), headers.bottom()), rect.max);
                let gutter = week_gutter(grid);
                Frame {
                    variant,
                    gutter: Rect::from_min_max(rect.min, pos2(rect.left() + gutter, rect.bottom())),
                    headers,
                    all_day: Rect::from_min_max(
                        pos2(rect.left() + gutter, headers.top() + DAY_HEADER),
                        pos2(rect.right(), headers.bottom()),
                    ),
                    bodies,
                    columns: Rect::from_min_max(
                        pos2(rect.left() + gutter, bodies.top()),
                        bodies.max,
                    ),
                }
            }
        };
        self.scroll_to_start(rows.len(), frame.bodies.height());
        self.scroll(ui, &frame);
        let painter = ui.painter_at(rect);
        let palette = exo_ui::palette(ui.ctx());
        let hovered = self
            .hovered_minute(ui, &frame)
            .filter(|_| !grid.details && !grid.finding && self.drag.is_none());
        let mut actions = Vec::new();
        if self.paint_timezone(ui, &painter, &frame, grid) {
            actions.push(GridAction::ZoneSettings);
        }
        if variant == Variant::Week {
            paint_day_headers(&painter, &frame, grid);
        }
        self.paint_axis(&painter, &frame, grid, hovered);
        // Under the events, which take their own clicks.
        let band = Rect::from_min_max(
            frame.all_day.min,
            pos2(frame.all_day.right(), frame.all_day.bottom()),
        );
        let all_day_background = ui.interact(
            band,
            ui.id()
                .with(("calendar-all-day-background", variant == Variant::Week)),
            Sense::click_and_drag(),
        );
        let body = Rect::from_min_max(
            frame.columns.min,
            pos2(frame.columns.right() - 1.0, frame.columns.bottom()),
        );
        let background = ui.interact(
            body,
            ui.id()
                .with(("calendar-background", variant == Variant::Week)),
            Sense::click_and_drag(),
        );
        self.drag_background(
            ui,
            &frame,
            grid,
            &background,
            &all_day_background,
            &mut actions,
        );
        match &attendee_rows {
            Some(days) => {
                self.paint_attendee_all_day(ui, &painter, &frame, grid, days, &mut actions)
            }
            None => self.paint_all_day(ui, &painter, &frame, grid, &rows, &mut actions),
        }
        self.paint_columns(ui, &frame, grid, hovered, &mut actions);
        self.expand_column(ui, &mut actions);
        self.follow_drag(ui, &frame, grid, &mut actions);
        self.scrollbar.show(
            ui,
            ui.id()
                .with(("calendar-scrollbar", variant == Variant::Week)),
            frame.columns,
            self.scroll,
            DAY_HEIGHT,
        );
        if variant == Variant::Pane {
            // Hairlines at the top and bottom, `width: calc(100% - 55px); right: 1rem`, fading in
            // over their first 16px.
            for y in [rect.top(), rect.bottom() - 1.0] {
                let line = Rect::from_min_max(
                    pos2(rect.right() - rem - (rect.width() - 55.0), y),
                    pos2(rect.right() - rem, y + 1.0),
                );
                paint_fade_in(&painter, line, 16.0, palette.border_base_muted);
            }
        }
        actions
    }

    /// The day column under `x` and the minutes from its midnight at `y` (unsnapped).
    fn time_at(&self, frame: &Frame, grid: &Grid<'_>, at: egui::Pos2) -> (usize, f32) {
        let count = grid.days.len().max(1);
        let width = frame.columns.width() / count as f32;
        let day =
            (((at.x - frame.columns.left()) / width).floor().max(0.0) as usize).min(count - 1);
        let minutes = (at.y - (frame.columns.top() - self.scroll)) / DAY_HEIGHT * 1440.0;
        (day, minutes.clamp(0.0, 1440.0))
    }

    /// Clicks and drags on empty grid: a 30-minute event at the quarter hour clicked, or the
    /// range dragged out; in the all-day row, all-day events. With the details up, a click only
    /// closes them.
    fn drag_background(
        &mut self,
        ui: &Ui,
        frame: &Frame,
        grid: &Grid<'_>,
        timed: &egui::Response,
        all_day: &egui::Response,
        actions: &mut Vec<GridAction>,
    ) {
        if grid.finding {
            return;
        }
        if grid.details {
            if timed.clicked()
                || all_day.clicked()
                || timed.drag_started()
                || all_day.drag_started()
            {
                actions.push(GridAction::Background);
            }
            return;
        }
        let snap = |minutes: f32| ((minutes / 15.0).round() * 15.0) as i32;
        // Picking free times refuses a press before now.
        let past = |day: usize, from: i32| {
            grid.sharing.is_some()
                && span(grid, day, from, from).is_some_and(|(at, _)| at < *grid.now)
        };
        if let Some(at) = timed.interact_pointer_pos() {
            let (day, minutes) = self.time_at(frame, grid, at);
            if timed.clicked() {
                let length = grid.sharing.map_or(30, |minutes| minutes as i32);
                let from = snap(minutes).min(1440 - length);
                if past(day, snap(minutes)) {
                    actions.push(GridAction::Past);
                } else if let Some((start, end)) = span(grid, day, from, from + length) {
                    actions.push(GridAction::Slot {
                        start,
                        end,
                        all_day: false,
                    });
                }
            } else if timed.drag_started() {
                // Starts at the quarter pressed, 15 minutes long.
                let origin = ui.input(|input| input.pointer.press_origin()).unwrap_or(at);
                let (day, minutes) = self.time_at(frame, grid, origin);
                let from = snap(minutes).min(1440 - 15);
                if past(day, snap(minutes)) {
                    actions.push(GridAction::Past);
                } else {
                    self.drag = Some(Drag::New {
                        day,
                        from,
                        to: from + 15,
                    });
                }
            }
        }
        // Free times are picked in the hours, not the all-day row.
        if grid.sharing.is_some() {
            return;
        }
        if let Some(at) = all_day.interact_pointer_pos() {
            let (day, _) = self.time_at(frame, grid, at);
            if all_day.clicked() {
                if let Some((start, end)) = days_span(grid, day, day) {
                    actions.push(GridAction::Slot {
                        start,
                        end,
                        all_day: true,
                    });
                }
            } else if all_day.drag_started() {
                self.drag = Some(Drag::NewAllDay { from: day, to: day });
            }
        }
    }

    /// While dragging: follows the pointer (scrolling the grid near its edges), and on release
    /// says what came of it.
    fn follow_drag(
        &mut self,
        ui: &Ui,
        frame: &Frame,
        grid: &Grid<'_>,
        actions: &mut Vec<GridAction>,
    ) {
        let Some(drag) = self.drag.clone() else {
            self.scrolled_at = None;
            return;
        };
        let (pointer, released, time) = ui.input(|input| {
            (
                input.pointer.interact_pos(),
                input.pointer.any_released() || !input.pointer.any_down(),
                input.time,
            )
        });
        let Some(pointer) = pointer else { return };
        if !matches!(drag, Drag::NewAllDay { .. }) {
            let direction = if pointer.y < frame.bodies.top() + AUTO_SCROLL_EDGE {
                -1.0
            } else if pointer.y > frame.bodies.bottom() - AUTO_SCROLL_EDGE {
                1.0
            } else {
                0.0
            };
            if direction != 0.0 {
                let elapsed = self.scrolled_at.map_or(0.0, |at| (time - at) as f32);
                self.scroll = (self.scroll + direction * AUTO_SCROLL_STEP * elapsed)
                    .clamp(0.0, (DAY_HEIGHT - frame.bodies.height()).max(0.0));
                self.scrolled_at = Some(time);
                ui.ctx().request_repaint();
            } else {
                self.scrolled_at = None;
            }
        }
        let snap = |minutes: f32| ((minutes / 15.0).round() * 15.0) as i32;
        let (day, minutes) = self.time_at(frame, grid, pointer);
        let updated = match drag {
            // While sharing, the pointer before now is ignored.
            Drag::New {
                day: first,
                from,
                to,
            } if grid.sharing.is_some()
                && span(grid, first, snap(minutes).min(from), from)
                    .is_some_and(|(at, _)| at < *grid.now) =>
            {
                Drag::New {
                    day: first,
                    from,
                    to,
                }
            }
            Drag::New {
                day: first, from, ..
            } => Drag::New {
                day: first,
                from,
                to: snap(minutes),
            },
            Drag::NewAllDay { from, .. } => Drag::NewAllDay { from, to: day },
            Drag::Event {
                key,
                resize,
                all_day,
                grab,
                start,
                end,
                original,
            } => {
                let length = end.timestamp().duration_since(start.timestamp());
                let (start, end) = if all_day {
                    let moved = days_span(grid, day, day).map(|(midnight, _)| midnight);
                    let first = moved
                        .and_then(|midnight| midnight.checked_sub(grab).ok())
                        .map(|at| at.date())
                        .and_then(|date| date.to_zoned(start.time_zone().clone()).ok());
                    first
                        .and_then(|first| first.checked_add(length).ok().map(|end| (first, end)))
                        .unwrap_or((start, end))
                } else if resize {
                    let at = span(grid, day, snap(minutes), snap(minutes)).map(|(at, _)| at);
                    let floor = start.checked_add(15.minutes()).ok();
                    let end = at
                        .zip(floor)
                        .map(|(at, floor)| if at < floor { floor } else { at })
                        .unwrap_or(end);
                    (start, end)
                } else {
                    let pointer_time = days_span(grid, day, day).and_then(|(midnight, _)| {
                        midnight
                            .checked_add(SignedDuration::from_secs_f64(f64::from(minutes) * 60.0))
                            .ok()
                    });
                    let moved = pointer_time
                        .and_then(|at| at.checked_sub(grab).ok())
                        .and_then(|at| {
                            let midnight = at.date().to_zoned(at.time_zone().clone()).ok()?;
                            let minutes = at
                                .timestamp()
                                .duration_since(midnight.timestamp())
                                .as_secs_f64()
                                / 60.0;
                            midnight
                                .checked_add(((minutes / 15.0).round() as i64 * 15).minutes())
                                .ok()
                        });
                    moved
                        .and_then(|start| start.checked_add(length).ok().map(|end| (start, end)))
                        .unwrap_or((start, end))
                };
                Drag::Event {
                    key,
                    resize,
                    all_day,
                    grab,
                    start,
                    end,
                    original,
                }
            }
        };
        self.drag = Some(updated.clone());
        if !released {
            return;
        }
        self.drag = None;
        self.scrolled_at = None;
        match updated {
            Drag::New { day, from, to } => {
                // Either way from where it started, a quarter hour at least.
                let (low, high) = if to < from {
                    (to, from + 15)
                } else {
                    (from, to.max(from + 15))
                };
                if let Some((start, end)) = span(grid, day, low, high.min(1440)) {
                    actions.push(GridAction::Slot {
                        start,
                        end,
                        all_day: false,
                    });
                }
            }
            Drag::NewAllDay { from, to } => {
                if let Some((start, end)) = days_span(grid, from.min(to), from.max(to)) {
                    actions.push(GridAction::Slot {
                        start,
                        end,
                        all_day: true,
                    });
                }
            }
            Drag::Event {
                key,
                start,
                end,
                original,
                ..
            } => {
                if (start.clone(), end.clone()) != original {
                    actions.push(GridAction::Change { key, start, end });
                }
            }
        }
    }

    /// The drag's event (the draft, or one being moved) as it's drawn now, if it's on screen.
    fn dragged(&self, grid: &Grid<'_>) -> Option<Event> {
        match self.drag.as_ref()? {
            Drag::New { day, from, to } => {
                let (low, high) = if to < from {
                    (*to, from + 15)
                } else {
                    (*from, (*to).max(from + 15))
                };
                let (start, end) = span(grid, *day, low, high.min(1440))?;
                Some(draft_like(grid, start, end, false))
            }
            Drag::NewAllDay { from, to } => {
                let (start, end) = days_span(grid, *from.min(to), *from.max(to))?;
                Some(draft_like(grid, start, end, true))
            }
            Drag::Event {
                key, start, end, ..
            } => {
                let event = grid
                    .events
                    .iter()
                    .chain(grid.draft)
                    .find(|event| event.key == *key)?;
                Some(Event {
                    start: start.clone(),
                    end: end.clone(),
                    ..event.clone()
                })
            }
        }
    }

    /// Starts moving or resizing `event` when its block's drag starts; a double click opens its
    /// form.
    fn take_event(
        &mut self,
        ui: &Ui,
        response: &egui::Response,
        event: &Event,
        rect: Rect,
        actions: &mut Vec<GridAction>,
    ) {
        if !event.editable || event.key.starts_with(FREE_SLOT) {
            return;
        }
        if response.double_clicked() && event.key != DRAFT && event.share.is_none() {
            actions.push(GridAction::Open(event.key.clone()));
        }
        if !response.drag_started() || self.drag.is_some() {
            return;
        }
        let Some(origin) = ui.input(|input| input.pointer.press_origin()) else {
            return;
        };
        let edge = if event
            .end
            .timestamp()
            .duration_since(event.start.timestamp())
            < SignedDuration::from_mins(30)
        {
            4.0
        } else if event
            .end
            .timestamp()
            .duration_since(event.start.timestamp())
            == SignedDuration::from_mins(30)
        {
            6.0
        } else {
            RESIZE_EDGE
        };
        let resize = !event.all_day && origin.y > rect.bottom() - edge;
        // Where on the event it was taken: the time under the pointer after its start, or for an
        // all-day block, the days.
        let grab = if event.all_day {
            let width = rect.width()
                / ((event
                    .end
                    .timestamp()
                    .duration_since(event.start.timestamp())
                    .as_hours()
                    / 24)
                    .max(1) as f32);
            SignedDuration::from_hours(
                24 * ((origin.x - rect.left()) / width.max(1.0)).floor() as i64,
            )
        } else {
            SignedDuration::from_secs_f64(f64::from((origin.y - rect.top()) / HOUR_HEIGHT) * 3600.0)
        };
        self.drag = Some(Drag::Event {
            key: event.key.clone(),
            resize,
            all_day: event.all_day,
            grab,
            start: event.start.clone(),
            end: event.end.clone(),
            original: (event.start.clone(), event.end.clone()),
        });
    }

    /// The initial scroll: about 6:30 am at the top, lower by a row's height per all-day row (up
    /// to two and a half).
    fn scroll_to_start(&mut self, rows: usize, viewport: f32) {
        let rows = rows.clamp(1, 3);
        if self.scrolled_for == Some(rows) {
            return;
        }
        self.scrolled_for = Some(rows);
        let target = DAY_HEIGHT * (6.5 / 24.0) + 2.0 + 24.0 * (rows as f32).min(2.5);
        self.scroll = target.min((DAY_HEIGHT - viewport).max(0.0));
        if let Some(hour) = self.reveal.take()
            && f32::from(hour) / 24.0 * DAY_HEIGHT > target + viewport
        {
            self.scroll = (DAY_HEIGHT - viewport).max(0.0);
        }
    }

    /// The wheel over the bodies scrolls the columns and the time axis together.
    fn scroll(&mut self, ui: &Ui, frame: &Frame) {
        if ui.rect_contains_pointer(frame.bodies) {
            self.scroll -= ui.input(|input| input.smooth_scroll_delta.y);
        }
        let scale = ui.ctx().pixels_per_point();
        self.scroll = ((self
            .scroll
            .clamp(0.0, (DAY_HEIGHT - frame.bodies.height()).max(0.0)))
            * scale)
            .round()
            / scale;
    }

    /// The gutter's header: the time zones' abbreviations, the secondary ones first; in the right
    /// pane, borders below and to the right. Returns whether one of the week's was clicked:
    /// they're buttons to Calendar Settings, but not while the form or Share Availability has the
    /// week.
    fn paint_timezone(&self, ui: &Ui, painter: &Painter, frame: &Frame, grid: &Grid<'_>) -> bool {
        let palette = exo_ui::palette(painter.ctx());
        let header = Rect::from_min_max(
            frame.gutter.min,
            pos2(frame.gutter.right(), frame.headers.bottom()),
        );
        let first = grid
            .days
            .first()
            .and_then(|day| day.to_zoned(grid.tz.clone()).ok());
        let abbreviation = first
            .as_ref()
            .map(|day| day.strftime("%Z").to_string())
            .unwrap_or_default();
        let width = AXIS_TEXT.width(painter.ctx(), &abbreviation);
        if frame.variant == Variant::Week {
            // Right-aligned at the bottom, inside `padding: 0 5px 12px 12px`, nudged down 3px.
            let top = header.bottom() - TIMEZONE_PADDING_BOTTOM - AXIS_TEXT.line_height
                + TIMEZONE_TEXT_TOP;
            let secondary = grid.zones.iter().map(|zone| {
                first
                    .as_ref()
                    .map(|day| super::availability::abbreviation(zone, day.timestamp()))
                    .unwrap_or_default()
            });
            let labels: Vec<String> = secondary.chain([abbreviation]).collect();
            let mut right = header.right() - TIMEZONE_PADDING_RIGHT;
            let buttons = !grid.form && grid.sharing.is_none();
            let mut clicked = false;
            for (index, label) in labels.iter().enumerate().rev() {
                let width = AXIS_TEXT.width(painter.ctx(), label);
                AXIS_TEXT.paint_line(
                    painter,
                    pos2(right - width, top),
                    label,
                    palette.text_base_subdued,
                    None,
                );
                let button = Rect::from_min_max(
                    pos2(right - width.max(ZONE_LABEL), top),
                    pos2(right, top + AXIS_TEXT.line_height),
                );
                clicked |= buttons
                    && ui
                        .interact(
                            button,
                            ui.id().with(("calendar-zone-label", index)),
                            Sense::click(),
                        )
                        .clicked();
                right -= width.max(ZONE_LABEL) + ZONE_GAP;
            }
            return clicked;
        }
        // Right-aligned in a 36px box at the bottom, above `padding-bottom: 2px` and the border.
        let right = header.right() - 1.0 - TIMEZONE_PADDING_RIGHT;
        AXIS_TEXT.paint_line(
            painter,
            pos2(
                right - width,
                header.bottom() - 1.0 - 2.0 - AXIS_TEXT.line_height,
            ),
            &abbreviation,
            palette.text_base_subdued,
            None,
        );
        // Its borders are a `border-image` gradient: light runs `#fff`, `#fff 30%`, `#f2f2f2`;
        // dark `#27292d`, `#27292d` (50%), `rgba(255,255,255,.09)`. The right border takes the
        // gradient's end.
        let (from, hold, to) = match exo_ui::theme(painter.ctx()) {
            exo_ui::Theme::Snow => (Color32::WHITE, 0.3, Color32::from_rgb(0xf2, 0xf2, 0xf2)),
            exo_ui::Theme::Carbon => (
                Color32::from_rgb(0x27, 0x29, 0x2d),
                0.5,
                Color32::from_white_alpha(23),
            ),
        };
        painter.vline(
            snap(header.right()) - 0.5,
            header.y_range(),
            Stroke::new(1.0, to),
        );
        let bottom = Rect::from_min_max(pos2(header.left(), header.bottom() - 1.0), header.max);
        painter.rect_filled(
            Rect::from_min_max(
                bottom.min,
                pos2(bottom.left() + hold * bottom.width(), bottom.bottom()),
            ),
            0.0,
            from,
        );
        paint_gradient(
            painter,
            Rect::from_min_max(
                pos2(bottom.left() + hold * bottom.width(), bottom.top()),
                bottom.max,
            ),
            from,
            to,
        );
        false
    }

    /// The quarter hour under the pointer, in minutes from midnight, but not midnight at either
    /// end.
    fn hovered_minute(&self, ui: &Ui, frame: &Frame) -> Option<i32> {
        // Not through a popover or toast above the grid.
        let pointer = ui
            .ctx()
            .pointer_hover_pos()
            .filter(|_| ui.rect_contains_pointer(frame.bodies))?;
        let top = frame.columns.top() - self.scroll;
        let minutes = (((pointer.y - top) / DAY_HEIGHT * 1440.0 / 15.0).round() * 15.0)
            .clamp(0.0, 1440.0) as i32;
        (minutes > 0 && minutes < 1440).then_some(minutes)
    }

    /// The hour labels down the gutter, scrolled with the columns, and its right border. The
    /// hovered quarter hour's label shows in the link colour. On the hour, each secondary time
    /// zone's own hour shows before it.
    fn paint_axis(&self, painter: &Painter, frame: &Frame, grid: &Grid<'_>, hovered: Option<i32>) {
        let palette = exo_ui::palette(painter.ctx());
        // The axis: `height: calc(100% + 1px); margin-top: -1px`.
        let axis = Rect::from_min_max(
            pos2(frame.gutter.left(), frame.bodies.top() - 1.0),
            pos2(frame.gutter.right(), frame.bodies.bottom()),
        );
        let painter = painter.with_clip_rect(axis);
        painter.vline(
            snap(axis.right()) - 0.5,
            axis.y_range(),
            Stroke::new(1.0, palette.border_base_muted),
        );
        let right = axis.right() - 1.0 - AXIS_PADDING_RIGHT;
        let quarters = (1..96).filter(|quarter| quarter % 4 == 0 || hovered == Some(quarter * 15));
        for quarter in quarters {
            // A label sits at the bottom of its 11px row, nudged down 7px.
            let row_bottom = axis.top() - self.scroll + quarter as f32 * SLOT;
            let top = row_bottom - AXIS_TEXT.line_height + AXIS_TEXT_TOP;
            if top > axis.bottom() || top + AXIS_TEXT.line_height < axis.top() {
                continue;
            }
            let label = hour_label(quarter / 4, quarter % 4 * 15);
            let width = AXIS_TEXT.width(painter.ctx(), &label);
            // The hour labels: Classic's `rgba(0,0,0,.5)`, Carbon `hsla(0,0%,100%,.5)`.
            let ink = palette.ink(0.5, 0.5);
            let color = if hovered == Some(quarter * 15) {
                palette.link_text_primary
            } else {
                ink
            };
            AXIS_TEXT.paint_line(&painter, pos2(right - width, top), &label, color, None);
            if quarter % 4 != 0 {
                continue;
            }
            let at = grid
                .days
                .first()
                .and_then(|day| day.to_zoned(grid.tz.clone()).ok())
                .and_then(|midnight| {
                    midnight
                        .checked_add(SignedDuration::from_mins(i64::from(quarter) * 15))
                        .ok()
                });
            let mut zone_right = right - width.max(ZONE_LABEL) - ZONE_GAP;
            for zone in grid.zones.iter().rev() {
                let Some(there) = at.as_ref().map(|at| at.with_time_zone(zone.clone())) else {
                    continue;
                };
                let label = hour_label(i32::from(there.hour()), i32::from(there.minute()));
                let width = AXIS_TEXT.width(painter.ctx(), &label);
                AXIS_TEXT.paint_line(&painter, pos2(zone_right - width, top), &label, ink, None);
                zone_right -= width.max(ZONE_LABEL) + ZONE_GAP;
            }
        }
    }

    /// The day columns: hour blocks (white in the right pane), the now line, the events.
    fn paint_columns(
        &mut self,
        ui: &Ui,
        frame: &Frame,
        grid: &Grid<'_>,
        hovered: Option<i32>,
        actions: &mut Vec<GridAction>,
    ) {
        let palette = exo_ui::palette(ui.ctx());
        let painter = ui.painter_at(frame.columns);
        let count = grid.days.len().max(1) as f32;
        let width = frame.columns.width() / count;
        let top = frame.columns.top() - self.scroll;
        for (index, day) in grid.days.iter().enumerate() {
            let left = frame.columns.left() + index as f32 * width;
            let column = Rect::from_min_max(pos2(left, top), pos2(left + width, top + DAY_HEIGHT));
            // Hour blocks: 44px with a 1px bottom border, inside the column's right border.
            let inner = Rect::from_min_max(column.min, pos2(column.right() - 1.0, column.bottom()));
            if frame.variant == Variant::Pane {
                painter.rect_filled(
                    exo_ui::pixel_snap(inner),
                    0.0,
                    palette.background_base_default,
                );
            }
            for hour in 1..=24 {
                let y = top + hour as f32 * HOUR_HEIGHT - 0.5;
                painter.hline(
                    snap(inner.left())..=snap(inner.right()),
                    snap(y + 0.5) - 0.5,
                    Stroke::new(1.0, palette.border_base_muted),
                );
            }
            painter.vline(
                snap(column.right()) - 0.5,
                frame.columns.y_range(),
                Stroke::new(1.0, palette.border_base_muted),
            );
            let Ok(midnight) = day.to_zoned(grid.tz.clone()) else {
                continue;
            };
            if grid.finding {
                // The hours outside work, shaded.
                for (from, to) in super::meet::disabled_hours(*day) {
                    let shade = Rect::from_min_max(
                        pos2(inner.left(), top + f32::from(from) * HOUR_HEIGHT),
                        pos2(inner.right(), top + f32::from(to) * HOUR_HEIGHT),
                    );
                    painter.rect_filled(
                        shade,
                        0.0,
                        super::meet::disabled_fill(exo_ui::theme(ui.ctx())),
                    );
                }
            }
            let dragged = self.dragged(grid);
            self.paint_events(ui, frame, inner, &midnight, grid, dragged.as_ref(), actions);
            if grid.now.date() == *day {
                // `top: calc({fraction of the day}% - 1px)`.
                let seconds = grid
                    .now
                    .time()
                    .duration_since(jiff::civil::Time::midnight())
                    .as_secs_f64() as f32;
                let y = top + seconds / 86_400.0 * DAY_HEIGHT - 1.0;
                painter.rect_filled(
                    Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 2.0)),
                    0.0,
                    NOW_LINE,
                );
                if frame.variant == Variant::Week {
                    painter.circle_filled(
                        pos2(
                            inner.left() - 4.5 + NOW_BULB / 2.0,
                            y - 3.0 + NOW_BULB / 2.0,
                        ),
                        NOW_BULB / 2.0,
                        NOW_LINE,
                    );
                }
            }
        }
        if let Some(minutes) = hovered {
            paint_indicator(
                &painter,
                frame.columns,
                top + minutes as f32 / 15.0 * SLOT - 1.5,
            );
        }
    }
}

/// While the pointer is over the grid, a dotted line at the quarter hour nearest it.
fn paint_indicator(painter: &Painter, columns: Rect, y: f32) {
    let mut x = columns.left();
    while x < columns.right() {
        painter.rect_filled(
            Rect::from_min_size(pos2(x + 0.75, y), vec2(3.0, 1.5)),
            0.75,
            INDICATOR,
        );
        x += 4.5;
    }
}

/// A cell of an all-day row: an event spanning `days` columns, or an empty column.
struct Cell<'a> {
    event: Option<&'a Event>,
    days: usize,
}

/// The all-day events in rows: each event under its first day, spanning the days it lasts,
/// longest first, and each row filled left to right with what fits.
fn all_day_rows<'a>(grid: &Grid<'a>, extra: Option<&'a Event>) -> Vec<Vec<Cell<'a>>> {
    let count = grid.days.len();
    let mut by_day: Vec<Vec<&Event>> = vec![Vec::new(); count];
    for event in shown(grid, extra).into_iter().filter(|event| event.all_day) {
        if let Some(first) = grid.days.iter().position(|day| event.on(*day)) {
            by_day[first].push(event);
        }
    }
    for events in &mut by_day {
        events.sort_by_key(|event| {
            std::cmp::Reverse(
                event
                    .end
                    .timestamp()
                    .duration_since(event.start.timestamp()),
            )
        });
    }
    let total: usize = by_day.iter().map(Vec::len).sum();
    let mut rows = vec![Vec::new()];
    let (mut placed, mut day) = (0, 0);
    while placed < total {
        if day >= count {
            day = 0;
            rows.push(Vec::new());
            continue;
        }
        let row = rows.last_mut().expect("a row");
        if by_day[day].is_empty() {
            row.push(Cell {
                event: None,
                days: 1,
            });
            day += 1;
            continue;
        }
        let event = by_day[day].remove(0);
        // The days from here it covers, at least one.
        let days = grid.days[day..]
            .iter()
            .take_while(|next| {
                next.to_zoned(grid.tz.clone())
                    .is_ok_and(|midnight| midnight < event.end)
            })
            .count()
            .max(1);
        row.push(Cell {
            event: Some(event),
            days,
        });
        day += days;
        placed += 1;
    }
    let row = rows.last_mut().expect("a row");
    row.extend((day..count).map(|_| Cell {
        event: None,
        days: 1,
    }));
    rows
}

/// The weekday headers: `Tue 17` centred over each column, today's on a fill. Classic sets them
/// all semibold, at `.6` (Carbon `.5`), today's at `.8` on `rgba(0,0,0,.05)` (Carbon white) with
/// 2px corners.
fn paint_day_headers(painter: &Painter, frame: &Frame, grid: &Grid<'_>) {
    let palette = exo_ui::palette(painter.ctx());
    let width = frame.columns.width() / grid.days.len().max(1) as f32;
    for (index, day) in grid.days.iter().enumerate() {
        let center = frame.columns.left() + (index as f32 + 0.5) * width;
        let title = Rect::from_min_size(
            pos2(
                center - DAY_TITLE_WIDTH / 2.0,
                frame.headers.top() + DAY_TITLE_TOP,
            ),
            vec2(DAY_TITLE_WIDTH, DAY_TITLE.line_height),
        );
        let today = *day == grid.now.date();
        if today {
            painter.rect_filled(exo_ui::pixel_snap(title), 2.0, palette.ink(0.05, 0.05));
        }
        let text = format!(
            "{} {}",
            WEEKDAYS[day.weekday().to_sunday_zero_offset() as usize],
            day.day()
        );
        let text_width = DAY_TITLE.width(painter.ctx(), &text);
        let color = if today {
            palette.ink(0.8, 0.8)
        } else {
            palette.ink(0.6, 0.5)
        };
        DAY_TITLE.paint_line(
            painter,
            pos2(title.center().x - text_width / 2.0, title.top() + 1.0),
            &text,
            color,
            None,
        );
    }
}

impl GridView {
    /// Notes the guests' column in `rect` (`guest`'s, or mine) when the pointer is over it there and
    /// in `within`, for [`Self::expand_column`].
    fn hover_column(&mut self, ui: &Ui, within: Rect, rect: Rect, column: (Date, Option<String>)) {
        // Nothing over the grid there: the week's own area doesn't count as covering it.
        let uncovered = |at: egui::Pos2| {
            ui.ctx().layer_id_at(at).is_none_or(|layer| {
                layer == ui.layer_id() || layer.order == egui::Order::Background
            })
        };
        if ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|at| rect.contains(at) && within.contains(at) && uncovered(at))
            && self.drag.is_none()
        {
            self.column_under = Some(column);
        }
    }

    /// Expands the guests' column the pointer has settled on (100ms polls, under 7px between two),
    /// once each time it does.
    fn expand_column(&mut self, ui: &Ui, actions: &mut Vec<GridAction>) {
        if let Some((_, guest)) = self
            .column_intent
            .update(ui.ctx(), self.column_under.take())
        {
            actions.push(GridAction::Expand(guest.clone()));
        }
    }

    /// The all-day row: a band (white in the right pane) with a bottom border, the events in it,
    /// each row's cells with a right border.
    fn paint_all_day(
        &mut self,
        ui: &Ui,
        painter: &Painter,
        frame: &Frame,
        grid: &Grid<'_>,
        rows: &[Vec<Cell<'_>>],
        actions: &mut Vec<GridAction>,
    ) {
        let palette = exo_ui::palette(ui.ctx());
        let band = Rect::from_min_max(
            frame.all_day.min,
            pos2(frame.all_day.right(), frame.all_day.bottom() - 1.0),
        );
        if frame.variant == Variant::Pane {
            painter.rect_filled(band, 0.0, palette.background_base_default);
        }
        painter.hline(
            snap(frame.all_day.left())..=snap(frame.all_day.right()),
            snap(frame.all_day.bottom()) - 0.5,
            Stroke::new(1.0, palette.border_base_muted),
        );
        let painter = painter.with_clip_rect(band);
        let column = band.width() / grid.days.len().max(1) as f32;
        for (index, row) in rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.iter().any(|cell| cell.event.is_some()))
        {
            let top = band.top() + ALL_DAY_PADDING_TOP + index as f32 * ALL_DAY_ROW;
            let mut left = band.left();
            for cell in row {
                let width = cell.days as f32 * column;
                painter.vline(
                    snap(left + width) - 0.5,
                    top..=top + ALL_DAY_ROW,
                    Stroke::new(1.0, palette.border_base_muted),
                );
                if let Some(event) = cell.event {
                    // `width: calc(100% - 6px); margin: 0 1px` inside the cell's border.
                    let rect = Rect::from_min_size(
                        pos2(left + 1.0, top),
                        vec2(width - 1.0 - 6.0, ALL_DAY_BLOCK),
                    );
                    let id = exo_ui::id(
                        ui.ctx(),
                        ("calendar-block", frame.variant, &event.key, "all-day"),
                    );
                    let response = ui.interact(rect.intersect(band), id, Sense::click_and_drag());
                    let focused = event.key == DRAFT;
                    if focused {
                        self.draft_anchor = Some(rect);
                    }
                    if response.clicked() && !grid.form {
                        actions.push(GridAction::Select {
                            key: event.key.clone(),
                            anchor: rect,
                        });
                    }
                    self.take_event(ui, &response, event, rect, actions);
                    let rect = rect.translate(jiggle(ui, &response, event, Vec2::X, actions));
                    let hovered = response.hovered();
                    if event.everyone_else_declined && !grid.form {
                        exo_ui::KeyTooltip {
                            text: "Everyone else declined",
                            keys: &[],
                        }
                        .show_below(ui, &response, 8.0);
                    }
                    let active = grid
                        .invite
                        .is_some_and(|uid| event.ical_uid.as_deref() == Some(uid))
                        || grid.selected == Some(event.key.as_str());
                    let look = Look {
                        glowing: block::glow(ui.ctx(), id, (hovered || active) && !focused),
                        past: event.end < *grid.now,
                        all_day: true,
                        preview: grid.form,
                        focused,
                        ..Look::default()
                    };
                    block::paint(ui, band, rect, event, SignedDuration::from_hours(24), look);
                }
                left += width;
            }
        }
    }
}

impl GridView {
    /// The all-day row while guests are overlaid: each day's columns side by side across the day
    /// less 5px (`flex: 1; margin-right: 5px`), the open ones 50%, each column's events stacked from
    /// its top, 2px narrower than it; no borders between the days. A click opens an event's details
    /// (they don't move), a collapsed column hides their text, and resting on a column expands it.
    fn paint_attendee_all_day(
        &mut self,
        ui: &Ui,
        painter: &Painter,
        frame: &Frame,
        grid: &Grid<'_>,
        days: &[Vec<super::guests::AllDayColumn<'_>>],
        actions: &mut Vec<GridAction>,
    ) {
        let palette = exo_ui::palette(ui.ctx());
        let band = Rect::from_min_max(
            frame.all_day.min,
            pos2(frame.all_day.right(), frame.all_day.bottom() - 1.0),
        );
        if frame.variant == Variant::Pane {
            painter.rect_filled(band, 0.0, palette.background_base_default);
        }
        painter.hline(
            snap(frame.all_day.left())..=snap(frame.all_day.right()),
            snap(frame.all_day.bottom()) - 0.5,
            Stroke::new(1.0, palette.border_base_muted),
        );
        let day_width = band.width() / grid.days.len().max(1) as f32;
        for ((index, columns), day) in days.iter().enumerate().zip(grid.days) {
            let container = Rect::from_min_size(
                pos2(
                    band.left() + index as f32 * day_width,
                    band.top() + ALL_DAY_PADDING_TOP,
                ),
                vec2(day_width - 5.0, band.height() - ALL_DAY_PADDING_TOP),
            );
            let widths = super::guests::widths(
                &columns
                    .iter()
                    .map(|(_, collapsed, _)| *collapsed)
                    .collect::<Vec<_>>(),
            );
            let mut left = container.left();
            for ((guest, collapsed, events), share) in columns.iter().zip(widths) {
                let width = container.width() * share;
                let rect = Rect::from_min_size(
                    pos2(left, container.top()),
                    vec2(width, container.height()),
                );
                self.hover_column(ui, band, rect, (*day, guest.clone()));
                for (row, event) in events.iter().enumerate() {
                    let block = Rect::from_min_size(
                        pos2(left + 1.0, container.top() + row as f32 * ALL_DAY_ROW),
                        vec2(width - 2.0, ALL_DAY_BLOCK),
                    );
                    let id = exo_ui::id(
                        ui.ctx(),
                        (
                            "calendar-block",
                            frame.variant,
                            &event.key,
                            "all-day",
                            *day,
                            guest.as_deref(),
                        ),
                    );
                    let response = ui.interact(block.intersect(band), id, Sense::click());
                    let focused = event.key == DRAFT;
                    if focused {
                        self.draft_anchor = Some(block);
                    }
                    if response.clicked() && !grid.form && !focused {
                        actions.push(GridAction::Select {
                            key: event.key.clone(),
                            anchor: block,
                        });
                    }
                    let active = grid
                        .invite
                        .is_some_and(|uid| event.ical_uid.as_deref() == Some(uid))
                        || grid.selected == Some(event.key.as_str());
                    let look = Look {
                        glowing: block::glow(
                            ui.ctx(),
                            id,
                            (response.hovered() || active) && !focused && !collapsed,
                        ),
                        past: event.end < *grid.now,
                        all_day: true,
                        preview: grid.form,
                        focused,
                        no_text: *collapsed,
                        clipped: true,
                        ..Look::default()
                    };
                    block::paint(ui, band, block, event, SignedDuration::from_hours(24), look);
                }
                left += width;
            }
        }
    }

    /// A day column's timed events, laid out, hovered ones on top, the one the form makes or a drag
    /// moves (`extra`) over them all.
    #[allow(clippy::too_many_arguments)] // The frame, the column and its day, what's shown, and what came of it.
    fn paint_events(
        &mut self,
        ui: &Ui,
        frame: &Frame,
        column: Rect,
        midnight: &Zoned,
        grid: &Grid<'_>,
        extra: Option<&Event>,
        actions: &mut Vec<GridAction>,
    ) {
        let clip = frame.columns;
        let day = midnight.date();
        let all: Vec<&Event> = shown(grid, extra)
            .into_iter()
            .filter(|event| !event.all_day && event.on(day))
            .collect();
        let width = column.width() - EVENTS_INSET;
        let day_end = midnight
            .tomorrow()
            .map_or(midnight.timestamp(), |next| next.timestamp());
        // Shared slots span the column under the rest, and events overlapping them are laid out from
        // 5% in (`inset: 0 1px auto 5%`). With guests' calendars shown, the draft spans the day over
        // their columns too, and nothing is indented.
        let columns_shown = !grid.guests.is_empty();
        let (slots, events): (Vec<&Event>, Vec<&Event>) = all.into_iter().partition(|event| {
            event.share.is_some()
                || (columns_shown && (event.key == DRAFT || event.key.starts_with(FREE_SLOT)))
        });
        let overlapping = if columns_shown {
            Vec::new()
        } else {
            overlapping_slots(&slots, &events)
        };
        let (indented, events): (Vec<&Event>, Vec<&Event>) = events
            .into_iter()
            .partition(|event| overlapping.contains(&event.key));
        let mut boxes: Vec<(&Event, Rect, bool, Option<InColumn>)> = Vec::new();
        // The guests' columns: mine, then theirs, side by side.
        let guest_columns = columns_shown.then(|| {
            super::guests::columns(
                grid.mine.then(|| events.clone()),
                grid.guests,
                grid.expanded,
                midnight,
            )
        });
        if let Some(columns) = &guest_columns {
            let widths = super::guests::widths(
                &columns
                    .iter()
                    .map(|column| column.collapsed)
                    .collect::<Vec<_>>(),
            );
            let mut left = column.left();
            for (guest_column, share) in columns.iter().zip(&widths) {
                let room = width * share;
                let rect = Rect::from_min_max(
                    pos2(left, column.top()),
                    pos2(left + room, column.bottom()),
                );
                self.hover_column(ui, frame.bodies, rect, (day, guest_column.guest.clone()));
                let spans: Vec<_> = guest_column
                    .events
                    .iter()
                    .map(|event| (event.start.timestamp(), event.end.timestamp()))
                    .collect();
                for place in layout::day_layout(&spans, midnight) {
                    let block = Rect::from_min_size(
                        pos2(
                            left + place.left / 100.0 * room + 1.0,
                            column.top() + place.top,
                        ),
                        vec2(place.width / 100.0 * room - 2.0, place.height),
                    );
                    // Guests' events show their time while there are two columns at most.
                    let time = guest_column.guest.is_none() || columns.len() <= 2;
                    boxes.push((
                        &guest_column.events[place.event],
                        block,
                        place.time_on_same_line,
                        Some(InColumn {
                            collapsed: guest_column.collapsed,
                            time,
                            guest: guest_column.guest.is_some(),
                        }),
                    ));
                }
                left += room;
            }
        }
        for slot in &slots {
            let top = (slot
                .start
                .timestamp()
                .max(midnight.timestamp())
                .duration_since(midnight.timestamp())
                .as_secs_f64()
                / 3600.0) as f32
                * HOUR_HEIGHT;
            let bottom = (slot
                .end
                .timestamp()
                .min(day_end)
                .duration_since(midnight.timestamp())
                .as_secs_f64()
                / 3600.0) as f32
                * HOUR_HEIGHT;
            // Over guests' columns, to the day's edge; without them, leaving room to click beside it.
            let right = if columns_shown {
                column.right()
            } else {
                column.left() + width - 1.0
            };
            let same_line = slot.share.is_none()
                && layout::day_layout(&[(slot.start.timestamp(), slot.end.timestamp())], midnight)
                    .first()
                    .is_some_and(|place| place.time_on_same_line);
            boxes.push((
                slot,
                Rect::from_min_max(
                    pos2(column.left() + 1.0, column.top() + top),
                    pos2(right, column.top() + bottom - 1.0),
                ),
                same_line,
                None,
            ));
        }
        let groups: [(&Vec<&Event>, f32, f32); 2] = [
            (&events, 0.0, width),
            (&indented, width * 0.05, width * 0.95 - 1.0),
        ];
        for (group, left, room) in groups.into_iter().filter(|_| !columns_shown) {
            let spans: Vec<_> = group
                .iter()
                .map(|event| (event.start.timestamp(), event.end.timestamp()))
                .collect();
            for place in layout::day_layout(&spans, midnight) {
                // `left: {x}%; width: calc({w}% - 2px); margin: 0 1px`.
                let rect = Rect::from_min_size(
                    pos2(
                        column.left() + left + place.left / 100.0 * room + 1.0,
                        column.top() + place.top,
                    ),
                    vec2(place.width / 100.0 * room - 2.0, place.height),
                );
                boxes.push((group[place.event], rect, place.time_on_same_line, None));
            }
        }
        // The columns' blocks' fills, for what the draft blends with.
        let theme = exo_ui::theme(ui.ctx());
        let preview =
            grid.form || grid.finding || grid.events.iter().any(|event| event.share.is_some());
        let column_blocks: Vec<(Rect, Color32)> = boxes
            .iter()
            .filter(|(.., in_column)| in_column.is_some())
            .map(|(event, rect, ..)| {
                let palette = exo_ui::BlockPalette {
                    unaccepted: event.style() == super::event::Style::Bordered,
                    create: preview,
                    past: event.end.timestamp().min(day_end) < grid.now.timestamp(),
                    focused: false,
                };
                (*rect, event.color.block(theme, palette).fill)
            })
            .collect();
        let mut glowing = Vec::new();
        for (event, rect, time_on_same_line, in_column) in boxes {
            // The right pane's grid and the week's can both show a block, on different layers.
            let id = exo_ui::id(ui.ctx(), ("calendar-block", frame.variant, &event.key, day));
            // The moved copy of a dragged event is drawn, not interacted with.
            let copy = extra.is_some_and(|extra| std::ptr::eq(extra, event)) && event.key != DRAFT;
            let id = if copy { id.with("moved") } else { id };
            // A guest's event opens its details but doesn't move; a collapsed column's bar only shows.
            let collapsed = in_column.is_some_and(|placed| placed.collapsed);
            let guest = in_column.is_some_and(|placed| placed.guest);
            let inert = copy || collapsed || guest;
            let id = if collapsed || guest {
                id.with("column")
            } else {
                id
            };
            let sense = if copy || collapsed {
                Sense::hover()
            } else if guest {
                Sense::click()
            } else {
                Sense::click_and_drag()
            };
            let response = ui.interact(rect.intersect(clip), id, sense);
            // The day's events (`width: calc(100% - 4px)`), 1px in from each side; `column` already
            // leaves out the day's right border.
            let anchor = Rect::from_x_y_ranges(
                column.left() + 1.0..=column.right() - EVENTS_INSET - 1.0,
                rect.y_range(),
            );
            if event.key == DRAFT {
                // Over guests' columns the draft spans the day, and the form goes beside that, 1px in.
                self.draft_anchor = Some(if columns_shown {
                    Rect::from_x_y_ranges(
                        column.left() + 1.0..=column.right() - 1.0,
                        anchor.y_range(),
                    )
                } else {
                    anchor
                });
            }
            // A slot's cross (`padding: 6px 6px 4px 4px` around it) removes it; a click elsewhere on
            // it does nothing.
            let cross = Rect::from_min_max(
                pos2(rect.right() - 18.0, rect.top()),
                pos2(rect.right(), rect.top() + 18.0),
            );
            if response.clicked() && event.share.is_some() {
                if response
                    .interact_pointer_pos()
                    .is_some_and(|at| cross.contains(at))
                {
                    actions.push(GridAction::Remove(event.key.clone()));
                }
            } else if (response.clicked() || self.select.as_ref() == Some(&event.key)) && !grid.form
            {
                self.select = None;
                actions.push(GridAction::Select {
                    key: event.key.clone(),
                    anchor,
                });
            }
            if !inert {
                self.take_event(ui, &response, event, rect, actions);
            }
            let rect = if inert {
                rect
            } else {
                rect.translate(jiggle(ui, &response, event, Vec2::Y, actions))
            };
            if event.editable && !inert && !grid.form {
                // `cursor: ns-resize` at the bottom edge.
                if response
                    .hover_pos()
                    .is_some_and(|at| at.y > rect.bottom() - RESIZE_EDGE)
                {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                }
            }
            let hovered = response.hovered();
            // The event's own tooltip, else `Everyone else declined` (not on previews), under the
            // block.
            if let Some(text) = event.tooltip.or_else(|| {
                (event.everyone_else_declined && !preview).then_some("Everyone else declined")
            }) {
                exo_ui::KeyTooltip { text, keys: &[] }.show_below(ui, &response, 8.0);
            }
            let focused = event.key == DRAFT || copy || event.key.starts_with(FREE_SLOT);
            let active = grid
                .invite
                .is_some_and(|uid| event.ical_uid.as_deref() == Some(uid))
                || grid.selected == Some(event.key.as_str());
            let start = event.start.timestamp().max(midnight.timestamp());
            let end = event.end.timestamp().min(day_end);
            let look = Look {
                glowing: block::glow(
                    ui.ctx(),
                    id,
                    (hovered || active) && !focused && event.share.is_none(),
                ),
                past: end < grid.now.timestamp(),
                time_on_same_line,
                // With slots picked, the rest are previews.
                preview,
                // A slot an event starts within half an hour of shows its time and cross on hover
                // only.
                covered: event.share.is_some()
                    && !hovered
                    && indented.iter().any(|other| {
                        other.start < event.end
                            && other.end > event.start
                            && other.start.timestamp()
                                < event.start.timestamp() + SignedDuration::from_mins(30)
                    }),
                focused,
                bare: event.key.starts_with(FREE_SLOT),
                // A column's event under the draft hides its text.
                no_text: in_column.is_some_and(|placed| {
                    placed.collapsed
                        || (!hovered
                            && slots.iter().any(|slot| {
                                (slot.key == DRAFT || slot.share.is_some())
                                    && overlaps_draft(slot, event)
                            }))
                }),
                no_time: in_column.is_some_and(|placed| !placed.time),
                ..Look::default()
            };
            // A hovered slot comes over the events.
            if event.share.is_some() && hovered {
                glowing.push((rect, event, end.duration_since(start), look));
            } else if event.share.is_some() {
                block::paint(ui, clip, rect, event, end.duration_since(start), look);
            } else if look.glowing > 0.0 || focused {
                glowing.push((rect, event, end.duration_since(start), look));
            } else {
                block::paint(ui, clip, rect, event, end.duration_since(start), look);
            }
        }
        // Hovered and active blocks over the rest; the focused one over them.
        glowing.sort_by_key(|(_, _, _, look)| look.focused);
        for (rect, event, span, look) in glowing {
            block::paint(ui, clip, rect, event, span, look);
            // Over guests' columns the draft and the shared slots blend with what's under them
            // (`mix-blend-mode: multiply`, Carbon's `screen`).
            if columns_shown && (event.key == DRAFT || event.share.is_some()) {
                blend_over(ui, clip, rect, event, span, look, &column_blocks);
            }
        }
    }
}

/// Repaints the parts of `event`'s block (drawn in `rect` with `look`) over `under`'s blocks as
/// `mix-blend-mode` mixes them: multiplied with the block's fill in Snow, screened in Carbon.
fn blend_over(
    ui: &Ui,
    clip: Rect,
    rect: Rect,
    event: &Event,
    span: SignedDuration,
    look: Look,
    under: &[(Rect, Color32)],
) {
    let theme = exo_ui::theme(ui.ctx());
    let own = if look.focused {
        event.color.block(
            theme,
            exo_ui::BlockPalette {
                focused: true,
                ..exo_ui::BlockPalette::default()
            },
        )
    } else {
        event.color.block(
            theme,
            exo_ui::BlockPalette {
                create: look.preview,
                ..exo_ui::BlockPalette::default()
            },
        )
    };
    let mix = |top: Color32, bottom: Color32| {
        let channel = |a: u8, b: u8| match theme {
            exo_ui::Theme::Snow => (u16::from(a) * u16::from(b) / 255) as u8,
            exo_ui::Theme::Carbon => {
                255 - ((255 - u16::from(a)) * (255 - u16::from(b)) / 255) as u8
            }
        };
        Color32::from_rgb(
            channel(top.r(), bottom.r()),
            channel(top.g(), bottom.g()),
            channel(top.b(), bottom.b()),
        )
    };
    // Screened over Carbon's dark page, the whole block lightens; multiplied over Snow's white,
    // it stays as it is.
    let page = exo_ui::palette(ui.ctx()).background_base_default;
    let base = (exo_ui::pixel_snap(rect), page);
    for (block_rect, beneath) in std::iter::once(&base)
        .filter(|_| theme == exo_ui::Theme::Carbon)
        .chain(under)
    {
        let overlap = exo_ui::pixel_snap(*block_rect)
            .intersect(exo_ui::pixel_snap(rect))
            .intersect(clip);
        if overlap.width() <= 0.0 || overlap.height() <= 0.0 {
            continue;
        }
        let beneath = *beneath;
        let colors = exo_ui::BlockColors {
            text: mix(own.text, beneath),
            fill: mix(own.fill, beneath),
            border: mix(own.fill, beneath),
        };
        block::paint(
            ui,
            overlap,
            rect,
            event,
            span,
            Look {
                colors: Some(colors),
                ..look
            },
        );
    }
}

/// `event` overlaps the draft and starts within three hours of its start.
fn overlaps_draft(draft: &Event, event: &Event) -> bool {
    event.start < draft.end
        && event.end > draft.start
        && event.start.timestamp() < draft.start.timestamp() + SignedDuration::from_hours(3)
}

/// How a block in a guests' column draws.
#[derive(Clone, Copy, Debug)]
struct InColumn {
    /// Its column is collapsed to bars, without text.
    collapsed: bool,
    /// It shows its time.
    time: bool,
    /// A guest's: a click opens its details, it doesn't move.
    guest: bool,
}

/// The events overlapping a slot: in each run of events that overlap one another, every one but
/// the slots, when the run has a slot.
fn overlapping_slots(slots: &[&Event], events: &[&Event]) -> Vec<String> {
    if slots.is_empty() || events.is_empty() {
        return Vec::new();
    }
    let mut all: Vec<(&Event, bool)> = slots
        .iter()
        .map(|slot| (*slot, true))
        .chain(events.iter().map(|event| (*event, false)))
        .collect();
    all.sort_by_key(|(event, _)| event.start.timestamp());
    let mut found = Vec::new();
    let mut run: Vec<(&Event, bool)> = Vec::new();
    let mut run_end = all[0].0.end.timestamp();
    for (event, slot) in all {
        if !run.is_empty() && event.start.timestamp() >= run_end {
            if run.iter().any(|(_, slot)| *slot) {
                found.extend(
                    run.iter()
                        .filter(|(_, slot)| !*slot)
                        .map(|(event, _)| event.key.clone()),
                );
            }
            run.clear();
        }
        run_end = if run.is_empty() {
            event.end.timestamp()
        } else {
            run_end.max(event.end.timestamp())
        };
        run.push((event, slot));
    }
    if run.iter().any(|(_, slot)| *slot) {
        found.extend(
            run.iter()
                .filter(|(_, slot)| !*slot)
                .map(|(event, _)| event.key.clone()),
        );
    }
    found
}

/// The events drawn: all but the one being edited, then the draft or the dragged copy.
fn shown<'a>(grid: &Grid<'a>, extra: Option<&'a Event>) -> Vec<&'a Event> {
    let dragging_draft = extra.is_some_and(|extra| extra.key == DRAFT);
    let mut events: Vec<&Event> = grid
        .events
        .iter()
        .filter(|event| Some(event.key.as_str()) != grid.hidden)
        .collect();
    if !dragging_draft {
        events.extend(grid.draft);
    }
    events.extend(extra);
    events
}

/// `day`'s minutes `from` to `to` as times.
fn span(grid: &Grid<'_>, day: usize, from: i32, to: i32) -> Option<(Zoned, Zoned)> {
    let midnight = grid.days.get(day)?.to_zoned(grid.tz.clone()).ok()?;
    Some((
        midnight.checked_add(i64::from(from).minutes()).ok()?,
        midnight.checked_add(i64::from(to).minutes()).ok()?,
    ))
}

/// The days `first` to `last` as an all-day span: midnight to the midnight after.
fn days_span(grid: &Grid<'_>, first: usize, last: usize) -> Option<(Zoned, Zoned)> {
    let start = grid.days.get(first)?.to_zoned(grid.tz.clone()).ok()?;
    let end = grid
        .days
        .get(last)?
        .tomorrow()
        .ok()?
        .to_zoned(grid.tz.clone())
        .ok()?;
    Some((start, end))
}

/// A new event being dragged out, drawn as the draft is.
fn draft_like(grid: &Grid<'_>, start: Zoned, end: Zoned, all_day: bool) -> Event {
    let color = grid
        .draft
        .map_or(exo_ui::CalendarColor::BLUE, |draft| draft.color);
    Event::draft(
        DRAFT.to_owned(),
        "(No title)".to_owned(),
        start,
        end,
        all_day,
        color,
    )
}

/// Where Chromium paints a border edge laid out at `at`: on the whole px.
fn snap(at: f32) -> f32 {
    (at + 0.5).floor()
}

/// The week's gutter: wider with secondary time zones, and wider still when one is off the
/// hour from this one on the week's first day.
fn week_gutter(grid: &Grid<'_>) -> f32 {
    if grid.zones.is_empty() {
        return WEEK_GUTTER;
    }
    let at = grid
        .days
        .first()
        .and_then(|day| day.to_zoned(grid.tz.clone()).ok())
        .map(|day| day.timestamp());
    let local = at.map_or(0, |at| grid.tz.to_offset(at).seconds());
    let off_hour = grid
        .zones
        .iter()
        .any(|zone| at.is_some_and(|at| (zone.to_offset(at).seconds() - local) % 3600 != 0));
    if off_hour {
        MINUTE_ZONES_GUTTER
    } else {
        ZONES_GUTTER
    }
}

/// An hour label: `9 am` on the hour, `9:15 am` otherwise.
pub fn hour_label(hour: i32, minute: i32) -> String {
    let (twelve, half) = match hour.rem_euclid(24) {
        0 => (12, "am"),
        hour @ 1..=11 => (hour, "am"),
        12 => (12, "pm"),
        hour => (hour - 12, "pm"),
    };
    if minute == 0 {
        format!("{twelve} {half}")
    } else {
        format!("{twelve}:{minute:02} {half}")
    }
}

/// A 1px line in `color` that fades in from transparent over its first `ramp` px.
fn paint_fade_in(painter: &Painter, line: Rect, ramp: f32, color: Color32) {
    let mid = (line.left() + ramp).min(line.right());
    paint_gradient(
        painter,
        Rect::from_min_max(line.min, pos2(mid, line.bottom())),
        Color32::TRANSPARENT,
        color,
    );
    painter.rect_filled(
        Rect::from_min_max(pos2(mid, line.top()), line.max),
        0.0,
        color,
    );
}

/// `rect` filled with a left-to-right gradient.
fn paint_gradient(painter: &Painter, rect: Rect, from: Color32, to: Color32) {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
    let mut mesh = Mesh::default();
    for (pos, color) in [
        (rect.left_top(), from),
        (rect.right_top(), to),
        (rect.right_bottom(), to),
        (rect.left_bottom(), from),
    ] {
        mesh.vertices.push(Vertex {
            pos,
            uv: WHITE_UV,
            color,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secondary_zone_widens_the_gutter_more_when_off_the_hour() {
        let new_york = TimeZone::get("America/New_York").expect("zone");
        let now = Date::constant(2026, 11, 17)
            .at(9, 0, 0, 0)
            .to_zoned(new_york.clone())
            .expect("a time");
        let days = [Date::constant(2026, 11, 15)];
        let gutter = |zones: &[TimeZone]| {
            let grid = Grid {
                days: &days,
                events: &[],
                now: &now,
                tz: &new_york,
                invite: None,
                selected: None,
                draft: None,
                hidden: None,
                form: false,
                details: false,
                finding: false,
                guests: &[],
                expanded: None,
                mine: true,
                sharing: None,
                zones,
            };
            week_gutter(&grid)
        };
        assert_eq!(gutter(&[]), WEEK_GUTTER);
        assert_eq!(
            gutter(&[TimeZone::get("Europe/London").expect("zone")]),
            ZONES_GUTTER
        );
        assert_eq!(
            gutter(&[TimeZone::get("Asia/Kolkata").expect("zone")]),
            MINUTE_ZONES_GUTTER
        );
    }

    #[test]
    fn hour_labels_show_minutes_only_off_the_hour() {
        assert_eq!(hour_label(8, 0), "8 am");
        assert_eq!(hour_label(12, 0), "12 pm");
        assert_eq!(hour_label(13, 15), "1:15 pm");
        assert_eq!(hour_label(0, 45), "12:45 am");
    }
}
