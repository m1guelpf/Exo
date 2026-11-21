//! The deterministic fixture world: a realistic three-month mailbox for Maya Okafor, Head of
//! Product at Brightmoor (an invented logistics-software startup), plus her calendars and
//! contacts. Everything is invented. The same seed and "now" always produce the same world.

mod assets;
mod bulk;
mod calendar;
mod cast;
mod compose;
mod conversations;
mod people;
mod rng;

pub use assets::portrait_png;
pub use calendar::personal as personal_calendars;
pub use people::someone_else;

use gmail::mime::{Email, Mailbox};
use gmail::{
    Filter, FilterAction, FilterCriteria, Format, Label, LabelColor, LabelListVisibility, SendAs,
    VacationSettings,
};
use jiff::{ToSpan, Zoned};

use crate::mail::search::Query;
use crate::mail::{Mailbox as Store, NewMessage, Selection, Threading};
use crate::oauth::Sessions;
use crate::world::{Account, Clock, Config, World};
use compose::{Composed, Post, State, Thread};
use conversations::{File, SCRIPTS, Script};
use rng::Rng;

/// Where the mailbox's history IDs start; any `startHistoryId` below it has "expired".
const HISTORY_START: u64 = 4_510_000;

const GOOGLE_ID: &str = "108452913874620153827";

/// A user label: name, optional `(text, background)` color, and whether it always shows.
struct LabelSpec(
    &'static str,
    Option<(&'static str, &'static str)>,
    LabelListVisibility,
);

const SHOW: LabelListVisibility = LabelListVisibility::LabelShow;
const IF_UNREAD: LabelListVisibility = LabelListVisibility::LabelShowIfUnread;
/// Hidden from Gmail's label list, which doesn't hide its chips (many real accounts hide the
/// labels their filters apply).
const HIDE: LabelListVisibility = LabelListVisibility::LabelHide;

const LABELS: &[LabelSpec] = &[
    LabelSpec("Projects", Some(("#ffffff", "#4a86e8")), SHOW),
    LabelSpec("Projects/Atlas Launch", Some(("#ffffff", "#3c78d8")), SHOW),
    LabelSpec("Projects/Exo", Some(("#ffffff", "#8e63ce")), SHOW),
    LabelSpec("Clients", Some(("#ffffff", "#16a766")), SHOW),
    LabelSpec("Clients/Calloway", Some(("#ffffff", "#149e60")), SHOW),
    LabelSpec("Clients/Harborlight", Some(("#ffffff", "#2a9c68")), SHOW),
    LabelSpec("Family", Some(("#ffffff", "#e07798")), SHOW),
    LabelSpec("Hiring", Some(("#000000", "#fad165")), SHOW),
    LabelSpec("Receipts", Some(("#000000", "#c9daf8")), SHOW),
    LabelSpec("Travel", Some(("#ffffff", "#ffad47")), SHOW),
    LabelSpec("Newsletters", None, HIDE),
    LabelSpec("Read later", None, IF_UNREAD),
];

