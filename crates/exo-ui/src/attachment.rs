//! An attachment card: a 3:2 tile with the file's name and a colored type badge, four to a row. The
//! thread view adds a download icon, compose a remove cross.

use egui::{Color32, CornerRadius, Rect, Ui, Vec2, pos2, text::LayoutJob, vec2};

use crate::{BoxShadow, Family, Font, TextStyle};

/// A card in a list:
/// `flex-basis: calc(25% - 8px); margin: 0 4px 8px; min-width: 120px; max-width: 154.16px`, 3:2
/// (`padding-top: 66.66%`).
pub const CARD_MARGIN_X: f32 = 4.0;
/// Below each card.
pub const CARD_MARGIN_BOTTOM: f32 = 8.0;
const CARD_MIN: f32 = 120.0;
const CARD_MAX: f32 = 154.16;
const ASPECT: f32 = 0.6666;
/// A card's name: 12px/16px, two lines at most, inside 8px of padding (9px on top).
const NAME: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// The type badge: 12px/12px bold uppercase, `top: 3px; padding: 0 6px`, on a 19px white pill
/// starting 5px above it.
const BADGE: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 12.0);

/// One card's size in a list `list_width` wide, and how many fit on a row.
pub fn attachment_grid(list_width: f32) -> (Vec2, usize) {
    let width = (0.25 * list_width - 2.0 * CARD_MARGIN_X).clamp(CARD_MIN, CARD_MAX);
    let per_row = ((list_width / (width + 2.0 * CARD_MARGIN_X)).floor() as usize).max(1);
    (vec2(width, width * ASPECT), per_row)
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_lowercase())
        .unwrap_or_default()
}

