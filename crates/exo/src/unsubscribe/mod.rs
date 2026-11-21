//! Unsubscribe: `⌘U` in the list or a thread, the palette's `Unsubscribe` and the thread header's
//! status all launch it. A conversation with nothing to unsubscribe from offers to block the sender
//! instead; one from a sender who can be named opens the Unsubscribe picker; either way an alert
//! asks first.
//!
//! How it unsubscribes ([`Target`]): a `mailto:` sends the request from your account
//! (`compose_send_unsubscribe`), a one-click link is posted to ([`one_click`]), anything else
//! opens in the browser. The picker's other rows then Mark Done or Trash everything from the
//! sender as a bulk action, which the toast's Undo puts back.

mod one_click;
mod target;

use std::sync::{Arc, LazyLock};

use exo_commands::{Command, SubPalette, Toast};
use exo_store::{BulkTarget, ListCutoff, ListId, Mutation, Scope, ThreadDetail};
use regex::Regex;

pub use one_click::{Reach, push};
pub(crate) use target::find;
use target::{Target, Unsubscribe};

use crate::{Mail, account::UndoToken, commands, compose::recipients, pickers};

/// What Unsubscribe's commands ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// `⌘U`, the palette's row, the header's status.
    Launch,
    /// A row of the Unsubscribe picker.
    Choose(Choice),
}

/// The Unsubscribe picker's unsubscribes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// `Unsubscribe`, and nothing else.
    Unsubscribe,
    /// `Unsubscribe, and Mark Done all from {sender}`.
    MarkDone,
    /// `Unsubscribe, and Trash all from {sender}`.
    Trash,
}

/// Unsubscribe's state between frames.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// The conversation whose messages the account was asked for, to launch on once read.
    reading: Option<String>,
    /// The picker's conversation, its messages and what unsubscribes it.
    picked: Option<(String, Arc<ThreadDetail>, Unsubscribe)>,
    /// The bulk actions started here, whose undo says `Unsubscribed.`.
    bulk: Vec<UndoToken>,
}

/// An unsubscribe waiting for its alert's `Confirm`.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    thread: String,
    detail: Arc<ThreadDetail>,
    unsubscribe: Unsubscribe,
    choice: Choice,
}

/// A query that already says where to look.
static IN_OR_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(^|\s)-?(in|label):").expect("valid regex"));

/// The Unsubscribe picker for `unsubscribe`: no input; then `Block Domain: {domain}` unless the
/// sender's domain is a webmail one or one of `own`.
fn palette(unsubscribe: &Unsubscribe, own: &[String]) -> SubPalette<commands::Action> {
    let row = |id: &'static str, title: String, choice: Choice, icon: &'static str| {
        Command::new(
            id,
            title,
            commands::Action::Unsubscribe(Action::Choose(choice)),
        )
        .icon(icon)
    };
    let from = &unsubscribe.sender;
    let mut rows = vec![
        row(
            "UNSUBSCRIBE",
            "Unsubscribe".into(),
            Choice::Unsubscribe,
            "data/unsubscribe",
        ),
        row(
            "UNSUBSCRIBE_AND_MARK_DONE",
            format!("Unsubscribe, and Mark Done all from {from}"),
            Choice::MarkDone,
            "data/done",
        ),
        row(
            "UNSUBSCRIBE_AND_TRASH",
            format!("Unsubscribe, and Trash all from {from}"),
            Choice::Trash,
            "data/discard",
        ),
    ];
    if let Some(domain) = from
        .rsplit_once('@')
        .map(|(_, domain)| domain.to_lowercase())
        .filter(|domain| !crate::format::names::is_popular_domain(domain) && !own.contains(domain))
    {
        let block = commands::Action::Pick(pickers::Action::Block {
            from: domain.clone(),
            domain: true,
            spam: false,
        });
        rows.push(
            Command::new("BLOCK_DOMAIN", format!("Block Domain: {domain}"), block)
                .icon("data/unsubscribe"),
        );
    }
    SubPalette::new("UNSUBSCRIBE", "Unsubscribe", "data/unsubscribe", rows).hide_input()
}

