//! The Images dialog: which remote images message bodies load. A line of explanation, three
//! radio buttons, then how to show a message's blocked images. Choosing one applies it at once;
//! there's no Save.

use egui::{Color32, Context, CursorIcon, Rect, Sense, Ui, pos2, vec2};
use exo_store::RemoteContentBlocking;
use exo_ui::{Dialog, DialogResponse, Family, Font, TextStyle, paint_radio};

use super::Action;

/// The subheader: 12px/20px lines, `padding-top: 24px`, 3px down (`top: 3px`).
const PADDING_TOP: f32 = 24.0;
const NUDGE: f32 = 3.0;
const LINE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
/// The radio rows follow the explanation and a `<br>`.
const ROWS_TOP: f32 = 2.0 * 20.0;
/// The footer, inside the subheader: `padding-top: 20px` and another 3px down, around the
/// keycaps' 21px line.
const FOOTER_PADDING: f32 = 20.0;
const FOOTER_LINE: f32 = 21.0;
const BODY_HEIGHT: f32 = PADDING_TOP + ROWS_TOP + 3.0 * 20.0 + FOOTER_PADDING + FOOTER_LINE;
/// The 12px radio, 3px below its line's top and 7px before its label.
const RADIO: f32 = 12.0;
const RADIO_TOP: f32 = 3.0;
const RADIO_GAP: f32 = 7.0;
/// The subheader's `rgba(0, 0, 0, .85)`.
const LABEL_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 217);
const SUBHEADER: &str = "Choose which images to display.";
/// The radio rows, in order.
const CHOICES: [(&str, RemoteContentBlocking); 3] = [
    ("Show all images", RemoteContentBlocking::None),
    (
        "Block all known tracking pixels",
        RemoteContentBlocking::ReduceTracking,
    ),
    ("Block all remote images", RemoteContentBlocking::All),
];

/// Draws the dialog over `area` with `current` checked; a click on another choice sets it.
pub fn dialog(
    ctx: &Context,
    area: Rect,
    open: bool,
    current: RemoteContentBlocking,
) -> DialogResponse<Option<Action>> {
    Dialog::new("images", "Images", BODY_HEIGHT)
        .show(ctx, area, open, |ui, body| picker(ui, body, current))
}

fn picker(ui: &mut Ui, body: Rect, current: RemoteContentBlocking) -> Option<Action> {
    let painter = ui.painter().clone();
    let top = body.top() + NUDGE + PADDING_TOP;
    LINE.paint_line(&painter, pos2(body.left(), top), SUBHEADER, LABEL_INK, None);
    let mut chosen = None;
    for (i, (label, choice)) in CHOICES.into_iter().enumerate() {
        let line_top = top + ROWS_TOP + i as f32 * LINE.line_height;
        paint_radio(
            &painter,
            pos2(body.left(), line_top + RADIO_TOP),
            choice == current,
        );
        LINE.paint_line(
            &painter,
            pos2(body.left() + RADIO + RADIO_GAP, line_top),
            label,
            LABEL_INK,
            None,
        );
        // The radio and its label take the click, with a pointer cursor.
        let hit = Rect::from_min_size(
            pos2(body.left(), line_top),
            vec2(
                RADIO + RADIO_GAP + LINE.width(ui.ctx(), label),
                LINE.line_height,
            ),
        );
        let response = ui
            .interact(hit, ui.id().with(("images-choice", i)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.clicked() && choice != current {
            chosen = Some(Action::SetImages(choice));
        }
    }
    // `To show blocked images in a message, hit ⌘ K → Show Images.`
    let footer_top = top + ROWS_TOP + 3.0 * LINE.line_height + NUDGE + FOOTER_PADDING;
    super::paint_dialog_footer(
        ui,
        body.left(),
        footer_top,
        "To show blocked images in a message, hit",
        exo_ui::KEYCAP_GAP,
        0.0,
        " → Show Images.",
    );
    chosen
}