/// The type badge's color for `filename`.
pub fn attachment_badge_color(filename: &str, mime_type: &str) -> Color32 {
    let extension = extension(filename);
    let is = |list: &[&str]| list.contains(&extension.as_str());
    let hex = if is(&[
        "csv", "dbf", "ods", "prn", "xla", "xlam", "xls", "xlsb", "xlsm", "xlsx", "xlt", "xltx",
        "xlw",
    ]) {
        0x8e_cc_8b
    } else if is(&[
        "otp", "odp", "pot", "potm", "potx", "ppa", "ppam", "pps", "ppsm", "ppsx", "ppt", "pptx",
        "pptm", "key", "keynote",
    ]) {
        0xf5_84_61
    } else if is(&[
        "doc", "docm", "docx", "dot", "dotx", "ott", "odt", "rtf", "wpt", "wps",
    ]) {
        0x88_af_e6
    } else if mime_type == "application/pdf" || extension == "pdf" {
        0xf7_7b_7b
    } else if extension == "ics" {
        0xac_88_e6
    } else if is(&["psd", "ai", "eps", "indd", "sketch"]) {
        0xff_b9_4f
    } else {
        0x8a_9a_b0
    };
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// The top of a card's bottom row, which holds its type badge and its download icon or progress
/// bar: 16px tall, 12px from the card's bottom.
pub fn attachment_bottom_row(card: Rect) -> f32 {
    card.bottom() - 12.0 - 16.0
}

/// How wide a card `card_width` wide draws `filename`'s type badge
/// (`padding: 0 6px; max-width: calc(100% - 20px)` in the card's 8px padding); 0 without an
/// extension.
pub fn attachment_badge_width(painter: &egui::Painter, filename: &str, card_width: f32) -> f32 {
    let kind = extension(filename).to_uppercase();
    if kind.is_empty() {
        return 0.0;
    }
    let text = BADGE
        .single_line(painter, &kind, Color32::PLACEHOLDER, None)
        .size()
        .x;
    (text + 12.0).min(card_width - 36.0)
}

/// Paints the card for `filename` in `card`, `hover` (0–1) into its hover shadow: the
/// background, the name without its extension and the type badge.
pub fn paint_attachment_card(ui: &Ui, card: Rect, filename: &str, mime_type: &str, hover: f32) {
    let palette = crate::palette(ui.ctx());
    let painter = ui.painter();
    let dark = crate::theme(ui.ctx()) == crate::Theme::Carbon;
    let snapped = crate::pixel_snap(card);
    if hover > 0.0 {
        // `box-shadow: 0 1px 3px 0 rgba(0,0,0,.3)`.
        BoxShadow::new(
            [0.0, 1.0],
            3.0,
            0.0,
            Color32::from_black_alpha((0.3 * hover * 255.0).round() as u8),
        )
        .paint(painter, snapped, CornerRadius::same(4));
    }
    painter.rect_filled(
        snapped,
        CornerRadius::same(4),
        palette.background_elevated_accent,
    );

    // The name without its extension, wrapping anywhere, two lines at most.
    let name = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
    // The name: `rgba(0,0,0,.7)`, `hsla(0,0%,100%,.7)` in Carbon.
    let name_color = if dark {
        Color32::from_white_alpha(179)
    } else {
        Color32::from_black_alpha(179)
    };
    let mut job = LayoutJob::single_section(name.to_owned(), NAME.format(name_color));
    job.wrap.max_width = card.width() - 16.0;
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = true;
    crate::paint_job(
        painter,
        pos2(card.left() + 8.0, card.top() + 9.0),
        job,
        &NAME,
    );

    // The type badge at the bottom left.
    let kind = extension(filename).to_uppercase();
    if !kind.is_empty() {
        let badge_top = attachment_bottom_row(card) + 3.0;
        let pill = Rect::from_min_size(
            pos2(card.left() + 8.0, badge_top - 5.0),
            vec2(
                attachment_badge_width(painter, filename, card.width()),
                19.0,
            ),
        );
        painter.rect_filled(
            crate::pixel_snap(pill),
            CornerRadius::same(2),
            Color32::WHITE,
        );
        BADGE.paint_line(
            painter,
            pos2(pill.left() + 6.0, badge_top),
            &kind,
            attachment_badge_color(filename, mime_type),
            Some(pill.width() - 12.0),
        );
    }
}

/// The progress bar, in place of the download icon while the file downloads or uploads, `progress`
/// (0–100) of the way: a 4px white track from 6px past the type badge to the card's padding, its
/// bottom on the bottom row's, filled at least a tenth (`min-width: 10%`) in `#72c0ea`.
pub fn paint_attachment_progress(ui: &Ui, card: Rect, filename: &str, progress: f32) {
    let painter = ui.painter();
    let bottom = attachment_bottom_row(card);
    let left = card.left() + 8.0 + attachment_badge_width(painter, filename, card.width()) + 6.0;
    let track = Rect::from_min_max(
        pos2(left, bottom + 12.0),
        pos2(card.right() - 8.0, bottom + 16.0),
    );
    if track.width() <= 0.0 {
        return;
    }
    painter.rect_filled(track, CornerRadius::same(2), Color32::WHITE);
    // `transition: width 100ms ease`.
    let shown = ui.ctx().animate_value_with_time(
        crate::id(
            ui.ctx(),
            ("exo-attachment-progress", filename, card.min.x as i32),
        ),
        progress.clamp(0.0, 100.0),
        0.1,
    );
    let fill = Rect::from_min_size(
        track.min,
        vec2(track.width() * (shown / 100.0).max(0.1), track.height()),
    );
    painter.rect_filled(
        fill,
        CornerRadius::same(2),
        Color32::from_rgb(0x72, 0xc0, 0xea),
    );
}

/// A small attachment card: 184×28 with `padding: 6px`, 4px corners, the cards of a search result's
/// strip and an event's files.
pub const SMALL_CARD_SIZE: Vec2 = Vec2::new(184.0, 28.0);
/// A small card's name: 12px/16px, `top: 1px`.
const SMALL_NAME: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// A small card's download glyph: 13px, `margin: 0 2px 0 4px`.
const SMALL_DOWNLOAD: f32 = 13.0;

/// Where a small card's download button is in `card`, while the card is hovered.
pub fn small_card_download(card: Rect) -> Rect {
    Rect::from_min_size(
        pos2(
            card.right() - 6.0 - 2.0 - SMALL_DOWNLOAD,
            card.top() + 6.0 + 1.0,
        ),
        Vec2::splat(SMALL_DOWNLOAD),
    )
}

/// A small card (`SmallAttachmentCard`): `#dce2ea` (`#555b64` in Carbon), the file's type in
/// its color on a white pill, then its name without the extension, cut with an ellipsis.
pub struct SmallAttachmentCard<'a> {
    /// The file's name.
    pub filename: &'a str,
    /// Its MIME type.
    pub mime_type: &'a str,
    /// The hover's shadow, 0–1 (`transition: box-shadow 250ms`).
    pub hover: f32,
    /// The pointer is on it: the download glyph shows at its right, at `.4`.
    pub hovered: bool,
    /// …and on the glyph, at `.6`.
    pub download_hovered: bool,
}

