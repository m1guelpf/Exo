use super::*;

fn day(text: &str) -> Date {
    text.parse().expect("a date")
}

#[test]
fn marking_done_celebrates_the_levels_and_milestones_it_crosses_once() {
    let (mut statistics, mut seen) = (Statistics::default(), Achievements::default());
    assert_eq!(mark_done(&mut statistics, &mut seen, 49), None);
    assert_eq!(
        mark_done(&mut statistics, &mut seen, 2),
        Some(Reached {
            kind: Kind::MarkDone,
            completed: 50,
            next: 50
        })
    );
    assert!(is_new(Card::MarkedDone, &seen));
    // 260 is a milestone of its own, past 100's level (with 100's step to the next).
    assert_eq!(
        mark_done(&mut statistics, &mut seen, 210),
        Some(Reached {
            kind: Kind::MarkDone,
            completed: 260,
            next: 200
        })
    );
    // Past 5,000, every 2,500.
    statistics.archived_all_time = 7_400;
    assert_eq!(
        mark_done(&mut statistics, &mut seen, 200),
        Some(Reached {
            kind: Kind::MarkDone,
            completed: 7_500,
            next: 2_500
        })
    );
    assert_eq!(statistics.archived_all_time, 7_600);
    // A count this device has celebrated isn't celebrated again.
    statistics.archived_all_time = 40;
    assert_eq!(mark_done(&mut statistics, &mut seen, 20), None);
}

#[test]
fn sends_celebrate_their_levels_and_numbers_and_count_weeks() {
    let (mut statistics, mut seen) = (Statistics::default(), Achievements::default());
    let monday = day("2026-11-16");
    for _ in 0..24 {
        assert_eq!(sent(&mut statistics, &mut seen, monday), None);
    }
    assert_eq!(
        sent(&mut statistics, &mut seen, day("2026-11-22")),
        Some(Reached {
            kind: Kind::Send,
            completed: 25,
            next: 75
        })
    );
    statistics.sends_all_time = 41;
    assert_eq!(
        sent(&mut statistics, &mut seen, monday),
        Some(Reached {
            kind: Kind::Send,
            completed: 42,
            next: 0
        })
    );
    assert_eq!(
        (statistics.sends_this_week, statistics.sends_week_max),
        (26, 26)
    );
    // The next week, last week's count is last week's.
    sent(&mut statistics, &mut seen, day("2026-11-23"));
    assert_eq!(sends_last_week(&statistics, day("2026-11-25")), 26);
    assert_eq!(sends_last_week(&statistics, day("2026-12-01")), 1);
    assert_eq!(sends_last_week(&statistics, day("2026-12-08")), 0);
    assert_eq!(statistics.sends_week_max, 26);
}

#[test]
fn get_me_to_zero_celebrates_the_first_level_it_crosses() {
    let (mut statistics, mut seen) = (Statistics::default(), Achievements::default());
    assert_eq!(
        cleared(&mut statistics, &mut seen, 120),
        Some(Reached {
            kind: Kind::Gmtz,
            completed: 25,
            next: 25
        })
    );
    assert_eq!(
        cleared(&mut statistics, &mut seen, 6_000),
        Some(Reached {
            kind: Kind::Gmtz,
            completed: 5_000,
            next: 5_000
        })
    );
    assert_eq!(
        cleared(&mut statistics, &mut seen, 4_000),
        Some(Reached {
            kind: Kind::Gmtz,
            completed: 10_000,
            next: 5_000
        })
    );
}

#[test]
fn inbox_zero_counts_days_and_the_weekly_streak() {
    let (mut statistics, mut seen) = (Statistics::default(), Achievements::default());
    let hit = hit_inbox_zero(&mut statistics, &mut seen, day("2026-11-17"));
    assert_eq!(
        hit,
        Hit {
            week: true,
            reached: Some(Reached {
                kind: Kind::InboxZero,
                completed: 1,
                next: 3
            })
        }
    );
    assert_eq!(
        (
            statistics.inbox_zero_streak_week_current,
            statistics.inbox_zero_streak_week_max,
            statistics.inbox_zero_days_all_time
        ),
        (1, 1, 1)
    );
    // The same day counts nothing more; the next day counts a day, not a week.
    assert!(!would_count(&statistics, day("2026-11-17")));
    let hit = hit_inbox_zero(&mut statistics, &mut seen, day("2026-11-18"));
    assert_eq!(
        hit,
        Hit {
            week: false,
            reached: Some(Reached {
                kind: Kind::InboxZeroDays,
                completed: 2,
                next: 1
            })
        }
    );
    // The next week continues the streak; a week missed starts it again.
    let hit = hit_inbox_zero(&mut statistics, &mut seen, day("2026-11-23"));
    assert!(
        hit.week
            && statistics.inbox_zero_streak_week_current == 2
            && statistics.inbox_zero_streak_week_max == 2
    );
    hit_inbox_zero(&mut statistics, &mut seen, day("2026-12-09"));
    assert_eq!(
        (
            statistics.inbox_zero_streak_week_current,
            statistics.inbox_zero_streak_week_max
        ),
        (1, 2)
    );
    assert_eq!(
        statistics.last_inbox_zero_week.as_deref(),
        Some("2026-12-07")
    );
}

#[test]
fn remaining_counts_to_the_next_level_then_every_step() {
    assert_eq!(remaining(Card::MarkedDone, 0), 50);
    assert_eq!(remaining(Card::MarkedDone, 5_000), 2_500);
    assert_eq!(remaining(Card::MarkedDone, 6_000), 1_500);
    assert_eq!(remaining(Card::CurrentStreak, 52), 13);
    assert_eq!(remaining(Card::InboxZero, 4), 1);
}

#[test]
fn popup_copy_depends_on_kind_and_count() {
    let reached = |kind, completed, next| Reached {
        kind,
        completed,
        next,
    };
    assert_eq!(
        title(reached(Kind::MarkDone, 1_000, 2_000)),
        "You've cleared 1,000 conversations!"
    );
    assert_eq!(
        body(reached(Kind::MarkDone, 100, 200), |_| 0),
        ["Nice job! Clear 200 more to reach the next level."]
    );
    assert_eq!(
        body(reached(Kind::MarkDone, 7_777, 2_500), |_| 0),
        ["Today's your lucky day!"]
    );
    assert_eq!(
        body(reached(Kind::Send, 5_000, 0), |_| 0),
        ["Exo wants to be you when it grows up."]
    );
    assert_eq!(
        body(reached(Kind::Send, 600, 500), |_| 2),
        ["Another task completed — keep going!"]
    );
    assert_eq!(
        body(reached(Kind::InboxZero, 13, 3), |_| 0),
        [
            "Nice job! Hit Inbox Zero 3 more weeks in a row to",
            "reach the next level."
        ]
    );
    assert_eq!(
        title(reached(Kind::InboxZero, 1, 3)),
        "You've started an Inbox Zero streak!"
    );
}

#[test]
fn statistics_are_abbreviated_to_k_and_m() {
    for (value, shown) in [
        (999, "999"),
        (1_000, "1k"),
        (1_500, "1.5k"),
        (1_234, "1.2k"),
        (150_000, "150k"),
        (123_456, "123k"),
        (1_000_000, "1M"),
        (2_550_000, "2.5M"),
    ] {
        assert_eq!(abbreviated(value), shown, "{value}");
    }
    assert_eq!(grouped(1_234_567), "1,234,567");
}
