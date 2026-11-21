//! The right pane in the Snippets folder: the highlighted snippet over the tips, or the tips
//! alone when nothing is highlighted or a snippet is being edited.

use egui::{Painter, Rect, pos2};
use exo_store::Snippet;
use exo_ui::{Family, Font, KeycapColors, Keycaps, TextStyle};

/// The tips' header and the snippet's name: 16px/24px.
const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
/// The tips' descriptions and the snippet's subject: 12px/16px, weight 600.
const DESCRIPTION: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 16.0);
/// A shortcut row's name: 12px/20px in a 24px row, at least 158px wide.
const SHORTCUT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const SHORTCUT_NAME: f32 = 158.0;
/// A variable, and the pane's text: 12px/16px.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// The snippet's metadata labels: 10px, weight 600.
const LABEL: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 10.0, 16.0);

const VARIABLES: [&str; 6] = [
    "{first_name}",
    "{last_name}",
    "{full_name}",
    "{sender_first_name}",
    "{sender_last_name}",
    "{sender_full_name}",
];

/// Paints the pane in `pane` (inside its border): `snippet`, when one is highlighted, over the
/// tips without the variables; else the tips.
pub(crate) fn show(
    painter: &Painter,
    pane: Rect,
    rem: f32,
    snippet: Option<&Snippet>,
    now: &jiff::Zoned,
) {
    let palette = exo_ui::palette(painter.ctx());
    // Fixed inks for the headers, the tips' shortcut rows and variables, and their darker
    // versions under a snippet, where the description takes the snippet's text colour.
    let header = palette.ink(0.8, 0.85);
    let (rows, description) = if snippet.is_some() {
        (palette.ink(0.9, 0.75), palette.ink(0.55, 0.55))
    } else {
        (palette.ink(0.7, 0.65), palette.text_base_accent)
    };
    // `padding: 32px 2rem`.
    let left = pane.left() + 2.0 * rem;
    let width = pane.width() - 4.0 * rem;
    let mut top = pane.top() + 32.0;
    if let Some(snippet) = snippet {
        // The name (`padding-bottom: 24px`), the subject (`margin-bottom: 8px`), the text's
        // first 550 characters in at most 5 lines (`margin-bottom: 24px`).
        let text_color = palette.ink(0.55, 0.55);
        HEADER.paint_line(
            painter,
            pos2(left, top),
            crate::compose::snippets::name(snippet),
            header,
            Some(width),
        );
        top += 24.0 + 24.0;
        if !snippet.subject.is_empty() {
            DESCRIPTION.paint_line(
                painter,
                pos2(left, top),
                &snippet.subject,
                text_color,
                Some(width),
            );
            top += 16.0 + 8.0;
        }
        let text: String = crate::compose::snippets::preview(snippet)
            .chars()
            .take(550)
            .collect();
        for line in TEXT.wrap(painter.ctx(), &text, width).into_iter().take(5) {
            TEXT.paint_line(painter, pos2(left, top), &line, text_color, Some(width));
            top += 16.0;
        }
        top += 24.0;
        // The exact date without the weekday, `--` when unknown.
        let date = |at: &str| {
            at.parse::<jiff::Timestamp>().map_or_else(
                |_| "--".to_owned(),
                |at| crate::format::dates::exact_friendly_omit_day_of_week(at, now),
            )
        };
        for (label, value) in [
            ("Modified", date(&snippet.date)),
            ("Last Sent", date(&snippet.analytics.last_sent)),
            ("Author", "Me".to_owned()),
        ] {
            LABEL.paint_line(
                painter,
                pos2(left, top),
                label,
                palette.ink(0.35, 0.55),
                None,
            );
            TEXT.paint_line(
                painter,
                pos2(left, top + 16.0),
                &value,
                palette.ink(0.9, 0.75),
                Some(width),
            );
            top += 40.0;
        }
        // The rule before the tips.
        painter.hline(
            left..=left + width,
            top + 8.0,
            egui::Stroke::new(1.0, palette.ink(0.1, 0.08)),
        );
        top += 32.0;
    } else {
        HEADER.paint_line(painter, pos2(left, top), "Tips", header, None);
        top += 57.0;
    }
    // `Instantly insert phrases, paragraphs, ` / ` or whole emails. Type less, do more!`.
    for line in [
        "Instantly insert phrases, paragraphs,",
        "or whole emails. Type less, do more!",
    ] {
        DESCRIPTION.paint_line(painter, pos2(left, top), line, description, None);
        top += 16.0;
    }
    top += 14.0;
    let colors = KeycapColors::shortcuts_pane(palette);
    for (name, keys) in [
        ("Create Snippet", &["C"][..]),
        ("Use Snippet Inline", &[";"][..]),
        ("Use Snippet", &["⌘", ";"][..]),
    ] {
        SHORTCUT.paint_line(painter, pos2(left, top), name, rows, None);
        Keycaps::new(painter.ctx(), keys, colors).paint(painter, left + SHORTCUT_NAME, top + 1.0);
        top += 24.0;
    }
    if snippet.is_some() {
        return;
    }
    // `margin-top: 32px`.
    top += 30.0;
    DESCRIPTION.paint_line(
        painter,
        pos2(left, top),
        "You can include these variables:",
        palette.text_base_accent,
        None,
    );
    top += 24.0;
    for variable in VARIABLES {
        TEXT.paint_line(painter, pos2(left, top), variable, rows, None);
        top += 24.0;
    }
    top += 20.0;
    for line in DESCRIPTION.wrap(
        painter.ctx(),
        "You can even include custom placeholders, and we'll warn you if you forget to fill them!",
        250.0,
    ) {
        DESCRIPTION.paint_line(
            painter,
            pos2(left, top),
            &line,
            palette.text_base_accent,
            None,
        );
        top += 16.0;
    }
    top += 8.0;
    TEXT.paint_line(painter, pos2(left, top), "{your_placeholder}", rows, None);
}
