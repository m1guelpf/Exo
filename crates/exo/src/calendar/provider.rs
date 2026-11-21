//! Signing in a calendar account an invite is to: the provider dropdown the invite card's `RSVP`
//! alert and the day view's `Add Calendar` takeover both ask with.

use egui::{Color32, Id, Pos2, Rect, Sense, Ui, Vec2, pos2, vec2};
use exo_ui::{BoxShadow, Family, Font, Menu, TextStyle, motion};

/// `Choose a provider`, `Google Calendar`; there's no `Microsoft`, as Exo is Gmail-only.
const ITEMS: [&str; 2] = ["Choose a provider", "Google Calendar"];
/// The dropdown's value: 14px/20px bold; the down caret 10px after it, 10px wide, 7.4px down the
/// line.
const VALUE: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 20.0);
const CARET_GAP: f32 = 10.0;
const CARET: Vec2 = vec2(10.0, 10.0 * 6.7 / 12.0);
const CARET_TOP: f32 = 7.4;
/// Its list: 18px left of the button and 8px under it, 201px wide.
const MENU_LEFT: f32 = 18.0;
const MENU_BELOW: f32 = 8.0;
const MENU_WIDTH: f32 = 201.0;

/// The provider dropdown's state: whether Google is chosen, and its list while it's open with
/// the row highlighted.
#[derive(Clone, Debug, Default)]
pub(super) struct ProviderPicker {
    pub chosen: bool,
    open: Option<usize>,
}

impl ProviderPicker {
    /// Paints the dropdown with its line's top left at `at`, `value` its text's ink and `caret`
    /// the caret's, and its list under it while open (above everything). Returns its height.
    pub fn show(&mut self, ui: &Ui, id: Id, at: Pos2, value: Color32, caret: Color32) -> f32 {
        let width = VALUE.paint_line(
            ui.painter(),
            at,
            ITEMS[usize::from(self.chosen)],
            value,
            None,
        );
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_downCaret",
            Rect::from_min_size(at + vec2(width + CARET_GAP, CARET_TOP), CARET),
            caret,
        );
        let button = Rect::from_min_size(at, vec2(width + CARET_GAP + CARET.x, VALUE.line_height));
        let response = ui
            .interact(button, id, Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            self.open = if self.open.is_some() {
                None
            } else {
                Some(usize::from(self.chosen))
            };
        }
        if let Some(highlighted) = self.open {
            let shown = Menu {
                items: &ITEMS,
                highlighted,
            }
            .show_at_least(
                ui,
                id.with("menu"),
                pos2(button.left() - MENU_LEFT, button.bottom() + MENU_BELOW),
                MENU_WIDTH,
            );
            if let Some(picked) = shown.clicked {
                self.chosen = picked == 1;
                self.open = None;
            } else if let Some(hovered) = shown.hovered {
                self.open = Some(hovered);
            } else if ui.input(|input| input.pointer.any_released())
                && !response.hovered()
                && !ui.rect_contains_pointer(shown.rect)
            {
                self.open = None;
            }
        }
        VALUE.line_height
    }

    /// Closes its list, if open: whether it was (Escape closes it first).
    pub fn close(&mut self) -> bool {
        self.open.take().is_some()
    }
}

/// `Please choose a provider to sign in as {email}`.
pub(super) fn ask(email: &str) -> String {
    format!("Please choose a provider to sign in as {email}")
}

/// What the takeover's buttons ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TakeoverAction {
    /// `Sign in`, with a provider chosen.
    SignIn,
    /// `No thanks`.
    Dismiss,
}

/// The takeover (`padding: 32px 2rem`): `Add Calendar` (16px/24px, black or white at `.8`, 28px
/// above the rest), the ask (14px/24px at `.55`), the dropdown 16px under it, and 16px under that
/// `Sign in` (disabled until a provider is chosen; 4px margin and `top: 2px`) and `No thanks`
/// (12px/20px at `.3`, `.5` hovered, 1rem after it and 8px down; the button's 28px box puts its
/// text 4px down).
const TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
const TITLE_GAP: f32 = 28.0;
const ASK: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 24.0);
const GAP: f32 = 16.0;
const SIGN_IN_TOP: f32 = 4.0 + 2.0;
const DECLINE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const DECLINE_TOP: f32 = 8.0;
const DECLINE_PADDING: Vec2 = vec2(0.0, 4.0);

/// The purple `Sign in` in Classic: 14px/20px bold uppercase, padded `8px 12px 4px`, 2px
/// corners on `#aeb1dd`, its label white at `.98` (Carbon `#27292d`), the glow `0 0 6px 0
/// #aeb1dd, 0 2px 12px 0` at `.5` hovered; Carbon's glows at `.2` always (`.7` hovered). Disabled,
/// its label is black at `.15`, and Snow's loses its fill (`background: unset`), Carbon's not.
const BUTTON: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 20.0);
const BUTTON_PADDING: Vec2 = vec2(12.0, 8.0);
const ACCENT: Color32 = Color32::from_rgb(0xae, 0xb1, 0xdd);