/// `query` looking in Spam or Trash while that's the list on screen, unless it says where to look
/// already.
fn scoped(query: String, list: &ListId) -> String {
    let folder = match list {
        ListId::Spam => "spam",
        ListId::Trash => "trash",
        _ => return query,
    };
    if IN_OR_LABEL.is_match(&query) {
        query
    } else {
        format!("{query} in:{folder}")
    }
}

impl Mail {
    /// Carries out an Unsubscribe command.
    pub(crate) fn unsubscribe_run(&mut self, action: Action) {
        match action {
            Action::Launch => self.launch_unsubscribe(),
            Action::Choose(choice) => {
                let Some((thread, detail, unsubscribe)) = self.unsubscribing.picked.take() else {
                    return;
                };
                self.commands.close_palette();
                self.confirm_unsubscribe(Plan {
                    thread,
                    detail,
                    unsubscribe,
                    choice,
                });
            }
        }
    }

    /// Launches Unsubscribe on the one conversation actions apply to, once its messages are at
    /// hand: the open thread's, or read from the store.
    fn launch_unsubscribe(&mut self) {
        let Some(id) = self.unsubscribe_thread() else {
            return;
        };
        match self
            .thread
            .as_ref()
            .filter(|open| open.id() == id)
            .and_then(crate::thread::ThreadView::shared_detail)
        {
            Some(detail) => self.unsubscribe_from(id, detail),
            None => {
                self.account.read_thread(&id);
                self.unsubscribing.reading = Some(id);
            }
        }
    }

    /// The one conversation Unsubscribe is for, if there is one (not several selected, nor a
    /// new message's draft).
    fn unsubscribe_thread(&self) -> Option<String> {
        if self.thread.is_none() && !self.inbox.is_exact() {
            return None;
        }
        match crate::actionable(self.thread.as_ref(), &self.inbox)[..] {
            [thread] if !thread.draft_only => Some(thread.id.clone()),
            _ => None,
        }
    }

    /// The account read `id`: if a launch asked for it and the conversation is still the one
    /// actions apply to, the launch goes on.
    pub(crate) fn unsubscribe_thread_read(
        &mut self,
        id: String,
        detail: Option<Arc<ThreadDetail>>,
    ) {
        if self
            .unsubscribing
            .reading
            .take_if(|reading| *reading == id)
            .is_none()
            || self.unsubscribe_thread().as_ref() != Some(&id)
        {
            return;
        }
        match detail {
            Some(detail) => self.unsubscribe_from(id, detail),
            None => self.block_instead(id),
        }
    }

    /// The picker for a sender with a plain address who isn't you; Unsubscribe's alert
    /// straight away for anyone else; the offer to block when there's nothing to unsubscribe
    /// from. Only a plain address names a sender, as the picker's other rows search for them
    /// (`senders::is_plain_address`).
    fn unsubscribe_from(&mut self, thread: String, detail: Arc<ThreadDetail>) {
        let own = self.own_domains();
        let found = find(&detail, &own, |id| {
            self.bodies
                .facts(id)
                .and_then(|facts| facts.unsubscribe.clone())
        });
        let Some(unsubscribe) = found else {
            return self.block_instead(thread);
        };
        if crate::senders::is_plain_address(&unsubscribe.sender)
            && !self.is_own_address(&unsubscribe.sender)
        {
            self.commands.open_sub_palette(palette(&unsubscribe, &own));
            self.unsubscribing.picked = Some((thread, detail, unsubscribe));
        } else {
            self.confirm_unsubscribe(Plan {
                thread,
                detail,
                unsubscribe,
                choice: Choice::Unsubscribe,
            });
        }
    }

