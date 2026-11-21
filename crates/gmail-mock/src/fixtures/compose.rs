//! Turns fixture threads into RFC 822 messages the way real clients write them: Gmail-style
//! quoted replies, threading headers, delivery headers, and the labels each copy would carry.

use gmail::mime::{Attachment, Email, Mailbox};
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

use super::cast;

/// One message in a fixture thread.
#[derive(Debug, Clone)]
pub struct Post {
    pub from: Mailbox,
    pub to: Vec<Mailbox>,
    pub cc: Vec<Mailbox>,
    /// Plain text; the HTML part is derived from it unless `html` is set.
    pub text: String,
    pub html: Option<String>,
    pub calendar: Option<String>,
    pub attachments: Vec<Attachment>,
    pub at: Timestamp,
}

/// Where a thread lives and how it looks in the inbox.
#[derive(Debug, Clone, Default)]
pub struct State {
    pub inbox: bool,
    /// How many of the latest received messages are unread.
    pub unread: usize,
    pub starred: bool,
    pub important: bool,
    pub category: Option<&'static str>,
    /// User label names.
    pub labels: Vec<&'static str>,
    pub spam: bool,
    pub trash: bool,
}

/// Mailing-list headers for bulk mail.
#[derive(Debug, Clone)]
pub struct List {
    pub id: String,
    pub unsubscribe: String,
}

#[derive(Debug, Clone)]
pub struct Thread {
    pub subject: String,
    pub posts: Vec<Post>,
    pub state: State,
    pub list: Option<List>,
    /// Quote the previous message in replies (person-to-person mail does; notifications don't).
    pub quote: bool,
}

/// A composed message ready to insert.
#[derive(Debug, Clone)]
pub struct Composed {
    pub raw: Vec<u8>,
    /// Label names; user labels are resolved to IDs on insert.
    pub labels: Vec<String>,
    pub at: Timestamp,
}

/// Composes every message of `thread` (oldest first).
pub fn compose(thread: &Thread, tz: &TimeZone) -> Vec<Composed> {
    let me = cast::me().email;
    let mut out = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    let mut previous: Option<(&Post, String, String)> = None;
    let received: Vec<usize> = (0..thread.posts.len())
        .filter(|&index| thread.posts[index].from.email != me)
        .collect();
    let starred = received.last().copied();

    for (index, post) in thread.posts.iter().enumerate() {
        let local = post.at.to_zoned(tz.clone());
        let domain = post
            .from
            .email
            .split_once('@')
            .map_or("mail.example", |(_, d)| d);
        let seed = fnv(&format!("{}{index}{}", thread.subject, post.from.email));
        let message_id = if domain == cast::DOMAIN || domain == "gmail.com" {
            format!("CA+{seed:016x}{:04x}@mail.gmail.com", index)
        } else {
            format!("{seed:016x}.{index}@{domain}")
        };

        let mut text = post.text.trim_end().to_owned();
        let mut html = post.html.clone().unwrap_or_else(|| text_to_html(&text));
        if thread.quote
            && let Some((parent, parent_text, parent_html)) = &previous
        {
            let attribution = attribution(&parent.from, &parent.at.to_zoned(tz.clone()));
            text = format!("{text}\n\n{attribution}\n{}", quote_text(parent_text));
            html = format!(
                "{html}<br><div class=\"gmail_quote gmail_quote_container\"><div dir=\"ltr\" class=\"gmail_attr\">{}<br></div>\
                 <blockquote class=\"gmail_quote\" style=\"margin:0px 0px 0px 0.8ex;border-left:1px solid rgb(204,204,204);padding-left:1ex\">{parent_html}</blockquote></div>",
                escape(&attribution)
            );
        }

        let mut headers = Vec::new();
        if post.from.email != me {
            let stamp = jiff::fmt::rfc2822::to_string(&local).unwrap_or_default();
            headers.push(("Delivered-To".to_owned(), me.to_owned()));
            headers.push((
                "Received".to_owned(),
                format!("from mail-sor-f41.google.com (mail-sor-f41.google.com. [209.85.220.41]) by mx.google.com; {stamp}"),
            ));
        }
        if let Some(list) = &thread.list {
            headers.push(("List-Id".to_owned(), list.id.clone()));
            headers.push((
                "List-Unsubscribe".to_owned(),
                format!("<{}>", list.unsubscribe),
            ));
            headers.push((
                "List-Unsubscribe-Post".to_owned(),
                "List-Unsubscribe=One-Click".to_owned(),
            ));
        }

        let subject = if index == 0 || !thread.quote {
            thread.subject.clone()
        } else {
            gmail::mime::reply_subject(&thread.subject)
        };
        let email = Email {
            from: Some(post.from.clone()),
            to: post.to.clone(),
            cc: post.cc.clone(),
            subject,
            text: Some(text.clone()),
            html: Some(html.clone()),
            calendar: post.calendar.clone(),
            attachments: post.attachments.clone(),
            message_id: Some(message_id.clone()),
            in_reply_to: thread.quote.then(|| ids.last().cloned()).flatten(),
            references: if thread.quote {
                ids.clone()
            } else {
                Vec::new()
            },
            date: Some(local),
            headers,
            ..Email::default()
        };
        ids.push(message_id);

        out.push(Composed {
            raw: email.to_rfc822(),
            labels: labels(
                &thread.state,
                post.from.email == me,
                Some(index) == starred,
                &received,
                index,
            ),
            at: post.at,
        });
        previous = Some((post, text, html));
    }
    out
}

fn labels(
    state: &State,
    from_me: bool,
    starred: bool,
    received: &[usize],
    index: usize,
) -> Vec<String> {
    let mut labels: Vec<String> = state.labels.iter().map(|&name| name.to_owned()).collect();
    if state.spam {
        labels.push("SPAM".into());
        labels.push("UNREAD".into());
        return labels;
    }
    if state.trash {
        labels.push("TRASH".into());
    }
    if from_me {
        labels.push("SENT".into());
    } else {
        labels.extend(state.category.map(str::to_owned));
        if state.inbox {
            labels.push("INBOX".into());
        }
        let position = received.iter().position(|&i| i == index).unwrap_or(0);
        if received.len() - position <= state.unread {
            labels.push("UNREAD".into());
        }
    }
    if state.important {
        labels.push("IMPORTANT".into());
    }
    if state.starred && starred {
        labels.push("STARRED".into());
    }
    labels
}

/// Gmail's reply attribution line.
fn attribution(from: &Mailbox, at: &Zoned) -> String {
    format!(
        "On {} {} <{}> wrote:",
        at.strftime("%a, %b %-d, %Y at %-I:%M %p"),
        from.name.as_deref().unwrap_or(&from.email),
        from.email
    )
}

fn quote_text(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                ">".to_owned()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The HTML Gmail's composer produces for plain paragraphs.
pub fn text_to_html(text: &str) -> String {
    let body: String = text
        .lines()
        .map(|line| {
            if line.is_empty() {
                "<div><br></div>".to_owned()
            } else {
                format!("<div>{}</div>", escape(line))
            }
        })
        .collect();
    format!("<div dir=\"ltr\">{body}</div>")
}

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Stable hash for deterministic IDs.
pub fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}
