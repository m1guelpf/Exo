//! The email a `mailto:` unsubscribe sends: a new message from the alias the list wrote to, to
//! the link's address with its subject, saying who asks, and sent at once with no signature, no
//! compose and no toast of its own. It goes through the drafts like any send, so offline it
//! waits, and it lands in Sent.

use exo_store::{Address, Draft, MessageDetail};
use jiff::Timestamp;

use super::{Fields, Open, factory, mailto, outgoing};

impl crate::Mail {
    /// Sends the unsubscribe request to `to` with `subject`, from the alias `message` (the one
    /// the unsubscribe was found in) was delivered to.
    pub(crate) fn compose_send_unsubscribe(
        &self,
        to: &str,
        subject: &str,
        message: &MessageDetail,
    ) {
        let account = self.compose_account();
        let from = account.delivered_to(message);
        let body = format!(
            "This is an unsubscribe request sent from Exo on behalf of {}",
            mailto::escape(&from.email)
        );
        let fields = Fields {
            from: Some(from),
            to: vec![Address {
                name: None,
                email: to.to_owned(),
            }],
            subject: subject.to_owned(),
            body,
            ..Fields::default()
        };
        let mut open = Open::new(
            factory::new_id(),
            None,
            factory::compose(&account, fields, Timestamp::now()),
        );
        open.sync_body();
        let email = outgoing::unsigned_message(
            &open,
            &self.snapshot.aliases,
            &Timestamp::now().to_zoned(self.now.time_zone().clone()),
        );
        let draft = Draft {
            id: open.id.clone(),
            thread: None,
            adopted: None,
            record: open.record,
            raw: String::new(),
            complete: false,
            sending: true,
        };
        self.account.save_draft(draft, email);
        self.account.send_draft(&open.id, Timestamp::now());
    }
}
