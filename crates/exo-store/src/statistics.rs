//! What Achievements count: the account's statistics, kept in its settings (`statistics`), and
//! the thresholds this device has celebrated, kept in the device's (`achievements`).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// `statistics`. Exo counts `sendsPrevWeek` and `sendsWeekMax` itself, from this week's sends
/// in two more keys.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Statistics {
    /// Weeks in a row with Inbox Zero, this one or the last included.
    pub inbox_zero_streak_week_current: u32,
    /// The longest streak.
    pub inbox_zero_streak_week_max: u32,
    /// Days with Inbox Zero.
    pub inbox_zero_days_all_time: u32,
    /// The Monday (`YYYY-MM-DD`) of the last week Inbox Zero was reached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_inbox_zero_week: Option<String>,
    /// The day (`YYYY-MM-DD`) it was last counted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_inbox_zero_day: Option<String>,
    /// Conversations marked done.
    pub archived_all_time: u64,
    /// Messages sent.
    pub sends_all_time: u64,
    /// Conversations Get Me To Zero cleared.
    pub gmtz_archived_all_time: u64,
    /// Messages sent the week before [`sends_week`](Self::sends_week).
    pub sends_prev_week: u64,
    /// The most messages sent in one week.
    pub sends_week_max: u64,
    /// `sendsWeek`: the Monday of the week [`sends_this_week`](Self::sends_this_week) counts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sends_week: Option<String>,
    /// `sendsThisWeek`: messages sent that week.
    pub sends_this_week: u64,
}

/// `achievements`: which thresholds this device has shown a popup for, by category
/// (`markDone`, `send`, `inboxZero`, `inboxZeroDays`, `gmtz`), and the cards marked `New` until
/// their category is opened, by the card's key (`INBOX_ZERO`, `MARKED_DONE`, `SENT_MESSAGES`,
/// `GET_ME_TO_ZERO`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Achievements {
    /// `seen-thresholds`.
    #[serde(rename = "seen-thresholds", skip_serializing_if = "BTreeMap::is_empty")]
    pub seen_thresholds: BTreeMap<String, Vec<u64>>,
    /// The cards showing `New`.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub new_badges: BTreeSet<String>,
}

impl Achievements {
    /// Nothing seen or new.
    pub fn is_empty(&self) -> bool {
        self.seen_thresholds.is_empty() && self.new_badges.is_empty()
    }
}

impl Statistics {
    /// Nothing counted yet.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statistics_and_thresholds_keep_their_key_names() {
        let statistics = Statistics {
            inbox_zero_streak_week_current: 2,
            last_inbox_zero_week: Some("2026-11-16".into()),
            sends_all_time: 42,
            ..Statistics::default()
        };
        let json = serde_json::to_value(&statistics).expect("serializes");
        assert_eq!(json["inboxZeroStreakWeekCurrent"], 2);
        assert_eq!(json["lastInboxZeroWeek"], "2026-11-16");
        assert_eq!(json["sendsAllTime"], 42);
        let mut seen = Achievements::default();
        seen.seen_thresholds.insert("markDone".into(), vec![50]);
        assert_eq!(
            serde_json::to_value(&seen).expect("serializes"),
            serde_json::json!({ "seen-thresholds": { "markDone": [50] } })
        );
    }
}
