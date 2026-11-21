//! The public API's shape: everything the UI calls can run on a spawned task.

mod support;

use exo_store::{ListId, Mutation, Store, ThreadId};

fn assert_send<T: Send>(_: T) {}

#[test]
fn futures_are_send() {
    fn check(store: &Store, undo: exo_store::Undo) {
        let id = ThreadId::new("t");
        assert_send(store.sync());
        assert_send(store.backfill(&ListId::Inbox, 50));
        assert_send(store.list(&ListId::Inbox, None, 50));
        assert_send(store.count(&ListId::Inbox));
        assert_send(store.thread(&id));
        assert_send(store.labels());
        assert_send(store.apply(std::slice::from_ref(&id), Mutation::MarkDone));
        assert_send(store.undo(undo));
        assert_send(store.flush());
        assert_send(store.pending());
        assert_send(store.part_data("m", None, "1"));
    }
    let _ = check;
}

#[test]
fn store_is_shareable() {
    fn shareable<T: Send + Sync + Clone + 'static>() {}
    shareable::<Store>();
}

#[tokio::test]
async fn account_names_must_be_file_names() {
    let dir = support::TempDir::new();
    for account in ["", "../escape", "a/b", ".hidden"] {
        let opened = Store::open(
            dir.path(),
            account,
            gmail::Endpoints::google(),
            gmail::StaticToken(String::new()),
        )
        .await;
        assert!(
            matches!(opened, Err(exo_store::Error::InvalidAccount(_))),
            "{account:?}"
        );
    }
}