/// Builds the world for `config`.
pub fn world(config: &Config) -> World {
    let now = &config.now;
    let tz = now.time_zone().clone();
    let me = cast::me();
    let mut rng = Rng::new(config.seed);
    let origin = config
        .origin
        .clone()
        .unwrap_or_else(|| url::Url::parse("http://127.0.0.1/").expect("a URL"));
    let account = Account {
        email: me.email.to_owned(),
        given_name: me.first.to_owned(),
        family_name: me.last.to_owned(),
        google_id: GOOGLE_ID.to_owned(),
        tz: tz.clone(),
        photo: config
            .photo
            .then(|| format!("{origin}__mock/photo/{GOOGLE_ID}")),
    };

    let mut mail = Store::new(me.email, &me.name(), tz.clone(), HISTORY_START);
    let labels = create_labels(&mut mail);
    configure_settings(&mut mail, &labels, now);

    let mut threads: Vec<Thread> = SCRIPTS
        .iter()
        .map(|script| script_thread(script, now, &mut rng))
        .collect();
    threads.extend(bulk::threads(now, &mut rng));
    threads.extend(calendar::invites(now));
    for list in threads.iter_mut().filter_map(|thread| thread.list.as_mut()) {
        list.unsubscribe = local_unsubscribe(&origin, &list.unsubscribe);
    }

    // Insert in delivery order, so history IDs grow with time like a real mailbox's.
    let mut messages: Vec<Composed> = threads
        .iter()
        .flat_map(|thread| compose::compose(thread, &tz))
        .collect();
    messages.sort_by_key(|message| message.at);
    for message in messages {
        let label_ids = message
            .labels
            .iter()
            .map(|name| {
                labels
                    .iter()
                    .find(|(n, _)| n == name)
                    .map_or_else(|| name.clone(), |(_, id)| id.clone())
            })
            .collect();
        mail.insert(NewMessage {
            raw: message.raw,
            labels: label_ids,
            threading: Threading::Headers,
            internal_date: message.at.as_millisecond(),
            apply_filters: false,
        })
        .expect("fixture messages are valid");
    }
    for raw in &config.extra_messages {
        let delivered = mail.insert(NewMessage {
            raw: raw.clone(),
            labels: ["INBOX", "UNREAD", "CATEGORY_PERSONAL"]
                .map(String::from)
                .to_vec(),
            threading: Threading::Headers,
            internal_date: now.timestamp().as_millisecond(),
            apply_filters: false,
        });
        if let Err(error) = delivered {
            log::warn!("skipping an extra message: {error:?}");
        }
    }
    create_drafts(&mut mail, now);

    World {
        config: config.clone(),
        clock: Clock::new(now.timestamp()),
        mail,
        calendar: calendar::calendars(now),
        others: std::collections::BTreeMap::new(),
        people: people::directory(GOOGLE_ID, account.photo.as_deref()),
        sessions: Sessions::default(),
        faults: Vec::new(),
        quota: crate::http::quota::Meter::new(config.quota),
        unsubscribes: Vec::new(),
        account,
    }
}

/// `link`, a newsletter's one-click unsubscribe link, as the mock's own endpoint at `origin`:
/// `https://brand.example/unsubscribe?u=…` becomes `{origin}__mock/unsubscribe/brand.example/
/// unsubscribe?u=…`.
fn local_unsubscribe(origin: &url::Url, link: &str) -> String {
    let rest = link.split_once("://").map_or(link, |(_, rest)| rest);
    format!("{origin}__mock/unsubscribe/{rest}")
}

/// Creates the user labels and returns `(name, id)` pairs.
fn create_labels(mail: &mut Store) -> Vec<(String, String)> {
    LABELS
        .iter()
        .map(|LabelSpec(name, color, visibility)| {
            let label = mail
                .create_label(Label {
                    name: Some((*name).into()),
                    label_list_visibility: Some(*visibility),
                    color: color.map(|(text, background)| LabelColor {
                        text_color: Some(text.into()),
                        background_color: Some(background.into()),
                    }),
                    ..Label::default()
                })
                .expect("fixture labels are valid");
            ((*name).to_owned(), label.id.unwrap_or_default())
        })
        .collect()
}

