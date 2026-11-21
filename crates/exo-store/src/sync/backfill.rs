//! Backfill: walking a list's `threads.list` pages back in time, fetching only the threads
//! that are missing or stale (for their rows: bodies come after, see `bodies`), and recording
//! how far each list is covered.

use std::collections::{HashMap, HashSet};

use futures::{StreamExt, stream};
use gmail::ListQuery;
use jiff::Timestamp;

use super::{ATTEMPTS, Rows, fetch_rows};
use crate::db::{ListCursor, MessageRow, ThreadRow};
use crate::error::{Error, Result};
use crate::ids::ListId;
use crate::provider;
use crate::quota::Lane;
use crate::split::{Me, gmail_search};
use crate::store::{Inner, Store, load_views, reindex};
use crate::view::Router;

/// Threads per `threads.list` page: a page holds the sync lock, which the next history check
/// waits for, and ten threads' rows are a few seconds of the budget's refill.
const PAGE_SIZE: u32 = 10;
/// `threads.get` calls in flight at once.
const FETCH_CONCURRENCY: usize = 5;
/// How many of a contact's conversations Gmail is asked for.
pub const RECENT_THREADS: u32 = 5;
/// How far back the All Mail pass goes.
const WINDOW_MS: i64 = 31 * 24 * 60 * 60 * 1000;
/// Inbox, Important and Other stop at this many threads.
pub(crate) const LIST_CAP: u32 = 1250;
/// The list on screen is backfilled ahead of everything but the user's own calls
/// ([`Lane::Show`]) until it has a screenful.
const FIRST_SCREEN: u32 = 25;
/// The list on screen is backfilled this far (the app's first read) before a round moves on.
pub(crate) const ON_SCREEN_AHEAD: u32 = 100;

/// Where a backfill pass stops, if not when Gmail runs out of pages.
#[derive(Clone, Copy)]
pub(super) enum Limit {
    /// Only when Gmail runs out.
    None,
    /// Once the pass reaches threads older than this many milliseconds.
    Window(i64),
    /// After this many threads.
    Cap(u32),
}

/// The passes `sync` advances one page at a time, before one per enabled split.
const PLAN: [(ListId, Limit); 4] = [
    (ListId::All, Limit::Window(WINDOW_MS)),
    (ListId::Inbox, Limit::Cap(LIST_CAP)),
    (ListId::Important, Limit::Cap(LIST_CAP)),
    (ListId::Other, Limit::Cap(LIST_CAP)),
];

impl Inner {
    /// One page of every background pass (Inbox, Important, Other and every split), then a page of
    /// the bodies the app shows first (`bodies`). Returns whether any has more to do.
    pub(super) async fn backfill_round(&self) -> Result<bool> {
        let splits = self.router().splits;
        let on_screen = self
            .on_screen
            .read()
            .expect("on-screen lock is never poisoned")
            .clone();
        let mut more = false;
        for (list, limit) in passes(&splits.routed_ids(), on_screen.as_ref()) {
            let shown = on_screen.as_ref() == Some(&list);
            let show = if shown { FIRST_SCREEN } else { 0 };
            loop {
                let page = {
                    let _sync = self.lock_sync().await?;
                    self.backfill_page(&list, limit, show).await?
                };
                if !(shown && page.more && page.seen < ON_SCREEN_AHEAD) {
                    more |= page.more;
                    break;
                }
            }
        }
        let bodies = {
            let _sync = self.lock_sync().await?;
            self.body_page().await?
        };
        Ok(more || bodies)
    }

