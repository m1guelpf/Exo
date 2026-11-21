//! System labels and the rules for user labels.

use gmail::{
    LABEL_PALETTE, Label, LabelColor, LabelListVisibility, LabelType, MessageListVisibility,
};

use crate::error::{Failure, Result};

/// System labels in the order `labels.list` returns them.
const SYSTEM: &[&str] = &[
    "CHAT",
    "SENT",
    "INBOX",
    "IMPORTANT",
    "TRASH",
    "DRAFT",
    "SPAM",
    "CATEGORY_FORUMS",
    "CATEGORY_UPDATES",
    "CATEGORY_PERSONAL",
    "CATEGORY_PROMOTIONS",
    "CATEGORY_SOCIAL",
    "STARRED",
    "UNREAD",
];

/// Names Gmail reserves beyond the system label IDs (its localized display names).
const RESERVED_NAMES: &[&str] = &[
    "inbox",
    "sent",
    "drafts",
    "spam",
    "trash",
    "starred",
    "important",
    "unread",
    "chats",
    "all mail",
    "scheduled",
    "snoozed",
    "bin",
];

/// The fixed system labels.
pub fn system() -> Vec<Label> {
    SYSTEM
        .iter()
        .map(|&id| {
            // Only CHAT carries visibility settings in `labels.list`.
            let hidden = id == "CHAT";
            Label {
                id: Some(id.into()),
                name: Some(id.into()),
                message_list_visibility: hidden.then_some(MessageListVisibility::Hide),
                label_list_visibility: hidden.then_some(LabelListVisibility::LabelHide),
                kind: Some(LabelType::System),
                ..Label::default()
            }
        })
        .collect()
}

/// Labels a client may not add to or remove from messages.
pub fn is_immutable(id: &str) -> bool {
    matches!(id, "DRAFT" | "SENT" | "CHAT")
}

/// Checks a user label's name against the reserved ones.
pub fn validate_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    if trimmed.is_empty()
        || SYSTEM.iter().any(|id| id.eq_ignore_ascii_case(trimmed))
        || RESERVED_NAMES.contains(&lower.as_str())
    {
        return Err(Failure::invalid_argument("Invalid label name"));
    }
    Ok(())
}

/// Both colors must be set and come from Gmail's palette.
pub fn validate_color(color: &LabelColor) -> Result<()> {
    for value in [&color.text_color, &color.background_color] {
        let Some(value) = value else {
            return Err(Failure::invalid_argument(
                "Label color requires both textColor and backgroundColor",
            ));
        };
        if !LABEL_PALETTE.contains(&value.to_lowercase().as_str()) {
            return Err(Failure::invalid_argument(format!(
                "Label color {value} is not on the allowed color palette"
            )));
        }
    }
    Ok(())
}
