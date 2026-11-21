//! The app shell around the list: the column layout, the left rail with the Mail/Calendar
//! switcher, and the right pane's footer.

use egui::{
    Color32, CornerRadius, Mesh, Painter, Pos2, Rect, Shape, Stroke, Ui, Vec2, epaint::Vertex,
    pos2, vec2,
};
use exo_ui::{BoxShadow, Family, Font, IconButton, KeyTooltip, Placement, TextStyle, Theme};

/// The left rail's width.
const RAIL_WIDTH: f32 = 40.0;

/// The window's columns below the tab strip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Columns {
    /// The left rail, border included.
    pub rail: Rect,
    /// The content pane: the thread list view.
    pub content: Rect,
    /// The right pane, its left border included.
    pub right: Rect,
}

/// Lays out the columns' flex row in `area` (the window below the tab strip) the way Chromium does:
/// the content pane grows into spare room, and when there is too little, every column shrinks in
/// proportion to its basis until it hits its minimum.
///
/// Beside a popped-out draft the content pane's basis
/// makes room for two columns and the back gutter, and it shrinks no narrower than 720px.
pub fn columns(area: Rect, rem: f32, popped_out: bool) -> Columns {
    let width = area.width();
    let right_basis = exo_ui::right_pane_width(width);
    // The content pane: `flex: 1 1 auto; width: calc(640px + 8rem); min-width: 520px`; popped
    // out `flex: 1 1 calc(2 * (640px + 7rem) + 40px + 4rem); min-width: 720px`.
    let (content_basis, content_min) = if popped_out {
        (2.0 * (640.0 + 7.0 * rem) + 40.0 + 4.0 * rem, 720.0)
    } else {
        (640.0 + 8.0 * rem, 520.0)
    };
    // (basis, minimum) per column.
    let sizes = [
        (RAIL_WIDTH, RAIL_WIDTH),
        (content_basis, content_min),
        (right_basis, 260.0),
    ];
    let free = width - sizes.iter().map(|(basis, _)| basis).sum::<f32>();
    let [rail, content, right] = if free >= 0.0 {
        [RAIL_WIDTH, content_basis + free, right_basis]
    } else {
        shrink(&sizes, width)
    };
    let rect = |left: f32, width: f32| {
        Rect::from_min_size(
            pos2(area.left() + left, area.top()),
            vec2(width, area.height()),
        )
    };
    Columns {
        rail: rect(0.0, rail),
        content: rect(rail, content),
        right: rect(rail + content, right),
    }
}

/// CSS flex shrinking with `flex-shrink: 1` to fit `width`: every column shrinks in proportion
/// to its basis, and one that would go below its minimum is frozen there.
fn shrink(sizes: &[(f32, f32); 3], width: f32) -> [f32; 3] {
    let mut frozen: [Option<f32>; 3] = [None; 3];
    loop {
        let fixed: f32 = frozen.iter().flatten().sum();
        let flexible: f32 = (0..3)
            .filter(|&i| frozen[i].is_none())
            .map(|i| sizes[i].0)
            .sum();
        let deficit = fixed + flexible - width;
        let widths = std::array::from_fn(|i| {
            frozen[i]
                .unwrap_or_else(|| sizes[i].0 - deficit * sizes[i].0 / flexible.max(f32::EPSILON))
        });
        let mut changed = false;
        for i in 0..3 {
            if frozen[i].is_none() && widths[i] < sizes[i].1 {
                frozen[i] = Some(sizes[i].1);
                changed = true;
            }
        }
        if !changed {
            return widths;
        }
    }
}

/// What the user did in the rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailAction {
    /// Clicked Mail.
    Mail,
    /// Clicked Calendar (opens See Your Week).
    Calendar,
}