    /// Backfills the next page of `list`, in [`Lane::Show`] while the pass has listed fewer
    /// than `show` threads and in [`Lane::Sync`] after. The caller holds the sync lock.
    pub(super) async fn backfill_page(
        &self,
        list: &ListId,
        limit: Limit,
        show: u32,
    ) -> Result<Page> {
        let mut db = self.db.clone();
        let mut cursor = ListCursor::filter_by_id(list.key())
            .first()
            .exec(&mut db)
            .await?;
        // Scrolled past what All Mail covers, a list without a pass of its own starts where that
        // ends rather than listing it all again.
        if cursor.is_none() && matches!(limit, Limit::None) && *list != ListId::All {
            cursor = ListCursor::filter_by_id(ListId::All.key())
                .first()
                .exec(&mut db)
                .await?
                .filter(|all| !all.complete)
                .and_then(|all| {
                    let reached = all.min_sort?;
                    Some(ListCursor {
                        id: list.key().to_owned(),
                        page_token: None,
                        min_sort: Some(reached),
                        complete: false,
                        seen: 0,
                        before: Some(reached),
                    })
                });
        }
        if cursor
            .as_ref()
            .is_some_and(|cursor| finished(cursor, limit))
        {
            return Ok(Page {
                listed: 0,
                seen: cursor.map_or(0, |cursor| cursor.seen),
                more: false,
            });
        }
        let router = self.router();
        let lane = if cursor.as_ref().map_or(0, |cursor| cursor.seen) < show {
            Lane::Show
        } else {
            Lane::Sync
        };
        let mut query = ListQuery {
            page_token: cursor.as_ref().and_then(|cursor| cursor.page_token.clone()),
            max_results: Some(PAGE_SIZE),
            ..selection(list, &router)?
        };
        let before = cursor.as_ref().and_then(|cursor| cursor.before);
        if let Some(before) = before {
            query.q = Some(listed_before(query.q.as_deref(), before));
        }
        let page = self
            .api
            .call(lane, ATTEMPTS, |gmail| gmail.threads(&query))
            .await?;
        let listed: Vec<(String, u64)> = page
            .threads
            .iter()
            .filter_map(|thread| Some((thread.id.clone()?, thread.history_id.unwrap_or_default())))
            .collect();
        let ids: Vec<String> = listed.iter().map(|(id, _)| id.clone()).collect();
        let fetched = self.fetch_stale(lane, &listed).await?;

        let next = page.next_page_token;
        let mut write = self.begin(&mut db).await?;
        for rows in &fetched {
            if let Some(id) = provider::save_rows(write.exec(), rows).await? {
                reindex(write.exec(), &id, &router).await?;
            }
        }
        // Everything Gmail lists after the oldest thread of this page is still unseen.
        let views = load_views(write.exec(), &ids).await?;
        let min_sort = views
            .values()
            .map(|view| view.listed_at())
            .chain(cursor.as_ref().and_then(|cursor| cursor.min_sort))
            .min();
        let seen = cursor.as_ref().map_or(0, |cursor| cursor.seen)
            + u32::try_from(ids.len()).unwrap_or(u32::MAX);
        let updated = ListCursor::upsert_by_id(list.key())
            .page_token(next.clone())
            .min_sort(min_sort)
            .complete(next.is_none())
            .seen(seen)
            .before(before)
            .exec(write.exec())
            .await?;
        write.publish().await?;
        Ok(Page {
            listed: ids.len(),
            seen: updated.seen,
            more: !finished(&updated, limit),
        })
    }
}

/// What one [`Inner::backfill_page`] did.
pub(super) struct Page {
    /// Threads Gmail listed on the page.
    pub listed: usize,
    /// Threads the pass has listed so far.
    pub seen: u32,
    /// The pass has more to do.
    pub more: bool,
}

impl Inner {
    /// The rows ([`fetch_rows`]) for `lane` of the `(id, historyId)` pairs Gmail listed that are
    /// missing locally or changed since they were stored.
    pub(crate) async fn fetch_stale(
        &self,
        lane: Lane,
        listed: &[(String, u64)],
    ) -> Result<Vec<Rows>> {
        let mut db = self.db.clone();
        let ids: Vec<&str> = listed.iter().map(|(id, _)| id.as_str()).collect();
        let stored: HashMap<String, u64> =
            ThreadRow::filter(ThreadRow::fields().id().in_list(ids.as_slice()))
                .exec(&mut db)
                .await?
                .into_iter()
                .map(|thread| (thread.id, thread.history_id))
                .collect();
        let stale: Vec<String> = listed
            .iter()
            .filter(|(id, history_id)| stored.get(id).is_none_or(|stored| history_id > stored))
            .map(|(id, _)| id.clone())
            .collect();
        // What a changed thread already has is never fetched in full again.
        let mut messages: HashMap<String, HashSet<String>> = HashMap::new();
        for row in MessageRow::filter(MessageRow::fields().thread_id().in_list(stale.as_slice()))
            .exec(&mut db)
            .await?
        {
            messages.entry(row.thread_id).or_default().insert(row.id);
        }
        let api = &self.api;
        let fetched = stream::iter(stale)
            .map(|id| {
                let stored = messages.remove(&id).unwrap_or_default();
                async move { fetch_rows(api, lane, ATTEMPTS, &id, &stored).await }
            })
            .buffer_unordered(FETCH_CONCURRENCY)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        Ok(fetched.into_iter().flatten().collect())
    }
}

