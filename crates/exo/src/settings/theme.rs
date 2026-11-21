//! The Theme dialog: three radio buttons, then a line about the dark theme. Choosing one applies
//! it at once; there's no Save. Snow and Carbon read `Light` and `Dark` here.

use egui::{Color32, Context, CursorIcon, Rect, Sense, Ui, pos2, vec2};
use exo_store::Theme;
use exo_ui::{Dialog, DialogResponse, Family, Font, TextStyle, paint_radio};

use super::Action;

/// The subheader's `padding-top: 20px` over its 12px/20px lines,
/// which sit 3px down (`top: 3px`).
const PADDING_TOP: f32 = 20.0;
const NUDGE: f32 = 3.0;
const LINE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
/// The footer's `padding: 17px 0 2px`, under the three lines; like every dialog footer, it
/// sits another 3px down.
const FOOTER_PADDING: [f32; 2] = [17.0, 2.0];
const BODY_HEIGHT: f32 = PADDING_TOP + 3.0 * 20.0 + FOOTER_PADDING[0] + 20.0 + FOOTER_PADDING[1];
/// The 12px radio sits on the line's baseline, `top: 2px`, 7px before its label; that's 3px
/// below the line's top.
const RADIO: f32 = 12.0;
const RADIO_TOP: f32 = 3.0;
const RADIO_GAP: f32 = 7.0;
/// The subheader's `rgba(0, 0, 0, .85)` and the footer's `rgba(0, 0, 0, .6)`.
const LABEL_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 217);
const FOOTER_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 153);
const FOOTER: &str =
    "The Dark theme can help reduce eyestrain and is best used in low light conditions.";

/// Draws the dialog over `area` with `current` checked; a click on a choice sets it.
pub fn dialog(
    ctx: &Context,
    area: Rect,
    open: bool,
    current: Theme,
) -> DialogResponse<Option<Action>> {
    let os = ctx.os();
    Dialog::new("theme", "Theme", BODY_HEIGHT)
        .show(ctx, area, open, |ui, body| picker(ui, body, current, os))
}

fn picker(
    ui: &mut Ui,
    body: Rect,
    current: Theme,
    os: egui::os::OperatingSystem,
) -> Option<Action> {
    let painter = ui.painter().clone();
    let match_os = format!("Match {}", super::os_name(os));
    let choices = [
        ("Light", Theme::Snow),
        ("Dark", Theme::Carbon),
        (match_os.as_str(), Theme::Auto),
    ];
    let top = body.top() + NUDGE + PADDING_TOP;
    let mut chosen = None;
    for (i, (label, theme)) in choices.into_iter().enumerate() {
        let line_top = top + i as f32 * LINE.line_height;
        paint_radio(
            &painter,
            pos2(body.left(), line_top + RADIO_TOP),
            theme == current,
        );
        let text_left = body.left() + RADIO + RADIO_GAP;
        LINE.paint_line(&painter, pos2(text_left, line_top), label, LABEL_INK, None);
        // The radio and its label take the click, with a pointer cursor.
        let hit = Rect::from_min_size(
            pos2(body.left(), line_top),
            vec2(
                RADIO + RADIO_GAP + LINE.width(ui.ctx(), label),
                LINE.line_height,
            ),
        );
        let response = ui
            .interact(hit, ui.id().with(("theme-choice", i)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.clicked() && theme != current {
            chosen = Some(Action::SetTheme(theme));
        }
    }
    let footer_top = top + 3.0 * LINE.line_height + NUDGE + FOOTER_PADDING[0];
    LINE.paint_line(
        &painter,
        pos2(body.left(), footer_top),
        FOOTER,
        FOOTER_INK,
        None,
    );
    chosen
}
