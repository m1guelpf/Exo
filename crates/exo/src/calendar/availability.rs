//! Free times as text: the slots picked on the week, merged and split by day in the zone they're
//! shared in, written as one sentence for a day of up to three slots, else as a list of days.

use jiff::{ToSpan, Zoned, civil::Date, tz::TimeZone};

/// Who the times are shared with, as far as the wording goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Person {
    /// On the account's team domains.
    pub internal: bool,
    /// Me.
    pub organizer: bool,
}

/// The text for `slots` (each a start and end) of a `minutes`-long meeting, in `tz`, for
/// `people`, as of `now`. Empty without slots.
pub fn text(
    slots: &[(Zoned, Zoned)],
    minutes: i64,
    people: &[Person],
    tz: &TimeZone,
    now: &Zoned,
) -> String {
    let days = by_day(slots, tz);
    let Some(first) = days.first() else {
        return String::new();
    };
    let now = now.with_time_zone(tz.clone());
    // The zone as it is at the earliest slot.
    let zone = days
        .iter()
        .flat_map(|(_, slots)| slots)
        .map(|(start, _)| start)
        .min()
        .map(|start| abbreviation(tz, start.timestamp()))
        .unwrap_or_default();
    let all_meeting_length = days
        .iter()
        .flat_map(|(_, slots)| slots)
        .all(|(start, end)| {
            end.timestamp().duration_since(start.timestamp()).as_secs() == minutes * 60
        });
    let length = duration(minutes, true);
    if days.len() > 1 || first.1.len() >= 4 {
        let lead = if all_meeting_length {
            "Do".to_owned()
        } else {
            format!("Does {length} during")
        };
        let us = if several_of_us(people) {
            " to meet with us"
        } else {
            ""
        };
        let mut lines = vec![format!("{lead} any of these times ({zone}) work{us}?")];
        for (_, slots) in &days {
            let ranges: Vec<String> = slots
                .iter()
                .map(|(start, end)| time_range(start, end))
                .collect();
            lines.push(format!(
                "- {}: {}",
                semi_relative(&slots[0].0, &now),
                ranges.join(", ")
            ));
        }
        return lines.join("\n");
    }
    let slots = &first.1;
    let start = &slots[0].0;
    let day = semi_relative(start, &now);
    let relative = start.date() == now.date()
        || now
            .date()
            .tomorrow()
            .is_ok_and(|tomorrow| start.date() == tomorrow);
    let ranges: Vec<String> = slots
        .iter()
        .map(|(start, end)| time_range(start, end))
        .collect();
    let ranges = match ranges.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!(
            "{}{} or {last}",
            rest.join(", "),
            if ranges.len() > 2 { "," } else { "" }
        ),
        None => String::new(),
    };
    let who = if several_of_us(people) { "We" } else { "I" };
    // Both left out leave the template's two spaces.
    let length = if all_meeting_length {
        ""
    } else {
        length.as_str()
    };
    let on = if !relative && !all_meeting_length {
        " on"
    } else {
        ""
    };
    format!("Does {length}{on} {day} work? {who} can do {ranges} {zone}.")
}

/// The slots in `tz`, overlapping ones merged (across days too), each cut at midnight, grouped by
/// their start's day in order.
pub fn by_day(slots: &[(Zoned, Zoned)], tz: &TimeZone) -> Vec<(Date, Vec<(Zoned, Zoned)>)> {
    let mut sorted: Vec<(Zoned, Zoned)> = slots
        .iter()
        .map(|(start, end)| {
            (
                start.with_time_zone(tz.clone()),
                end.with_time_zone(tz.clone()),
            )
        })
        .collect();
    sorted.sort_by_key(|(start, _)| start.timestamp());
    let mut merged: Vec<(Zoned, Zoned)> = Vec::new();
    for (start, end) in sorted {
        match merged.last_mut() {
            Some(last) if start <= last.1 => {
                if end > last.1 {
                    last.1 = end;
                }
            }
            _ => merged.push((start, end)),
        }
    }
    let mut days: Vec<(Date, Vec<(Zoned, Zoned)>)> = Vec::new();
    for slot in merged.iter().flat_map(split_across_days) {
        let date = slot.0.date();
        match days.last_mut() {
            Some((day, slots)) if *day == date => slots.push(slot),
            _ => days.push((date, vec![slot])),
        }
    }
    days
}