/// The left rail: the Mail/Calendar switcher, its knob under Calendar while `calendar` (the
/// week view is up). At Inbox Zero (`zero`) it's see-through over the photo, at once: Carbon's
/// border goes (`border-right: 1px solid transparent`), Snow's stays, black at .09.
pub fn rail(ui: &Ui, rect: Rect, zero: bool, calendar: bool) -> Option<RailAction> {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter();
    let border = match (zero, palette.theme) {
        (false, _) => Some(palette.border_base_default),
        (true, Theme::Snow) => Some(Color32::from_black_alpha(23)),
        (true, Theme::Carbon) => None,
    };
    if !zero {
        painter.rect_filled(rect, 0.0, palette.background_base_default);
    }
    if let Some(border) = border {
        painter.vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, border));
    }

    // The switcher: 30px wide, 4px padding, `margin-top: -9px` under the rail's
    // 36px top padding. Centered in the 39px inside the border it lays out at 4.5px, but
    // Chromium paints it (and everything in it) at 5px.
    let switcher = Rect::from_min_size(
        pos2(rect.left() + 5.0, rect.top() + 36.0 - 9.0),
        vec2(30.0, 60.0),
    );
    // Its well: `#f2f4fa`, `rgba(24, 25, 26, .5)` in Carbon, black at .15 over the photo.
    let well = match (zero, exo_ui::theme(ui.ctx())) {
        (true, _) => Color32::from_black_alpha(38),
        (false, Theme::Snow) => Color32::from_rgb(0xf2, 0xf4, 0xfa),
        (false, Theme::Carbon) => Color32::from_rgba_unmultiplied(24, 25, 26, 128),
    };
    painter.rect_filled(switcher, CornerRadius::same(15), well);
    // The knob under the active app slides 29px down to Calendar in 75ms.
    let slide = exo_ui::motion::transition(
        ui.ctx(),
        ui.id().with("rail-knob"),
        calendar,
        0.075,
        exo_ui::motion::CubicBezier::new(0.25, 0.08, 0.01, 0.96),
    );
    let knob = Rect::from_min_size(
        switcher.min + vec2(3.0, 3.0 + 29.0 * slide),
        Vec2::splat(24.0),
    );
    for shadow in [
        BoxShadow::new([0.0, 0.0], 0.5, 0.0, Color32::from_black_alpha(26)),
        BoxShadow::new([0.0, 0.0], 6.0, 0.0, Color32::from_black_alpha(38)),
    ] {
        // Under the photo's see-through knob, only outside it, as CSS clips a box-shadow.
        if zero {
            shadow.paint_outside(painter, knob)
        } else {
            shadow.paint(painter, knob, CornerRadius::same(12))
        }
    }
    // White at .15 over the photo.
    let knob_fill = if zero {
        Color32::from_white_alpha(38)
    } else {
        palette.background_elevated_default
    };
    painter.rect_filled(knob, CornerRadius::same(12), knob_fill);

    let button = |row: f32| {
        Rect::from_min_size(
            switcher.min + vec2(3.0, 4.0 + 28.0 * row),
            Vec2::splat(24.0),
        )
    };
    // The 14×12 legacy glyphs, centered (a `top: -0.5px` nudge doesn't move them at 2×:
    // Chromium paints them on the pixel): `#919295`, `#797a7c` hovered, the active app's
    // `#4c4c4c`; in Carbon `#5d5e5f`, the active one `#c3c4c6`.
    let mut action = None;
    let rows = [
        (
            0.0,
            "legacy/DEPRECATED_emailApp",
            RailAction::Mail,
            !calendar,
            KeyTooltip {
                text: "Mail",
                keys: &["1"],
            },
        ),
        (
            1.0,
            "legacy/DEPRECATED_calendarApp",
            RailAction::Calendar,
            calendar,
            KeyTooltip {
                text: "Calendar",
                keys: &["2"],
            },
        ),
    ];
    // Over the photo, white at .3, .5 hovered and .7 for the active app.
    let white = |alpha: f32| Color32::from_white_alpha((alpha * 255.0).round() as u8);
    let (rest, hovered, active_color) = match (zero, palette.theme) {
        (true, _) => (white(0.3), white(0.5), white(0.7)),
        (false, Theme::Snow) => (
            Color32::from_rgb(0x91, 0x92, 0x95),
            Color32::from_rgb(0x79, 0x7a, 0x7c),
            palette.menu_icon_hover,
        ),
        (false, Theme::Carbon) => (
            Color32::from_rgb(0x5d, 0x5e, 0x5f),
            Color32::from_rgb(0x5d, 0x5e, 0x5f),
            palette.menu_icon_hover,
        ),
    };
    for (row, icon, app, active, tooltip) in rows {
        let hit = button(row);
        let icon_rect = exo_ui::pixel_snap(Rect::from_center_size(hit.center(), vec2(14.0, 12.0)));
        let colors = if active {
            (active_color, active_color)
        } else {
            (rest, hovered)
        };
        let response = IconButton::new(icon).colors(colors.0, colors.1).show(
            ui,
            ui.id().with(("rail", row as u8)),
            hit,
            icon_rect,
        );
        // To the right, vertically centred: 4px up and 8px out.
        tooltip.show_at(ui, &response, Placement::Right(12.0));
        if response.clicked() && !active {
            action = Some(app);
        }
    }
    action
}

