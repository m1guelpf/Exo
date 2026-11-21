//! A message's attachment cards: four to a row, each with the file's name, a colored type badge
//! and, on hover, a download icon; an image as its thumbnail; a file being downloaded with its
//! progress bar; and `Download all` under several.

use std::collections::{HashMap, HashSet};

use egui::{
    Color32, ColorImage, CornerRadius, Rect, Sense, TextureHandle, TextureOptions, Ui, Vec2, pos2,
    vec2,
};
use exo_store::{Attachment, MessageDetail};
use exo_ui::{CARD_MARGIN_BOTTOM, CARD_MARGIN_X, Family, Font, TextStyle};

use crate::attachment_files::Key;

/// The card list's margin: `16px 0 0 -8px`.
const LIST_MARGIN_TOP: f32 = 16.0;
const LIST_MARGIN_LEFT: f32 = -8.0;
/// The list's wrapper ends a pixel lower when it holds attachments (measured).
const LIST_EXTRA: f32 = 1.0;
/// `Download all`'s row under the cards, around its box (4px over and, in Classic, 8px under a
/// 16px line) in a line box 2px taller.
const DOWNLOAD_LINE: f32 = 30.0;
/// `Download all`: 12px/16px, `top: 1px`, `margin-left: 3px`.
const DOWNLOAD_ALL: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// `Download all`'s icon: Classic's 13px glyph, `margin-right: 10px`, `top: 3px`.
const DOWNLOAD_ALL_ICON: f32 = 13.0;
/// A card's download icon: Classic's 13px.
const DOWNLOAD_ICON: f32 = 13.0;
/// A thumbnail shows only for an image under 10,000px each way.
pub(crate) const MAX_THUMBNAIL_SIDE: u32 = 10_000;

/// What a click on the attachments asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Click {
    /// A card: preview it (with the others the list can preview), or only download it when ⌘
    /// or Ctrl was held or the download icon was hit.
    Open {
        attachment: Attachment,
        preview: bool,
    },
    /// `Download all`: message `message`'s files as one zip.
    DownloadAll { message: String },
    /// A card dragged out, from this point.
    Drag {
        attachment: Attachment,
        at: egui::Pos2,
    },
}

/// What the cards show besides the message: how far a clicked file has downloaded, the images'
/// thumbnails, and the messages whose files are being zipped.
#[derive(Default)]
pub(crate) struct Files {
    /// Clicked files being fetched, by message and part: the percentage arrived.
    progress: HashMap<Key, f32>,
    /// Image attachments' thumbnails, by message and part; `None` once asked for, until it
    /// comes, or for good when there's none to show.
    thumbnails: HashMap<Key, Option<TextureHandle>>,
    /// Messages whose `Download all` is zipping.
    zipping: HashSet<String>,
}

impl Files {
    /// Clicked files being fetched now, and how far each has come.
    pub(crate) fn set_progress(&mut self, progress: HashMap<Key, f32>) {
        self.progress = progress;
    }

    /// The image attachments of `messages` whose thumbnails haven't been asked for yet; each is
    /// asked for once.
    pub(crate) fn thumbnail_requests<'a>(
        &mut self,
        messages: impl IntoIterator<Item = &'a MessageDetail>,
    ) -> Vec<Attachment> {
        let mut requests = Vec::new();
        for attachment in messages.into_iter().flat_map(listed) {
            let key = key(attachment);
            if crate::attachment_files::is_image(attachment) && !self.thumbnails.contains_key(&key)
            {
                self.thumbnails.insert(key, None);
                requests.push(attachment.clone());
            }
        }
        requests
    }

    /// The thumbnail of the attachment under `key`, cropped to a card already, or `None` when it
    /// has none.
    pub(crate) fn set_thumbnail(
        &mut self,
        ctx: &egui::Context,
        key: Key,
        image: Option<ColorImage>,
    ) {
        let texture = image.map(|image| {
            ctx.load_texture(
                format!("exo-thumbnail-{}-{}", key.0, key.1),
                image,
                TextureOptions::LINEAR,
            )
        });
        self.thumbnails.insert(key, texture);
    }

    /// The messages whose `Download all` is zipping.
    pub(crate) fn set_zipping(&mut self, zipping: &HashSet<String>) {
        self.zipping.clone_from(zipping);
    }

    fn thumbnail(&self, attachment: &Attachment) -> Option<&TextureHandle> {
        self.thumbnails.get(&key(attachment))?.as_ref()
    }
}

fn key(attachment: &Attachment) -> Key {
    (attachment.message_id.clone(), attachment.part_id.clone())
}

/// The attachments a message lists: its files, not its inline images or its calendar invite
/// (which shows as the invite card instead).
pub(crate) fn listed(message: &MessageDetail) -> impl Iterator<Item = &Attachment> {
    message
        .attachments
        .iter()
        .filter(|attachment| !attachment.inline && !is_invite(attachment))
}

