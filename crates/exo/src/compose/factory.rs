//! New drafts: who a reply goes to, its subject and threading headers, which alias it's from, and
//! the quote behind its "…".

use exo_store::{
    Address, Alias, AttachmentSource, Contact, DraftAction, DraftAttachment, DraftRecord,
    MessageDetail,
};
use jiff::{Timestamp, Zoned, tz::TimeZone};

use super::recipients;

/// A new draft id: `draft00` and 14 hex digits.
pub(crate) fn new_id() -> String {
    let random = getrandom::u64().unwrap_or_else(|_| Timestamp::now().as_nanosecond() as u64);
    format!("draft00{:014x}", random & 0x00ff_ffff_ffff_ffff)
}

/// A new `Message-ID` without angle brackets: the clamped time in base 36, a UUID v4, and the
/// sender's domain (Exo names no product in its headers).
pub(crate) fn new_rfc822_id(from: &str, now: Timestamp) -> String {
    let time = now
        .as_millisecond()
        .clamp(78_364_164_096, 2_821_109_907_455);
    let domain = from
        .rsplit_once('@')
        .map_or("localhost", |(_, domain)| domain);
    format!("{}.{}@{domain}", base36(time as u64), uuid_v4())
}

fn base36(mut value: u64) -> String {
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(digits[(value % 36) as usize]);
        value /= 36;
        if value == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII digits")
}

pub(crate) use exo_editor::uuid_v4;

/// The account a draft is written in.
pub(crate) struct Account<'a> {
    /// The send-as aliases, from Gmail.
    pub aliases: &'a [Alias],
    /// The account's address, the fallback when there are no aliases yet.
    pub email: &'a str,
    /// Dates in quotes are written in this zone.
    pub tz: TimeZone,
}

impl Account<'_> {
    /// The default alias, else the primary.
    pub fn default_from(&self) -> Address {
        let alias = self
            .aliases
            .iter()
            .find(|alias| alias.default)
            .or_else(|| self.aliases.iter().find(|alias| alias.primary));
        match alias {
            Some(alias) => Address {
                name: alias.name.clone(),
                email: alias.email.clone(),
            },
            None => Address {
                name: None,
                email: self.email.to_owned(),
            },
        }
    }

    fn active_alias(&self, email: &str) -> Option<&Alias> {
        self.aliases
            .iter()
            .find(|alias| alias.active && recipients::same_address(&alias.email, email))
    }

    fn in_send_as(&self, email: &str) -> bool {
        self.aliases
            .iter()
            .any(|alias| recipients::same_address(&alias.email, email))
            || recipients::same_address(self.email, email)
    }

    /// The alias a reply to `message` is from: the first active alias among its To, Cc, From and
    /// Bcc, else its `Delivered-To` (not the primary, unless it's the default), else the default.
    pub fn addressee(&self, message: &MessageDetail) -> Address {
        let default = self.default_from();
        let on_message = message
            .to
            .iter()
            .chain(&message.cc)
            .chain(&message.from)
            .chain(&message.bcc)
            .find_map(|contact| self.active_alias(&contact.email));
        let delivered = || {
            message
                .headers
                .iter()
                .filter(|header| header.name.eq_ignore_ascii_case("Delivered-To"))
                .find_map(|header| {
                    self.active_alias(header.value.trim())
                        .filter(|alias| !alias.primary || alias.default)
                })
        };
        let found = on_message.or_else(delivered);
        match found {
            Some(alias) if !recipients::same_address(&alias.email, &default.email) => Address {
                name: alias.name.clone().or_else(|| {
                    message
                        .to
                        .iter()
                        .chain(&message.cc)
                        .find(|contact| recipients::same_address(&contact.email, &alias.email))
                        .and_then(|contact| contact.name.clone())
                }),
                email: alias.email.clone(),
            },
            _ => default,
        }
    }

    /// The alias `message` was delivered to (its last `Delivered-To` that's one of yours, as long
    /// as it can send), else its [addressee](Self::addressee).
    pub fn delivered_to(&self, message: &MessageDetail) -> Address {
        let delivered = message
            .headers
            .iter()
            .rev()
            .filter(|header| header.name.eq_ignore_ascii_case("Delivered-To"))
            .find_map(|header| self.active_alias(header.value.trim()));
        match delivered {
            Some(alias) => Address {
                name: alias.name.clone(),
                email: alias.email.clone(),
            },
            None => self.addressee(message),
        }
    }
}

