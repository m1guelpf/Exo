//! New-mail notifications.
//!
//! Exo decides which messages are new mail from its own sync ([`exo_store::Arrival`]):
//! - mail in the inbox, from someone else (not the account or one of its aliases);
//! - Important's only, while Important and Other are on (the dialog's `If Important • Other is on,
//!   notify for Important only`);
//! - not when the thread is muted, the message has been read (unless a reminder brought the
//!   thread back), or notifications are off on this device or for the account.
//!
//! What it shows: the sender as the title, the snippet as macOS's subtitle and the subject as
//! the body, with the default sound. A click opens the conversation.

use exo_store::{Arrival, DeviceSettings, Settings};

use super::{Click, Notice, Open};

/// Whether `arrival` notifies, with the account's `settings` and this `device`'s. `reminder`:
/// a reminder brought the thread back, which notifies though it's read.
pub fn notifies(
    arrival: &Arrival,
    settings: &Settings,
    device: &DeviceSettings,
    reminder: bool,
) -> bool {
    let from_me = arrival.from.as_ref().is_some_and(|from| from.is_me);
    settings.email_notifications(device)
        && arrival.inbox
        && !from_me
        && !arrival.muted
        && (arrival.unread || reminder)
        && (settings.unified_inbox || arrival.important)
}

/// Which of a round's `arrivals` (oldest first) notify, after a sync `gap`: those that pass
/// [`notifies`], or after a gap only the newest of them, so a device that was away gets only
/// the newest waiting one.
pub fn notifying<'a>(
    arrivals: &'a [Arrival],
    settings: &Settings,
    device: &DeviceSettings,
    gap: bool,
) -> Vec<&'a Arrival> {
    let mut notifying: Vec<&Arrival> = arrivals
        .iter()
        .filter(|arrival| notifies(arrival, settings, device, false))
        .collect();
    if gap {
        notifying.drain(..notifying.len().saturating_sub(1));
    }
    notifying
}

/// The notification for `arrival` in `account`.
pub fn notice(arrival: &Arrival, account: &str) -> Notice {
    let from = arrival.from.as_ref();
    let title = from
        .and_then(|from| from.name.clone())
        .filter(|name| !name.is_empty())
        .or_else(|| from.map(|from| from.email.clone()))
        .unwrap_or_default();
    Notice {
        key: format!("EMAIL-{}", arrival.message),
        title,
        // None without a snippet.
        subtitle: Some(arrival.snippet.clone()).filter(|snippet| !snippet.is_empty()),
        body: arrival.subject.clone(),
        button: None,
        click: Click {
            account: account.to_owned(),
            open: Open::Thread {
                thread: arrival.thread.clone(),
                message: arrival.message.clone(),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use exo_store::Contact;

    use super::*;

    fn arrival(subject: &str) -> Arrival {
        Arrival {
            thread: "t1".into(),
            message: format!("m-{subject}"),
            date: jiff::Timestamp::UNIX_EPOCH,
            from: Some(Contact {
                name: Some("Priya Raman".into()),
                email: "priya@brightmoor.co".into(),
                is_me: false,
            }),
            subject: subject.into(),
            snippet: "Can you look at the checklist before 3?".into(),
            unread: true,
            inbox: true,
            important: true,
            muted: false,
        }
    }

    fn on() -> (Settings, DeviceSettings) {
        (Settings::default(), DeviceSettings::default())
    }

    #[test]
    fn new_inbox_mail_from_someone_else_notifies() {
        let (settings, device) = on();
        assert!(notifies(&arrival("Launch"), &settings, &device, false));
        let skipped = [
            (
                "read",
                Arrival {
                    unread: false,
                    ..arrival("a")
                },
            ),
            (
                "muted",
                Arrival {
                    muted: true,
                    ..arrival("b")
                },
            ),
            (
                "not in the inbox",
                Arrival {
                    inbox: false,
                    ..arrival("c")
                },
            ),
            (
                "mine",
                Arrival {
                    from: Some(Contact {
                        name: None,
                        email: "maya@brightmoor.co".into(),
                        is_me: true,
                    }),
                    ..arrival("d")
                },
            ),
        ];
        for (why, arrival) in skipped {
            assert!(!notifies(&arrival, &settings, &device, false), "{why}");
        }
    }

    #[test]
    fn a_reminder_notifies_though_read() {
        let (settings, device) = on();
        let read = Arrival {
            unread: false,
            ..arrival("a")
        };
        assert!(notifies(&read, &settings, &device, true));
        assert!(
            !notifies(
                &Arrival {
                    muted: true,
                    ..read
                },
                &settings,
                &device,
                true
            ),
            "muted still wins"
        );
    }

    #[test]
    fn important_only_while_important_and_other_are_on() {
        let (mut settings, device) = on();
        let other = Arrival {
            important: false,
            ..arrival("Sale")
        };
        assert!(
            !notifies(&other, &settings, &device, false),
            "Other's mail is quiet"
        );
        settings.unified_inbox = true;
        assert!(
            notifies(&other, &settings, &device, false),
            "one Inbox: everything"
        );
    }

    #[test]
    fn off_on_this_device_or_for_the_account() {
        let (mut settings, mut device) = on();
        device.disable_notifications = true;
        assert!(!notifies(&arrival("a"), &settings, &device, true));
        device.disable_notifications = false;
        settings.disable_notifications = true;
        assert!(!notifies(&arrival("a"), &settings, &device, true));
    }

    #[test]
    fn after_a_gap_only_the_newest() {
        let (settings, device) = on();
        let arrivals = [
            arrival("one"),
            arrival("two"),
            Arrival {
                unread: false,
                ..arrival("three")
            },
        ];
        let subjects = |notifying: Vec<&Arrival>| {
            notifying
                .into_iter()
                .map(|arrival| arrival.subject.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            subjects(notifying(&arrivals, &settings, &device, false)),
            ["one", "two"]
        );
        assert_eq!(
            subjects(notifying(&arrivals, &settings, &device, true)),
            ["two"],
            "the newest that notifies"
        );
    }

    #[test]
    fn sender_snippet_and_subject() {
        let notice = notice(&arrival("Launch checklist"), "maya@brightmoor.co");
        assert_eq!(notice.key, "EMAIL-m-Launch checklist");
        assert_eq!(notice.title, "Priya Raman");
        assert_eq!(
            notice.subtitle.as_deref(),
            Some("Can you look at the checklist before 3?")
        );
        assert_eq!(notice.body, "Launch checklist");
        assert_eq!(
            notice.click,
            Click {
                account: "maya@brightmoor.co".into(),
                open: Open::Thread {
                    thread: "t1".into(),
                    message: "m-Launch checklist".into()
                }
            }
        );
        let nameless = super::notice(
            &Arrival {
                from: Some(Contact {
                    name: None,
                    email: "ops@example.com".into(),
                    is_me: false,
                }),
                snippet: String::new(),
                ..arrival("x")
            },
            "maya@brightmoor.co",
        );
        assert_eq!(
            (nameless.title.as_str(), nameless.subtitle),
            ("ops@example.com", None)
        );
    }
}
