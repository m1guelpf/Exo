//! The thread's header: the subject wrapped around the floating actions, the star, the label
//! chips and the status line (Unsubscribe), and the back gutter on the left.

use egui::{Color32, CornerRadius, Mesh, Rect, Sense, Shape, Ui, Vec2, pos2, vec2};
use exo_store::ThreadDetail;
use exo_ui::{
    BoxShadow, Family, Font, IconButton, KeyTooltip, LabelChip, Placement, TextStyle, chip_hover,
    motion,
};

use super::{Action, Layout};
use crate::body::Bodies;

/// The header's padding: `24px 4rem 20px`.
pub(super) const PADDING_TOP: f32 = 24.0;
const PADDING_BOTTOM: f32 = 20.0;
/// Once the messages scroll under it: `padding-bottom: 12px` and a 1px bottom border.
const SCROLLED_PADDING_BOTTOM: f32 = 12.0;
/// How far above its unscrolled bottom the scrolled header ends, its border included.
pub(super) const SCROLLED_INSET: f32 = PADDING_BOTTOM - SCROLLED_PADDING_BOTTOM - 1.0;
/// The subject: 21px/32px, `top: 3px; margin-right: 12px`.
pub(super) const SUBJECT_LINE: f32 = 32.0;
const SUBJECT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 21.0, SUBJECT_LINE);
const SUBJECT_NUDGE: f32 = 3.0;
const SUBJECT_MARGIN: f32 = 12.0;
/// The actions, floated right: five 24px buttons, `margin: 11px -4px -3px 0` around their
/// 16px height, so only the subject's first line (which starts above its 24px margin box) stops
/// at it.
const ACTIONS_WIDTH: f32 = 120.0;
/// The status line: 12px/16px bold, `padding: 8px 0 1px; margin-bottom: 3px`.
const STATUS: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
const STATUS_HEIGHT: f32 = 8.0 + 16.0 + 1.0 + 3.0;
/// Chips sit on the subject's line box, below its top (measured).
const CHIP_TOP: f32 = 11.0;
/// The space after each label chip.
const CHIP_MARGIN: f32 = 6.0;

/// A pending bulk action's status: `#ef8e8e`, a literal in Snow and Carbon.
const PENDING: Color32 = Color32::from_rgb(0xef, 0x8e, 0x8e);

/// What the header's status line says (no AI summaries).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Status {
    /// A bulk action the conversation waits on: `Pending: Mark Done`.
    Pending(String),
    /// The conversation's last one-click action: `View order`, `Track Order`.
    Action(exo_mail::Action),
    /// The conversation can be unsubscribed from.
    Unsubscribe,
}

/// What the keyboard can focus in the header, in tab order (`[tabindex]`): the status line when
/// it does something, then Done, Remind Me and ⌘K (the conversation arrows have no tab index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Control {
    Status,
    Done,
    Remind,
    Command,
}

impl Control {
    /// Where the actions' tab indices start: after the status line's 1, if it's
    /// there.
    pub(super) fn tab_start(controls: &[Self]) -> u8 {
        if controls.contains(&Self::Status) {
            2
        } else {
            1
        }
    }

    /// Its tab index: the status line's is 1, the actions' `start + 2…4`.
    pub(super) fn tab_index(self, start: u8) -> u8 {
        match self {
            Self::Status => 1,
            Self::Done => start + 2,
            Self::Remind => start + 3,
            Self::Command => start + 4,
        }
    }

    /// What clicking it does, the status line saying `status`.
    pub(super) fn action(self, status: Option<&Status>) -> Action {
        match self {
            Self::Status => match status {
                Some(Status::Action(action)) => Action::OpenUrl(action.url.clone()),
                _ => Action::Unsubscribe,
            },
            Self::Done => Action::ToggleDone,
            Self::Remind => Action::Remind,
            Self::Command => Action::CommandPalette,
        }
    }
}

/// The header's controls for its `status` line, in tab order.
pub(super) fn controls(status: Option<&Status>) -> Vec<Control> {
    let status =
        matches!(status, Some(Status::Unsubscribe | Status::Action(_))).then_some(Control::Status);
    status
        .into_iter()
        .chain([Control::Done, Control::Remind, Control::Command])
        .collect()
}