/// A slot over midnight as one piece a day; one ending at midnight doesn't reach the next.
fn split_across_days((start, end): &(Zoned, Zoned)) -> Vec<(Zoned, Zoned)> {
    let midnight = |date: Date| date.to_zoned(start.time_zone().clone()).ok();
    let mut last = end.date();
    if midnight(last).is_some_and(|at| at == *end) && last > start.date() {
        last = last.yesterday().unwrap_or(last);
    }
    if last <= start.date() {
        return vec![(start.clone(), end.clone())];
    }
    let mut pieces = Vec::new();
    let mut day = start.date();
    while day <= last {
        let from = if day == start.date() {
            Some(start.clone())
        } else {
            midnight(day)
        };
        let to = if day == last {
            Some(end.clone())
        } else {
            day.tomorrow().ok().and_then(midnight)
        };
        if let (Some(from), Some(to)) = (from, to) {
            pieces.push((from, to));
        }
        let Ok(next) = day.tomorrow() else { break };
        day = next;
    }
    pieces
}

/// Several of us meet: someone outside and a teammate besides me.
fn several_of_us(people: &[Person]) -> bool {
    people.iter().any(|person| !person.internal)
        && people
            .iter()
            .any(|person| person.internal && !person.organizer)
}

/// A meeting's length: `30 min`, `an hour` (in a sentence), `1.5 hrs`.
pub fn duration(minutes: i64, natural: bool) -> String {
    if minutes > 60 {
        // The quotient as it prints: `2`, `1.5`.
        format!("{} hrs", minutes as f64 / 60.0)
    } else if minutes == 60 && natural {
        "an hour".to_owned()
    } else {
        format!("{minutes} min")
    }
}

/// A time range on one day: `2:00–3:00 pm`, or `11:30 am–12:30 pm` across noon.
pub fn time_range(start: &Zoned, end: &Zoned) -> String {
    let (from, to) = (clock(start), clock(end));
    if from.1 == to.1 {
        format!("{}–{} {}", from.0, to.0, to.1)
    } else {
        format!("{} {}–{} {}", from.0, from.1, to.0, to.1)
    }
}

/// A time on the 12-hour clock: `2:05` and `pm`.
fn clock(at: &Zoned) -> (String, &'static str) {
    let hour = at.hour();
    let twelve = if hour % 12 == 0 { 12 } else { hour % 12 };
    (
        format!("{twelve}:{:02}", at.minute()),
        if hour >= 12 { "pm" } else { "am" },
    )
}

/// A day, relative while it's near: `today (Nov 17)`, `tomorrow (Nov 18)`, `Wed Nov 25` within
/// the week, `Nov 30` after, the year added outside this one (`Jan 5 2027`). `now` is in `at`'s
/// zone.
pub fn semi_relative(at: &Zoned, now: &Zoned) -> String {
    let today = now.date();
    let month_day = format!("{} {}", at.strftime("%b"), at.day());
    if at.date() == today {
        return format!("today ({month_day})");
    }
    if today.tomorrow().is_ok_and(|tomorrow| at.date() == tomorrow) {
        return format!("tomorrow ({month_day})");
    }
    let absolute = if at.year() == today.year() {
        month_day
    } else {
        format!("{month_day} {}", at.year())
    };
    // Before the start of the eighth day from today.
    let week = today
        .checked_add(8.days())
        .ok()
        .and_then(|day| day.to_zoned(at.time_zone().clone()).ok());
    if week.is_some_and(|week| *at < week) {
        format!("{} {absolute}", at.strftime("%a"))
    } else {
        absolute
    }
}