    /// Whether `email` is one of your addresses.
    fn is_own_address(&self, email: &str) -> bool {
        recipients::same_address(self.account.email(), email)
            || self
                .snapshot
                .aliases
                .iter()
                .any(|alias| recipients::same_address(&alias.email, email))
    }

    /// No unsubscribe link, so the offer to block the sender, unless there's none to block or it's
    /// you.
    fn block_instead(&mut self, thread: String) {
        if let Some(sender) = self.actionable_sender() {
            self.pickers.confirm_block_instead(sender.email, thread);
        }
    }

    /// Asks before `plan`.
    fn confirm_unsubscribe(&mut self, plan: Plan) {
        let unsubscribe = &plan.unsubscribe;
        let name = exo_ui::wbrise(if unsubscribe.sender.is_empty() {
            &unsubscribe.list_name
        } else {
            &unsubscribe.sender
        });
        let (title, text) = match plan.choice {
            Choice::Unsubscribe => (
                "Unsubscribe",
                format!("Would you like to unsubscribe from {name}?"),
            ),
            Choice::MarkDone => (
                "Unsubscribe & Mark Done",
                format!(
                    "Would you like to unsubscribe and Mark Done all existing messages from {name}?"
                ),
            ),
            Choice::Trash => (
                "Unsubscribe & Trash",
                format!(
                    "Would you like to unsubscribe and Trash all existing messages from {name}?"
                ),
            ),
        };
        self.pickers.confirm_unsubscribe(title, text, plan);
    }

    /// `plan`'s alert said `Confirm`: it unsubscribes, says so unless the browser does it, and
    /// for a bulk row moves on and Marks Done or Trashes all from the sender.
    pub(crate) fn unsubscribe_confirmed(&mut self, plan: Plan) {
        let Plan {
            thread,
            detail,
            unsubscribe,
            choice,
        } = plan;
        let automatic = match &unsubscribe.target {
            Target::Mailto { to, subject } => {
                if let Some(message) = detail
                    .messages
                    .iter()
                    .find(|message| message.id == unsubscribe.message)
                {
                    self.compose_send_unsubscribe(to, subject, message);
                }
                true
            }
            Target::OneClick(url) if self.account.reach().permits(url) => {
                self.account.unsubscribe(url);
                true
            }
            // A one-click link Exo won't post to is a web link, as it is to the browser.
            Target::OneClick(url) | Target::Web(url) => {
                self.open_url(url.as_str());
                false
            }
        };
        if automatic {
            let text = if self.snapshot.connection.is_online() {
                "Unsubscribed."
            } else {
                "Unsubscribe will happen when online."
            };
            self.commands.toast(Toast::new(text));
            // A bulk Mark Done moves on from an inbox, a bulk Trash always, a lone Unsubscribe
            // never.
            let advance = match choice {
                Choice::Unsubscribe => false,
                Choice::MarkDone => self.search.is_none() && crate::lists::is_inbox(&self.list),
                Choice::Trash => true,
            };
            if advance {
                self.move_away();
            }
        }
        let (name, mutation, query, doing) = match choice {
            Choice::Unsubscribe => return,
            Choice::MarkDone => (
                "Mark Done",
                Mutation::MarkDone,
                format!("in:inbox {}", unsubscribe.query),
                "Marking Done",
            ),
            Choice::Trash => (
                "Trash",
                Mutation::Trash,
                unsubscribe.query.clone(),
                "Trashing",
            ),
        };
        let query = scoped(query, &self.list);
        // The loaded rows it's for, which show `Pending:` while it runs: the conversation, and
        // the sender's others it would change.
        let sender = &unsubscribe.sender;
        let threads = self
            .inbox
            .threads()
            .iter()
            .filter(|row| {
                row.id == thread
                    || row
                        .contact
                        .as_ref()
                        .is_some_and(|contact| recipients::same_address(&contact.email, sender))
            })
            .filter(|row| choice != Choice::MarkDone || !row.done)
            .map(|row| row.id.clone())
            .collect();
        // Everything there is now.
        let cutoff = ListCutoff {
            scope: Scope::Search(query.clone()),
            filter: None,
            keep: Vec::new(),
            before: self.now.timestamp(),
        };
        let target = BulkTarget {
            list: Some(cutoff),
            ..BulkTarget::default()
        };
        let token = self.account.undo_token();
        self.account.bulk(
            token,
            crate::bulk::Request {
                name,
                query: Some(query),
                mutation,
                target,
                threads,
            },
        );
        self.unsubscribing.bulk.push(token);
        self.undoable = None;
        let text = format!("{doing} all from {sender}.");
        let text = if automatic {
            format!("Unsubscribed, and {text}")
        } else {
            text
        };
        self.commands
            .toast(Toast::new(text).undo(commands::Action::Undo(token)));
    }