/// The status line's underline when focused, in Classic: `box-shadow: 0 2px #aeb1dd`.
const STATUS_FOCUS: Color32 = Color32::from_rgb(0xae, 0xb1, 0xdd);

/// The status line for `detail`: the bulk action it waits on, else its last one-click
/// `action`, else Unsubscribe when there's something to unsubscribe from (a usable
/// `List-Unsubscribe`, not of one of `own_domains`, or a rendered body's unsubscribe link).
pub(super) fn status(
    detail: &ThreadDetail,
    bodies: &Bodies,
    pending: Option<&str>,
    action: Option<exo_mail::Action>,
    own_domains: &[String],
) -> Option<Status> {
    if let Some(pending) = pending {
        return Some(Status::Pending(pending.to_owned()));
    }
    if let Some(action) = action {
        return Some(Status::Action(action));
    }
    crate::unsubscribe::find(detail, own_domains, |id| {
        bodies.facts(id).and_then(|facts| facts.unsubscribe.clone())
    })
    .map(|_| Status::Unsubscribe)
}

/// The subject as the header shows it.
pub(super) fn title(detail: &ThreadDetail) -> &str {
    let subject = detail.subject.trim();
    if subject.is_empty() { "-" } else { subject }
}

/// Breaks `text` into lines the way the subject wraps: at spaces, the first line stopping at the
/// floating actions, and the last line leaving room for the subject's right margin.
fn wrap(ui: &Ui, text: &str, first: f32, full: f32) -> Vec<String> {
    let painter = ui.painter();
    let width = |line: &str| {
        SUBJECT
            .single_line(painter, line.trim_end(), Color32::PLACEHOLDER, None)
            .size()
            .x
    };
    let limit = |line: usize| if line == 0 { first } else { full };
    let mut lines: Vec<String> = vec![String::new()];
    for word in text.split_inclusive(' ') {
        let index = lines.len() - 1;
        let current = &mut lines[index];
        let candidate = format!("{current}{word}");
        if current.is_empty() || width(&candidate) <= limit(index) {
            *current = candidate;
        } else {
            lines.push(word.to_owned());
        }
    }
    // The margin after the subject has to fit on its last line too.
    let last = lines.len() - 1;
    if width(&lines[last]) + SUBJECT_MARGIN > limit(last)
        && let Some(cut) = lines[last].trim_end().rfind(' ')
    {
        let moved = lines[last][cut + 1..].to_owned();
        lines[last].truncate(cut + 1);
        lines.push(moved);
    }
    lines
}

/// Where the subject's lines and the label chips after them go.
struct Arrangement {
    lines: Vec<String>,
    /// Each chip's name, line and left edge.
    chips: Vec<(String, usize, f32)>,
    /// Lines in all, chips included.
    line_count: usize,
}

fn arrange(ui: &Ui, layout: &Layout, detail: &ThreadDetail, chips: &[String]) -> Arrangement {
    let painter = ui.painter();
    let label = layout.label;
    let float_left = label.width() - ACTIONS_WIDTH + 4.0;
    let lines = wrap(ui, title(detail), float_left, label.width());
    let mut line = lines.len() - 1;
    let end = SUBJECT
        .single_line(painter, lines[line].trim_end(), Color32::PLACEHOLDER, None)
        .size()
        .x;
    let mut x = label.left() + end + SUBJECT_MARGIN;
    let mut placed = Vec::new();
    for name in chips {
        let width = LabelChip::new(painter, name, false).width();
        let limit = if line == 0 {
            label.left() + float_left
        } else {
            label.right()
        };
        if x + width > limit && x > label.left() {
            line += 1;
            x = label.left();
        }
        placed.push((name.clone(), line, x));
        x += width + CHIP_MARGIN;
    }
    Arrangement {
        lines,
        chips: placed,
        line_count: line + 1,
    }
}