/// The zone's abbreviation at `at`, a bare offset written as one (`-03` is `GMT−03:00`, `+0530`
/// is `GMT+05:30`).
pub fn abbreviation(tz: &TimeZone, at: jiff::Timestamp) -> String {
    let abbreviation = tz.to_offset_info(at).abbreviation().to_owned();
    if !abbreviation.chars().any(|ch| ch.is_ascii_digit()) {
        return abbreviation;
    }
    let abbreviation = abbreviation.replacen('-', "−", 1);
    let prefix = if abbreviation.starts_with("GMT") {
        ""
    } else {
        "GMT"
    };
    let digits = abbreviation
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit() || *ch == ':')
        .filter(char::is_ascii_digit)
        .count();
    if digits >= 3 {
        let (head, minutes) = abbreviation.split_at(abbreviation.len() - 2);
        let colon = if head.ends_with(':') { "" } else { ":" };
        format!("{prefix}{head}{colon}{minutes}")
    } else {
        format!("{prefix}{abbreviation}:00")
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    fn new_york() -> TimeZone {
        TimeZone::get("America/New_York").expect("zone")
    }

    /// gmail-mock's now: Tuesday, November 17th 2026, 9:00 in New York.
    fn now() -> Zoned {
        date(2026, 11, 17)
            .at(9, 0, 0, 0)
            .to_zoned(new_york())
            .expect("exists")
    }

    fn at(month: i8, day: i8, hour: i8, minute: i8) -> Zoned {
        date(2026, month, day)
            .at(hour, minute, 0, 0)
            .to_zoned(new_york())
            .expect("exists")
    }

    fn slot(month: i8, day: i8, from: (i8, i8), to: (i8, i8)) -> (Zoned, Zoned) {
        (at(month, day, from.0, from.1), at(month, day, to.0, to.1))
    }

    const ME: Person = Person {
        internal: true,
        organizer: true,
    };
    const TEAMMATE: Person = Person {
        internal: true,
        organizer: false,
    };
    const OUTSIDE: Person = Person {
        internal: false,
        organizer: false,
    };

    fn render(slots: &[(Zoned, Zoned)], minutes: i64, people: &[Person]) -> String {
        text(slots, minutes, people, &new_york(), &now())
    }

    #[test]
    fn nothing_picked_is_no_text() {
        assert_eq!(render(&[], 30, &[ME]), "");
    }

    #[test]
    fn one_meeting_length_slot_leaves_two_spaces() {
        // The duration and `on` both go, leaving the template's two spaces.
        assert_eq!(
            render(&[slot(11, 20, (14, 0), (14, 30))], 30, &[ME]),
            "Does  Fri Nov 20 work? I can do 2:00–2:30 pm EST."
        );
    }

    #[test]
    fn a_longer_slot_names_the_length_and_the_day() {
        assert_eq!(
            render(&[slot(11, 20, (14, 0), (15, 0))], 30, &[ME]),
            "Does 30 min on Fri Nov 20 work? I can do 2:00–3:00 pm EST."
        );
        assert_eq!(
            render(&[slot(11, 20, (14, 0), (16, 0))], 60, &[ME]),
            "Does an hour on Fri Nov 20 work? I can do 2:00–4:00 pm EST."
        );
        assert_eq!(
            render(&[slot(11, 20, (14, 0), (16, 0))], 45, &[ME]),
            "Does 45 min on Fri Nov 20 work? I can do 2:00–4:00 pm EST."
        );
    }

    #[test]
    fn today_and_tomorrow_drop_on() {
        assert_eq!(
            render(&[slot(11, 17, (14, 0), (15, 0))], 30, &[ME]),
            "Does 30 min today (Nov 17) work? I can do 2:00–3:00 pm EST."
        );
        assert_eq!(
            render(&[slot(11, 18, (14, 0), (15, 0))], 30, &[ME]),
            "Does 30 min tomorrow (Nov 18) work? I can do 2:00–3:00 pm EST."
        );
        assert_eq!(
            render(&[slot(11, 18, (14, 0), (14, 30))], 30, &[ME]),
            "Does  tomorrow (Nov 18) work? I can do 2:00–2:30 pm EST."
        );
    }

    #[test]
    fn two_and_three_slots_join_with_or() {
        let two = [
            slot(11, 20, (14, 0), (14, 30)),
            slot(11, 20, (16, 0), (16, 30)),
        ];
        assert_eq!(
            render(&two, 30, &[ME]),
            "Does  Fri Nov 20 work? I can do 2:00–2:30 pm or 4:00–4:30 pm EST."
        );
        // Three take a comma before `or`.
        let three = [
            slot(11, 20, (9, 0), (9, 30)),
            slot(11, 20, (14, 0), (14, 30)),
            slot(11, 20, (16, 0), (16, 30)),
        ];
        assert_eq!(
            render(&three, 30, &[ME]),
            "Does  Fri Nov 20 work? I can do 9:00–9:30 am, 2:00–2:30 pm, or 4:00–4:30 pm EST."
        );
    }

    #[test]
    fn ranges_across_noon_name_both_halves() {
        assert_eq!(
            render(&[slot(11, 20, (11, 30), (12, 30))], 30, &[ME]),
            "Does 30 min on Fri Nov 20 work? I can do 11:30 am–12:30 pm EST."
        );
        assert_eq!(
            time_range(&at(11, 20, 0, 0), &at(11, 20, 0, 45)),
            "12:00–12:45 am"
        );
    }

    #[test]
    fn we_with_an_outsider_and_a_teammate() {
        let one = [slot(11, 20, (14, 0), (14, 30))];
        assert_eq!(
            render(&one, 30, &[ME, TEAMMATE, OUTSIDE]),
            "Does  Fri Nov 20 work? We can do 2:00–2:30 pm EST."
        );
        // An outsider alone, or a teammate alone, is still I.
        assert_eq!(
            render(&one, 30, &[ME, OUTSIDE]),
            "Does  Fri Nov 20 work? I can do 2:00–2:30 pm EST."
        );
        assert_eq!(
            render(&one, 30, &[ME, TEAMMATE]),
            "Does  Fri Nov 20 work? I can do 2:00–2:30 pm EST."
        );
    }

    #[test]
    fn several_days_are_a_list() {
        let slots = [
            slot(11, 19, (13, 45), (15, 30)),
            slot(11, 18, (13, 0), (13, 30)),
        ];
        assert_eq!(
            render(&slots, 30, &[ME]),
            "Does 30 min during any of these times (EST) work?\n- tomorrow (Nov 18): 1:00–1:30 pm\n- Thu Nov 19: 1:45–3:30 pm"
        );
        let lengths = [
            slot(11, 18, (13, 0), (13, 30)),
            slot(11, 19, (9, 0), (9, 30)),
            slot(11, 19, (11, 0), (11, 30)),
        ];
        assert_eq!(
            render(&lengths, 30, &[ME]),
            "Do any of these times (EST) work?\n- tomorrow (Nov 18): 1:00–1:30 pm\n- Thu Nov 19: 9:00–9:30 am, 11:00–11:30 am"
        );
        assert_eq!(
            render(&lengths, 30, &[ME, TEAMMATE, OUTSIDE]),
            "Do any of these times (EST) work to meet with us?\n- tomorrow (Nov 18): 1:00–1:30 pm\n- Thu Nov 19: 9:00–9:30 am, 11:00–11:30 am"
        );
        let hour = [
            slot(11, 18, (13, 0), (15, 0)),
            slot(11, 19, (9, 0), (10, 0)),
        ];
        assert_eq!(
            render(&hour, 60, &[ME]),
            "Does an hour during any of these times (EST) work?\n- tomorrow (Nov 18): 1:00–3:00 pm\n- Thu Nov 19: 9:00–10:00 am"
        );
    }

    #[test]
    fn four_slots_on_a_day_are_a_list() {
        let slots: Vec<_> = (0..4)
            .map(|index| slot(11, 20, (9 + index, 0), (9 + index, 30)))
            .collect();
        assert_eq!(
            render(&slots, 30, &[ME]),
            "Do any of these times (EST) work?\n- Fri Nov 20: 9:00–9:30 am, 10:00–10:30 am, 11:00–11:30 am, 12:00–12:30 pm"
        );
    }

    #[test]
    fn days_are_named_by_how_far_off_they_are() {
        let now = now();
        // The weekday until the eighth day from today.
        assert_eq!(semi_relative(&at(11, 24, 10, 0), &now), "Tue Nov 24");
        assert_eq!(semi_relative(&at(11, 25, 0, 0), &now), "Nov 25");
        assert_eq!(semi_relative(&at(11, 30, 10, 0), &now), "Nov 30");
        let next_year = date(2027, 1, 5)
            .at(10, 0, 0, 0)
            .to_zoned(new_york())
            .expect("exists");
        assert_eq!(semi_relative(&next_year, &now), "Jan 5 2027");
        let list = [
            slot(11, 18, (13, 0), (13, 30)),
            slot(11, 30, (9, 0), (9, 30)),
        ];
        assert_eq!(
            render(&list, 30, &[ME]),
            "Do any of these times (EST) work?\n- tomorrow (Nov 18): 1:00–1:30 pm\n- Nov 30: 9:00–9:30 am"
        );
    }

    #[test]
    fn overlapping_slots_merge_and_midnight_splits() {
        let overlapping = [
            slot(11, 20, (14, 0), (15, 0)),
            slot(11, 20, (14, 30), (16, 0)),
        ];
        assert_eq!(
            render(&overlapping, 30, &[ME]),
            "Does 30 min on Fri Nov 20 work? I can do 2:00–4:00 pm EST."
        );
        // 11 pm to 1 am is a piece on each day; one ending at midnight stays on its day.
        let late = [(at(11, 20, 23, 0), at(11, 21, 1, 0))];
        assert_eq!(
            render(&late, 30, &[ME]),
            "Does 30 min during any of these times (EST) work?\n- Fri Nov 20: 11:00 pm–12:00 am\n- Sat Nov 21: 12:00–1:00 am"
        );
        let to_midnight = [(at(11, 20, 23, 0), at(11, 21, 0, 0))];
        assert_eq!(
            render(&to_midnight, 30, &[ME]),
            "Does 30 min on Fri Nov 20 work? I can do 11:00 pm–12:00 am EST."
        );
    }

    #[test]
    fn times_are_written_in_the_shared_zone() {
        let london = TimeZone::get("Europe/London").expect("zone");
        let slots = [slot(11, 20, (14, 0), (14, 30))];
        assert_eq!(
            text(&slots, 30, &[ME], &london, &now()),
            "Does  Fri Nov 20 work? I can do 7:00–7:30 pm GMT."
        );
        // The zone's name at the earliest slot.
        let summer = [(
            date(2027, 7, 1)
                .at(9, 0, 0, 0)
                .to_zoned(new_york())
                .expect("exists"),
            date(2027, 7, 1)
                .at(9, 30, 0, 0)
                .to_zoned(new_york())
                .expect("exists"),
        )];
        assert_eq!(
            text(&summer, 30, &[ME], &new_york(), &now()),
            "Does  Jul 1 2027 work? I can do 9:00–9:30 am EDT."
        );
    }

    #[test]
    fn lengths_read_in_minutes_or_hours() {
        assert_eq!(duration(15, true), "15 min");
        assert_eq!(duration(60, true), "an hour");
        assert_eq!(duration(60, false), "60 min");
        assert_eq!(duration(90, true), "1.5 hrs");
        assert_eq!(duration(120, true), "2 hrs");
    }

    #[test]
    fn bare_offsets_read_as_gmt() {
        let at = now().timestamp();
        assert_eq!(
            abbreviation(&TimeZone::get("America/Sao_Paulo").expect("zone"), at),
            "GMT−03:00"
        );
        assert_eq!(
            abbreviation(&TimeZone::get("Asia/Kathmandu").expect("zone"), at),
            "GMT+05:45"
        );
        assert_eq!(abbreviation(&new_york(), at), "EST");
    }
}
