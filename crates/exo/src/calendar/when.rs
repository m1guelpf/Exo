//! A new event's When: the typed text, read by the time parser as the calendar reads it, its
//! three suggestions listed under the field, committed with Enter, Tab or a click.

use egui::{Color32, CornerRadius, Rect, Sense, Ui, pos2, vec2};
use exo_ui::{BoxShadow, Family, Font, TextStyle, Theme};
use jiff::{SignedDuration, Timestamp, ToSpan, Zoned, tz::TimeZone};

use crate::time_suggest::{self, Options, ZoneTime};

/// The suggestions' text, 12px/16px.
const SMALL: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// A suggestion: 10px over and under a 16px line, the first 3px down (and every one nudged
/// `top: 1px`), 6px under the last.
const ITEM: f32 = 10.0 + 16.0 + 10.0;
const LIST_TOP: f32 = 3.0;
const LIST_BOTTOM: f32 = 6.0;
/// The input's box: its 16px line, 2px of padding and its 2px underline.
const INPUT: f32 = 20.0;
/// The list sits 12px under the input.
const GAP: f32 = 12.0;
/// The line with the time here, under the input: 4px under it, 8px over the next row.
pub const ZONE_LINE: f32 = 4.0 + 16.0 + 8.0;
/// Where that line's text sits, from the input line's top.
pub const ZONE_LINE_TOP: f32 = INPUT + 4.0;
/// A draft shorter than this shows its start alone.
const SHORT: SignedDuration = SignedDuration::from_mins(44);

/// A suggestion's time.
#[derive(Clone, Debug)]
pub struct Choice {
    pub start: Zoned,
    pub end: Zoned,
    pub all_day: bool,
    /// The time as typed in another zone.
    pub zone: Option<ZoneTime>,
}

/// The field's state.
#[derive(Default)]
pub struct When {
    /// What the input holds.
    pub text: String,
    /// The times the text was written for; a change made elsewhere (the grid, ⌘⇧=) writes the
    /// new ones out.
    written: Option<(Option<(Zoned, Zoned)>, bool)>,
    /// The zone the time was typed in.
    pub zone: Option<ZoneTime>,
    choices: Vec<Choice>,
    /// The highlighted suggestion.
    pub selected: usize,
    /// The text reads as nothing.
    pub error: bool,
    /// Where the suggestions' list ended this frame, if it showed.
    pub list_bottom: Option<f32>,
}

impl When {
    /// Writes `when` out unless the text was written for it already.
    pub fn sync(&mut self, when: Option<&(Zoned, Zoned)>, all_day: bool, now: &Zoned) {
        if self.written.as_ref().is_some_and(|(written, was_all_day)| {
            written.as_ref() == when && *was_all_day == all_day
        }) {
            return;
        }
        self.text = when
            .map(|(start, end)| when_text(start, end, all_day, now))
            .unwrap_or_default();
        self.zone = None;
        self.choices.clear();
        self.error = false;
        self.mark_written(when, all_day);
    }

    /// The text now stands for `when`.
    pub fn mark_written(&mut self, when: Option<&(Zoned, Zoned)>, all_day: bool) {
        self.written = Some((when.cloned(), all_day));
    }

    /// The text changed: its suggestions, and whether it reads as anything.
    pub fn edited(&mut self, now: &Zoned) {
        self.choices = suggest(&self.text, now);
        self.selected = 0;
        self.error = !self.text.is_empty() && self.choices.is_empty();
    }

    /// The field took the keyboard: without a time yet, the text's suggestions.
    pub fn focused(&mut self, has_time: bool, now: &Zoned) {
        self.choices = if has_time {
            Vec::new()
        } else {
            suggest(&self.text, now)
        };
        self.selected = 0;
    }

    /// Escape: the text gone.
    pub fn clear(&mut self) {
        self.text.clear();
        self.choices.clear();
        self.error = false;
    }

    /// The suggestions listed (the field focused).
    pub fn choices(&self) -> &[Choice] {
        &self.choices
    }

