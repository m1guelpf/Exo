//! ⌘C in the list: the cursor's conversation copied as the thread view's ⌘C copies it, without
//! opening it. A conversation of one message copies as that message, of more as the conversation.
//! Its bodies come from the store (sync usually has them already) and are rendered off screen for
//! their new parts.

use std::sync::{
    Arc,
    mpsc::{Receiver, Sender, channel},
};

use exo_commands::Hint;
use exo_store::{MessageDetail, ThreadDetail, ThreadId};

use crate::{Mail, body::Facts, clipboard, compose};

type Read = (String, Option<Arc<ThreadDetail>>);

/// The copy under way.
pub(crate) struct Listed {
    /// The conversation being copied, and its messages once the store has read them.
    target: Option<(String, Option<Arc<ThreadDetail>>)>,
    read: (Sender<Read>, Receiver<Read>),
}

impl Default for Listed {
    fn default() -> Self {
        Self {
            target: None,
            read: channel(),
        }
    }
}

/// The hint after copying a message, with `more` messages to copy or without.
pub(crate) fn message_hint(more: bool) -> Hint {
    let hint = Hint::new()
        .text("Message copied to clipboard.")
        .text("Hit")
        .keys(["⌘", "V"])
        .text("to use as new draft.");
    if more {
        hint.text("Hit")
            .keys(["⌘", "C"])
            .text("again to copy conversation.")
    } else {
        hint
    }
}

/// The hint after copying a conversation.
pub(crate) fn conversation_hint() -> Hint {
    Hint::new().text("Conversation copied to clipboard.")
}

impl Mail {
    /// ⌘C in the list: starts copying the cursor's conversation, unless several are selected or
    /// it's a new message being written.
    pub(crate) fn copy_listed(&mut self) {
        if self.inbox.is_selecting() {
            return;
        }
        let Some(id) = self
            .inbox
            .cursor()
            .filter(|thread| !thread.draft_only)
            .map(|thread| thread.id.clone())
        else {
            return;
        };
        self.copy_listed.target = Some((id.clone(), None));
        let (sender, ctx) = (self.copy_listed.read.0.clone(), self.ctx.clone());
        self.account.loader().run(
            move |store| async move {
                let thread = ThreadId::new(id.clone());
                if let Err(error) = store.load_bodies(&thread).await {
                    log::info!("fetching the bodies of {id} to copy it: {error}");
                }
                let detail = store
                    .thread(&thread)
                    .await
                    .inspect_err(|error| log::error!("reading thread {id}: {error}"))
                    .ok()
                    .flatten();
                (id, detail.map(Arc::new))
            },
            move |read| {
                let _ = sender.send(read);
                ctx.request_repaint();
            },
        );
    }

    /// Takes in the conversation the store read, and has its messages rendered. Call every
    /// frame.
    pub(crate) fn copy_listed_frame(&mut self) {
        while let Ok((id, detail)) = self.copy_listed.read.1.try_recv() {
            let Some((target, waiting)) = self
                .copy_listed
                .target
                .as_mut()
                .filter(|(target, _)| *target == id)
            else {
                continue;
            };
            let Some(detail) = detail else {
                self.copy_listed.target = None;
                continue;
            };
            let messages: Vec<&MessageDetail> = detail
                .messages
                .iter()
                .filter(|message| !message.is_draft())
                .collect();
            if messages.is_empty() {
                self.copy_listed.target = None;
                continue;
            }
            self.bodies.render(target, &messages, self.account.email());
            *waiting = Some(detail);
        }
    }

    /// Conversation `id`'s messages, rendered: if it's the one being copied, onto the clipboard
    /// it goes.
    pub(crate) fn copy_listed_rendered(&mut self, id: &str, facts: &[(String, Facts)]) {
        let Some((_, Some(detail))) = self
            .copy_listed
            .target
            .take_if(|(target, detail)| target == id && detail.is_some())
        else {
            return;
        };
        let messages: Vec<(&MessageDetail, &str)> = detail
            .messages
            .iter()
            .filter(|message| !message.is_draft())
            .filter_map(|message| {
                Some((
                    message,
                    facts
                        .iter()
                        .find(|(rendered, _)| *rendered == message.id)?
                        .1
                        .unquoted_html
                        .as_str(),
                ))
            })
            .collect();
        match messages[..] {
            [] => {}
            [(message, html)] => {
                let contents = super::message(message, html, &self.now);
                clipboard::write(&self.ctx, &contents.html, &contents.text);
                self.commands.hint(message_hint(false));
                self.copied_message =
                    Some((contents.html, compose::Fields::of_message(message, html)));
            }
            _ => {
                let contents = super::conversation(&messages, &self.now);
                clipboard::write(&self.ctx, &contents.html, &contents.text);
                self.commands.hint(conversation_hint());
            }
        }
    }
}
