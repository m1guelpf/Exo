//! What a draft sends, as Exo writes it to its Gmail draft: the body HTML around the editor's
//! (exo-editor's `outgoing`), the From alias's signature, the quote, and the MIME around them,
//! with a plain-text alternative. Exo adds no headers of its own.

use exo_store::{Address, Alias, DraftRecord};
use jiff::Zoned;

use super::{draft::Open, plain, signature};

/// The HTML body Exo sends for `open`.
pub(crate) fn html(open: &Open, aliases: &[Alias]) -> String {
    body_html(open, aliases, false)
}

/// [`html`], with no signature at all when `skip_signature`.
fn body_html(open: &Open, aliases: &[Alias], skip_signature: bool) -> String {
    let signature_html = signature::of(open.record.from.as_ref(), aliases);
    let parts = exo_editor::outgoing::Parts {
        signature: signature_html.as_deref(),
        include_signature_content: signature::includes_content(open),
        skip_signature,
        quote: open.record.quoted_content.as_deref(),
        inlined: open.record.quoted_inlined,
    };
    exo_editor::outgoing::html(open.editor.document(), &parts)
}

/// The message `open` sends, dated `now`, without its attachments' bytes (the account adds
/// them). `account_name` names a From with no name of its own. It always has a Message-ID:
/// without one, mail-builder would make one from the machine's host name.
pub(crate) fn message(
    open: &Open,
    aliases: &[Alias],
    account_name: Option<&str>,
    now: &Zoned,
) -> gmail::mime::Email {
    email(open, aliases, account_name, now, html(open, aliases))
}

/// [`message`] without a signature, as Unsubscribe sends its request.
pub(crate) fn unsigned_message(open: &Open, aliases: &[Alias], now: &Zoned) -> gmail::mime::Email {
    email(open, aliases, None, now, body_html(open, aliases, true))
}

fn email(
    open: &Open,
    aliases: &[Alias],
    account_name: Option<&str>,
    now: &Zoned,
    html: String,
) -> gmail::mime::Email {
    let record = &open.record;
    gmail::mime::Email {
        from: record
            .from
            .as_ref()
            .map(|from| resolve_from(from, aliases, account_name)),
        to: mailboxes(&record.to),
        cc: mailboxes(&record.cc),
        bcc: mailboxes(&record.bcc),
        subject: record.subject.clone(),
        text: Some(plain::from_html(&html)),
        html: Some(format!("<html><head></head><body>{html}</body></html>")),
        message_id: Some(record.rfc822_id.clone()),
        in_reply_to: threading(record)
            .then(|| record.in_reply_to_rfc822.clone())
            .flatten(),
        references: if threading(record) {
            record.references.clone()
        } else {
            Vec::new()
        },
        date: Some(now.clone()),
        ..gmail::mime::Email::default()
    }
}

/// Replies and forwards carry threading headers; new messages and forked replies don't.
fn threading(record: &DraftRecord) -> bool {
    !matches!(
        record.action,
        exo_store::DraftAction::Compose | exo_store::DraftAction::EditSubjectLineReply
    )
}

fn mailboxes(list: &[Address]) -> Vec<gmail::mime::Mailbox> {
    list.iter()
        .filter(|address| !address.email.is_empty())
        .map(|address| gmail::mime::Mailbox {
            name: address.name.clone(),
            email: address.email.clone(),
        })
        .collect()
}

