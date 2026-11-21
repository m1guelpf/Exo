//! What triage does to everything from a sender or domain: a Gmail filter carries the choice to
//! future mail (from them, adding Trash or a category), and the mail already there is relabelled.
//! Both are undone together.

use std::sync::LazyLock;

use exo_store::{Store, ThreadId};
use regex::Regex;

/// An address made only of characters that mean nothing to Gmail's search or filter criteria.
/// The address comes from someone else's From header, and goes into a filter and a `from:"…"`
/// search: a quote, space, brace or leading `-` in it could widen either to mail that isn't
/// theirs (`a@b.com" OR in:inbox`), so only addresses like this can be blocked or moved.
static PLAIN_ADDRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[a-z0-9_][a-z0-9._%+-]*@[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$").expect("valid regex")
});

/// Whether `email` can be blocked or moved: see [`PLAIN_ADDRESS`]. Its domain then can be too.
pub(crate) fn is_plain_address(email: &str) -> bool {
    PLAIN_ADDRESS.is_match(email)
}

/// Labels a move to Important takes off.
const NOT_IMPORTANT: [&str; 4] = [
    "CATEGORY_PROMOTIONS",
    "CATEGORY_UPDATES",
    "CATEGORY_SOCIAL",
    "CATEGORY_FORUMS",
];

/// A job on everything from `from`, an address or a domain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SenderJob {
    /// Block: their mail goes to Trash from now on, and what's there now does too, `thread`
    /// included.
    Block {
        /// The address or domain.
        from: String,
        /// The conversation it was blocked from.
        thread: String,
    },
    /// Their mail goes to Other (or Important) from now on, and their inbox conversations now,
    /// but `thread`, which moves on its own.
    Importance {
        /// The address or domain.
        from: String,
        /// Other rather than Important.
        other: bool,
        /// The conversation it was asked from.
        thread: String,
    },
}

/// What a job changed, to undo it.
#[derive(Debug)]
pub struct Done {
    /// The filter made, none when Gmail already had it.
    filter: Option<String>,
    replaced: Option<gmail::Filter>,
    /// Relabels to undo: threads, the labels added, the labels removed.
    relabels: Vec<(Vec<ThreadId>, Vec<String>, Vec<String>)>,
}

/// Runs `job`.
///
/// # Errors
/// When Gmail can't be reached or refuses the filter; what was done by then stays done.
pub async fn run(store: &Store, job: &SenderJob) -> exo_store::Result<Done> {
    let strings = |labels: &[&str]| {
        labels
            .iter()
            .map(|label| (*label).to_owned())
            .collect::<Vec<_>>()
    };
    match job {
        SenderJob::Block { from, thread } => {
            let (filter, replaced) = store.create_sender_filter(from, "TRASH").await?;
            let query = format!("from:\"{from}\"");
            let thread = ThreadId::new(thread.clone());
            let mut all = store.find_threads(&query).await?;
            let in_inbox = store.find_threads(&format!("{query} in:inbox")).await?;
            if !all.contains(&thread) {
                all.push(thread);
            }
            let (add, remove) = (strings(&["TRASH"]), strings(&["INBOX", "SPAM"]));
            let trashed = store.relabel(&all, &add, &remove).await?;
            // Trash removed Inbox from every message; putting back is per thread.
            let relabels = vec![
                (trashed, add, Vec::new()),
                (in_inbox, Vec::new(), strings(&["INBOX"])),
            ];
            Ok(Done {
                filter,
                replaced,
                relabels,
            })
        }
        SenderJob::Importance {
            from,
            other,
            thread,
        } => {
            let category = if *other {
                "CATEGORY_SOCIAL"
            } else {
                "CATEGORY_PERSONAL"
            };
            let (filter, replaced) = store.create_sender_filter(from, category).await?;
            let threads: Vec<ThreadId> = store
                .find_threads(&format!("in:inbox from:\"{from}\""))
                .await?
                .into_iter()
                .filter(|found| found.as_str() != thread)
                .collect();
            let (add, remove) = if *other {
                (strings(&[category]), strings(&["CATEGORY_PERSONAL"]))
            } else {
                (strings(&[category]), strings(&NOT_IMPORTANT))
            };
            let moved = store.relabel(&threads, &add, &remove).await?;
            Ok(Done {
                filter,
                replaced,
                relabels: vec![(moved, add, remove)],
            })
        }
    }
}

/// Undoes what `done` changed: the filter goes (the one it replaced comes back), and the
/// relabelled conversations get their labels back, in reverse.
///
/// # Errors
/// When Gmail can't be reached.
pub async fn undo(store: &Store, done: Done) -> exo_store::Result<()> {
    store
        .delete_sender_filter(done.filter.as_deref(), done.replaced.as_ref())
        .await?;
    for (threads, added, removed) in done.relabels.into_iter().rev() {
        store.relabel(&threads, &removed, &added).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_addresses_can_be_blocked() {
        for plain in [
            "ann@x.com",
            "a.b+news@mail.example.co.uk",
            "_x-1@sub-1.example.org",
        ] {
            assert!(is_plain_address(plain), "{plain}");
        }
        for hostile in [
            "a@b.com\" OR in:inbox OR \"x@y.com",
            "x@evil.com OR gmail.com",
            "-x@y.com",
            "{a b}@x.com",
            "a@b.com in:inbox",
            "a@-b.com",
            "a@b",
            "",
        ] {
            assert!(!is_plain_address(hostile), "{hostile}");
        }
    }
}
