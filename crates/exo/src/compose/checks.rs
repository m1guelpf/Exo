//! The checks Send runs first, in order, each with its alert copy. Checks offering "Send anyway"
//! are passed over once the user picks it.

use std::sync::LazyLock;

use regex::Regex;

use super::{draft::Open, files, recipients};

/// A problem Send found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Problem {
    /// The alert's title, if it has one.
    pub title: Option<&'static str>,
    pub text: String,
    /// "Send anyway" / "Cancel" (and which check to skip then), or just "OK".
    pub kind: Kind,
}

/// How an alert can be answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `OK` only; for From problems it then expands the header to From.
    Ok { focus_from: bool },
    /// `Send anyway` (skipping this check) or `Cancel`.
    SendAnyway(Bypass),
}

/// Checks "Send anyway" skips.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Bypass {
    pub subject: bool,
    pub attachments: bool,
    pub variables: bool,
}

/// The first problem with sending `open` as `html` at `now`, given the account's aliases
/// (`from_ok`: its From is one of them) and what's already been waved through.
pub(crate) fn check(
    open: &Open,
    html: &str,
    from_ok: bool,
    bypass: Bypass,
    now: jiff::Timestamp,
) -> Option<Problem> {
    let almost = Some("Almost there…");
    let sorry = Some("Sorry!");
    if !bypass.subject && !open.is_snippet() && open.record.subject.trim().is_empty() {
        return Some(Problem {
            title: almost,
            text: "Your message doesn't have a subject.".into(),
            kind: Kind::SendAnyway(Bypass {
                subject: true,
                ..bypass
            }),
        });
    }
    // Inline images go only while the HTML still shows them.
    let sent = open.record.attachments.iter().filter(|attachment| {
        attachment
            .content_id
            .as_ref()
            .is_none_or(|cid| html.contains(&format!("cid:{cid}")))
    });
    if files::exceeds_gmail_limit(sent.map(|attachment| attachment.size), html) {
        return Some(Problem {
            title: sorry,
            text: "This message is too large for Gmail. Shorten the message or remove attachments."
                .into(),
            kind: Kind::Ok { focus_from: false },
        });
    }
    // A snippet needs no recipient, only valid ones.
    let valid = if open.is_snippet() {
        open.recipients().all(recipients::sendable)
    } else {
        open.has_valid_recipients()
    };
    if !valid {
        let text = if open.recipients().next().is_none() {
            "Please add at least one recipient."
        } else {
            "Please double-check the email addresses you entered."
        };
        return Some(Problem {
            title: almost,
            text: text.into(),
            kind: Kind::Ok { focus_from: false },
        });
    }
    let count = open.recipients().count();
    if count > recipients::MAX_RECIPIENTS {
        let over = count - recipients::MAX_RECIPIENTS;
        let noun = if over == 1 { "recipient" } else { "recipients" };
        return Some(Problem {
            title: sorry,
            text: format!(
                "Gmail can only send to 100 recipients at once.\nPlease remove at least {over} {noun}."
            ),
            kind: Kind::Ok { focus_from: false },
        });
    }
    // Saving a snippet stops at the size and recipient checks.
    if open.is_snippet() {
        return None;
    }
    // A reminder must be neither in the past nor before the send time.
    if let Some(reminder) = open.record.reminder {
        let send_at = open.record.send_later.as_ref().map(|later| later.at);
        if reminder.at < now || send_at.is_some_and(|at| reminder.at < at) {
            let text = if send_at.is_some() {
                "We can't set a reminder before the send time.\nPlease update either setting and try again."
            } else {
                "We can't yet set reminders in the past.\nPlease update the reminder and try again."
            };
            return Some(Problem {
                title: Some("Temporal anomaly…"),
                text: text.into(),
                kind: Kind::Ok { focus_from: false },
            });
        }
    }
    if !from_ok {
        return Some(Problem {
            title: None,
            text: "Sorry, you can't send from this address.\n\nPlease change the from address and try again.".into(),
            kind: Kind::Ok { focus_from: true },
        });
    }
    let text = authored_text(html);
    let has_attachments = open
        .record
        .attachments
        .iter()
        .any(|attachment| attachment.content_id.is_none());
    if !bypass.attachments && !has_attachments && !html.contains("<img") {
        let subject = if open.is_new_email() {
            open.record.subject.as_str()
        } else {
            ""
        };
        if let Some(phrase) = mentions_attachment(&text).or_else(|| mentions_attachment(subject)) {
            return Some(Problem {
                title: almost,
                text: format!("You wrote \"{phrase}\", and there are no attachments."),
                kind: Kind::SendAnyway(Bypass {
                    attachments: true,
                    ..bypass
                }),
            });
        }
    }
    if !bypass.variables {
        let subject = if open.is_new_email() {
            open.record.subject.as_str()
        } else {
            ""
        };
        if let Some(token) = VARIABLE.find(&text).or_else(|| VARIABLE.find(subject)) {
            return Some(Problem {
                title: Some("One last thing…"),
                text: format!(
                    "You wrote \"{}\", and it hasn't been replaced.",
                    token.as_str()
                ),
                kind: Kind::SendAnyway(Bypass {
                    variables: true,
                    ..bypass
                }),
            });
        }
    }
    None
}