fn configure_settings(mail: &mut Store, labels: &[(String, String)], now: &Zoned) {
    let me = cast::me();
    let id = |name: &str| {
        labels
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| id.clone())
            .unwrap_or_default()
    };
    mail.settings.send_as = vec![
        SendAs {
            send_as_email: Some(me.email.into()),
            display_name: Some(me.name()),
            reply_to_address: Some(String::new()),
            signature: Some(format!(
                "<div dir=\"ltr\"><div><b>{}</b></div><div>{}, Brightmoor</div><div><a href=\"https://brightmoor.co\">brightmoor.co</a></div></div>",
                me.name(),
                me.title
            )),
            is_primary: Some(true),
            is_default: Some(true),
            ..SendAs::default()
        },
        SendAs {
            send_as_email: Some("hello@mayaokafor.com".into()),
            display_name: Some(me.name()),
            reply_to_address: Some(String::new()),
            signature: Some("<div dir=\"ltr\">Maya</div>".into()),
            is_default: Some(false),
            treat_as_alias: Some(false),
            verification_status: Some(gmail::VerificationStatus::Accepted),
            ..SendAs::default()
        },
    ];
    let thanksgiving = (now.date() + 9.days())
        .to_zoned(now.time_zone().clone())
        .ok();
    mail.settings.vacation = VacationSettings {
        enable_auto_reply: Some(false),
        response_subject: Some("Out of office".into()),
        // Gmail web's rich-text responder: an HTML body only.
        response_body_plain_text: None,
        response_body_html: Some("<div>Hi! I'm out for Thanksgiving and back on Monday. For anything urgent about Atlas, please reach Priya (priya@brightmoor.co).</div><div><br></div><div>Maya</div>".into()),
        restrict_to_contacts: Some(false),
        restrict_to_domain: Some(false),
        start_time: thanksgiving.as_ref().map(|start| start.timestamp().as_millisecond()),
        end_time: thanksgiving.and_then(|start| start.checked_add(4.days()).ok()).map(|end| end.timestamp().as_millisecond()),
    };
    let filters = [
        (
            "newsletter@freightsignals.com",
            vec![id("Newsletters")],
            vec![],
        ),
        ("receipts@ledgerly.app", vec![id("Receipts")], vec![]),
        (
            "notifications@linkwise.com",
            vec![],
            vec!["INBOX".to_owned()],
        ),
        ("callowayfreight.com", vec![id("Clients/Calloway")], vec![]),
    ];
    for (from, add, remove) in filters {
        let filter_id = mail.next_opaque_id("ANe1Bm");
        mail.settings.filters.push(Filter {
            id: Some(filter_id),
            criteria: Some(FilterCriteria {
                from: Some(from.into()),
                ..FilterCriteria::default()
            }),
            action: Some(FilterAction {
                add_label_ids: add,
                remove_label_ids: remove,
                forward: None,
            }),
        });
    }
}

fn local(now: &Zoned, days_ago: i64, hour: i8, minute: i8) -> Zoned {
    (now.date() - days_ago.days())
        .at(hour, minute, 0, 0)
        .to_zoned(now.time_zone().clone())
        .expect("fixture times exist in the account zone")
}

/// A scripted conversation as a thread: timing drawn from the seed, replies addressed to the
/// previous sender with everyone else on cc.
fn script_thread(script: &Script, now: &Zoned, rng: &mut Rng) -> Thread {
    let (days_ago, hour, minute) = script.start;
    let mut at = local(now, days_ago, hour, minute).timestamp();
    let mut participants: Vec<&cast::Person> = Vec::new();
    for key in script
        .posts
        .iter()
        .map(|(key, _)| *key)
        .chain(script.to.iter().copied())
        .chain(script.cc.iter().copied())
    {
        let person = cast::get(key);
        if !participants.iter().any(|p| p.key == person.key) {
            participants.push(person);
        }
    }
    let work = participants
        .iter()
        .any(|p| matches!(p.circle, cast::Circle::Colleague | cast::Circle::Work));
    let count = script.posts.len();
    let mut posts = Vec::new();
    let mut senders: Vec<&cast::Person> = Vec::new();
    for (index, (key, body)) in script.posts.iter().enumerate() {
        if index > 0 {
            at += rng.between(script.pace.0, script.pace.1).minutes();
        }
        at = waking_hours(at, now.time_zone(), work);
        // Nothing arrives in the future; keep a minute between the remaining posts.
        let remaining = i64::try_from(count - index).unwrap_or(1);
        at = at.min(now.timestamp() - remaining.minutes());
        let sender = cast::get(key);
        let (to, cc): (Vec<&cast::Person>, Vec<&cast::Person>) = if index == 0 {
            (
                script.to.iter().map(|k| cast::get(k)).collect(),
                script.cc.iter().map(|k| cast::get(k)).collect(),
            )
        } else {
            let reply_to = senders
                .iter()
                .rev()
                .find(|p| p.key != sender.key)
                .copied()
                .or_else(|| script.to.first().map(|k| cast::get(k)))
                .unwrap_or(sender);
            let cc = participants
                .iter()
                .filter(|p| p.key != sender.key && p.key != reply_to.key)
                .copied()
                .collect();
            (vec![reply_to], cc)
        };
        let attachments = script
            .files
            .iter()
            .filter(|(post, _)| *post == index)
            .map(|(_, file)| attachment(file))
            .collect();
        posts.push(Post {
            from: sender.mailbox(),
            to: to.iter().map(|p| p.mailbox()).collect(),
            cc: cc.iter().map(|p| p.mailbox()).collect(),
            text: (*body).to_owned(),
            html: None,
            calendar: None,
            attachments,
            at,
        });
        senders.push(sender);
    }
    let important = participants.iter().any(|p| {
        matches!(
            p.circle,
            cast::Circle::Colleague | cast::Circle::Work | cast::Circle::Personal
        )
    });
    Thread {
        subject: script.subject.to_owned(),
        posts,
        state: State {
            inbox: script.inbox,
            unread: script.unread,
            starred: script.starred,
            important,
            category: Some("CATEGORY_PERSONAL"),
            labels: script.labels.to_vec(),
            ..State::default()
        },
        list: None,
        quote: true,
    }
}