/// The new-message draft, with `fields` filled in.
pub(crate) fn compose(account: &Account<'_>, fields: Fields, now: Timestamp) -> DraftRecord {
    let from = fields.from.unwrap_or_else(|| account.default_from());
    let rfc822_id = new_rfc822_id(&from.email, now);
    let mut record = DraftRecord {
        action: DraftAction::Compose,
        from: Some(from),
        to: fields.to,
        cc: fields.cc,
        bcc: fields.bcc,
        subject: fields.subject,
        body: fields.body,
        quoted_content: None,
        quoted_inlined: false,
        in_reply_to: None,
        in_reply_to_rfc822: None,
        references: Vec::new(),
        rfc822_id,
        fingerprint: String::new(),
        attachments: Vec::new(),
        send_later: None,
        reminder: None,
    };
    record.fingerprint = fingerprint(&record);
    record
}

/// What a new message starts with (from a `mailto:` or a pasted message).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Fields {
    pub from: Option<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    /// Body HTML.
    pub body: String,
}

impl Fields {
    /// A new message to `email` (`Compose to {email}`).
    pub fn to(email: &str) -> Self {
        Self {
            to: vec![super::recipients::parse(email)],
            ..Self::default()
        }
    }

    /// A copied message's fields (with its new part as the body), which pasting it makes a draft
    /// of.
    pub fn of_message(message: &MessageDetail, unquoted_html: &str) -> Self {
        let list = |contacts: &[Contact]| contacts.iter().map(address).collect();
        Self {
            from: message.from.as_ref().filter(|from| from.is_me).map(address),
            to: list(&message.to),
            cc: list(&message.cc),
            bcc: list(&message.bcc),
            subject: message.subject.clone(),
            body: unquoted_html.to_owned(),
        }
    }
}

/// The original message's body, as the quote needs it: the rendered (sanitized) HTML's
/// `<body>` contents and that element's attributes.
#[derive(Clone, Debug, Default)]
pub(crate) struct QuotedBody {
    pub inner_html: String,
    pub class: String,
    pub style: String,
    pub id: String,
}

/// A reply, reply all or forward of `message`. `in_sent`: the message is in Sent (then every
/// send-as address counts as you).
pub(crate) fn reply(
    account: &Account<'_>,
    action: DraftAction,
    message: &MessageDetail,
    in_sent: bool,
    body: &QuotedBody,
    now: Timestamp,
) -> DraftRecord {
    let from = account.addressee(message);
    let (to, cc) = recipients_for(account, action, message, in_sent, &from.email);
    let sent = message.date.to_zoned(account.tz.clone());
    let quoted_content = Some(quote(action, message, &sent, body));
    let mut record = DraftRecord {
        action,
        to,
        cc,
        bcc: Vec::new(),
        subject: subject(action, &message.subject),
        body: String::new(),
        quoted_content,
        quoted_inlined: false,
        in_reply_to: Some(message.id.clone()),
        in_reply_to_rfc822: message.rfc822_id.clone(),
        references: references(message),
        rfc822_id: new_rfc822_id(&from.email, now),
        from: Some(from),
        fingerprint: String::new(),
        attachments: Vec::new(),
        send_later: None,
        reminder: None,
    };
    copy_attachments(&mut record, message, action == DraftAction::Forward);
    record.fingerprint = fingerprint(&record);
    record
}

/// Whether `record` already has `attachment` of a Gmail message.
fn has_attachment(record: &DraftRecord, attachment: &exo_store::Attachment) -> bool {
    record.attachments.iter().any(|kept| match &kept.source {
        AttachmentSource::Gmail {
            message_id,
            part_id,
            ..
        } => *message_id == attachment.message_id && *part_id == attachment.part_id,
        AttachmentSource::Local => false,
    })
}

/// How many of `message`'s files `record` doesn't have (for `Include Original Attachments`).
pub(crate) fn missing_originals(record: &DraftRecord, message: &MessageDetail) -> usize {
    message
        .attachments
        .iter()
        .filter(|attachment| !attachment.inline && !has_attachment(record, attachment))
        .count()
}

