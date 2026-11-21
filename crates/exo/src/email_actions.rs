//! A conversation's one-click actions: what its messages declare (schema.org) or link to that Exo
//! recognizes (an issue, an order, a Google file), as exo-mail finds them. The header's status line
//! and the palette offer the last one; ⌘O lists them all above the links.

use exo_mail::Action;
use exo_store::ThreadDetail;

/// Senders, and senders' domains, whose actions win over a List-Unsubscribe header.
const SENDERS: [&str; 3] = [
    "hmstore@emm.hermanmiller.com",
    "customerservice@hmstore.hermanmiller.com",
    "gitlab@grammarly.io",
];
const DOMAINS: [&str; 4] = [
    "mail.linear.app",
    "github.com",
    "email-marriott.com",
    "info.ms.aa.com",
];

/// Whether the conversation's actions show: it has no List-Unsubscribe header, or its sender is one
/// of the few whose actions matter more.
pub(crate) fn have_priority(detail: &ThreadDetail, own_domains: &[String]) -> bool {
    if crate::unsubscribe::find(detail, own_domains, |_| None).is_none() {
        return true;
    }
    // The latest sender who isn't me.
    let sender = detail
        .messages
        .iter()
        .rev()
        .filter(|message| !message.is_draft())
        .find_map(|message| message.from.as_ref().filter(|from| !from.is_me));
    sender.is_some_and(|sender| {
        let email = sender.email.to_lowercase();
        SENDERS.contains(&email.as_str())
            || email
                .rsplit_once('@')
                .is_some_and(|(_, domain)| DOMAINS.contains(&domain))
    })
}

/// The actions of messages `ids` of `detail`, in order and each once (the same text and URL),
/// `actions` giving a message's; none without priority.
pub(crate) fn of(
    detail: &ThreadDetail,
    ids: &[&str],
    own_domains: &[String],
    actions: impl Fn(&str) -> Option<Vec<Action>>,
) -> Vec<Action> {
    if !have_priority(detail, own_domains) {
        return Vec::new();
    }
    let mut found: Vec<Action> = Vec::new();
    for action in ids.iter().filter_map(|id| actions(id)).flatten() {
        if !found
            .iter()
            .any(|seen| seen.text == action.text && seen.url == action.url)
        {
            found.push(action);
        }
    }
    found
}
