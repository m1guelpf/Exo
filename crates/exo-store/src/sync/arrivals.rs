//! New mail: the messages a round of history brought that the store didn't have, as a
//! notification needs them (who sent it, its subject and snippet, and where the thread is).

use std::collections::BTreeSet;

use jiff::Timestamp;

use crate::error::Result;
use crate::ids::labels::{INBOX, SH_IMPORTANT, UNREAD};
use crate::model::Contact;
use crate::parse::unescape_html;
use crate::store::{Inner, Store, load_view, load_views};
use crate::view::{Identity, ThreadView, ViewMessage};

/// A message new to the store: Gmail's history added it, the store didn't have it before (it
/// wasn't backfilled or written by Exo first), and it's still there once the round is applied.
/// Drafts never are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrival {
    /// Its thread.
    pub thread: String,
    /// The message.
    pub message: String,
    /// When Gmail got it (`internalDate`).
    pub date: Timestamp,
    /// Who sent it; `is_me` for one of the account's own addresses.
    pub from: Option<Contact>,
    /// Its subject.
    pub subject: String,
    /// Gmail's snippet of it, unescaped.
    pub snippet: String,
    /// It's unread.
    pub unread: bool,
    /// It's in the inbox: it has Gmail's `INBOX` label, and its thread is routed there (not
    /// muted, not spam).
    pub inbox: bool,
    /// Its thread is Important by the importance rule, or was moved there, whatever splits take
    /// it out of Important's tab.
    pub important: bool,
    /// Its thread is muted.
    pub muted: bool,
}

impl Arrival {
    fn new(view: &ThreadView, message: &ViewMessage, me: &Identity) -> Self {
        let lists = view.list_ids(me);
        let from = message.row.sender.0.as_ref().map(|sender| Contact {
            name: sender.name.clone(),
            email: sender.email.clone(),
            is_me: me.is_me(&sender.email),
        });
        Self {
            thread: view.id.clone(),
            message: message.row.id.clone(),
            date: Timestamp::from_millisecond(message.row.internal_date).unwrap_or_default(),
            from,
            subject: message.row.subject.clone(),
            snippet: unescape_html(&message.row.snippet),
            unread: message.has(UNREAD),
            inbox: message.has(INBOX) && lists.contains(&INBOX),
            important: lists.contains(&SH_IMPORTANT),
            muted: view.is_muted(me),
        }
    }
}

impl Inner {
    /// The arrivals among `messages` (thread and message ids), oldest first.
    pub(super) async fn arrivals(&self, messages: &[(String, String)]) -> Result<Vec<Arrival>> {
        let threads: Vec<String> = messages
            .iter()
            .map(|(thread, _)| thread.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut db = self.db.clone();
        let views = load_views(&mut db, &threads).await?;
        let me = self.me();
        let mut arrivals: Vec<Arrival> = messages
            .iter()
            .filter_map(|(thread, id)| {
                let view = views.get(thread)?;
                let message = view.messages.iter().find(|message| message.row.id == *id)?;
                (!message.is_draft()).then(|| Arrival::new(view, message, &me))
            })
            .collect();
        arrivals.sort_by(|a, b| (a.date, &a.message).cmp(&(b.date, &b.message)));
        Ok(arrivals)
    }
}

impl Store {
    /// Thread `thread`'s newest message (drafts aside) as an [`Arrival`], for a notification
    /// about the thread as it stands (a reminder bringing it back). `None` when the store doesn't
    /// have it.
    ///
    /// # Errors
    /// On database failures.
    pub async fn latest_arrival(&self, thread: &str) -> Result<Option<Arrival>> {
        let mut db = self.inner.db.clone();
        let Some(view) = load_view(&mut db, thread).await? else {
            return Ok(None);
        };
        let me = self.inner.me();
        Ok(view
            .messages
            .iter()
            .rev()
            .find(|message| !message.is_draft())
            .map(|message| Arrival::new(&view, message, &me)))
    }
}
