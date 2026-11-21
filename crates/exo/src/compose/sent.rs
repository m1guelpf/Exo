//! A reply being sent, shown in its thread at once: the outgoing message shows as a sent one
//! the moment Send is pressed, dated when it goes out, from what it sends, until Gmail's copy is
//! in the thread, which then takes its place (its focus and expansion with it).

use std::sync::Arc;

use exo_store::{Address, Alias, Body, Contact, MessageDetail, ThreadDetail};
use jiff::Timestamp;

use super::{draft::Open, plain, recipients};
use crate::thread::ThreadView;

/// One reply on show.
struct Shown {
    /// Its draft.
    draft: String,
    thread: String,
    message: MessageDetail,
    /// Gmail's sent message, once it went.
    sent_as: Option<String>,
}

/// The replies on show, and the last thread copy made with them.
#[derive(Default)]
pub(super) struct ShownSends {
    shown: Vec<Shown>,
    /// The thread as stored, the messages added to it, and the result.
    copy: Option<(Arc<ThreadDetail>, Vec<String>, Arc<ThreadDetail>)>,
}

impl ShownSends {
    /// Shows `open`, a reply sent as `html` (the body Gmail gets), going out `at`. Returns the
    /// message id it's shown under.
    pub fn show(
        &mut self,
        open: &Open,
        html: &str,
        aliases: &[Alias],
        at: Timestamp,
    ) -> Option<String> {
        let thread = open.thread.clone()?;
        let message = message(open, html, aliases, at);
        let id = message.id.clone();
        self.forget(&open.id);
        self.shown.push(Shown {
            draft: open.id.clone(),
            thread,
            message,
            sent_as: None,
        });
        Some(id)
    }

    /// The draft shown as message `message`.
    pub fn draft_of(&self, message: &str) -> Option<&str> {
        self.shown
            .iter()
            .find(|shown| shown.message.id == message)
            .map(|shown| shown.draft.as_str())
    }

    /// The send of `draft` was taken back, or failed: its message goes.
    pub fn forget(&mut self, draft: &str) {
        self.shown.retain(|shown| shown.draft != draft);
    }

    /// Gmail sent `draft` as `message`: Gmail's copy replaces it once the thread has it, if the
    /// thread is on screen (`showing`); otherwise it goes now.
    pub fn sent(&mut self, draft: &str, message: &str, showing: Option<&str>) {
        let on_screen = |shown: &Shown| showing == Some(shown.thread.as_str());
        self.shown
            .retain(|shown| shown.draft != draft || on_screen(shown));
        for shown in self.shown.iter_mut().filter(|shown| shown.draft == draft) {
            shown.sent_as = Some(message.to_owned());
        }
    }

    /// Thread `thread` closed: messages already sent go (Gmail's copy shows from now on).
    pub fn close(&mut self, thread: &str) {
        self.shown
            .retain(|shown| shown.thread != thread || shown.sent_as.is_none());
    }

    /// Gives `view` the thread's latest copy (`detail`) with the replies being sent in it.
    pub fn set_detail(&mut self, view: &mut ThreadView, detail: Arc<ThreadDetail>) {
        let thread = detail.id.as_str().to_owned();
        let thread = thread.as_str();
        self.shown.retain(|shown| {
            let arrived = shown.sent_as.as_ref().filter(|id| {
                shown.thread == thread && detail.messages.iter().any(|message| message.id == **id)
            });
            if let Some(id) = arrived {
                view.rename_message(&shown.message.id, id);
            }
            arrived.is_none()
        });
        let ids: Vec<String> = self
            .shown
            .iter()
            .filter(|shown| shown.thread == thread)
            .map(|shown| shown.message.id.clone())
            .collect();
        if ids.is_empty() {
            view.set_detail(detail);
            return;
        }
        let reused = self
            .copy
            .as_ref()
            .filter(|(source, shown, _)| Arc::ptr_eq(source, &detail) && *shown == ids)
            .map(|(_, _, copy)| copy.clone());
        let copy = reused.unwrap_or_else(|| {
            let mut copy = (*detail).clone();
            copy.messages.extend(
                self.shown
                    .iter()
                    .filter(|shown| shown.thread == thread)
                    .map(|shown| shown.message.clone()),
            );
            let copy = Arc::new(copy);
            self.copy = Some((detail, ids, copy.clone()));
            copy
        });
        view.set_detail(copy);
    }
}

/// The message `open` sends, as Gmail will have it in Sent: from you, to its recipients,
/// dated `at`. Its files show once Gmail's copy arrives.
fn message(open: &Open, html: &str, aliases: &[Alias], at: Timestamp) -> MessageDetail {
    let record = &open.record;
    let contact = |address: &Address| Contact {
        name: address.name.clone(),
        email: address.email.clone(),
        is_me: aliases
            .iter()
            .any(|alias| recipients::same_address(&alias.email, &address.email)),
    };
    let text = plain::from_html(html);
    MessageDetail {
        id: format!("sending-{}", open.id),
        labels: vec!["SENT".into()],
        date: at,
        from: record.from.as_ref().map(|from| Contact {
            is_me: true,
            ..contact(from)
        }),
        to: record.to.iter().map(contact).collect(),
        cc: record.cc.iter().map(contact).collect(),
        bcc: record.bcc.iter().map(contact).collect(),
        reply_to: Vec::new(),
        subject: record.subject.clone(),
        snippet: text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(200)
            .collect(),
        rfc822_id: Some(record.rfc822_id.clone()),
        in_reply_to: record.in_reply_to_rfc822.clone(),
        references: record.references.clone(),
        list_id: None,
        list_unsubscribe: None,
        list_unsubscribe_post: None,
        headers: Vec::new(),
        body: Body {
            html: Some(format!("<html><head></head><body>{html}</body></html>")),
            text: Some(text),
        },
        attachments: Vec::new(),
        failure: None,
        scheduled: None,
        sending: true,
    }
}
