//! The right pane while searching with no one to show: Tips, the operators with what they do.

use egui::{Painter, Rect, pos2};
use exo_ui::{Family, Font, TextStyle};

/// The header: 16px/24px.
const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
/// A tip's operator: 12px/16px bold, `top: 1px`; its description the same, regular.
const PROMPT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
const DESCRIPTION: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// `@container (width < 280px)`: each tip's prompt above its description.
const STACKED_BELOW: f32 = 280.0;

/// The tips for a Gmail account, without `is:shared` (team accounts only).
const TIPS: [(&str, &str); 15] = [
    ("from:nicole", "from Nicole"),
    ("to:roman", "to Roman"),
    ("\"be brilliant\"", "contains \"be brilliant\""),
    ("has:attachment", "with attachments"),
    ("subject:lunch", "subject contains \"lunch\""),
    ("in:sent", "in Sent"),
    ("in:inbox", "in the Inbox"),
    ("-in:inbox", "not in the Inbox"),
    ("in:fundraising", "in this label"),
    ("is:unread", "unread conversations"),
    ("is:starred", "starred conversations"),
    ("before:2017/06/01", "before June 2017"),
    ("after:2017/06/01", "June 2017 or later"),
    ("older_than:3d", "more than 3 days ago"),
    ("newer_than:1m", "1 month ago or later"),
];

/// The footer: `padding: 22px 2rem 12px` around 28px buttons.
const FOOTER: f32 = 62.0;

/// Whether Tips overflow `pane` above its footer, which turns on the line over the footer
/// (set as Tips show and kept while searching).
pub(crate) fn overflow(pane: Rect, rem: f32) -> bool {
    let stacked = pane.width() - 4.0 * rem < STACKED_BELOW;
    let tip = if stacked {
        PROMPT.line_height + DESCRIPTION.line_height
    } else {
        PROMPT.line_height
    };
    let height = 8.0 + TIPS.len() as f32 * tip + (TIPS.len() - 1) as f32 * 16.0;
    height > pane.bottom() - FOOTER - (pane.top() + 32.0 + 24.0 + 24.0)
}

/// Paints Tips in `pane` (the right pane inside its border), the list cut off above the footer.
pub(crate) fn show(painter: &Painter, pane: Rect, rem: f32) {
    let bottom = pane.bottom() - FOOTER;
    let palette = exo_ui::palette(painter.ctx());
    // `padding: 32px 2rem`; the header's 24px line, `padding-bottom: 24px`, then the list's
    // `padding-top: 8px`.
    let left = pane.left() + 2.0 * rem;
    let width = pane.width() - 4.0 * rem;
    // The header: black at .8, white at .85; the tips .7 and .65.
    HEADER.paint_line(
        painter,
        pos2(left, pane.top() + 32.0),
        "Tips",
        palette.ink(0.8, 0.85),
        None,
    );
    let color = palette.ink(0.7, 0.65);
    let list = Rect::from_min_max(
        pos2(pane.left(), pane.top() + 32.0 + 24.0 + 24.0),
        pos2(pane.right(), bottom),
    );
    let painter = painter.with_clip_rect(list.intersect(painter.clip_rect()));
    let stacked = width < STACKED_BELOW;
    let mut top = list.top() + 8.0;
    for (at, (prompt, description)) in TIPS.iter().enumerate() {
        // `margin-top: 16px`, none for the first.
        if at > 0 {
            top += 16.0;
        }
        let prompt_width =
            PROMPT.paint_line(&painter, pos2(left, top + 1.0), prompt, color, Some(width));
        // Stacked, or beside it after the prompt's `min-width: 9.5rem`.
        let description_left = if stacked {
            left
        } else {
            left + prompt_width.max(9.5 * rem)
        };
        if stacked {
            top += PROMPT.line_height;
        }
        DESCRIPTION.paint_line(
            &painter,
            pos2(description_left, top + 1.0),
            description,
            color,
            Some(width),
        );
        top += DESCRIPTION.line_height;
    }
}
