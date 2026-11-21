//! The thread list's right-click menu (`Thread actions`). It's a native menu
//! ([`crate::desktop::context_menu`]); its items run the commands their keys run.

use exo_store::{DraftAction, ListId, Mutation};

use crate::{
    commands::Action,
    compose,
    desktop::context_menu::{MenuItem, MenuKey},
    mailbox::ThreadSummary,
    pickers, remind,
};

/// One entry of the menu: what it shows, and what choosing it runs.
pub(crate) struct Entry {
    /// The native menu's item.
    pub item: MenuItem,
    /// What it runs; none for a separator.
    pub action: Option<Action>,
}

/// The menu for `threads`, the conversations it acts on: the selection when the row clicked is
/// in it, else that row. `exact` is whether the selection is exactly the rows loaded; `list` the
/// list on screen.
pub(crate) fn entries(threads: &[&ThreadSummary], exact: bool, list: &ListId) -> Vec<Entry> {
    let every = |of: &[&&ThreadSummary], test: fn(&ThreadSummary) -> bool| {
        !of.is_empty() && of.iter().all(|thread| test(thread))
    };
    // Drafts of their own can't be marked done, starred or read (Exo syncs no labels on drafts).
    let messages: Vec<&&ThreadSummary> =
        threads.iter().filter(|thread| !thread.draft_only).collect();
    let done = every(&messages, |thread| thread.done);
    let starred = every(&messages, |thread| thread.starred);
    let unread = every(&messages, |thread| {
        thread.unread || thread.reminder_unread()
    });
    let trashed = every(&messages, |thread| thread.has_label("TRASH"));
    let snoozable = !threads.is_empty() && threads.iter().all(|thread| !thread.draft_only);
    // A draft can be started on one conversation with messages.
    let draftable = matches!(threads, [thread] if !thread.draft_only);
    let apply = |mutation| Some(Action::Apply(mutation));
    let item = |title: &str,
                key: char,
                shift: bool,
                symbol: &'static str,
                enabled: bool,
                action: Option<Action>| Entry {
        item: MenuItem::Item {
            title: title.to_owned(),
            key: Some(MenuKey {
                key,
                shift,
                command: false,
            }),
            symbol: Some(symbol),
            enabled,
        },
        action,
    };
    let separator = || Entry {
        item: MenuItem::Separator,
        action: None,
    };
    let reply = |kind| Some(Action::Compose(compose::Action::Reply(kind)));
    vec![
        if done {
            item(
                "Mark not done",
                'e',
                true,
                "tray.and.arrow.up",
                *list != ListId::Spam && !messages.is_empty(),
                apply(Mutation::MarkNotDone),
            )
        } else {
            item(
                "Mark done",
                'e',
                false,
                "checkmark",
                *list != ListId::Spam && !messages.is_empty(),
                apply(Mutation::MarkDone),
            )
        },
        item(
            "Remind me",
            'h',
            false,
            "clock",
            exact && snoozable,
            Some(Action::Remind(remind::Action::Open { button: false })),
        ),
        if starred {
            item(
                "Unstar",
                's',
                false,
                "star.slash",
                true,
                apply(Mutation::Unstar),
            )
        } else {
            item(
                "Star",
                's',
                false,
                "star",
                !messages.is_empty(),
                apply(Mutation::Star),
            )
        },
        item(
            "Label",
            'l',
            false,
            "tag",
            exact && !messages.is_empty(),
            Some(Action::Pick(pickers::Action::OpenLabel)),
        ),
        separator(),
        item(
            "Reply",
            'r',
            false,
            "arrowshape.turn.up.left",
            draftable,
            reply(DraftAction::Reply),
        ),
        item(
            "Reply all",
            'a',
            false,
            "arrowshape.turn.up.left.2",
            draftable,
            reply(DraftAction::ReplyAll),
        ),
        item(
            "Forward",
            'f',
            false,
            "arrowshape.turn.up.right",
            draftable,
            reply(DraftAction::Forward),
        ),
        separator(),
        if unread {
            item(
                "Mark read",
                'u',
                false,
                "envelope.open",
                true,
                apply(Mutation::MarkRead),
            )
        } else {
            item(
                "Mark unread",
                'u',
                false,
                "envelope.badge",
                !messages.is_empty(),
                apply(Mutation::MarkUnread),
            )
        },
        if trashed {
            item(
                "Remove from trash",
                '#',
                false,
                "trash.slash",
                true,
                apply(Mutation::Untrash),
            )
        } else {
            item(
                "Trash",
                '#',
                false,
                "trash",
                !messages.is_empty(),
                apply(Mutation::Trash),
            )
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(id: &str) -> ThreadSummary {
        ThreadSummary {
            id: id.into(),
            participants: Vec::new(),
            drafts: 0,
            draft_only: false,
            subject: String::new(),
            snippet: String::new(),
            labels: Vec::new(),
            label_ids: vec!["INBOX".into()],
            recipients: false,
            received: jiff::Timestamp::UNIX_EPOCH,
            unread: false,
            starred: false,
            done: false,
            muted: false,
            attachment: crate::mailbox::Attachments::None,
            contact: None,
            removed_by: Vec::new(),
            messages: 1,
            search: None,
            failure: None,
            sending: false,
            scheduled: None,
            reminder: None,
            files: Vec::new(),
        }
    }

    /// Each item's title and whether it's on, separators as `—`.
    fn shown(entries: &[Entry]) -> Vec<(String, bool)> {
        entries
            .iter()
            .map(|entry| match &entry.item {
                MenuItem::Item { title, enabled, .. } => (title.clone(), *enabled),
                MenuItem::Separator => ("—".into(), true),
            })
            .collect()
    }

    #[test]
    fn one_conversation_gets_every_item_named_for_its_state() {
        let mut starred = thread("t1");
        starred.starred = true;
        starred.unread = true;
        let menu = shown(&entries(&[&starred], true, &ListId::Important));
        let titles: Vec<&str> = menu.iter().map(|(title, _)| title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Mark done",
                "Remind me",
                "Unstar",
                "Label",
                "—",
                "Reply",
                "Reply all",
                "Forward",
                "—",
                "Mark read",
                "Trash"
            ]
        );
        assert!(menu.iter().all(|(_, enabled)| *enabled));
    }

    #[test]
    fn several_conversations_cant_be_replied_to_and_spam_cant_be_marked_done() {
        let (one, two) = (thread("t1"), thread("t2"));
        let menu = shown(&entries(&[&one, &two], true, &ListId::Spam));
        let off: Vec<&str> = menu
            .iter()
            .filter(|(_, enabled)| !enabled)
            .map(|(title, _)| title.as_str())
            .collect();
        assert_eq!(off, ["Mark done", "Reply", "Reply all", "Forward"]);
        let not_exact = shown(&entries(&[&one, &two], false, &ListId::Important));
        let off: Vec<&str> = not_exact
            .iter()
            .filter(|(_, enabled)| !enabled)
            .map(|(title, _)| title.as_str())
            .collect();
        assert_eq!(
            off,
            ["Remind me", "Label", "Reply", "Reply all", "Forward"],
            "a select-all past the loaded rows"
        );
    }

    #[test]
    fn a_draft_of_its_own_has_nothing_to_do() {
        let mut draft = thread("d1");
        draft.draft_only = true;
        let menu = shown(&entries(&[&draft], true, &ListId::Drafts));
        assert!(
            menu.iter()
                .filter(|(title, _)| title != "—")
                .all(|(_, enabled)| !enabled),
            "{menu:?}"
        );
    }
}