/// The From's mailbox: its own name, else its alias's, else the account's.
fn resolve_from(
    from: &Address,
    aliases: &[Alias],
    account_name: Option<&str>,
) -> gmail::mime::Mailbox {
    let alias_name = || {
        aliases
            .iter()
            .find(|alias| alias.email.eq_ignore_ascii_case(&from.email))
            .and_then(|alias| alias.name.clone())
    };
    gmail::mime::Mailbox {
        name: from
            .name
            .clone()
            .or_else(alias_name)
            .or_else(|| account_name.map(str::to_owned)),
        email: from.email.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exo_store::DraftAction;

    /// mail-builder writes 7-bit MIME.
    fn rfc822(email: gmail::mime::Email) -> String {
        String::from_utf8(email.to_rfc822()).expect("7-bit")
    }

    fn alias(signature: &str) -> Vec<Alias> {
        vec![Alias {
            email: "maya@brightmoor.co".into(),
            name: Some("Maya Okafor".into()),
            signature: Some(signature.into()),
            default: true,
            primary: true,
            active: true,
        }]
    }

    fn open(action: DraftAction) -> Open {
        let record = DraftRecord {
            action,
            from: Some(Address { name: None, email: "maya@brightmoor.co".into() }),
            to: vec![Address { name: Some("Tom Becker".into()), email: "tom@brightmoor.co".into() }],
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: "Re: Design crit".into(),
            body: String::new(),
            quoted_content: Some("<div class=\"gmail_quote\">On Tue, Tom wrote:<br/><blockquote class=\"gmail_quote\"><div>Heads up</div></blockquote></div>".into()),
            quoted_inlined: false,
            in_reply_to: Some("m1".into()),
            in_reply_to_rfc822: Some("b@x".into()),
            references: vec!["a@x".into(), "b@x".into()],
            rfc822_id: "c.d@brightmoor.co".into(),
            fingerprint: String::new(),
            attachments: Vec::new(),
            send_later: None,
            reminder: None,
        };
        let mut open = Open::new("draft00aaaaaaaaaaaaaa".into(), Some("t1".into()), record);
        open.editor.type_text("Sounds good");
        open
    }

    #[test]
    fn a_reply_threads_and_carries_signature_and_quote() {
        let aliases = alias("<div dir=\"ltr\">Maya</div>");
        let raw = rfc822(message(
            &open(DraftAction::Reply),
            &aliases,
            None,
            &"2026-11-17T09:00:00-05:00[America/New_York]"
                .parse()
                .unwrap(),
        ));
        assert!(raw.contains("In-Reply-To: <b@x>"), "{raw}");
        assert!(raw.contains("References: <a@x> <b@x>"), "{raw}");
        assert!(raw.contains("Message-ID: <c.d@brightmoor.co>"), "{raw}");
        assert!(
            raw.contains("From: \"Maya Okafor\" <maya@brightmoor.co>"),
            "the alias names a nameless From: {raw}"
        );
        assert!(raw.contains("multipart/alternative"), "{raw}");
        assert!(
            !raw.contains("X-Mailer") && !raw.contains("X-Superhuman"),
            "no headers of Exo's own"
        );
        let html = html(&open(DraftAction::Reply), &aliases);
        assert!(
            html.contains("Sounds good")
                && html.contains("gmail_signature")
                && html.contains("Heads up"),
            "{html}"
        );
        let new = rfc822(message(
            &open(DraftAction::Compose),
            &aliases,
            None,
            &"2026-11-17T09:00:00-05:00[America/New_York]"
                .parse()
                .unwrap(),
        ));
        assert!(!new.contains("In-Reply-To"), "new mail starts a thread");
    }

    #[test]
    fn a_reply_without_signatures_on_replies_sends_an_empty_block() {
        let aliases = alias("<div dir=\"ltr\">Maya</div>");
        let reply = Open {
            signature_on_replies: false,
            ..open(DraftAction::Reply)
        };
        let sent = html(&reply, &aliases);
        assert!(
            sent.contains("<div class=\"gmail_signature\"><br/></div>") && !sent.contains(">Maya<"),
            "{sent}"
        );
        let forward = Open {
            signature_on_replies: false,
            ..open(DraftAction::Forward)
        };
        assert!(!html(&forward, &aliases).contains(">Maya<"));
        let new = Open {
            signature_on_replies: false,
            ..open(DraftAction::Compose)
        };
        assert!(
            html(&new, &aliases).contains(">Maya<"),
            "new mail always carries it"
        );
    }
}