impl SmallAttachmentCard<'_> {
    /// Paints it in `card` with `painter`.
    pub fn paint(&self, ui: &Ui, painter: &egui::Painter, card: Rect) {
        let Self {
            filename,
            mime_type,
            hover,
            hovered,
            download_hovered,
        } = *self;
        let dark = crate::theme(ui.ctx()) == crate::Theme::Carbon;
        let snapped = crate::pixel_snap(card);
        if hover > 0.0 {
            // `box-shadow: 0 1px 3px 0 rgba(0,0,0,.3)`, `.7` in Carbon.
            let alpha = if dark { 0.7 } else { 0.3 };
            BoxShadow::new(
                [0.0, 1.0],
                3.0,
                0.0,
                Color32::from_black_alpha((alpha * hover * 255.0).round() as u8),
            )
            .paint(painter, snapped, CornerRadius::same(4));
        }
        let background = if dark {
            Color32::from_rgb(0x55, 0x5b, 0x64)
        } else {
            Color32::from_rgb(0xdc, 0xe2, 0xea)
        };
        painter.rect_filled(snapped, CornerRadius::same(4), background);
        let content = card.shrink(6.0);
        let mut name_left = content.left() + 8.0;
        // The file type: bold uppercase, `top: 3px`, `padding: 0 6px`, at most half the card, on
        // its white `::before` 4px above it and 18px tall.
        let kind = extension(filename).to_uppercase();
        if !kind.is_empty() {
            let text = BADGE
                .single_line(painter, &kind, Color32::PLACEHOLDER, None)
                .size()
                .x;
            let width = (text + 12.0).min(content.width() / 2.0);
            let top = content.top() + 3.0;
            let pill = Rect::from_min_size(pos2(content.left(), top - 4.0), vec2(width, 18.0));
            painter.rect_filled(
                crate::pixel_snap(pill),
                CornerRadius::same(2),
                Color32::WHITE,
            );
            BADGE.paint_line(
                painter,
                pos2(pill.left() + 6.0, top),
                &kind,
                attachment_badge_color(filename, mime_type),
                Some(width - 12.0),
            );
            name_left = pill.right() + 8.0;
        }
        let mut name_right = content.right();
        if hovered {
            let download = small_card_download(card);
            let glyph = "legacy/DEPRECATED_download";
            let ink = if dark { Color32::WHITE } else { Color32::BLACK };
            crate::paint_icon_on(
                painter,
                glyph,
                crate::icon_rect(ui.ctx(), glyph, download),
                ink.gamma_multiply(if download_hovered { 0.6 } else { 0.4 }),
            );
            name_right = download.left() - 4.0;
        }
        // The name: black or white at .7.
        let name = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
        let name_color = if dark {
            Color32::from_white_alpha(179)
        } else {
            Color32::from_black_alpha(179)
        };
        SMALL_NAME.paint_line(
            painter,
            pos2(name_left, content.top() + 1.0),
            name,
            name_color,
            Some((name_right - name_left).max(0.0)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_cards_to_a_row_at_648() {
        let (size, per_row) = attachment_grid(648.0);
        assert_eq!((size.x, per_row), (154.0, 4));
        assert!((size.y - 102.656).abs() < 0.01, "{}", size.y);
    }
}