/// The subject's box (its glyphs' 26px on each of its
/// lines), for its copy tooltip; none without a subject.
pub(super) fn subject_rect(
    ui: &Ui,
    layout: &Layout,
    detail: &ThreadDetail,
    chips: &[String],
) -> Option<Rect> {
    if detail.subject.trim().is_empty() {
        return None;
    }
    let arrangement = arrange(ui, layout, detail, chips);
    let painter = ui.painter();
    let width = arrangement
        .lines
        .iter()
        .map(|line| {
            SUBJECT
                .single_line(painter, line.trim_end(), Color32::PLACEHOLDER, None)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let label = layout.label;
    let top = label.top() + SUBJECT_NUDGE + 3.0;
    Some(Rect::from_min_size(
        pos2(label.left(), top),
        vec2(
            width,
            (arrangement.lines.len() - 1) as f32 * SUBJECT_LINE + 26.0,
        ),
    ))
}

/// The header's height in layout (unscrolled).
pub(super) fn height(
    ui: &Ui,
    layout: &Layout,
    detail: &ThreadDetail,
    chips: &[String],
    has_status: bool,
) -> f32 {
    let lines = arrange(ui, layout, detail, chips).line_count as f32;
    PADDING_TOP
        + lines * SUBJECT_LINE
        + if has_status { STATUS_HEIGHT } else { 0.0 }
        + PADDING_BOTTOM
}

/// Paints the header over the messages' top and handles its clicks. Once the messages have
/// scrolled under it, it gets a background and a bottom border. `typing`: one of
/// its drafts has the keyboard; `focus`: the control the keyboard is on.
#[expect(
    clippy::too_many_arguments,
    reason = "one header, drawn from the thread view's state; a struct would only rename them"
)]
pub(super) fn show(
    ui: &Ui,
    layout: &Layout,
    detail: &ThreadDetail,
    chips: &[String],
    height: f32,
    status: Option<&Status>,
    scrolled: bool,
    typing: bool,
    focus: Option<Control>,
    selection: &mut exo_ui::TextSelection,
    now: &jiff::Zoned,
) -> Vec<Action> {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter();
    let mut actions = Vec::new();
    let top = layout.header.top();
    if scrolled {
        let bottom = top + height - SCROLLED_INSET;
        // Over a widened column the background and its border run the column's width too, under
        // the header's own.
        let spans = if layout.is_wide() {
            vec![layout.column.x_range(), layout.header.x_range()]
        } else {
            vec![layout.header.x_range()]
        };
        for span in spans {
            let rect = Rect::from_x_y_ranges(span, top..=bottom);
            painter.rect_filled(
                exo_ui::pixel_snap(rect),
                0.0,
                palette.background_base_default,
            );
            // `border-bottom: 1px solid rgba(0,0,0,.085)`, `hsla(0,0%,100%,.085)` in Carbon.
            let line = exo_ui::pixel_snap(Rect::from_min_max(
                pos2(rect.left(), bottom - 1.0),
                rect.right_bottom(),
            ));
            painter.rect_filled(line, 0.0, palette.ink(0.085, 0.085));
        }
    }
    let label = layout.label;

    // The actions float right: Done, Remind Me, ⌘K, then Previous and Next thread.
    let id = ui.id().with("thread-header");
    let right = label.right();
    let done = detail.labels.iter().all(|label| label != "INBOX");
    let reminded = matches!(detail.reminder, Some(exo_store::ReminderStatus::Set { .. }));
    // The legacy glyphs, 11px down: the 16×14 check, the 14px clock and command, the 16px
    // chevrons. A done thread's check is `#8892b9` 2 units wide, `#5f74aa` hovered; a reminded
    // thread's clock the bolder one in `#e0bfe0`, `#cd7acf` hovered.
    let remind = if reminded {
        crate::inbox::REMIND_SET_ICON
    } else {
        crate::inbox::REMIND_ICON
    };
    let buttons: [(&str, f32, Vec2, Action); 5] = [
        (
            "legacy/DEPRECATED_threadActionsDone",
            110.0,
            vec2(16.0, 14.0),
            Action::ToggleDone,
        ),
        (remind, 86.0, Vec2::splat(14.0), Action::Remind),
        (
            "legacy/DEPRECATED_threadActionsCommand",
            63.0,
            Vec2::splat(14.0),
            Action::CommandPalette,
        ),
        (CHEVRON_UP, 40.0, Vec2::splat(16.0), Action::Navigate(-1)),
        (CHEVRON_DOWN, 16.0, Vec2::splat(16.0), Action::Navigate(1)),
    ];
    for (icon, from_right, size, action) in buttons {
        let icon_rect = Rect::from_min_size(pos2(right - from_right, label.top() + 11.0), size);
        // A focused button lights its glyph as hovering does.
        let lit = focus.is_some_and(|focus| focus.action(status) == action);
        let response = if done && action == Action::ToggleDone {
            let response = ui
                .interact(icon_rect.expand(4.0), id.with(icon), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let color = if response.hovered() || lit {
                Color32::from_rgb(0x5f, 0x74, 0xaa)
            } else {
                palette.actions_done_default
            };
            exo_ui::paint_icon_stroked(ui, icon, exo_ui::pixel_snap(icon_rect), color, 2.0);
            response
        } else if action == Action::Remind {
            // Its tooltip hangs off the button's 24px box (5px around the glyph).
            let button = IconButton::new(icon).lit(lit);
            let button = if reminded {
                button.colors(
                    Color32::from_rgb(0xe0, 0xbf, 0xe0),
                    Color32::from_rgb(0xcd, 0x7a, 0xcf),
                )
            } else {
                button
            };
            button.show(ui, id.with(icon), icon_rect.expand(5.0), icon_rect)
        } else {
            IconButton::new(icon)
                .lit(lit)
                .show(ui, id.with(icon), icon_rect.expand(4.0), icon_rect)
        };
        // With a draft of the thread's being written, Remind Me's key is compose's ⌘⇧H.
        let remind_keys: &[&str] = if typing {
            &["⌘", "shift", "H"]
        } else {
            &["H"]
        };
        // Their tooltips show while they have the keyboard too.
        match (action == Action::Remind).then_some(detail.reminder.as_ref()) {
            Some(reminder) => {
                let status = reminder.map(|reminder| crate::remind::status(reminder, now));
                KeyTooltip {
                    text: "Remind Me",
                    keys: remind_keys,
                }
                .show_at_focused(
                    ui,
                    &response,
                    Placement::Below {
                        gap: 8.0,
                        shift: 10.0,
                    },
                    lit,
                    status.as_deref(),
                );
            }
            None => action_tooltip(ui, &response, &action, done, lit),
        }
        if response.clicked() {
            actions.push(action);
        }
    }

    // The subject, its star and its labels.
    let arrangement = arrange(ui, layout, detail, chips);
    let starred = detail
        .messages
        .iter()
        .any(exo_store::MessageDetail::is_starred);
    let star = exo_ui::animated_star(ui.ctx(), id.with(("status", &detail.id)), starred);
    let status_at = pos2(label.left() - 1.0, label.top() + 9.0);
    if detail
        .messages
        .iter()
        .any(|message| message.failure.is_some())
    {
        exo_ui::paint_failed_status(painter, status_at);
    } else {
        exo_ui::paint_thread_status(painter, status_at, exo_ui::Unread::No, star);
    }
    for (at, line) in arrangement.lines.iter().enumerate() {
        let line_top = label.top() + at as f32 * SUBJECT_LINE;
        // A selectable subject: its selection fills the line box, which its `top: 3px` doesn't
        // move.
        let galley = SUBJECT.single_line(painter, line.trim_end(), Color32::PLACEHOLDER, None);
        selection.add(
            painter,
            pos2(label.left(), line_top),
            galley,
            SUBJECT_LINE,
            super::SELECTED_SUBJECT,
            " ",
            palette.text_selection_default,
        );
        SUBJECT.paint_line(
            painter,
            pos2(label.left(), line_top + SUBJECT_NUDGE),
            line.trim_end(),
            palette.text_base_default,
            None,
        );
    }
    for (name, line, x) in arrangement.chips {
        let chip = LabelChip::new(painter, &name, false);
        let line_top = label.top() + line as f32 * SUBJECT_LINE;
        // A chip's clickable strip: 28px tall.
        let strip = Rect::from_min_size(pos2(x, line_top + 3.0), vec2(chip.width(), 28.0));
        // A pointer cursor over a chip.
        let response = ui
            .interact(strip, id.with(("label", &name)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let hover = chip_hover(
            ui.ctx(),
            response.id,
            response.hovered(),
            response.is_pointer_button_down_on(),
        );
        chip.paint(painter, pos2(x, line_top + CHIP_TOP), hover);
        if response.clicked() {
            actions.push(Action::ShowLabel(name.clone()));
        }
    }

    // The status line under the subject: a button with a tooltip when it does something, plain
    // text otherwise.
    let status_top = label.top() + arrangement.line_count as f32 * SUBJECT_LINE + 1.0;
    match status {
        Some(status @ (Status::Unsubscribe | Status::Action(_))) => {
            let (text, keys) = match status {
                Status::Action(action) => (action.text.as_str(), &["⌘", "O"]),
                _ => ("Unsubscribe", &["⌘", "U"]),
            };
            let width = STATUS
                .single_line(painter, text, Color32::PLACEHOLDER, None)
                .size()
                .x;
            let hit = Rect::from_min_size(pos2(label.left(), status_top), vec2(width, 25.0));
            // A pointer cursor over a status line that does something.
            let response = ui
                .interact(hit, id.with("status"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            STATUS.paint_line(
                painter,
                pos2(label.left(), status_top + 8.0),
                text,
                palette.text_base_accent,
                None,
            );
            let focused = focus == Some(Control::Status);
            if focused {
                // `box-shadow: 0 2px #aeb1dd` under the button's box (`padding: 8px 0 1px`).
                let line = Rect::from_min_size(pos2(label.left(), hit.bottom()), vec2(width, 2.0));
                painter.rect_filled(exo_ui::pixel_snap(line), 0.0, STATUS_FOCUS);
            }
            // The tooltip's middle left on the status's middle right, 10px out, its middle 15.5px
            // below the status's top.
            let mut anchor = response.clone();
            anchor.rect.max.y = hit.top() + 31.0;
            KeyTooltip { text, keys }.show_at_focused(
                ui,
                &anchor,
                Placement::Right(10.0),
                focused,
                None,
            );
            if response.clicked() {
                actions.push(Control::Status.action(Some(status)));
            }
        }
        Some(Status::Pending(text)) => {
            STATUS.paint_line(
                painter,
                pos2(label.left(), status_top + 8.0),
                text,
                PENDING,
                None,
            );
        }
        None => {}
    }
    actions
}

/// Previous and Next thread's legacy chevrons.
const CHEVRON_UP: &str = "legacy/DEPRECATED_actionChevronUp";
const CHEVRON_DOWN: &str = "legacy/DEPRECATED_actionChevronDown";

/// A new message's header: `New Message` and the conversation arrows.
pub(super) fn show_title(ui: &Ui, layout: &Layout, title: &str) -> Vec<Action> {
    let palette = exo_ui::palette(ui.ctx());
    let label = layout.label;
    let id = ui.id().with("thread-header");
    let mut actions = Vec::new();
    for (icon, from_right, action) in [
        (CHEVRON_UP, 40.0, Action::Navigate(-1)),
        (CHEVRON_DOWN, 16.0, Action::Navigate(1)),
    ] {
        let icon_rect = Rect::from_min_size(
            pos2(label.right() - from_right, label.top() + 11.0),
            Vec2::splat(16.0),
        );
        let response =
            IconButton::new(icon).show(ui, id.with(icon), icon_rect.expand(4.0), icon_rect);
        action_tooltip(ui, &response, &action, false, false);
        if response.clicked() {
            actions.push(action);
        }
    }
    SUBJECT.paint_line(
        ui.painter(),
        pos2(label.left(), label.top() + SUBJECT_NUDGE),
        title,
        palette.text_base_default,
        None,
    );
    actions
}

/// The tooltip under one of the header's actions: Mark Done and ⌘K 8px under and 10px right of
/// centre, the conversation arrows 10px under.
fn action_tooltip(ui: &Ui, response: &egui::Response, action: &Action, done: bool, focused: bool) {
    let (tooltip, placement) = match action {
        Action::ToggleDone if done => (
            KeyTooltip {
                text: "Mark not Done",
                keys: &["shift", "E"],
            },
            Placement::Below {
                gap: 8.0,
                shift: 10.0,
            },
        ),
        Action::ToggleDone => (
            KeyTooltip {
                text: "Mark Done",
                keys: &["E"],
            },
            Placement::Below {
                gap: 8.0,
                shift: 10.0,
            },
        ),
        Action::CommandPalette => (
            KeyTooltip {
                text: "Exo Command",
                keys: &["⌘", "K"],
            },
            Placement::Below {
                gap: 8.0,
                shift: 10.0,
            },
        ),
        Action::Navigate(-1) => (
            KeyTooltip {
                text: "Previous Conversation",
                keys: &["K"],
            },
            Placement::Below {
                gap: 10.0,
                shift: 0.0,
            },
        ),
        Action::Navigate(_) => (
            KeyTooltip {
                text: "Next Conversation",
                keys: &["J"],
            },
            Placement::Below {
                gap: 10.0,
                shift: 0.0,
            },
        ),
        _ => return,
    };
    tooltip.show_at_focused(ui, response, placement, focused, None);
}

/// A new message's header: one line, no status, and a 4px
/// `margin-bottom`.
pub(super) fn title_height() -> f32 {
    PADDING_TOP + SUBJECT_LINE + PADDING_BOTTOM + 4.0
}

/// The back button: `top: 25px; left: 2rem`, a 32px button
/// with a 16px arrow in it.
pub(super) fn back_button_rect(layout: &Layout) -> Rect {
    Rect::from_min_size(
        pos2(
            layout.action_pane.left() + 2.0 * layout.rem,
            layout.action_pane.top() + 25.0,
        ),
        Vec2::splat(32.0),
    )
}

/// The back button in the gutter, and the gutter's soft
/// gradient while the pointer is over it; `focused` while the gutter has the keyboard.
pub(super) fn back_button(ui: &Ui, layout: &Layout, hovered: bool, focused: bool) -> bool {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter();
    let pane = layout.action_pane;
    // The gutter's gradient: the accent at 20%, 10% at 35.1%, then nothing (Carbon: `rgba(13, 13,
    // 28, …)` at 15% and 10%), 75% of the left pane's width at most its clickable's, fading over
    // 100ms; gone below 864px of thread pane or 20px of clickable.
    let opacity = motion::transition(
        ui.ctx(),
        ui.id().with("back-gutter"),
        hovered,
        0.1,
        motion::CubicBezier::EASE,
    );
    let clickable = layout.gutter_clickable();
    if opacity > 0.0 && pane.width() + layout.header.width() >= 864.0 && clickable > 20.0 {
        let width = (0.75 * layout.left_pane).min(clickable);
        let ([r, g, b], first) = match exo_ui::theme(ui.ctx()) {
            exo_ui::Theme::Snow => ([174, 177, 221], 0.2),
            exo_ui::Theme::Carbon => ([13, 13, 28], 0.15),
        };
        let stop = |alpha: f32| {
            Color32::from_rgba_unmultiplied(r, g, b, (alpha * opacity * 255.0).round() as u8)
        };
        let stops = [
            (0.0, stop(first)),
            (0.351, stop(0.1)),
            (1.0, Color32::TRANSPARENT),
        ];
        let mut mesh = Mesh::default();
        for (i, (at, color)) in stops.into_iter().enumerate() {
            let x = pane.left() + at * width;
            mesh.colored_vertex(pos2(x, pane.top()), color);
            mesh.colored_vertex(pos2(x, pane.bottom()), color);
            if i > 0 {
                let base = 2 * i as u32;
                mesh.add_triangle(base - 2, base, base + 1);
                mesh.add_triangle(base - 2, base + 1, base - 1);
            }
        }
        painter.add(Shape::mesh(mesh));
    }
    if pane.width() < 1.0 {
        // Beside a popped-out draft the gutter has no room: its 32px button spills out of it,
        // 8px in and 25px down, a caret in it, with the button's tooltip and click but not its
        // raised circle.
        let rect = Rect::from_min_size(
            pos2(pane.left() + 8.0, pane.top() + 25.0),
            Vec2::splat(32.0),
        );
        let icon = rect.shrink(8.0);
        let button = ui
            .interact(rect, ui.id().with("compact-back-button"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        KeyTooltip {
            text: "Back",
            keys: &["esc"],
        }
        .show_at(
            ui,
            &button,
            Placement::Below {
                gap: 10.0,
                shift: 0.0,
            },
        );
        let color = if button.hovered() {
            palette.icon_base_hover
        } else {
            palette.icon_base_default
        };
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_actionBackCompact",
            exo_ui::pixel_snap(icon),
            color,
        );
        return button.clicked();
    }
    let button = back_button_rect(layout);
    let on_button = hovered && ui.rect_contains_pointer(button);
    // Only the button has the tooltip, 10px under it. Hover-only, so the gutter under it still
    // takes the click.
    let tip = ui.interact(button, ui.id().with("back-button-tooltip"), Sense::hover());
    KeyTooltip {
        text: "Back",
        keys: &["esc"],
    }
    .show_at(
        ui,
        &tip,
        Placement::Below {
            gap: 10.0,
            shift: 0.0,
        },
    );
    // Over the gutter the button is a raised circle, fading in over 150ms (`transition:
    // box-shadow 150ms ease, background 150ms ease`).
    let raised = motion::transition(
        ui.ctx(),
        ui.id().with("back-button"),
        hovered,
        0.15,
        motion::CubicBezier::EASE,
    );
    // With the keyboard on the gutter, in Classic: `0 0 6px 0 #aeb1dd, 0 2px 12px 0
    // rgba(174,177,221,.5)` instead of the hover's shadows. The button has no background
    // (`background: none`), and a shadow doesn't paint under its box: the pane shows inside.
    let lit = motion::transition(
        ui.ctx(),
        ui.id().with("back-button-focus"),
        focused,
        0.15,
        motion::CubicBezier::EASE,
    );
    if lit > 0.0 {
        let glow = |alpha: f32| {
            Color32::from_rgba_unmultiplied(174, 177, 221, (alpha * lit * 255.0).round() as u8)
        };
        let circle = exo_ui::pixel_snap(button);
        BoxShadow::new([0.0, 0.0], 6.0, 0.0, glow(1.0)).paint(
            painter,
            circle,
            CornerRadius::same(16),
        );
        BoxShadow::new([0.0, 2.0], 12.0, 0.0, glow(0.5)).paint(
            painter,
            circle,
            CornerRadius::same(16),
        );
        painter.rect_filled(
            circle,
            CornerRadius::same(16),
            palette.background_base_default.gamma_multiply(lit),
        );
    }
    if raised > 0.0 {
        let glow = |alpha: f32| {
            Color32::from_rgba_unmultiplied(174, 177, 221, (alpha * 255.0).round() as u8)
        };
        let ink =
            |alpha: f32| Color32::from_rgba_unmultiplied(13, 13, 28, (alpha * 255.0).round() as u8);
        let shadows = match exo_ui::theme(ui.ctx()) {
            exo_ui::Theme::Snow => [
                BoxShadow::new([0.0, 0.0], 6.0, 0.0, glow(0.4)),
                BoxShadow::new([0.0, 0.0], 6.0, 0.0, glow(0.45)),
                BoxShadow::new([0.0, 0.0], 0.5, 0.0, glow(0.4)),
            ],
            // In Carbon, over the button itself; over the rest of the gutter, the gutter's own
            // shadows (its inset highlight left out).
            exo_ui::Theme::Carbon if on_button => [
                BoxShadow::new([0.0, 0.0], 6.0, 0.0, ink(0.2)),
                BoxShadow::new([0.0, 0.0], 6.0, 0.0, ink(0.25)),
                BoxShadow::new([0.0, 0.0], 0.5, 0.0, ink(0.2)),
            ],
            exo_ui::Theme::Carbon => [
                BoxShadow::new([0.0, 0.0], 6.0, 0.0, Color32::from_black_alpha(26)),
                BoxShadow::new([0.0, 2.0], 6.0, 0.0, Color32::from_black_alpha(38)),
                BoxShadow::new([0.0, 0.0], 0.5, 0.0, Color32::from_black_alpha(26)),
            ],
        };
        let circle = exo_ui::pixel_snap(button);
        let mut lifted = painter.clone();
        lifted.multiply_opacity(raised);
        if !focused {
            for shadow in shadows {
                shadow.paint(&lifted, circle, CornerRadius::same(16));
            }
        }
        lifted.rect_filled(
            circle,
            CornerRadius::same(16),
            palette.background_elevated_default,
        );
    }
    // Only the button itself lights the arrow.
    let color = if on_button {
        palette.icon_base_hover
    } else {
        palette.icon_base_default
    };
    exo_ui::paint_icon(
        ui,
        "legacy/DEPRECATED_actionBack",
        exo_ui::pixel_snap(button.shrink(8.0)),
        color,
    );
    // The gutter around it takes the click.
    false
}
