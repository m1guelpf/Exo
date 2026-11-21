//! What the open conversation prints ([`crate::print`]): each message as the view shows it now,
//! with the same participants, dates, snippets and statuses, and the markers between them.

use egui::{Color32, Context, LayerId};
use exo_store::{Contact, MessageDetail, ThreadDetail};

use super::{
    Entry, Item, ThreadView, attachments, markers,
    message::{self, DATE, FROM, FROM_MIN_WIDTH, FROM_PADDING, SNIPPET, STATUS, State, Status},
};
use crate::{
    body::Bodies,
    format::dates,
    print::{Conversation, File, Header, Message, Page, Row, When},
};

/// The 19px paperclip and its 8px margin.
const PAPERCLIP: f32 = 19.0 + 8.0;

impl ThreadView {
    /// The conversation as it prints on `page`, titled `title`; `None` before it has loaded.
    /// `ctx` measures the collapsed rows' text, to cut their snippets where the screen would at
    /// the page's width.
    pub(crate) fn printout(
        &self,
        ctx: &Context,
        bodies: &Bodies,
        title: String,
        page: Page,
    ) -> Option<Conversation> {
        let detail = self.detail.as_deref()?;
        let messages = self
            .order(detail)
            .into_iter()
            .filter_map(|entry| {
                let at = match entry {
                    Entry::Message(at) => at,
                    Entry::Trashed(_) => {
                        return Some(Message::Marker {
                            text: markers::TRASHED.to_owned(),
                            link: Some(markers::SHOW.to_owned()),
                            when: None,
                        });
                    }
                    Entry::Reminder => {
                        let Some(exo_store::ReminderStatus::Returned { at, .. }) = detail.reminder
                        else {
                            return None;
                        };
                        return Some(Message::Marker {
                            text: markers::REMINDER.to_owned(),
                            link: None,
                            when: Some(dates::row(at, &self.now)),
                        });
                    }
                };
                let item = &self.items[at];
                let message = detail
                    .messages
                    .iter()
                    .find(|message| message.id == item.id)?;
                let state = State {
                    focused: false,
                    show_focus: false,
                    header_expanded: item.header_expanded,
                    collapsible: false,
                    composing: false,
                    offline: !self.online,
                    full_names: self.full_names,
                    reply_labels: false,
                };
                Some(if item.expanded {
                    self.print_expanded(item, message, detail, &state, bodies)
                } else {
                    self.print_collapsed(ctx, item, message, detail, &state, bodies, page)
                })
            })
            .collect();
        let inline = detail
            .messages
            .iter()
            .flat_map(|message| {
                message
                    .attachments
                    .iter()
                    .filter(|attachment| attachment.content_id.is_some())
                    .cloned()
            })
            .collect();
        Some(Conversation {
            title,
            subject: detail.subject.clone(),
            labels: self.chips.clone(),
            scrolled: self.scroll > 0.0,
            messages,
            inline,
            page,
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one row, from the thread's state"
    )]
    fn print_collapsed(
        &self,
        ctx: &Context,
        item: &Item,
        message: &MessageDetail,
        detail: &ThreadDetail,
        state: &State,
        bodies: &Bodies,
        page: Page,
    ) -> Message {
        let painter = ctx.layer_painter(LayerId::background());
        let width = |style: &exo_ui::TextStyle, text: &str| {
            style
                .single_line(&painter, text, Color32::PLACEHOLDER, None)
                .size()
                .x
        };
        let from = message::participants(message, detail, false, self.full_names);
        let (when, date_width) = match Status::of(message, state) {
            Some(status) => {
                let text = status.collapsed_title(&self.now);
                let measured = width(&STATUS, &text);
                (
                    When::Status {
                        text,
                        color: status.ink(),
                    },
                    measured,
                )
            }
            None => {
                let date = dates::row(message.date, &self.now);
                let measured = width(&DATE, &date) + DATE.letter_spacing;
                (When::Date(date), measured)
            }
        };
        let files = attachments::listed(message).next().is_some();
        let from_box = (width(&FROM, &from) + FROM_PADDING).max(FROM_MIN_WIDTH);
        let date_box = date_width + if files { PAPERCLIP } else { 0.0 };
        // The snippet: `flex: 1 1 0; margin-right: .5rem`.
        let available = (page.message_width() - from_box - date_box - 0.5 * page.rem()).max(0.0);
        let snippet = message::snippet(bodies.facts(&item.id), available, |text| {
            width(&SNIPPET, text) > available
        });
        Message::Collapsed {
            from,
            snippet,
            files,
            when,
        }
    }

    fn print_expanded(
        &self,
        item: &Item,
        message: &MessageDetail,
        detail: &ThreadDetail,
        state: &State,
        bodies: &Bodies,
    ) -> Message {
        let status = Status::of(message, state);
        let header = if item.header_expanded {
            let rows = [
                ("From", message.from.as_slice()),
                ("To", &message.to),
                ("Cc", &message.cc),
                ("Bcc", &message.bcc),
            ]
            .into_iter()
            .filter(|(_, contacts)| !contacts.is_empty())
            .map(|(label, contacts)| Row {
                label,
                contacts: contacts.iter().map(named).collect(),
            })
            .collect();
            let when = match status {
                Some(status) => When::Status {
                    text: status.full_title(&self.now),
                    color: status.ink(),
                },
                None => When::Date(dates::expanded(message.date, &self.now)),
            };
            Header::Full {
                rows,
                when,
                chevron: !matches!(status, Some(Status::Failed(_))),
                tracking_blocked: bodies.tracking_blocked(&item.id),
            }
        } else {
            let when = match status {
                Some(status) => When::Status {
                    text: status.title(&self.now),
                    color: status.ink(),
                },
                None => When::Date(dates::row(message.date, &self.now)),
            };
            Header::Compact {
                participants: message::participants(message, detail, true, self.full_names),
                when,
            }
        };
        let files = attachments::listed(message)
            .map(|attachment| {
                let (name, kind) = attachment
                    .filename
                    .rsplit_once('.')
                    .map_or((attachment.filename.as_str(), ""), |(stem, extension)| {
                        (stem, extension)
                    });
                File {
                    name: name.to_owned(),
                    kind: kind.to_uppercase(),
                    color: exo_ui::attachment_badge_color(
                        &attachment.filename,
                        &attachment.mime_type,
                    ),
                }
            })
            .collect();
        Message::Expanded {
            header,
            body: bodies.printable(&item.id, item.show_quoted),
            files,
        }
    }
}

/// A contact as the full header names it: its name, if it has one, and its address.
fn named(contact: &Contact) -> (Option<String>, String) {
    (
        contact.name.clone().filter(|name| !name.is_empty()),
        contact.email.clone(),
    )
}
