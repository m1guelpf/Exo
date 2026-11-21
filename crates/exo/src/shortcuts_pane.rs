//! The right pane's shortcut panels: the keys that apply while conversations are selected, and
//! where `g` goes while a sequence is pending. Both are drawn the same way. Rows for features Exo
//! doesn't have are left out, as in the shortcuts sheet.

use egui::{Painter, Rect, pos2};
use exo_ui::{Family, Font, KeycapColors, Keycaps, TextStyle};

/// The header: 16px/24px.
const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
/// A subheader: 12px/16px bold, 1px down.
const SUBHEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
/// A shortcut's name: 12px/20px, 3px down.
const NAME: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
/// `td { padding-top: 4px }` above a 20px line.
const ROW: f32 = 24.0;
/// Measured: a row's keycaps sit 3px below its cell's content box (`top: 2px` on a line whose
/// inline-blocks start 1px down).
const KEYS_DOWN: f32 = 3.0;

/// One line of a panel.
enum Line {
    /// A group title (`Navigation`, `Actions`).
    Group(&'static str),
    /// A shortcut and its keys, space-separated (`mod shift A`).
    Shortcut(&'static str, &'static str),
}

const MULTI_SELECT: &[Line] = &[
    Line::Group("Navigation"),
    Line::Shortcut("Next Conversation", "J"),
    Line::Shortcut("Previous Conversation", "K"),
    Line::Shortcut("Select Conversation", "X"),
    Line::Shortcut("Clear Selection", "esc"),
    Line::Shortcut("Select All From Here", "mod A"),
    Line::Shortcut("Select All", "mod shift A"),
    Line::Group("Actions"),
    Line::Shortcut("Mark Done (Archive)", "E"),
    Line::Shortcut("Remind Me (Snooze)", "H"),
    Line::Shortcut("Star", "S"),
    Line::Shortcut("Mark Read or Unread", "U"),
    Line::Shortcut("Mute", "shift M"),
    Line::Shortcut("Trash", "#"),
    Line::Shortcut("Discard Draft", "⌘ shift ,"),
    Line::Shortcut("Mark as Spam", "!"),
    Line::Shortcut("Move", "V"),
    Line::Shortcut("Add or Remove Label", "L"),
    Line::Shortcut("Remove from Label", "Y"),
    Line::Shortcut("Remove All Labels", "shift Y"),
];

/// The actions that only work on loaded rows, which a select-all leaves out.
const LOADED_ONLY: [&str; 3] = ["Remind Me (Snooze)", "Discard Draft", "Remove All Labels"];

/// The Go to panel's rows with Important and Other on; the unified inbox names the first `Inbox`
/// and leaves `Other` out.
const GO_TO: &[Line] = &[
    Line::Shortcut("Important", "G then I"),
    Line::Shortcut("Starred", "G then S"),
    Line::Shortcut("Drafts", "G then D"),
    Line::Shortcut("Sent", "G then T"),
    Line::Shortcut("Done", "G then E"),
    Line::Shortcut("Reminders", "G then H"),
    Line::Shortcut("Snippets", "G then ;"),
    Line::Shortcut("Other", "G then O"),
    Line::Shortcut("All Mail", "G then A"),
    Line::Shortcut("Muted", "G then M"),
    Line::Shortcut("Spam", "G then !"),
    Line::Shortcut("Trash", "G then #"),
];

/// The unified inbox's first row.
const UNIFIED_INBOX: Line = Line::Shortcut("Inbox", "G then I");

/// Which panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    /// Conversations are selected; too many to act on row by row with `select_all`, which
    /// leaves out what only works on loaded rows.
    MultiSelect {
        /// A select-all reaching rows not loaded, or over 50 rows.
        select_all: bool,
    },
    /// A `g` sequence is pending.
    GoTo {
        /// The unified inbox is on (`unifiedInbox`).
        unified: bool,
    },
}

/// Paints `panel` in `pane` (the right pane inside its border).
pub fn show(painter: &Painter, pane: Rect, rem: f32, panel: Panel) {
    let palette = exo_ui::palette(painter.ctx());
    let (title, lines): (&str, Vec<&Line>) = match panel {
        Panel::MultiSelect { select_all } => (
            "Shortcuts",
            MULTI_SELECT
                .iter()
                .filter(|line| {
                    !select_all
                        || !matches!(line, Line::Shortcut(name, _) if LOADED_ONLY.contains(name))
                })
                .collect(),
        ),
        Panel::GoTo { unified } => (
            "Go to…",
            GO_TO
                .iter()
                .filter_map(|line| match line {
                    Line::Shortcut("Other", _) if unified => None,
                    Line::Shortcut("Important", _) if unified => Some(&UNIFIED_INBOX),
                    line => Some(line),
                })
                .collect(),
        ),
    };
    // `padding: 32px 2rem`; the header's 24px line and `padding-bottom: 24px`.
    let left = pane.left() + 2.0 * rem;
    let width = pane.width() - 4.0 * rem;
    // The header: black at .8, white at .85.
    HEADER.paint_line(
        painter,
        pos2(left, pane.top() + 32.0),
        title,
        palette.ink(0.8, 0.85),
        None,
    );
    // A shortcut's name: .7 in both themes.
    let name_color = palette.ink(0.7, 0.7);

    let colors = KeycapColors::shortcuts_pane(palette);
    let keys = |shortcut: &str| {
        Keycaps::new(
            painter.ctx(),
            &shortcut.split(' ').collect::<Vec<_>>(),
            colors,
        )
    };
    // `table { width: 100% }` in auto layout: each column gets its widest cell, and the room left
    // over goes to them in proportion to those widths.
    let names = lines
        .iter()
        .map(|line| match line {
            Line::Group(title) => {
                SUBHEADER
                    .single_line(painter, title, palette.text_base_accent, None)
                    .size()
                    .x
            }
            Line::Shortcut(name, _) => NAME.single_line(painter, name, name_color, None).size().x,
        })
        .fold(0.0, f32::max);
    let caps = lines
        .iter()
        .filter_map(|line| {
            if let Line::Shortcut(_, shortcut) = line {
                Some(keys(shortcut).width())
            } else {
                None
            }
        })
        .fold(0.0, f32::max);
    let keys_left = left + width * names / (names + caps);

    let mut top = pane.top() + 32.0 + 24.0 + 24.0;
    for (i, line) in lines.iter().enumerate() {
        match line {
            Line::Group(title) => {
                // `padding-top: 28px`, 8px for the first.
                top += if i == 0 { 8.0 } else { 28.0 };
                SUBHEADER.paint_line(
                    painter,
                    pos2(left, top + 1.0),
                    title,
                    palette.text_base_accent,
                    None,
                );
                top += SUBHEADER.line_height;
            }
            Line::Shortcut(name, shortcut) => {
                top += ROW - NAME.line_height;
                NAME.paint_line(painter, pos2(left, top + 3.0), name, name_color, None);
                keys(shortcut).paint(painter, keys_left, top + KEYS_DOWN);
                top += NAME.line_height;
            }
        }
    }
}