/// A `{variable}` left in: non-space text in braces, not starting with `#`.
static VARIABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{[^#\s]\S*\}").expect("valid regex"));

/// The phrases that mean an attachment is missing, each wrapped as
/// `(?:\b|\s)(…)(?:\b|\s)`.
static ATTACHMENT_PHRASES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    let noun = "(?:deck|doc|document|excel|file|gif|image|memo|nda|pdf|pic|picture|ppt|presentation|proposal|recording|report|screenshot|sheet|spreadsheet|video)s?";
    let patterns = [
        "see (?:the )?(?:attached|attachments?|included)".to_owned(),
        "(?:is|are) attached".to_owned(),
        "[aA]ttached,? (?:\\w+ ){0,4}(?:is|are|find)".to_owned(),
        "attached to this (?:email|message)".to_owned(),
        "i(?:'m| am) attaching".to_owned(),
        "i(?:'ve| have) attached".to_owned(),
        "i attach(?:ed)?".to_owned(),
        "find (?:\\w+ ){0,4}(?:attached|included)".to_owned(),
        format!("attached {noun}"),
        format!("{noun} attached"),
        regex::escape("the attached"),
        regex::escape("(attached)"),
    ];
    patterns
        .iter()
        .map(|pattern| {
            Regex::new(&format!(r"(?im)(?:\b|\s)({pattern})(?:\b|\s)")).expect("valid regex")
        })
        .collect()
});

/// The first missing-attachment phrase in `text`, as written.
fn mentions_attachment(text: &str) -> Option<String> {
    ATTACHMENT_PHRASES.iter().find_map(|pattern| {
        pattern
            .captures(text)
            .map(|captures| captures[1].to_owned())
    })
}

/// The text the user wrote: before any quote, without blockquotes or the signature.
fn authored_text(html: &str) -> String {
    let html = html
        .split("<div class=\"gmail_quote")
        .next()
        .unwrap_or_default();
    super::plain::authored(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use exo_store::{Address, DraftAction, DraftRecord};

    fn open(subject: &str, to: &[&str], body: &str) -> Open {
        let record = DraftRecord {
            action: DraftAction::Compose,
            from: None,
            to: to.iter().map(|email| recipients::parse(email)).collect(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: subject.into(),
            body: String::new(),
            quoted_content: None,
            quoted_inlined: false,
            in_reply_to: None,
            in_reply_to_rfc822: None,
            references: Vec::new(),
            rfc822_id: "x@y".into(),
            fingerprint: String::new(),
            attachments: Vec::new(),
            send_later: None,
            reminder: None,
        };
        let mut open = Open::new("d".into(), None, record);
        open.editor.type_text(body);
        open
    }

    fn first(open: &Open, bypass: Bypass) -> Option<Problem> {
        let html = open.editor.html();
        check(open, &html, true, bypass, jiff::Timestamp::now())
    }

    #[test]
    fn a_missing_subject_can_be_waved_through() {
        let draft = open("", &["ann@x.com"], "Hi");
        let problem = first(&draft, Bypass::default()).expect("no subject");
        assert_eq!(problem.text, "Your message doesn't have a subject.");
        let Kind::SendAnyway(bypass) = problem.kind else {
            panic!("send anyway")
        };
        assert_eq!(first(&draft, bypass), None);
    }

    #[test]
    fn recipients_must_be_there_and_valid() {
        assert_eq!(
            first(&open("Hi", &[], "Hi"), Bypass::default())
                .map(|p| p.text)
                .as_deref(),
            Some("Please add at least one recipient.")
        );
        assert_eq!(
            first(&open("Hi", &["bob"], "Hi"), Bypass::default())
                .map(|p| p.text)
                .as_deref(),
            Some("Please double-check the email addresses you entered.")
        );
        let mut many = open("Hi", &[], "Hi");
        many.record.to = (0..102)
            .map(|i| Address {
                name: None,
                email: format!("p{i}@x.com"),
            })
            .collect();
        assert_eq!(
            first(&many, Bypass::default()).map(|p| p.text).as_deref(),
            Some(
                "Gmail can only send to 100 recipients at once.\nPlease remove at least 2 recipients."
            )
        );
    }

    #[test]
    fn mentioning_an_attachment_without_one_asks() {
        let problem = first(
            &open("Deck", &["ann@x.com"], "I've attached the deck."),
            Bypass::default(),
        )
        .expect("asks");
        assert_eq!(
            problem.text,
            "You wrote \"I've attached\", and there are no attachments."
        );
        assert_eq!(
            first(
                &open("Deck", &["ann@x.com"], "Thanks for the deck."),
                Bypass::default()
            ),
            None
        );
    }

    #[test]
    fn a_reminder_must_come_after_now_and_the_send() {
        let mut open = open("Hi", &["ann@x.com"], "Hello");
        let now = jiff::Timestamp::now();
        open.record.reminder = Some(exo_store::DraftReminder {
            at: now - jiff::SignedDuration::from_mins(1),
            keep_on_reply: false,
            automatic: false,
        });
        let problem = first(&open, Bypass::default()).expect("refused");
        assert_eq!(
            (problem.title, problem.text.as_str()),
            (
                Some("Temporal anomaly…"),
                "We can't yet set reminders in the past.\nPlease update the reminder and try again."
            )
        );
        open.record.reminder = Some(exo_store::DraftReminder {
            at: now + jiff::SignedDuration::from_hours(1),
            keep_on_reply: false,
            automatic: false,
        });
        assert_eq!(first(&open, Bypass::default()), None);
        open.record.send_later = Some(exo_store::SendLater {
            at: now + jiff::SignedDuration::from_hours(2),
            abort_on_reply: false,
            current_message_ids: Vec::new(),
        });
        assert_eq!(first(&open, Bypass::default()).map(|problem| problem.text), Some("We can't set a reminder before the send time.\nPlease update either setting and try again.".to_owned()));
    }

    #[test]
    fn variables_left_in_ask() {
        let problem = first(
            &open("Hi", &["ann@x.com"], "Hi {first_name}, hello"),
            Bypass::default(),
        )
        .expect("asks");
        assert_eq!(
            problem.text,
            "You wrote \"{first_name}\", and it hasn't been replaced."
        );
    }
}