/// What the user did in the footer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FooterAction {
    /// Clicked the calendar icon (See Your Day).
    Calendar,
    /// Clicked the gear (settings menu).
    Settings,
}

/// The right pane's actions bar, anchored to the bottom of `pane` (the pane's content, inside
/// its border): the gear and the calendar, and no Refer or Help, as Exo has no team to send them
/// to. The gear carries the new-setting dot while `settings_badge`, and is brand-colored, hovered
/// or not, while `settings_open`. Over Inbox Zero's photo (`zero`) the glyphs are white at .8, 1
/// hovered.
pub fn footer(
    ui: &Ui,
    pane: Rect,
    rem: f32,
    settings_badge: bool,
    settings_open: bool,
    composing: bool,
    zero: bool,
) -> Option<FooterAction> {
    // `padding: 22px 2rem 12px`; the actions `margin-right: -5px`; clickables padded 5px: the
    // gear's (14px; 24px tall) 6px on its left, the calendar's (12px; 22px tall, centred) 7px
    // either side.
    let top = pane.bottom() - 12.0 - 24.0;
    let right = pane.right() - 2.0 * rem + 5.0;
    let mut action = None;
    let buttons = [
        (
            "legacy/DEPRECATED_settings",
            FooterAction::Settings,
            Rect::from_min_max(pos2(right - 25.0, top), pos2(right, top + 24.0)),
            Vec2::splat(14.0),
        ),
        (
            "legacy/DEPRECATED_calendar",
            FooterAction::Calendar,
            Rect::from_min_max(
                pos2(right - 51.0, top + 1.0),
                pos2(right - 25.0, top + 23.0),
            ),
            Vec2::splat(12.0),
        ),
    ];
    for (i, (icon, clicked, button, size)) in buttons.into_iter().enumerate() {
        // Each glyph sits 5px (7px for the calendar) in from its clickable's right edge.
        let inset = if clicked == FooterAction::Calendar {
            7.0
        } else {
            5.0
        };
        let icon_rect = Rect::from_min_size(
            pos2(
                button.right() - inset - size.x,
                button.center().y - size.y / 2.0,
            ),
            size,
        );
        let mut icon_button = IconButton::new(icon);
        if zero {
            icon_button = icon_button.colors(Color32::from_white_alpha(204), Color32::WHITE);
        }
        if clicked == FooterAction::Settings && settings_open {
            // The open menu's gear: `#ce9ece`.
            let on = Color32::from_rgb(0xce, 0x9e, 0xce);
            icon_button = icon_button.colors(on, on);
        }
        let response = icon_button.show(ui, ui.id().with(("footer", i)), button, icon_rect);
        // Above, 6px clear of the button, with no other vertical padding; none while the menu is
        // open.
        if clicked == FooterAction::Settings && !settings_open {
            KeyTooltip {
                text: "Settings",
                keys: &[],
            }
            .show_above(ui, &response, -6.0);
        }
        if clicked == FooterAction::Calendar {
            // The calendar's shortcuts, kept 8px inside the window. While a draft (or the search
            // bar) has the keyboard, the keys it would type don't work, so the rows say what does.
            let arrow = exo_ui::KEYCAP_ARROW;
            let rows = if composing {
                [
                    KeyTooltip {
                        text: "Open Day",
                        keys: &["⌘", "shift", "0"],
                    },
                    KeyTooltip {
                        text: "Open Week",
                        keys: &["⌘", "K", arrow, "Open Week"],
                    },
                    KeyTooltip {
                        text: "Create Event",
                        keys: &["⌘", "K", arrow, "Create Event"],
                    },
                ]
            } else {
                [
                    KeyTooltip {
                        text: "Open Day",
                        keys: &["0"],
                    },
                    KeyTooltip {
                        text: "Open Week",
                        keys: &["2"],
                    },
                    KeyTooltip {
                        text: "Create Event",
                        keys: &["B"],
                    },
                ]
            };
            let within =
                egui::Rangef::new(f32::NEG_INFINITY, ui.ctx().content_rect().right() - 8.0);
            exo_ui::ShortcutTooltip {
                rows: &rows,
                note: None,
            }
            .show_calendar_above(ui, &response, -6.0, within);
        }
        if response.clicked() {
            action = Some(clicked);
        }
        if clicked == FooterAction::Settings && settings_badge {
            // The dot sits 2px over the 14px gear's top right corner (`top: -2px; right: -2px`).
            settings_dot(
                ui,
                Rect::from_min_size(icon_rect.min + vec2(10.0, -2.0), Vec2::splat(6.0)),
                zero,
            );
        }
    }
    action
}