/// Whether `attachment` is a calendar invite (`.ics`).
pub(super) fn is_invite(attachment: &Attachment) -> bool {
    let mime = attachment.mime_type.to_ascii_lowercase();
    mime == "text/calendar" || mime == "application/ics" || extension(&attachment.filename) == "ics"
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_lowercase())
        .unwrap_or_default()
}

/// One card's size for a message column `inner_width` wide, and how many fit on a row.
fn geometry(inner_width: f32) -> (Vec2, usize) {
    exo_ui::attachment_grid(inner_width - LIST_MARGIN_LEFT)
}

/// How much `count` attachments add below a body: their rows, and `Download all` under more
/// than one.
pub(crate) fn height(count: usize, inner_width: f32) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let (card, per_row) = geometry(inner_width);
    let rows = count.div_ceil(per_row) as f32;
    let download_all = if count > 1 { DOWNLOAD_LINE } else { 0.0 };
    LIST_MARGIN_TOP + rows * (card.y + CARD_MARGIN_BOTTOM) + download_all + LIST_EXTRA
}

/// Paints `message`'s attachments as cards from `top` (the body's bottom) in a message's content
/// column `inner`, with what `files` knows of them, and returns what was clicked.
pub(crate) fn show(
    ui: &Ui,
    inner: Rect,
    top: f32,
    message: &MessageDetail,
    files: &Files,
) -> Option<Click> {
    let (size, per_row) = geometry(inner.width());
    let list_left = inner.left() + LIST_MARGIN_LEFT;
    let mut clicked = None;
    let listed: Vec<&Attachment> = listed(message).collect();
    for (at, attachment) in listed.iter().enumerate() {
        let (row, column) = (at / per_row, at % per_row);
        let min = pos2(
            list_left + CARD_MARGIN_X + column as f32 * (size.x + 2.0 * CARD_MARGIN_X),
            top + LIST_MARGIN_TOP + row as f32 * (size.y + CARD_MARGIN_BOTTOM),
        );
        let card = Rect::from_min_size(min, size);
        if let Some(click) = show_card(
            ui,
            card,
            attachment,
            files,
            ui.id().with(("attachment", &message.id, at)),
        ) {
            clicked = Some(click);
        }
    }
    if listed.len() > 1 {
        let rows = listed.len().div_ceil(per_row) as f32;
        let line_top = top + LIST_MARGIN_TOP + rows * (size.y + CARD_MARGIN_BOTTOM);
        if download_all(
            ui,
            list_left,
            line_top,
            files.zipping.contains(&message.id),
            ui.id().with(("download-all", &message.id)),
        ) {
            clicked = Some(Click::DownloadAll {
                message: message.id.clone(),
            });
        }
    }
    clicked
}

