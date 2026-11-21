//! The status bar at the bottom of the window: the offline bar, else the Out of Office bar, else
//! Quick Tips' (`crate::quick_tips`).
//!
//! The offline bar says what's down and what Exo does meanwhile. The Out of Office bar says how
//! to turn replies off.

use exo_store::{Connection, Outage};
use exo_ui::{MessageBarColor, MessageBarRun};

/// A status bar, as it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusBar {
    /// The title, before `text-transform: uppercase`.
    pub title: &'static str,
    /// The description and its next step, with their links and keycaps.
    pub runs: Vec<MessageBarRun<'static>>,
    /// The background.
    pub color: MessageBarColor,
}

impl StatusBar {
    /// The bar for `connection`, if something's down; `week` is the calendar's week view being
    /// open.
    pub fn offline(connection: &Connection, week: bool) -> Option<Self> {
        let text = MessageBarRun::Text;
        let link = MessageBarRun::Link;
        let offline = if week {
            "Events will sync when next online. Your updates may overwrite changes from others."
        } else {
            "Messages will send and receive when next online."
        };
        let catch_up = "Exo will work offline and catch up when Gmail responds.";
        let (title, description, next, color) = match connection.outage? {
            Outage::Offline => (
                "Offline",
                "Good news: Exo works offline!",
                vec![text(offline)],
                MessageBarColor::Blue,
            ),
            Outage::Auth => (
                "Offline",
                "Sorry! Google OAuth is not responding right now.",
                vec![text(
                    "Exo will reconnect automatically when Google responds.",
                )],
                MessageBarColor::Blue,
            ),
            Outage::Gmail => (
                "Offline",
                "Gmail is not responding right now.",
                vec![text(catch_up)],
                MessageBarColor::Blue,
            ),
            Outage::Throttled => (
                "Note",
                "Gmail is temporarily throttling requests.",
                vec![text(catch_up)],
                MessageBarColor::Blue,
            ),
            // Fatal: red.
            Outage::GmailDisabled => (
                "Error",
                "Your Gmail account has been disabled.",
                vec![
                    text("Please open "),
                    link("mail.google.com", "https://mail.google.com?shxr=0"),
                    text(" for more information."),
                ],
                MessageBarColor::Red,
            ),
        };
        // A full disk can look like the network failing; throttling can't.
        let next = if connection.disk_low && connection.outage != Some(Outage::Throttled) {
            vec![text("This may be caused by lack of disk space.")]
        } else {
            next
        };
        let runs = [text(description), text(" ")]
            .into_iter()
            .chain(next)
            .collect();
        Some(Self { title, runs, color })
    }

    /// The bar while Out of Office Reply's replies go out: green, with ⌘K's keycaps. It can't be
    /// dismissed.
    pub fn out_of_office() -> Self {
        Self {
            title: "Out of Office Reply is On",
            runs: vec![
                MessageBarRun::Text("To turn off automatic replies, hit"),
                MessageBarRun::Keys(&["⌘", "K"]),
                MessageBarRun::Text(" → Out of Office Reply."),
            ],
            color: MessageBarColor::Green,
        }
    }

    /// The description as one line of text.
    pub fn text(&self) -> String {
        self.runs.iter().map(MessageBarRun::text).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(outage: Outage) -> StatusBar {
        StatusBar::offline(
            &Connection {
                outage: Some(outage),
                disk_low: false,
            },
            false,
        )
        .expect("a bar")
    }

    #[test]
    fn each_outage_has_its_copy() {
        assert_eq!(StatusBar::offline(&Connection::default(), false), None);
        let offline = bar(Outage::Offline);
        assert_eq!(
            (offline.title, offline.text().as_str()),
            (
                "Offline",
                "Good news: Exo works offline! Messages will send and receive when next online."
            )
        );
        assert_eq!(
            bar(Outage::Auth).text(),
            "Sorry! Google OAuth is not responding right now. Exo will reconnect automatically when Google responds."
        );
        assert_eq!(
            bar(Outage::Gmail).text(),
            "Gmail is not responding right now. Exo will work offline and catch up when Gmail responds."
        );
        let throttled = bar(Outage::Throttled);
        assert_eq!(
            (throttled.title, throttled.text().as_str()),
            (
                "Note",
                "Gmail is temporarily throttling requests. Exo will work offline and catch up when Gmail responds."
            )
        );
        let disabled = bar(Outage::GmailDisabled);
        assert_eq!(
            (disabled.title, disabled.color),
            ("Error", MessageBarColor::Red)
        );
        assert_eq!(
            disabled.text(),
            "Your Gmail account has been disabled. Please open mail.google.com for more information."
        );
    }

    #[test]
    fn the_week_view_and_a_full_disk_change_the_next_step() {
        let offline = Connection {
            outage: Some(Outage::Offline),
            disk_low: false,
        };
        let week = StatusBar::offline(&offline, true).expect("a bar");
        assert_eq!(
            week.text(),
            "Good news: Exo works offline! Events will sync when next online. Your updates may overwrite changes from others."
        );
        let full = StatusBar::offline(
            &Connection {
                disk_low: true,
                ..offline
            },
            false,
        )
        .expect("a bar");
        assert_eq!(
            full.text(),
            "Good news: Exo works offline! This may be caused by lack of disk space."
        );
        let throttled = StatusBar::offline(
            &Connection {
                outage: Some(Outage::Throttled),
                disk_low: true,
            },
            false,
        )
        .expect("a bar");
        assert!(
            throttled.text().ends_with("catch up when Gmail responds."),
            "throttling isn't the disk"
        );
    }

    #[test]
    fn out_of_office_says_how_to_turn_it_off() {
        let bar = StatusBar::out_of_office();
        assert_eq!(
            (bar.title, bar.color),
            ("Out of Office Reply is On", MessageBarColor::Green)
        );
        assert_eq!(
            bar.text(),
            "To turn off automatic replies, hit⌘K → Out of Office Reply."
        );
    }
}