/// Whether a reply's recipients going from `before` to `now` (lowercased addresses) adds someone
/// who isn't on `message`, the one it answers (its To, Cc and From): what makes Exo offer the
/// message's files.
pub(crate) fn adds_newcomer(now: &[String], before: &[String], message: &MessageDetail) -> bool {
    let on_message = |email: &String| {
        message
            .to
            .iter()
            .chain(&message.cc)
            .chain(&message.from)
            .any(|contact| contact.email.eq_ignore_ascii_case(email))
    };
    now.iter()
        .any(|email| !before.contains(email) && !on_message(email))
}

/// Every attachment of `message` (`all`: a forward, or `Include Original Attachments`), else only
/// its inline images (so a reply's quote shows them); each once, by reference to its Gmail part.
pub(crate) fn copy_attachments(record: &mut DraftRecord, message: &MessageDetail, all: bool) {
    for attachment in &message.attachments {
        if has_attachment(record, attachment) || !(all || attachment.inline) {
            continue;
        }
        record.attachments.push(DraftAttachment {
            uuid: uuid_v4(),
            name: attachment.filename.clone(),
            mime_type: attachment.mime_type.clone(),
            size: attachment.size.into(),
            content_id: attachment.content_id.clone().filter(|_| attachment.inline),
            source: AttachmentSource::Gmail {
                message_id: attachment.message_id.clone(),
                attachment_id: attachment.attachment_id.clone(),
                part_id: attachment.part_id.clone(),
            },
        });
    }
}

/// Re-aims an existing reply draft at another action (pressing R where a Reply All draft
/// exists): recipients and subject recomputed, id, body and `Message-ID` kept.
pub(crate) fn retarget(
    record: &mut DraftRecord,
    account: &Account<'_>,
    action: DraftAction,
    message: &MessageDetail,
    in_sent: bool,
    body: &QuotedBody,
) {
    let from = account.addressee(message);
    let (to, cc) = recipients_for(account, action, message, in_sent, &from.email);
    let sent = message.date.to_zoned(account.tz.clone());
    record.action = action;
    record.to = to;
    record.cc = cc;
    record.subject = subject(action, &message.subject);
    if !record.quoted_inlined {
        record.quoted_content = Some(quote(action, message, &sent, body));
    }
    record.from = Some(from);
    copy_attachments(record, message, action == DraftAction::Forward);
    record.fingerprint = fingerprint(record);
}

/// A reply made a new message (its subject edited): it answers nothing, so it goes out without
/// `In-Reply-To` and `References` and starts its own thread. Recipients, body, quote and files
/// stay.
pub(crate) fn unthread(record: &mut DraftRecord) {
    record.action = DraftAction::EditSubjectLineReply;
    record.in_reply_to = None;
    record.in_reply_to_rfc822 = None;
    record.references.clear();
}

/// To and Cc of a reply.
fn recipients_for(
    account: &Account<'_>,
    action: DraftAction,
    message: &MessageDetail,
    in_sent: bool,
    reply_from: &str,
) -> (Vec<Address>, Vec<Address>) {
    if action == DraftAction::Forward {
        return (Vec::new(), Vec::new());
    }
    let all = action == DraftAction::ReplyAll;
    let is_self =
        |contact: &Contact| contact.is_me || (in_sent && account.in_send_as(&contact.email));
    // Cc without you, the alias replied from, or anyone in To; once each.
    let cc = |list: Vec<&Contact>, to: &[Address]| -> Vec<Address> {
        let mut out: Vec<Address> = Vec::new();
        for contact in list {
            if contact.is_me
                || recipients::same_address(&contact.email, reply_from)
                || to.iter().any(|address| address.email == contact.email)
                || out.iter().any(|address| address.email == contact.email)
            {
                continue;
            }
            out.push(address(contact));
        }
        out
    };
    let to_and_cc = || {
        if all {
            message.to.iter().chain(&message.cc).collect()
        } else {
            Vec::new()
        }
    };
    let only_cc = || {
        if all {
            message.cc.iter().collect()
        } else {
            Vec::new()
        }
    };
    if !message.reply_to.is_empty() {
        let from = message.from.as_ref();
        let to: Vec<Address> = message
            .reply_to
            .iter()
            .map(|contact| {
                let mut address = address(contact);
                if address.name.is_none()
                    && let Some(from) =
                        from.filter(|from| recipients::same_address(&from.email, &contact.email))
                {
                    address.name.clone_from(&from.name);
                }
                address
            })
            .collect();
        let cc = cc(to_and_cc(), &to);
        return (to, cc);
    }
    let Some(from) = &message.from else {
        return (Vec::new(), Vec::new());
    };
    if is_self(from) && message.to.iter().any(|contact| !contact.is_me) {
        let to: Vec<Address> = message.to.iter().map(address).collect();
        let cc = cc(only_cc(), &to);
        return (to, cc);
    }
    let to = vec![address(from)];
    let cc = if is_self(from) {
        cc(only_cc(), &to)
    } else {
        cc(to_and_cc(), &to)
    };
    (to, cc)
}