/// Moves a send time out of the night (to the next morning) and, for work mail, off the
/// weekend (to Monday morning), keeping the minutes so times don't all line up.
fn waking_hours(at: jiff::Timestamp, tz: &jiff::tz::TimeZone, work: bool) -> jiff::Timestamp {
    let local = at.to_zoned(tz.clone());
    let mut date = local.date();
    let mut morning = false;
    if local.hour() >= 22 {
        date = date.tomorrow().unwrap_or(date);
        morning = true;
    } else if local.hour() < 7 {
        morning = true;
    }
    if work {
        while date.weekday().to_monday_one_offset() > 5 {
            date = date.tomorrow().unwrap_or(date);
            morning = true;
        }
    }
    if !morning {
        return at;
    }
    date.at(8, local.minute(), 0, 0)
        .to_zoned(tz.clone())
        .map_or(at, |zoned| zoned.timestamp())
}

fn attachment(file: &File) -> gmail::mime::Attachment {
    let (filename, content_type, data) = match file {
        File::Pdf { name, title, lines } => (*name, "application/pdf", assets::pdf(title, lines)),
        File::Png { name, bars } => (*name, "image/png", assets::chart_png(bars)),
        File::Csv { name, body } => (*name, "text/csv", body.as_bytes().to_vec()),
    };
    gmail::mime::Attachment {
        filename: filename.to_owned(),
        content_type: content_type.to_owned(),
        data,
        content_id: None,
    }
}

