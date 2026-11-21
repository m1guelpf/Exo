//! Copy Attachments: a message's files on the clipboard as references to its Gmail parts, with the
//! account they're in, which ⌘V in a draft adds to it. Another account's are refused, as Gmail
//! parts can only be sent from their own mailbox.

use exo_store::{Attachment, AttachmentSource, DraftAttachment};
use serde::{Deserialize, Serialize};

/// What Copy Attachments puts on the clipboard (`{attachments, account}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Copied {
    /// The address of the account the files are in.
    pub(crate) account: String,
    pub(crate) attachments: Vec<File>,
}

/// One file, as its metadata: enough to send it from its Gmail message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct File {
    pub(crate) message_id: String,
    pub(crate) attachment_id: Option<String>,
    pub(crate) part_id: String,
    pub(crate) name: String,
    pub(crate) mime_type: String,
    pub(crate) size: u64,
}

/// The alert a paste of another account's files shows.
pub(crate) const OTHER_ACCOUNT: &str = "Sorry! Attachments cannot be copy & pasted between email accounts. If you'd like to include attachments, first download them and then attach them to this draft.";

/// The files Copy Attachments copies of a message: all but its inline images.
pub(crate) fn copied(attachments: &[Attachment]) -> impl Iterator<Item = &Attachment> {
    attachments
        .iter()
        .filter(|attachment| !is_inline_image(attachment))
}

/// An inline part that's an image, or of no known type but shown by a Content-ID.
fn is_inline_image(attachment: &Attachment) -> bool {
    attachment.inline
        && (super::is_image(attachment)
            || (attachment.mime_type == "application/octet-stream"
                && attachment.content_id.is_some()))
}

impl Copied {
    /// `attachments` of `account`'s message, for the clipboard.
    pub(crate) fn new(account: &str, attachments: &[Attachment]) -> Self {
        let attachments = copied(attachments)
            .map(|attachment| File {
                message_id: attachment.message_id.clone(),
                attachment_id: attachment.attachment_id.clone(),
                part_id: attachment.part_id.clone(),
                name: attachment.filename.clone(),
                mime_type: attachment.mime_type.clone(),
                size: u64::from(attachment.size),
            })
            .collect();
        Self {
            account: account.to_owned(),
            attachments,
        }
    }

    /// The files as a draft's, by reference to their Gmail parts, each with a new id.
    pub(crate) fn into_draft_attachments(self) -> Vec<DraftAttachment> {
        self.attachments
            .into_iter()
            .map(|file| DraftAttachment {
                uuid: crate::compose::uuid_v4(),
                name: file.name,
                mime_type: file.mime_type,
                size: file.size,
                content_id: None,
                source: AttachmentSource::Gmail {
                    message_id: file.message_id,
                    attachment_id: file.attachment_id,
                    part_id: file.part_id,
                },
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(part: &str, filename: &str, mime_type: &str, inline: bool) -> Attachment {
        Attachment {
            message_id: "18f3a".into(),
            attachment_id: Some(format!("att-{part}")),
            part_id: part.into(),
            filename: filename.into(),
            mime_type: mime_type.into(),
            size: 3,
            content_id: inline.then(|| format!("cid-{part}")),
            inline,
        }
    }

    #[test]
    fn a_messages_files_are_copied_but_not_its_inline_images() {
        let attachments = [
            attachment("1", "deck.pdf", "application/pdf", false),
            attachment("2", "logo.png", "image/png", true),
            attachment("3", "photo.jpg", "image/jpeg", false),
        ];
        let copied = Copied::new("maya@brightmoor.co", &attachments);
        assert_eq!(
            copied
                .attachments
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>(),
            ["deck.pdf", "photo.jpg"]
        );
        let json = serde_json::to_string(&copied).expect("serializes");
        assert_eq!(
            serde_json::from_str::<Copied>(&json).expect("parses"),
            copied
        );
        let draft = copied.into_draft_attachments();
        assert_eq!(
            draft[0].source,
            AttachmentSource::Gmail {
                message_id: "18f3a".into(),
                attachment_id: Some("att-1".into()),
                part_id: "1".into()
            }
        );
        assert_ne!(draft[0].uuid, draft[1].uuid);
    }
}