fn address(contact: &Contact) -> Address {
    Address {
        name: contact.name.clone(),
        email: contact.email.clone(),
    }
}

/// A reply's or forward's subject: `Fwd: ` always; `Re: ` unless it already starts with `re:`.
pub(crate) fn subject(action: DraftAction, subject: &str) -> String {
    if action == DraftAction::Forward {
        return format!("Fwd: {subject}");
    }
    if subject
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("re:"))
    {
        subject.to_owned()
    } else {
        format!("Re: {subject}")
    }
}

/// A reply's References: the message's References and its Message-ID, else its In-Reply-To and
/// Message-ID, else its Message-ID.
fn references(message: &MessageDetail) -> Vec<String> {
    let mut references = if !message.references.is_empty() {
        message.references.clone()
    } else {
        message.in_reply_to.iter().cloned().collect()
    };
    references.extend(message.rfc822_id.clone());
    references.retain(|reference| !reference.is_empty());
    references
}

/// The draft's fingerprint: To and Cc addresses and the files' ids (sorted) as created, which tell
/// whether a reply's recipients or files were touched.
pub(crate) fn fingerprint(record: &DraftRecord) -> String {
    let join = |list: &[Address]| {
        list.iter()
            .map(|address| address.email.as_str())
            .collect::<String>()
    };
    let mut files: Vec<&str> = record
        .attachments
        .iter()
        .map(|attachment| attachment.uuid.as_str())
        .collect();
    files.sort_unstable();
    format!(
        "{}|{}|{}",
        join(&record.to),
        join(&record.cc),
        files.concat()
    )
}