/// Two reply drafts in existing threads and two new messages.
fn create_drafts(mail: &mut Store, now: &Zoned) {
    let me = cast::me();
    let replies = [
        (
            "MSA v3 redlines",
            "Thanks Ben. Daniel's positions match what I heard from Rachel. One question on §7.2 before we counter: is 3x annual fees the cap for all data incidents, or only for breaches caused by us?\n\nMaya",
            40,
        ),
        (
            "Coffee next week?",
            "Hi Grace! Wednesday morning works great. How about 9am at Harborlight on 5th?",
            12,
        ),
    ];
    for (subject, body, minutes_ago) in replies {
        let context = mail.context(now.timestamp());
        let query = Query::compile(&format!("subject:\"{subject}\""), &context);
        let Some(parent) = mail
            .select_messages(&Selection {
                query: Some(query),
                label_ids: Vec::new(),
                include_spam_trash: false,
            })
            .first()
            .map(|message| message.to_wire(Format::Metadata, &[]))
        else {
            continue;
        };
        let from = parent
            .header("From")
            .map(Mailbox::parse)
            .unwrap_or_else(|| me.mailbox());
        let mut email = Email {
            from: Some(me.mailbox()),
            to: vec![from],
            subject: gmail::mime::reply_subject(subject),
            text: Some(body.into()),
            html: Some(compose::text_to_html(body)),
            date: now.checked_sub(minutes_ago.minutes()).ok(),
            message_id: Some(format!(
                "CA+draft{:x}@mail.gmail.com",
                compose::fnv(subject)
            )),
            ..Email::default()
        };
        email.reply_to_message(&parent);
        let at = now.timestamp() - minutes_ago.minutes();
        mail.create_draft(
            email.to_rfc822(),
            parent.thread_id.clone(),
            at.as_millisecond(),
        )
        .expect("fixture drafts are valid");
    }

    let fresh = [
        (
            cast::get("felix").mailbox(),
            "Invoice for the launch illustrations",
            "Hi Felix,\n\nThank you again for the illustrations, the team loves them. Could you send the invoice to marcus@brightmoor.co? He'll process it this month.\n\nBest,\nMaya",
            95,
        ),
        (
            Mailbox::new("Lena Fischer", "lena@brightmoor.co"),
            "Atlas is here: routing that thinks like a dispatcher",
            "Today we're launching Atlas, a routing engine built for the people who keep freight moving.\n\nDispatchers make hundreds of small decisions a day. Atlas does the tedious math (lanes, appointments, hours of service) so they can focus on the calls only a person can make.\n\nIn beta, teams cut median dispatch time by 41%.\n\n[draft: add customer quote from Rachel]",
            300,
        ),
    ];
    for (to, subject, body, minutes_ago) in fresh {
        let email = Email {
            from: Some(me.mailbox()),
            to: vec![to],
            subject: subject.into(),
            text: Some(body.into()),
            html: Some(compose::text_to_html(body)),
            date: now.checked_sub(minutes_ago.minutes()).ok(),
            message_id: Some(format!(
                "CA+draft{:x}@mail.gmail.com",
                compose::fnv(subject)
            )),
            ..Email::default()
        };
        let at = now.timestamp() - minutes_ago.minutes();
        mail.create_draft(email.to_rfc822(), None, at.as_millisecond())
            .expect("fixture drafts are valid");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_world() {
        let config = Config::default();
        let (a, b) = (world(&config), world(&config));
        assert_eq!(a.mail.history.current(), b.mail.history.current());
        let all = Selection {
            query: None,
            label_ids: Vec::new(),
            include_spam_trash: true,
        };
        let ids = |w: &World| -> Vec<(String, Vec<u8>)> {
            w.mail
                .select_messages(&all)
                .iter()
                .map(|m| (m.id.clone(), m.raw.clone()))
                .collect()
        };
        assert_eq!(ids(&a), ids(&b));
    }

    #[test]
    fn mailbox_has_the_promised_shape() {
        let world = world(&Config::default());
        let mail = &world.mail;
        assert!(
            mail.thread_count() >= 200,
            "{} threads",
            mail.thread_count()
        );
        let inbox = mail.label("INBOX").unwrap();
        assert!(inbox.threads_total.unwrap() >= 20);
        assert!(inbox.threads_unread.unwrap() >= 5);
        for label in [
            "SENT",
            "DRAFT",
            "SPAM",
            "TRASH",
            "STARRED",
            "IMPORTANT",
            "CATEGORY_PROMOTIONS",
            "CATEGORY_SOCIAL",
            "CATEGORY_FORUMS",
        ] {
            assert!(
                mail.label(label).unwrap().messages_total.unwrap() > 0,
                "{label} is empty"
            );
        }
        let longest = mail
            .select_threads(&Selection {
                query: None,
                label_ids: Vec::new(),
                include_spam_trash: true,
            })
            .iter()
            .map(|(id, _)| mail.thread(id).unwrap().len())
            .max()
            .unwrap();
        assert!(longest >= 15, "longest thread has {longest} messages");
        let newest = mail.select_messages(&Selection {
            query: None,
            label_ids: Vec::new(),
            include_spam_trash: true,
        })[0]
            .internal_date;
        assert!(newest <= world.config.now.timestamp().as_millisecond());
    }
}