/// The footer's sync status: 12px/16px.
const SYNC_STATUS: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);

/// The footer's sync status, floating with no team button beside it: `text` at the actions bar's
/// left, `bottom: 27px` above the pane's bottom, black or white at .3, faded to `opacity`.
pub fn sync_status(painter: &Painter, pane: Rect, rem: f32, text: &str, opacity: f32) {
    let color = exo_ui::palette(painter.ctx())
        .ink(0.3, 0.3)
        .gamma_multiply(opacity);
    SYNC_STATUS.paint_line(
        painter,
        pos2(
            pane.left() + 2.0 * rem,
            pane.bottom() - 27.0 - SYNC_STATUS.line_height,
        ),
        text,
        color,
        None,
    );
}

/// The new-setting dot: a 6px circle filled with `linear-gradient(287.85deg, #b5aeff 20.94%,
/// #e6a0fe 79.06%)` inside a 1px outline, `hsl(216, 100%, 99%)`, or `#18191a` in Carbon but
/// not at Inbox Zero (`zero`).
fn settings_dot(ui: &Ui, rect: Rect, zero: bool) {
    let painter = ui.painter();
    let outline = match (zero, exo_ui::theme(ui.ctx())) {
        (true, _) | (false, Theme::Snow) => Color32::from_rgb(0xfa, 0xfc, 0xff),
        (false, Theme::Carbon) => Color32::from_rgb(0x18, 0x19, 0x1a),
    };
    painter.circle_filled(rect.center(), rect.width() / 2.0 + 1.0, outline);
    let angle = 287.85_f32.to_radians();
    // CSS angles point up at 0deg and turn clockwise; the gradient line spans the box's
    // projection onto that direction.
    let direction = vec2(angle.sin(), -angle.cos());
    let length = rect.width() * angle.sin().abs() + rect.height() * angle.cos().abs();
    let (from, to) = (
        Color32::from_rgb(0xb5, 0xae, 0xff),
        Color32::from_rgb(0xe6, 0xa0, 0xfe),
    );
    let color_at = |point: Pos2| {
        let t = (point - rect.center()).dot(direction) / length + 0.5;
        let t = ((t - 0.2094) / (0.7906 - 0.2094)).clamp(0.0, 1.0);
        exo_ui::motion::mix(from, to, t)
    };
    let mut mesh = Mesh::default();
    let center = rect.center();
    mesh.vertices.push(Vertex {
        pos: center,
        uv: egui::epaint::WHITE_UV,
        color: color_at(center),
    });
    const SEGMENTS: u32 = 32;
    for i in 0..=SEGMENTS {
        let theta = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let point = center + rect.width() / 2.0 * vec2(theta.cos(), theta.sin());
        mesh.vertices.push(Vertex {
            pos: point,
            uv: egui::epaint::WHITE_UV,
            color: color_at(point),
        });
        if i > 0 {
            mesh.add_triangle(0, i, i + 1);
        }
    }
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(window: f32) -> [f32; 3] {
        let columns = columns(
            Rect::from_min_size(Pos2::ZERO, vec2(window, 800.0)),
            exo_ui::rem(window, false),
            false,
        );
        [
            columns.rail.width(),
            columns.content.width(),
            columns.right.width(),
        ]
    }

    #[test]
    fn content_takes_the_spare_room() {
        // At 1440px: 40 + 1047.64 + 352.36.
        let [rail, content, right] = widths(1440.0);
        assert_eq!(rail, 40.0);
        assert!((right - 352.368).abs() < 0.01, "{right}");
        assert!((content - 1047.632).abs() < 0.01, "{content}");
    }

    #[test]
    fn narrow_windows_shrink_in_proportion() {
        let [rail, content, right] = widths(1024.0);
        assert_eq!(rail, 40.0);
        assert!((rail + content + right - 1024.0).abs() < 0.01);
        assert!(content >= 520.0 && right >= 260.0);
    }

    #[test]
    fn a_popped_out_draft_widens_the_content_pane() {
        // At 1280px with a draft popped out: 40 + 980 + 260.
        let columns = columns(
            Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0)),
            exo_ui::rem(1280.0, true),
            true,
        );
        assert!(
            (columns.content.width() - 980.0).abs() < 0.01,
            "{:?}",
            columns.content
        );
        assert_eq!(columns.right.width(), 260.0);
    }
}