/// Paints `Sign in` over `rect`; returns whether it was clicked.
fn sign_in_button(ui: &Ui, rect: Rect, enabled: bool) -> bool {
    let carbon = exo_ui::theme(ui.ctx()) == exo_ui::Theme::Carbon;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(
        rect,
        exo_ui::id(ui.ctx(), "calendar-takeover-sign-in"),
        sense,
    );
    let response = if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    let hover = motion::transition(
        ui.ctx(),
        response.id.with("hover"),
        enabled && response.hovered(),
        0.15,
        motion::CubicBezier::EASE,
    );
    let glow = motion::mix(
        Color32::from_rgba_unmultiplied_const(0xae, 0xb1, 0xdd, if carbon { 51 } else { 0 }),
        Color32::from_rgba_unmultiplied_const(0xae, 0xb1, 0xdd, if carbon { 179 } else { 128 }),
        hover,
    );
    let snapped = exo_ui::pixel_snap(rect);
    // Only an enabled button hovers.
    if carbon || hover > 0.0 {
        let near = if carbon { 1.0 } else { hover };
        BoxShadow::new([0.0, 0.0], 6.0, 0.0, ACCENT)
            .with_opacity(near)
            .paint_outside(ui.painter(), snapped);
        BoxShadow::new([0.0, 2.0], 12.0, 0.0, glow).paint_outside(ui.painter(), snapped);
    }
    if enabled || carbon {
        ui.painter().rect_filled(snapped, 2.0, ACCENT);
    }
    let ink = match (enabled, carbon) {
        (false, _) => Color32::from_black_alpha(38),
        (true, false) => Color32::from_rgba_unmultiplied_const(255, 255, 255, 250),
        (true, true) => Color32::from_rgb(0x27, 0x29, 0x2d),
    };
    BUTTON.paint_line(
        ui.painter(),
        rect.min + BUTTON_PADDING,
        "SIGN IN",
        ink,
        None,
    );
    response.clicked()
}

/// Paints the day view's `Add Calendar` for signed-out calendar account `email` over `pane`.
pub(super) fn show_takeover(
    ui: &Ui,
    pane: Rect,
    rem: f32,
    email: &str,
    picker: &mut ProviderPicker,
) -> Option<TakeoverAction> {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter_at(pane);
    let left = pane.left() + 2.0 * rem;
    let width = pane.width() - 4.0 * rem;
    let mut top = pane.top() + 32.0;
    TITLE.paint_line(
        &painter,
        pos2(left, top),
        "Add Calendar",
        palette.ink(0.8, 0.8),
        None,
    );
    top += TITLE.line_height + TITLE_GAP;
    for line in ASK.wrap(ui.ctx(), &ask(email), width) {
        ASK.paint_line(
            &painter,
            pos2(left, top),
            &line,
            palette.ink(0.55, 0.55),
            None,
        );
        top += ASK.line_height;
    }
    top += GAP;
    // The chosen provider: black (Carbon white) at `.75`; the caret black (Carbon white), which
    // draws as at `.5`.
    top += picker.show(
        ui,
        exo_ui::id(ui.ctx(), "calendar-takeover-provider"),
        pos2(left, top),
        palette.ink(0.75, 0.75),
        palette.ink(0.5, 0.5),
    ) + GAP;

    let sign_in = Rect::from_min_size(
        pos2(left, top + SIGN_IN_TOP),
        vec2(
            BUTTON.width(ui.ctx(), "SIGN IN") + 2.0 * BUTTON_PADDING.x,
            exo_ui::DIALOG_ACTION_HEIGHT,
        ),
    );
    let mut action = sign_in_button(ui, sign_in, picker.chosen).then_some(TakeoverAction::SignIn);
    let decline = Rect::from_min_size(
        pos2(sign_in.right() + rem, top + DECLINE_TOP),
        vec2(DECLINE.width(ui.ctx(), "No thanks"), DECLINE.line_height) + 2.0 * DECLINE_PADDING,
    );
    let response = ui
        .interact(
            decline,
            exo_ui::id(ui.ctx(), "calendar-takeover-decline"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let ink = motion::transition(
        ui.ctx(),
        response.id.with("hover"),
        response.hovered(),
        0.15,
        motion::CubicBezier::EASE,
    );
    DECLINE.paint_line(
        &painter,
        decline.min + DECLINE_PADDING,
        "No thanks",
        motion::mix(palette.ink(0.3, 0.3), palette.ink(0.5, 0.5), ink),
        None,
    );
    if response.clicked() {
        action = Some(TakeoverAction::Dismiss);
    }
    action
}