/// One card in `card`: the file's name and badge, or its thumbnail; its download icon on hover,
/// or its progress bar while it downloads.
fn show_card(
    ui: &Ui,
    card: Rect,
    attachment: &Attachment,
    files: &Files,
    id: egui::Id,
) -> Option<Click> {
    let palette = exo_ui::palette(ui.ctx());
    // A pointer cursor over the card.
    let response = ui
        .interact(card, id, Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.drag_started()
        && let Some(at) = response.interact_pointer_pos()
    {
        return Some(Click::Drag {
            attachment: attachment.clone(),
            at,
        });
    }
    let hover = exo_ui::motion::transition(
        ui.ctx(),
        response.id.with("hover"),
        response.hovered(),
        0.25,
        exo_ui::motion::CubicBezier::EASE,
    );
    let thumbnail = files.thumbnail(attachment);
    match thumbnail {
        Some(texture) => paint_image_card(
            ui,
            card,
            texture,
            hover,
            exo_ui::motion::transition(
                ui.ctx(),
                response.id.with("overlay"),
                response.hovered(),
                0.125,
                exo_ui::motion::CubicBezier::LINEAR,
            ),
        ),
        None => exo_ui::paint_attachment_card(
            ui,
            card,
            &attachment.filename,
            &attachment.mime_type,
            hover,
        ),
    }
    let mut on_icon = false;
    let progress = files.progress.get(&key(attachment)).copied();
    let bottom = exo_ui::attachment_bottom_row(card);
    if let Some(progress) = progress {
        exo_ui::paint_attachment_progress(ui, card, &attachment.filename, progress);
    } else if !extension(&attachment.filename).is_empty() && response.hovered() {
        // The download icon: Classic's 13px glyph 2px in from the padding, `top: 1px`; .4
        // while the card is hovered, .7 on the glyph itself.
        let icon = Rect::from_min_size(
            pos2(card.right() - 8.0 - 2.0 - DOWNLOAD_ICON, bottom + 1.0),
            Vec2::splat(DOWNLOAD_ICON),
        );
        // It takes clicks 4px out (`padding: 4px; margin: -4px`).
        let target = icon.expand(4.0);
        on_icon = response.hover_pos().is_some_and(|pos| target.contains(pos));
        let opacity = exo_ui::motion::transition(
            ui.ctx(),
            response.id.with("icon"),
            on_icon,
            0.15,
            exo_ui::motion::CubicBezier::EASE,
        );
        // The glyph: black at .8 (white in Carbon), or white on a thumbnail.
        let ink = if thumbnail.is_some() {
            Color32::WHITE
        } else {
            palette.ink(0.8, 0.8)
        };
        let name = "legacy/DEPRECATED_download";
        exo_ui::paint_icon(
            ui,
            name,
            exo_ui::icon_rect(ui.ctx(), name, icon),
            ink.gamma_multiply(0.4 + 0.3 * opacity),
        );
    }
    if !response.clicked() {
        return None;
    }
    let modifiers = ui.input(|input| input.modifiers);
    // The download icon only downloads, as ⌘/Ctrl-click does anywhere on the card. ⇧ isn't
    // checked.
    Some(Click::Open {
        attachment: attachment.clone(),
        preview: !(on_icon || modifiers.command || modifiers.ctrl),
    })
}

/// An image card: the thumbnail, cover-cropped already, in the card's 4px
/// corners, with its hover shadow and, `overlay` (0–1) into the hover, the 50% black overlay.
fn paint_image_card(ui: &Ui, card: Rect, texture: &TextureHandle, hover: f32, overlay: f32) {
    let painter = ui.painter();
    let snapped = exo_ui::pixel_snap(card);
    if hover > 0.0 {
        // `box-shadow: 0 1px 3px 0 rgba(0,0,0,.3)`, `.7` in Carbon.
        let alpha = exo_ui::palette(ui.ctx()).ink(0.3, 0.7).a();
        exo_ui::BoxShadow::new(
            [0.0, 1.0],
            3.0,
            0.0,
            Color32::from_black_alpha((f32::from(alpha) * hover).round() as u8),
        )
        .paint(painter, snapped, CornerRadius::same(4));
    }
    let image = egui::epaint::RectShape::filled(snapped, CornerRadius::same(4), Color32::WHITE)
        .with_texture(
            texture.id(),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        );
    painter.add(image);
    if overlay > 0.0 {
        // The overlay: black, to `.5` on hover (`.125s linear`).
        painter.rect_filled(
            snapped,
            CornerRadius::same(4),
            Color32::from_black_alpha((127.5 * overlay).round() as u8),
        );
    }
}

/// `Download all` at `left`, `top`: the download glyph and the words at .4 (.6
/// hovered), or while `zipping` the spinner at .4 and `Downloading…`. Returns whether it was
/// clicked.
fn download_all(ui: &Ui, left: f32, top: f32, zipping: bool, id: egui::Id) -> bool {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter();
    // `Download all`: `margin-left: 3px; padding: 4px 4px 8px 0`; black (white in Carbon).
    let text_left = left + 3.0 + DOWNLOAD_ALL_ICON + 10.0;
    let label = if zipping {
        "Downloading…"
    } else {
        "Download all"
    };
    let label_width = DOWNLOAD_ALL
        .single_line(painter, label, Color32::PLACEHOLDER, None)
        .size()
        .x;
    let target = Rect::from_min_size(
        pos2(left + 3.0, top),
        vec2(DOWNLOAD_ALL_ICON + 10.0 + label_width + 4.0, DOWNLOAD_LINE),
    );
    let response = ui.interact(
        target,
        id,
        if zipping {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    let response = if zipping {
        response
    } else {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    };
    let opacity = if !zipping && response.hovered() {
        0.6
    } else {
        0.4
    };
    let ink = palette.ink(1.0, 1.0);
    // `top: 1px` and the 4px padding, then the 13px glyph on the text's baseline makes the line
    // box 18px, its text a pixel lower than a 16px one (measured).
    let line = top + 4.0 + 1.0 + 1.0;
    let icon = Rect::from_min_size(
        pos2(left + 3.0, line - 1.0 + 3.0),
        Vec2::splat(DOWNLOAD_ALL_ICON),
    );
    // The glyph: black (white) at .8.
    let glyph = palette.ink(0.8, 0.8);
    if zipping {
        exo_ui::paint_spinner(
            painter,
            icon,
            ui.input(|input| input.time),
            glyph.gamma_multiply(0.4),
        );
    } else {
        let name = "legacy/DEPRECATED_download";
        exo_ui::paint_icon(
            ui,
            name,
            exo_ui::icon_rect(ui.ctx(), name, icon),
            glyph.gamma_multiply(opacity),
        );
    }
    DOWNLOAD_ALL.paint_line(
        painter,
        pos2(text_left, line),
        label,
        ink.gamma_multiply(opacity),
        None,
    );
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_cards_to_a_row_at_640() {
        let (size, per_row) = geometry(640.0);
        assert_eq!((size.x, per_row), (154.0, 4));
        assert!((size.y - 102.656).abs() < 0.01, "{}", size.y);
        // One card adds 16 + 102.656 + 8 (+1).
        assert!((height(1, 640.0) - 127.656).abs() < 0.01);
        // Two add `Download all`'s line.
        assert!((height(2, 640.0) - 127.656 - DOWNLOAD_LINE).abs() < 0.01);
    }
}