impl Store {
    /// Brings `email`'s latest conversations into the store from Gmail, for when local mail
    /// doesn't reach back far enough: the newest [`RECENT_THREADS`] threads matching
    /// `(from:… OR to:… OR cc:… OR bcc:…) AND -in:DRAFT`, fetched unless they're stored and
    /// current, through the store's retry policy. [`recent_threads`](Self::recent_threads) then
    /// lists them.
    ///
    /// # Errors
    /// As [`sync`](Self::sync).
    pub async fn fetch_recent_threads(&self, email: &str) -> Result<()> {
        let inner = &self.inner;
        // Saving a thread needs the account's identity; it doesn't need the sync to wait.
        inner.bootstrapped().await?;
        let query = ListQuery {
            q: Some(format!(
                "(from:{email} OR to:{email} OR cc:{email} OR bcc:{email}) AND -in:DRAFT"
            )),
            max_results: Some(RECENT_THREADS),
            ..ListQuery::default()
        };
        let page = inner
            .api
            .call(Lane::Show, ATTEMPTS, |gmail| gmail.threads(&query))
            .await?;
        let listed: Vec<(String, u64)> = page
            .threads
            .iter()
            .filter_map(|thread| Some((thread.id.clone()?, thread.history_id.unwrap_or_default())))
            .collect();
        let fetched = inner.fetch_stale(Lane::Show, &listed).await?;
        if fetched.is_empty() {
            return Ok(());
        }
        let router = inner.router();
        let mut db = inner.db.clone();
        let mut write = inner.begin(&mut db).await?;
        for rows in &fetched {
            if let Some(id) = provider::save_rows(write.exec(), rows).await? {
                reindex(write.exec(), &id, &router).await?;
            }
        }
        write.publish().await
    }
}

/// A round's passes: the [`PLAN`], then one per routed split, with the list on screen moved to
/// the front.
fn passes(splits: &[&str], on_screen: Option<&ListId>) -> Vec<(ListId, Limit)> {
    let mut passes: Vec<(ListId, Limit)> = PLAN
        .into_iter()
        .chain(
            splits
                .iter()
                .map(|id| (ListId::Split((*id).to_owned()), Limit::Cap(LIST_CAP))),
        )
        .collect();
    if let Some(at) = passes.iter().position(|(list, _)| Some(list) == on_screen) {
        let shown = passes.remove(at);
        passes.insert(0, shown);
    }
    passes
}

/// `q` narrowed to threads with mail before `before` (epoch milliseconds), as Gmail's
/// `before:` takes it: in seconds, rounded up so nothing before it is left out.
fn listed_before(q: Option<&str>, before: i64) -> String {
    let seconds = before.saturating_add(999).div_euclid(1000);
    match q {
        Some(q) => format!("({q}) before:{seconds}"),
        None => format!("before:{seconds}"),
    }
}

fn finished(cursor: &ListCursor, limit: Limit) -> bool {
    cursor.complete
        || match limit {
            Limit::None => false,
            Limit::Window(window) => cursor
                .min_sort
                .is_some_and(|oldest| oldest < Timestamp::now().as_millisecond() - window),
            Limit::Cap(cap) => cursor.seen >= cap,
        }
}

/// What Gmail is asked to list for `list`.
pub(crate) fn selection(list: &ListId, router: &Router) -> Result<ListQuery> {
    let me = &router.me;
    let query = |q: String| ListQuery {
        q: Some(q),
        ..ListQuery::default()
    };
    let labels = |id: &str| ListQuery {
        label_ids: vec![id.to_owned()],
        ..ListQuery::default()
    };
    Ok(match list {
        ListId::All => ListQuery {
            include_spam_trash: true,
            ..ListQuery::default()
        },
        ListId::Inbox | ListId::Important | ListId::Other | ListId::Split(_) => {
            let me = Me {
                addresses: &me.addresses,
                name: me.name.as_deref(),
            };
            query(gmail_search(list.key(), &router.splits.defined, &me))
        }
        ListId::Done => query("-in:INBOX".into()),
        ListId::Muted => labels(
            me.muted_label
                .as_deref()
                .ok_or(Error::NotSynced("the mute label"))?,
        ),
        // Every reminder is local (`db::coverage`), so this only fetches what the label has.
        ListId::Reminders => labels(
            me.snoozed_label
                .as_deref()
                .ok_or(Error::NotSynced("the reminder labels"))?,
        ),
        other => labels(other.key()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pass_past_evicted_mail_lists_what_came_before_it() {
        assert_eq!(
            listed_before(None, 1_700_000_000_001),
            "before:1700000001",
            "up to the second after"
        );
        assert_eq!(
            listed_before(Some("in:inbox OR label:vip"), 1_700_000_000_000),
            "(in:inbox OR label:vip) before:1700000000"
        );
    }

    #[test]
    fn the_list_on_screen_goes_first() {
        let split = ListId::Split("vip".into());
        let order = |on_screen: Option<&ListId>| {
            passes(&["vip"], on_screen)
                .into_iter()
                .map(|(list, _)| list)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            order(None),
            [
                ListId::All,
                ListId::Inbox,
                ListId::Important,
                ListId::Other,
                split.clone()
            ]
        );
        assert_eq!(
            order(Some(&ListId::Important)),
            [
                ListId::Important,
                ListId::All,
                ListId::Inbox,
                ListId::Other,
                split.clone()
            ]
        );
        assert_eq!(
            order(Some(&split)),
            [
                split.clone(),
                ListId::All,
                ListId::Inbox,
                ListId::Important,
                ListId::Other
            ]
        );
        assert_eq!(
            order(Some(&ListId::Starred)),
            order(None),
            "lists without a pass don't change the round"
        );
    }
}