    /// ↑ (-1) / ↓ (1), round the list.
    pub fn step(&mut self, delta: isize) {
        let count = self.choices.len() as isize;
        if count > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(count) as usize;
        }
    }

    /// Takes suggestion `index`: the text becomes its time written out, in the zone it was typed
    /// in with that zone's abbreviation (written as an all-day event when the form already was
    /// one). Returns its start, end and whether it's all day.
    pub fn commit(&mut self, index: usize, all_day: bool, now: &Zoned) -> Option<Choice> {
        let choice = self.choices.get(index)?.clone();
        self.text = match &choice.zone {
            Some(zone) => {
                let (start, end) = (
                    zone.start.to_zoned(now.time_zone().clone()),
                    zone.end.to_zoned(now.time_zone().clone()),
                );
                format!(
                    "{} {}",
                    when_text(&start, &end, all_day, now),
                    abbreviation(zone.tz.as_ref().unwrap_or(now.time_zone()), zone.start)
                )
            }
            None => when_text(&choice.start, &choice.end, choice.all_day, now),
        };
        self.zone = choice.zone.clone();
        self.choices.clear();
        self.error = false;
        self.mark_written(
            Some(&(choice.start.clone(), choice.end.clone())),
            choice.all_day,
        );
        Some(choice)
    }

    /// The line under the field for a time typed in another zone: the
    /// time here, with this zone's abbreviation. It's drawn 4px under the input's box.
    pub fn local_line(
        &self,
        when: Option<&(Zoned, Zoned)>,
        all_day: bool,
        now: &Zoned,
    ) -> Option<String> {
        self.zone.as_ref()?;
        let (start, end) = when?;
        Some(format!(
            "{} {}",
            when_text(start, end, all_day, now),
            abbreviation(now.time_zone(), start.timestamp())
        ))
    }

    /// Paints the list under the field's `input` line, 12px under its box:
    /// in the week's modal from 14px left of the input, at most 410px wide; in the right pane
    /// across `form` less 10px each side. Returns the suggestion clicked.
    pub fn paint_list(
        &mut self,
        ui: &Ui,
        input: Rect,
        form: Rect,
        pane: bool,
        now: &Zoned,
    ) -> Option<usize> {
        let ctx = ui.ctx().clone();
        let palette = exo_ui::palette(&ctx);
        let (left, right) = if pane {
            (form.left() + 10.0, form.right() - 10.0)
        } else {
            (
                input.left() - 14.0,
                (input.left() - 14.0 + 410.0).min(form.right() - 10.0),
            )
        };
        let top = input.top() + INPUT + GAP;
        let list = Rect::from_min_max(
            pos2(left, top),
            pos2(
                right,
                top + LIST_TOP + self.choices.len() as f32 * ITEM + LIST_BOTTOM,
            ),
        );
        self.list_bottom = Some(list.bottom());
        let snow = palette.theme == Theme::Snow;
        // The list and its highlighted row keep their own colours, not the palette's.
        let background = if snow {
            Color32::WHITE
        } else {
            Color32::from_rgb(0x27, 0x29, 0x2d)
        };
        let selected = if snow {
            Color32::from_rgb(0xf2, 0xf4, 0xfa)
        } else {
            Color32::from_rgb(0x46, 0x4b, 0x53)
        };
        let (text, dim) = (palette.ink(0.9, 0.9), palette.ink(0.5, 0.5));
        // `padding: 10px calc(2rem - 10px)`.
        let padding = 2.0 * exo_ui::rem(ctx.content_rect().width(), false) - 10.0;
        // Several zones among the suggestions show each one's time here instead of its length.
        let mut zones: Vec<Option<&str>> = self
            .choices
            .iter()
            .map(|choice| {
                choice
                    .zone
                    .as_ref()
                    .and_then(|zone| zone.tz.as_ref())
                    .and_then(TimeZone::iana_name)
            })
            .collect();
        zones.dedup();
        let many_zones = zones.len() > 1;
        let mut clicked = None;
        egui::Area::new(exo_ui::id(&ctx, "calendar-form-when-list"))
            .order(egui::Order::Tooltip)
            .fixed_pos(list.min)
            .constrain(false)
            .show(&ctx, |area| {
                let painter = area.painter().clone();
                let snapped = exo_ui::pixel_snap(list);
                BoxShadow::new([0.0, 2.0], 8.0, 0.0, Color32::from_black_alpha(26)).paint(
                    &painter,
                    snapped,
                    CornerRadius::same(4),
                );
                painter.rect_filled(snapped, CornerRadius::same(4), background);
                for (index, choice) in self.choices.iter().enumerate() {
                    let row = Rect::from_min_size(
                        pos2(
                            list.left(),
                            list.top() + LIST_TOP + 1.0 + index as f32 * ITEM,
                        ),
                        vec2(list.width(), ITEM),
                    );
                    let hit = area
                        .interact(
                            row,
                            exo_ui::id(area.ctx(), ("calendar-form-when-choice", index)),
                            Sense::click(),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if hit.hovered() {
                        self.selected = index;
                    }
                    if index == self.selected {
                        painter.rect_filled(row, 0.0, selected);
                    }
                    let line = pos2(row.left() + padding, row.top() + 11.0);
                    let duration = duration_text(choice, many_zones, now);
                    let duration_width = SMALL.width(&ctx, &duration);
                    SMALL.paint_line(
                        &painter,
                        pos2(row.right() - padding - duration_width, line.y),
                        &duration,
                        dim,
                        None,
                    );
                    let mut x = line.x;
                    let room = row.right() - padding - duration_width - 6.0;
                    for (segment, dimmed) in time_label(choice, now) {
                        x += SMALL.paint_line(
                            &painter,
                            pos2(x, line.y),
                            &segment,
                            if dimmed { dim } else { text },
                            Some((room - x).max(0.0)),
                        );
                    }
                    if hit.clicked() {
                        clicked = Some(index);
                    }
                }
            });
        clicked
    }
}

