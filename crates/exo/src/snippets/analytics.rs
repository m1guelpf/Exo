//! The folder's numbers, counted from the messages Exo sends with a snippet: a send each, when
//! the last went, and a reply once someone else writes in that conversation after it. Opens need
//! read statuses, which Exo hasn't got.

use std::sync::mpsc::{Receiver, Sender, channel};

use exo_store::{SnippetSend, ThreadDetail, ThreadId};

/// The sends found replied to, by snippet, as the store answers.
type Found = Vec<(String, SnippetSend)>;

/// Where the store's answers to a reply check arrive.
pub(crate) struct Replies {
    found: (Sender<Found>, Receiver<Found>),
}

impl Default for Replies {
    fn default() -> Self {
        Self { found: channel() }
    }
}

/// Whether `thread` has a reply to its message `message`: a later message, not a draft, from
/// someone else.
fn replied(thread: &ThreadDetail, message: &str) -> bool {
    let Some(at) = thread.messages.iter().position(|kept| kept.id == message) else {
        return false;
    };
    thread.messages[at + 1..]
        .iter()
        .any(|later| !later.is_draft() && later.from.as_ref().is_some_and(|from| !from.is_me))
}

/// `round(part / sends * 100)%`, `0%` with no sends.
pub(super) fn percentage(part: u32, sends: u32) -> String {
    let percent = if sends == 0 {
        0.0
    } else {
        (f64::from(part) / f64::from(sends) * 100.0).round()
    };
    format!("{percent}%")
}

impl crate::Mail {
    /// A message that used the snippets `used` went out as `message` in `thread`: each counts a
    /// send, sent now, whose replies the folder looks for.
    pub(crate) fn snippets_sent(&mut self, used: &[String], thread: &ThreadId, message: &str) {
        if used.is_empty() {
            return;
        }
        let at = jiff::Timestamp::now().to_string();
        self.account.update_settings(|settings| {
            for snippet in settings
                .snippets
                .iter_mut()
                .filter(|snippet| used.contains(&snippet.id))
            {
                let analytics = &mut snippet.analytics;
                analytics.sends += 1;
                analytics.last_sent.clone_from(&at);
                analytics.unreplied.push(SnippetSend {
                    thread: thread.as_str().to_owned(),
                    message: message.to_owned(),
                });
            }
        });
    }

    /// Opening the folder: the store is asked which sends still waiting for a reply have one now.
    pub(crate) fn check_snippet_replies(&mut self) {
        let waiting: Found = self
            .account
            .settings()
            .snippets
            .iter()
            .flat_map(|snippet| {
                snippet
                    .analytics
                    .unreplied
                    .iter()
                    .map(|send| (snippet.id.clone(), send.clone()))
            })
            .collect();
        if waiting.is_empty() {
            return;
        }
        let (sender, ctx) = (self.snippets.replies.found.0.clone(), self.ctx.clone());
        self.account.loader().run(
            move |store| async move {
                let mut found = Found::new();
                let mut threads: Vec<&str> = waiting
                    .iter()
                    .map(|(_, send)| send.thread.as_str())
                    .collect();
                threads.sort_unstable();
                threads.dedup();
                for thread in threads {
                    let detail = match store.thread(&ThreadId::new(thread.to_owned())).await {
                        Ok(Some(detail)) => detail,
                        Ok(None) => continue,
                        Err(error) => {
                            log::info!("checking snippet replies in {thread}: {error}");
                            continue;
                        }
                    };
                    found.extend(
                        waiting
                            .iter()
                            .filter(|(_, send)| {
                                send.thread == thread && replied(&detail, &send.message)
                            })
                            .cloned(),
                    );
                }
                found
            },
            move |found| {
                if !found.is_empty() {
                    let _ = sender.send(found);
                    ctx.request_repaint();
                }
            },
        );
    }

    /// Counts the replies a check found: each send counts once, and is no longer looked at.
    pub(super) fn snippet_replies_arrived(&mut self) {
        while let Ok(found) = self.snippets.replies.found.1.try_recv() {
            self.account.update_settings(|settings| {
                for (id, send) in &found {
                    let Some(snippet) = settings
                        .snippets
                        .iter_mut()
                        .find(|snippet| snippet.id == *id)
                    else {
                        continue;
                    };
                    let before = snippet.analytics.unreplied.len();
                    snippet
                        .analytics
                        .unreplied
                        .retain(|waiting| waiting != send);
                    if snippet.analytics.unreplied.len() < before {
                        snippet.analytics.replies += 1;
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exo_store::{Contact, MessageDetail};

    fn message(id: &str, from_me: bool, draft: bool) -> MessageDetail {
        MessageDetail {
            id: id.into(),
            labels: if draft {
                vec!["DRAFT".into()]
            } else {
                Vec::new()
            },
            date: jiff::Timestamp::UNIX_EPOCH,
            from: Some(Contact {
                name: None,
                email: if from_me {
                    "maya@brightmoor.co"
                } else {
                    "tom@brightmoor.co"
                }
                .into(),
                is_me: from_me,
            }),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: String::new(),
            snippet: String::new(),
            rfc822_id: None,
            in_reply_to: None,
            references: Vec::new(),
            list_id: None,
            list_unsubscribe: None,
            list_unsubscribe_post: None,
            headers: Vec::new(),
            body: exo_store::Body::default(),
            attachments: Vec::new(),
            failure: None,
            scheduled: None,
            sending: false,
        }
    }

    fn thread(messages: Vec<MessageDetail>) -> ThreadDetail {
        ThreadDetail {
            id: ThreadId::new("t".to_owned()),
            subject: String::new(),
            labels: Vec::new(),
            muted: false,
            reminder: None,
            messages,
        }
    }

    #[test]
    fn a_reply_is_someone_elses_later_message() {
        let earlier = thread(vec![
            message("theirs", false, false),
            message("sent", true, false),
        ]);
        assert!(!replied(&earlier, "sent"), "what came before doesn't count");
        let mine = thread(vec![
            message("sent", true, false),
            message("follow-up", true, false),
            message("draft", false, true),
        ]);
        assert!(!replied(&mine, "sent"), "nor my own follow-up, nor a draft");
        let answered = thread(vec![
            message("sent", true, false),
            message("reply", false, false),
        ]);
        assert!(replied(&answered, "sent"));
        assert!(
            !replied(&answered, "gone"),
            "a message the store doesn't have"
        );
    }

    #[test]
    fn percentages_round_to_whole_numbers() {
        assert_eq!(percentage(3, 12), "25%");
        assert_eq!(percentage(450, 1500), "30%");
        assert_eq!(percentage(1, 3), "33%");
        assert_eq!(percentage(0, 0), "0%", "no sends yet");
    }
}