    /// An undo ran: one of Unsubscribe's bulk actions says what's left (`Unsubscribed.`).
    pub(crate) fn unsubscribe_undone(&mut self, token: UndoToken) {
        if let Some(at) = self
            .unsubscribing
            .bulk
            .iter()
            .position(|bulk| *bulk == token)
        {
            self.unsubscribing.bulk.remove(at);
            self.commands.toast(Toast::new("Unsubscribed."));
        }
    }

    /// Moves away from the conversation: the cursor moves one the way Auto Advance goes, or the
    /// other way from the end; from a thread, that conversation opens, or the list shows when
    /// there's none. With Auto Advance off the cursor stays and a thread goes back to the list.
    fn move_away(&mut self) {
        let toward = self.account.settings().desktop_auto_advance.direction();
        let from = self.inbox.cursor().map(|row| row.id.clone());
        if toward != 0 {
            self.inbox.move_cursor(toward);
            if self.inbox.cursor().map(|row| &row.id) == from.as_ref() {
                self.inbox.move_cursor(-toward);
            }
        }
        let to = self
            .inbox
            .cursor()
            .map(|row| row.id.clone())
            .filter(|to| Some(to) != from.as_ref());
        if self.thread.is_some() {
            match to {
                Some(next) => self.open_thread(next),
                None => self.close_thread(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unsubscribe(sender: &str) -> Unsubscribe {
        let target = Target::Web(url::Url::parse("https://shop.example/u").expect("a URL"));
        Unsubscribe {
            target,
            sender: sender.into(),
            list_name: String::new(),
            message: "m1".into(),
            query: format!("from:\"{sender}\""),
        }
    }

    #[test]
    fn the_picker_names_the_sender_and_blocks_a_domain_unless_webmail_or_yours() {
        let rows = palette(&unsubscribe("deals@Shop.example"), &[]).titles("");
        assert_eq!(
            rows,
            [
                "Unsubscribe",
                "Unsubscribe, and Mark Done all from deals@Shop.example",
                "Unsubscribe, and Trash all from deals@Shop.example",
                "Block Domain: shop.example"
            ]
        );
        assert_eq!(
            palette(&unsubscribe("someone@gmail.com"), &[])
                .titles("")
                .len(),
            3,
            "webmail"
        );
        assert_eq!(
            palette(
                &unsubscribe("news@brightmoor.co"),
                &["brightmoor.co".into()]
            )
            .titles("")
            .len(),
            3,
            "yours"
        );
    }

    #[test]
    fn bulk_queries_look_where_the_list_is() {
        assert_eq!(
            scoped("from:\"a@b.co\"".into(), &ListId::Spam),
            "from:\"a@b.co\" in:spam"
        );
        assert_eq!(
            scoped("from:\"a@b.co\"".into(), &ListId::Trash),
            "from:\"a@b.co\" in:trash"
        );
        assert_eq!(
            scoped("in:inbox from:\"a@b.co\"".into(), &ListId::Spam),
            "in:inbox from:\"a@b.co\""
        );
        assert_eq!(
            scoped("from:\"a@b.co\"".into(), &ListId::Inbox),
            "from:\"a@b.co\""
        );
    }
}