/// The calendar's suggestions for `text` (none for no text).
fn suggest(text: &str, now: &Zoned) -> Vec<Choice> {
    if text.is_empty() {
        return Vec::new();
    }
    let options = Options {
        calendar: true,
        allow_past: true,
        ..Options::default()
    };
    let tz = now.time_zone().clone();
    time_suggest::suggest(text, &options, now)
        .into_iter()
        .filter_map(|suggestion| {
            let start = suggestion.start.to_zoned(tz.clone());
            let end = suggestion.end?.to_zoned(tz.clone());
            Some(Choice {
                start,
                end,
                all_day: suggestion.all_day,
                zone: suggestion.zone,
            })
        })
        .collect()
}

/// The zone's abbreviation at `at`: `GMT`, `EST`.
fn abbreviation(tz: &TimeZone, at: Timestamp) -> String {
    tz.to_offset_info(at).abbreviation().to_owned()
}

/// The parts both the field's text and a suggestion's label use.
struct Parts {
    date: String,
    start_time: String,
    end_time: String,
    /// The end's date, for an event over more than a day.
    end_date: Option<String>,
    /// All day, and a single day.
    one_day: bool,
}

fn parts(start: &Zoned, end: &Zoned, all_day: bool, now: &Zoned) -> Parts {
    let this_year = start.year() == now.year() && end.year() == now.year();
    let date = |at: &Zoned| {
        let base = format!(
            "{}, {} {}",
            at.strftime("%a"),
            at.strftime("%b"),
            crate::format::dates::ordinal(at.day())
        );
        if this_year {
            base
        } else {
            format!("{base}, {}", at.year())
        }
    };
    let other_day = start.date() != end.date();
    let start_time = if other_day || (start.hour() < 12 && end.hour() >= 12) {
        start.strftime("%-I:%M %p")
    } else {
        start.strftime("%-I:%M")
    };
    let end_date = other_day.then(|| {
        if all_day {
            date(&end.checked_sub(1.day()).unwrap_or_else(|_| end.clone()))
        } else {
            date(end)
        }
    });
    Parts {
        date: date(start),
        start_time: start_time.to_string(),
        end_time: end.strftime("%-I:%M %p").to_string(),
        end_date,
        one_day: all_day
            && end.timestamp().duration_since(start.timestamp()) == SignedDuration::from_hours(24),
    }
}

/// The field's text: `Tue, Nov 17th 9:00 – 9:30 AM`, the end's date when it's another day, the
/// year outside this one, `Thu, Nov 26th (all day)` or `Thu, Nov 26th – Fri, Nov 27th` for
/// all-day events.
pub fn when_text(start: &Zoned, end: &Zoned, all_day: bool, now: &Zoned) -> String {
    let parts = parts(start, end, all_day, now);
    let mut text = if all_day {
        format!(
            "{} {}",
            parts.date,
            if parts.one_day { "(all day)" } else { "–" }
        )
    } else {
        format!("{} {} –", parts.date, parts.start_time)
    };
    match (&parts.end_date, all_day) {
        (Some(_), true) if parts.one_day => {}
        (Some(end_date), true) => text = format!("{text} {end_date}"),
        (Some(end_date), false) => text = format!("{text} {end_date} {}", parts.end_time),
        (None, _) => text = format!("{text} {}", parts.end_time),
    }
    text
}

