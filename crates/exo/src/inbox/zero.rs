//! Inbox Zero's button: `You've hit Inbox Zero!`, or the weekly streak, white over the photo at the
//! bottom left; it opens the Inbox Zero achievements. The app paints the photo and hides the right
//! pane.

use egui::{CursorIcon, Rect, Sense, Ui, Vec2, pos2};
use exo_ui::{Family, Font, Placement, TextStyle, motion, paint_icon};

/// The button's copy: 12px, on a 16px line.
const COPY: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);

/// The button's copy for a weekly streak of `streak`.
fn copy(streak: u32) -> String {
    if streak > 1 {
        format!("You've hit Inbox Zero {streak} weeks in a row!")
    } else {
        "You've hit Inbox Zero!".to_owned()
    }
}

/// Paints the Inbox Zero button for the content pane `content` (the window right of the rail),
/// unless the streak animation has the window (`hidden`). Returns whether it was clicked.
pub(super) fn show(
    ui: &Ui,
    content: Rect,
    rem: f32,
    compact: bool,
    streak: u32,
    hidden: bool,
) -> bool {
    let id = ui.id().with("share-inbox-zero");
    // Hidden, it's gone at once and comes back over 150ms (`ease`); and below 600px wide it's gone.
    let opacity = motion::transition_in(
        ui.ctx(),
        id.with("shown"),
        !hidden,
        0.15,
        motion::CubicBezier::EASE,
    );
    if hidden || ui.ctx().content_rect().width() <= 600.0 {
        return false;
    }
    // The button's box: at the bottom, 6px up, `calc(3.5rem + 40px - 20px + 6px)` (70px at ≤1152px)
    // from the window's edge, then the clickable's padding: `10px 18px 10px 20px`.
    let left = content.left() - 40.0 + if compact { 70.0 } else { 3.5 * rem + 26.0 } + 20.0;
    let top = content.bottom() - 6.0 - 10.0 - COPY.line_height;
    let text = copy(streak);
    let width = COPY
        .single_line(ui.painter(), &text, egui::Color32::PLACEHOLDER, None)
        .size()
        .x;
    let icon_width =
        exo_ui::icon_size(ui.ctx(), "legacy/DEPRECATED_flame").map_or(16.0, |size| size.x);
    let clickable = Rect::from_min_max(
        pos2(left - 20.0, top - 10.0),
        pos2(
            left + width + 7.0 + icon_width + 18.0,
            top + COPY.line_height + 10.0,
        ),
    );
    let response = ui
        .interact(clickable, id, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    // `hsla(0,0%,100%,.8)`, `.98` hovered (over 150ms).
    let hover = motion::transition(
        ui.ctx(),
        id.with("hover"),
        response.hovered(),
        0.15,
        motion::CubicBezier::EASE,
    );
    let color =
        egui::Color32::from_white_alpha(((0.8 + 0.18 * hover) * opacity * 255.0).round() as u8);
    COPY.paint_line(ui.painter(), pos2(left, top), &text, color, None);
    // The flame: at its own size, its bottom on the text's baseline, `margin-left: 7px; top: 3px`,
    // in the text's color.
    let name = "legacy/DEPRECATED_flame";
    let size = exo_ui::icon_size(ui.ctx(), name).unwrap_or(Vec2::splat(16.0));
    let bottom = top + COPY.baseline(ui.ctx()) + 3.0;
    let icon = exo_ui::pixel_snap(Rect::from_min_size(
        pos2(left + width + 7.0, bottom - size.y),
        size,
    ));
    paint_icon(ui, name, icon, color);
    // `Achievements` and a 🎉 5px after it, above.
    exo_ui::KeyTooltip {
        text: "Achievements 🎉",
        keys: &[],
    }
    .show_at(ui, &response, Placement::Above(0.0));
    response.clicked()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_copy_counts_weeks_once_the_streak_is_longer_than_one() {
        assert_eq!(super::copy(0), "You've hit Inbox Zero!");
        assert_eq!(super::copy(1), "You've hit Inbox Zero!");
        assert_eq!(super::copy(3), "You've hit Inbox Zero 3 weeks in a row!");
    }
}