/// The quote: the reply's attribution and quoted message, or the forward's header and body.
fn quote(action: DraftAction, message: &MessageDetail, sent: &Zoned, body: &QuotedBody) -> String {
    let from = message.from.as_ref();
    if action == DraftAction::Forward {
        let raw = |contact: &Contact| recipients::format(&address(contact));
        let to: Vec<String> = message.to.iter().map(raw).collect();
        let cc: Vec<String> = message.cc.iter().map(raw).collect();
        let to: Vec<&str> = to.iter().map(String::as_str).collect();
        let cc: Vec<&str> = cc.iter().map(String::as_str).collect();
        let from = from.map(raw).unwrap_or_default();
        let header = exo_editor::outgoing::Forwarded {
            from: &from,
            to: &to,
            cc: &cc,
            subject: &message.subject,
        };
        return exo_editor::outgoing::forward_quote(
            sent,
            &header,
            &body.class,
            &body.style,
            &body.id,
            &body.inner_html,
        );
    }
    let (name, email) = from.map_or((String::new(), String::new()), |from| {
        (from.name.clone().unwrap_or_default(), from.email.clone())
    });
    exo_editor::outgoing::reply_quote(
        sent,
        &name,
        &email,
        &body.class,
        &body.style,
        &body.id,
        &body.inner_html,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "maya@brightmoor.co";

    fn contact(name: &str, email: &str) -> Contact {
        Contact {
            name: Some(name.into()),
            email: email.into(),
            is_me: email == ME,
        }
    }

    fn aliases() -> Vec<Alias> {
        vec![
            Alias {
                email: ME.into(),
                name: Some("Maya Okafor".into()),
                signature: None,
                default: true,
                primary: true,
                active: true,
            },
            Alias {
                email: "hello@mayaokafor.com".into(),
                name: Some("Maya Okafor".into()),
                signature: None,
                default: false,
                primary: false,
                active: true,
            },
        ]
    }

    fn message(from: Contact, to: Vec<Contact>, cc: Vec<Contact>) -> MessageDetail {
        MessageDetail {
            id: "m1".into(),
            labels: vec!["INBOX".into()],
            date: "2026-11-17T08:31:00-05:00".parse::<Timestamp>().unwrap(),
            from: Some(from),
            to,
            cc,
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: "Design crit moved to 3pm".into(),
            snippet: String::new(),
            rfc822_id: Some("b@x".into()),
            in_reply_to: Some("a@x".into()),
            references: Vec::new(),
            list_id: None,
            list_unsubscribe: None,
            list_unsubscribe_post: None,
            headers: Vec::new(),
            body: exo_store::Body::default(),
            attachments: Vec::new(),
            failure: None,
            scheduled: None,
            sending: false,
        }
    }

    fn account(aliases: &[Alias]) -> Account<'_> {
        Account {
            aliases,
            email: ME,
            tz: TimeZone::get("America/New_York").unwrap(),
        }
    }

    fn emails(list: &[Address]) -> Vec<&str> {
        list.iter().map(|address| address.email.as_str()).collect()
    }

    #[test]
    fn replies_go_to_the_sender_and_reply_all_copies_the_rest() {
        let aliases = aliases();
        let account = account(&aliases);
        let crit = message(
            contact("Tom Becker", "tom@brightmoor.co"),
            vec![
                contact("Maya", ME),
                contact("Priya Raman", "priya@brightmoor.co"),
            ],
            vec![contact("Dan", "dan@brightmoor.co")],
        );
        let now = Timestamp::now();
        let reply = reply(
            &account,
            DraftAction::Reply,
            &crit,
            false,
            &QuotedBody::default(),
            now,
        );
        assert_eq!(emails(&reply.to), ["tom@brightmoor.co"]);
        assert!(reply.cc.is_empty(), "plain Reply never copies anyone");
        assert_eq!(reply.subject, "Re: Design crit moved to 3pm");
        assert_eq!(
            reply.references,
            ["a@x", "b@x"],
            "In-Reply-To then Message-ID"
        );
        assert_eq!(reply.in_reply_to_rfc822.as_deref(), Some("b@x"));
        let all = super::reply(
            &account,
            DraftAction::ReplyAll,
            &crit,
            false,
            &QuotedBody::default(),
            now,
        );
        assert_eq!(emails(&all.to), ["tom@brightmoor.co"]);
        assert_eq!(
            emails(&all.cc),
            ["priya@brightmoor.co", "dan@brightmoor.co"],
            "everyone else, without me"
        );
        let forward = super::reply(
            &account,
            DraftAction::Forward,
            &crit,
            false,
            &QuotedBody::default(),
            now,
        );
        assert!(forward.to.is_empty() && forward.cc.is_empty());
        assert_eq!(forward.subject, "Fwd: Design crit moved to 3pm");
        assert!(
            forward
                .quoted_content
                .as_deref()
                .is_some_and(|quote| quote.contains("---------- Forwarded message ----------"))
        );
    }

    #[test]
    fn forwards_take_the_files_and_replies_only_on_request() {
        let aliases = aliases();
        let account = account(&aliases);
        let mut deck = message(
            contact("Daniel Reyes", "daniel@brightmoor.co"),
            vec![contact("Maya", ME)],
            Vec::new(),
        );
        let file = |part_id: &str, inline: bool| exo_store::Attachment {
            message_id: deck.id.clone(),
            attachment_id: Some(format!("att-{part_id}")),
            part_id: part_id.into(),
            filename: if inline {
                "logo.png".into()
            } else {
                "deck.pdf".into()
            },
            mime_type: if inline {
                "image/png".into()
            } else {
                "application/pdf".into()
            },
            size: 10,
            content_id: inline.then(|| "logo".into()),
            inline,
        };
        deck.attachments = vec![file("2", false), file("3", true)];
        let now = Timestamp::now();
        let forward = reply(
            &account,
            DraftAction::Forward,
            &deck,
            false,
            &QuotedBody::default(),
            now,
        );
        assert_eq!(forward.attachments.len(), 2, "every file and inline image");
        let mut answer = reply(
            &account,
            DraftAction::Reply,
            &deck,
            false,
            &QuotedBody::default(),
            now,
        );
        assert_eq!(
            answer.attachments.len(),
            1,
            "only the inline image its quote shows"
        );
        assert_eq!(missing_originals(&answer, &deck), 1);
        copy_attachments(&mut answer, &deck, true);
        copy_attachments(&mut answer, &deck, true);
        assert_eq!(
            answer.attachments.len(),
            2,
            "Include Original Attachment adds it once"
        );
        assert_eq!(missing_originals(&answer, &deck), 0);
    }

    #[test]
    fn only_someone_new_to_the_thread_prompts_for_the_files() {
        let crit = message(
            contact("Tom Becker", "tom@brightmoor.co"),
            vec![
                contact("Maya", ME),
                contact("Priya Raman", "priya@brightmoor.co"),
            ],
            Vec::new(),
        );
        let before = vec!["tom@brightmoor.co".to_owned()];
        let with = |extra: &str| [before.clone(), vec![extra.to_owned()]].concat();
        assert!(
            !adds_newcomer(&with("priya@brightmoor.co"), &before, &crit),
            "Priya was on it"
        );
        assert!(
            adds_newcomer(&with("dan@brightmoor.co"), &before, &crit),
            "Dan wasn't"
        );
        assert!(
            !adds_newcomer(
                &with("dan@brightmoor.co"),
                &with("dan@brightmoor.co"),
                &crit
            ),
            "already added"
        );
    }

    #[test]
    fn replying_to_your_own_message_goes_to_its_recipients() {
        let aliases = aliases();
        let account = account(&aliases);
        let mine = message(
            contact("Maya", ME),
            vec![contact("Ann", "ann@x.com")],
            vec![contact("Bob", "bob@x.com")],
        );
        let all = reply(
            &account,
            DraftAction::ReplyAll,
            &mine,
            true,
            &QuotedBody::default(),
            Timestamp::now(),
        );
        assert_eq!(emails(&all.to), ["ann@x.com"]);
        assert_eq!(emails(&all.cc), ["bob@x.com"]);
        let note = message(contact("Maya", ME), vec![contact("Maya", ME)], Vec::new());
        assert_eq!(
            emails(
                &reply(
                    &account,
                    DraftAction::Reply,
                    &note,
                    true,
                    &QuotedBody::default(),
                    Timestamp::now()
                )
                .to
            ),
            [ME]
        );
    }

    #[test]
    fn reply_to_wins_and_takes_the_senders_name() {
        let aliases = aliases();
        let account = account(&aliases);
        let mut list = message(
            contact("Ops", "ops@x.com"),
            vec![contact("Maya", ME)],
            Vec::new(),
        );
        list.reply_to = vec![Contact {
            name: None,
            email: "ops@x.com".into(),
            is_me: false,
        }];
        let reply = reply(
            &account,
            DraftAction::Reply,
            &list,
            false,
            &QuotedBody::default(),
            Timestamp::now(),
        );
        assert_eq!(
            reply.to,
            [Address {
                name: Some("Ops".into()),
                email: "ops@x.com".into()
            }]
        );
    }

    #[test]
    fn replies_come_from_the_alias_the_message_was_sent_to() {
        let aliases = aliases();
        let account = account(&aliases);
        let to_alias = message(
            contact("Ann", "ann@x.com"),
            vec![contact("Maya", "hello@mayaokafor.com")],
            Vec::new(),
        );
        assert_eq!(account.addressee(&to_alias).email, "hello@mayaokafor.com");
        let to_primary = message(
            contact("Ann", "ann@x.com"),
            vec![contact("Maya", ME)],
            Vec::new(),
        );
        assert_eq!(account.addressee(&to_primary).email, ME);
    }

    #[test]
    fn subjects_keep_one_re() {
        assert_eq!(subject(DraftAction::Reply, "RE: Lunch"), "RE: Lunch");
        assert_eq!(subject(DraftAction::Reply, "Fwd: Lunch"), "Re: Fwd: Lunch");
        assert_eq!(subject(DraftAction::Forward, "Re: Lunch"), "Fwd: Re: Lunch");
    }

    #[test]
    fn draft_ids_are_hex_and_message_ids_take_the_senders_domain() {
        let id = new_id();
        assert!(
            id.len() == 21
                && id.starts_with("draft00")
                && id[5..].chars().all(|c| c.is_ascii_hexdigit()),
            "{id}"
        );
        let rfc = new_rfc822_id(ME, "2026-11-17T14:00:00Z".parse().unwrap());
        let (time, rest) = rfc.split_once('.').unwrap();
        assert_eq!(time.len(), 8, "8 base-36 digits");
        assert!(rest.ends_with("@brightmoor.co"), "{rfc}");
    }
}