/// A suggestion's time: `Wed, Nov 18th · 2:00 – 2:30 PM`, in the zone it was typed in
/// (`… PM in London`), as runs of text and whether each is dimmed (the dots). An all-day
/// suggestion of one day is its date.
fn time_label(choice: &Choice, now: &Zoned) -> Vec<(String, bool)> {
    let tz = now.time_zone();
    let (start, end, zone) = match &choice.zone {
        Some(zone) => (
            zone.start.to_zoned(tz.clone()),
            zone.end.to_zoned(tz.clone()),
            Some(zone.label.as_str()),
        ),
        None => (choice.start.clone(), choice.end.clone(), None),
    };
    let parts = parts(&start, &end, choice.all_day, now);
    let end_time = match zone {
        Some(zone) => format!("{} {zone}", parts.end_time),
        None => parts.end_time.clone(),
    };
    let plain = |text: String| (text, false);
    let dot = || (" · ".to_owned(), true);
    let mut runs = vec![plain(parts.date.clone())];
    if !choice.all_day {
        runs.extend([dot(), plain(format!("{} –", parts.start_time))]);
    } else if !parts.one_day {
        runs.push(plain(" –".to_owned()));
    }
    match &parts.end_date {
        Some(end_date) if !parts.one_day => {
            runs.push(plain(format!(" {end_date}")));
            if !choice.all_day {
                runs.extend([dot(), plain(end_time)]);
            }
        }
        Some(_) => {}
        None => runs.push(plain(format!(" {end_time}"))),
    }
    runs
}

/// A suggestion's length: `30 mins`, `1 hr`, `1.5 hrs`, `all day`, `3 days`, `2 wks`, `4 mos`,
/// `1 yr`; or, when the suggestions are in several zones, the start here (`9:00 AM EST`).
fn duration_text(choice: &Choice, many_zones: bool, now: &Zoned) -> String {
    if many_zones {
        return format!(
            "{} {}",
            choice.start.strftime("%-I:%M %p"),
            abbreviation(now.time_zone(), choice.start.timestamp())
        );
    }
    let millis = choice
        .end
        .timestamp()
        .duration_since(choice.start.timestamp())
        .as_millis() as f64;
    // To one decimal place.
    let round = |unit: f64| (millis / unit * 10.0).round() / 10.0;
    let plural = |count: f64, unit: &str| {
        let number = if count.fract() == 0.0 {
            format!("{}", count as i64)
        } else {
            format!("{count}")
        };
        format!("{number} {unit}{}", if count > 1.0 { "s" } else { "" })
    };
    const MINUTE: f64 = 60_000.0;
    const DAY: f64 = 24.0 * 60.0 * MINUTE;
    let (minutes, hours, days, weeks, months, years) = (
        round(MINUTE),
        round(60.0 * MINUTE),
        round(DAY),
        round(7.0 * DAY),
        round(30.0 * DAY),
        round(365.0 * DAY),
    );
    if minutes < 60.0 {
        plural(minutes, "min")
    } else if hours < 24.0 || !choice.all_day {
        plural(hours, "hr")
    } else if days < 7.0 {
        if days == 1.0 {
            "all day".to_owned()
        } else {
            plural(days, "day")
        }
    } else if weeks < 4.0 {
        plural(weeks, "wk")
    } else if months < 12.0 {
        plural(months, "mo")
    } else {
        plural(years, "yr")
    }
}

/// A draft's time on its block when it was typed in another zone, read in
/// that zone (`2:00 PM GMT`, `2:00 - 3:00 PM GMT`).
pub fn zone_block_time(zone: &ZoneTime, now: &Zoned) -> String {
    let tz = now.time_zone();
    let (start, end) = (
        zone.start.to_zoned(tz.clone()),
        zone.end.to_zoned(tz.clone()),
    );
    let abbreviation = abbreviation(zone.tz.as_ref().unwrap_or(tz), zone.start);
    if end.timestamp().duration_since(start.timestamp()) < SHORT {
        return format!("{} {abbreviation}", start.strftime("%-I:%M %p"));
    }
    let start_format = if start.date() != end.date() || (start.hour() < 12 && end.hour() >= 12) {
        "%-I:%M %p"
    } else {
        "%-I:%M"
    };
    format!(
        "{} - {} {abbreviation}",
        start.strftime(start_format),
        end.strftime("%-I:%M %p")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Zoned {
        "2026-11-17T09:00[America/New_York]".parse().expect("now")
    }

    fn at(text: &str) -> Zoned {
        format!("{text}[America/New_York]").parse().expect("time")
    }

    fn label(choice: &Choice) -> String {
        time_label(choice, &now())
            .into_iter()
            .map(|(text, _)| text)
            .collect()
    }

    #[test]
    fn when_is_written_with_its_day_times_and_year() {
        let now = now();
        assert_eq!(
            when_text(
                &at("2026-11-17T09:00"),
                &at("2026-11-17T09:30"),
                false,
                &now
            ),
            "Tue, Nov 17th 9:00 – 9:30 AM"
        );
        assert_eq!(
            when_text(
                &at("2026-11-17T11:30"),
                &at("2026-11-17T12:30"),
                false,
                &now
            ),
            "Tue, Nov 17th 11:30 AM – 12:30 PM"
        );
        assert_eq!(
            when_text(
                &at("2026-11-17T23:00"),
                &at("2026-11-18T01:00"),
                false,
                &now
            ),
            "Tue, Nov 17th 11:00 PM – Wed, Nov 18th 1:00 AM"
        );
        assert_eq!(
            when_text(&at("2026-11-26T00:00"), &at("2026-11-27T00:00"), true, &now),
            "Thu, Nov 26th (all day)"
        );
        assert_eq!(
            when_text(&at("2026-11-26T00:00"), &at("2026-11-28T00:00"), true, &now),
            "Thu, Nov 26th – Fri, Nov 27th"
        );
        assert_eq!(
            when_text(
                &at("2027-01-01T14:00"),
                &at("2027-01-01T15:00"),
                false,
                &now
            ),
            "Fri, Jan 1st, 2027 2:00 – 3:00 PM"
        );
    }

    #[test]
    fn suggestions_label_their_time_length_and_zone() {
        let now = now();
        let mut when = When {
            text: "tomorrow 2pm".into(),
            ..When::default()
        };
        when.edited(&now);
        let rows: Vec<(String, String)> = when
            .choices()
            .iter()
            .map(|choice| (label(choice), duration_text(choice, false, &now)))
            .collect();
        let expected = [
            ("Wed, Nov 18th · 2:00 – 2:30 PM", "30 mins"),
            ("Wed, Nov 18th · 2:00 – 3:00 PM", "1 hr"),
            ("Wed, Nov 18th · 2:00 – 2:15 PM", "15 mins"),
        ];
        assert_eq!(
            rows,
            expected.map(|(label, length)| (label.to_owned(), length.to_owned()))
        );

        when.text = "tomorrow 2pm london".into();
        when.edited(&now);
        assert_eq!(
            label(&when.choices()[0]),
            "Wed, Nov 18th · 2:00 – 2:30 PM in London"
        );
        let committed = when.commit(0, false, &now).expect("a suggestion");
        assert_eq!(
            (committed.start, committed.end),
            (at("2026-11-18T09:00"), at("2026-11-18T09:30"))
        );
        assert_eq!(when.text, "Wed, Nov 18th 2:00 – 2:30 PM GMT");
        assert_eq!(
            when.local_line(
                Some(&(at("2026-11-18T09:00"), at("2026-11-18T09:30"))),
                false,
                &now
            )
            .as_deref(),
            Some("Wed, Nov 18th 9:00 – 9:30 AM EST")
        );
        assert_eq!(
            zone_block_time(when.zone.as_ref().expect("typed in London"), &now),
            "2:00 PM GMT"
        );

        when.text = "blah".into();
        when.edited(&now);
        assert!(when.error && when.choices().is_empty());
    }

    #[test]
    fn lengths_are_written_in_the_largest_unit() {
        let now = now();
        let choice = |end: &str, all_day: bool| Choice {
            start: at("2026-11-18T00:00"),
            end: at(end),
            all_day,
            zone: None,
        };
        assert_eq!(
            duration_text(&choice("2026-11-18T01:30", false), false, &now),
            "1.5 hrs"
        );
        assert_eq!(
            duration_text(&choice("2026-11-19T00:00", true), false, &now),
            "all day"
        );
        assert_eq!(
            duration_text(&choice("2026-11-21T00:00", true), false, &now),
            "3 days"
        );
        assert_eq!(
            duration_text(&choice("2026-12-02T00:00", true), false, &now),
            "2 wks"
        );
        assert_eq!(
            duration_text(&choice("2026-11-21T00:00", false), false, &now),
            "72 hrs"
        );
        assert_eq!(label(&choice("2026-11-19T00:00", true)), "Wed, Nov 18th");
        assert_eq!(
            label(&choice("2026-11-21T00:00", true)),
            "Wed, Nov 18th – Fri, Nov 20th"
        );
    }
}
