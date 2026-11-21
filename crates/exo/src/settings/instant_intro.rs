//! The Instant Intro dialog: the line Instant Intro starts a reply with
//! (`customInstantIntro`), which must name `{first_name}` and nothing else in braces. Save checks
//! it (`Sorry!`); Reset puts the default line back in the field, after asking.
//!
//! Metrics at a 1280×800 window.

use egui::{Color32, Context, Rect, Sense, Ui, pos2, vec2};
use exo_store::Settings as AccountSettings;
use exo_ui::{
    ActionKind, Alert, DIALOG_ACTION_HEIGHT, Dialog, DialogAction, DialogInput, DialogResponse,
    Family, Font, KeycapColors, Keycaps, TextStyle,
};

/// 12px/20px text, 3px down (`top: 3px`); the input bold.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const INPUT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);
const NUDGE: f32 = 3.0;
/// From the body's top: the subheader's line (`padding-top: 24px`), the input's (the
/// subheader's 28px margin under it), the footer's (`padding-top: 28px` under the
/// input), and the buttons' (`margin-top: 32px` under the footer, 3px down).
const SUBHEADER_TOP: f32 = 24.0;
const INPUT_TOP: f32 = 72.0;
const FOOTER_TOP: f32 = 120.0;
const ACTIONS_TOP: f32 = 172.0;
/// The body: the buttons' 28px row and its 7px margin.
const BODY_HEIGHT: f32 = 207.0;
/// The subheader's `rgba(0, 0, 0, .85)`; the footer's `.6`.
const INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 217);
const FOOTER_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 153);
/// The footer's keycaps: 19px caps 4px apart, 1px above the line; the period after them
/// `padding-left: 1px`.
const KEY_HEIGHT: f32 = 19.0;
/// Their labels' baseline, 14px down the cap (the footer's 20px line, measured).
const KEY_BASELINE: f32 = 14.0;
const SUBHEADER: &str =
    "Do you get lots of introductions? Use Instant Intro to instantly move senders to BCC!";
const FOOTER: &str = "To use Instant Intro, open a message and hit ";
const PLACEHOLDER: &str = "Try: Thanks {first_name}! (to BCC)";
/// The most characters the input takes.
const MAX_LENGTH: usize = 5000;

/// What has the keyboard: the input (`tabIndex: 0`), then Reset (1, while it's enabled), Cancel
/// (2) and Save (3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Input,
    Reset,
    Cancel,
    Save,
}

/// Something done in the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Tab (1) or ⇧Tab (-1).
    Tab(isize),
    /// Enter: in the input, Save; on a button, that button.
    Enter,
    /// A button, clicked.
    Click(Focus),
    /// A click in the input.
    FocusInput,
    /// The alert's action, by index (its first rightmost).
    AlertAction(usize),
    /// Enter or Space in the alert: its selected action.
    AlertConfirm,
    /// ← / → in the alert.
    AlertCycle(isize),
    /// Escape in the alert.
    AlertDismiss,
}

/// The alert up over the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Asking {
    /// `Sorry!`: what's wrong with the line, `OK`.
    Invalid(&'static str),
    /// `Reset Intro`: `OK` or `Cancel`.
    Reset,
}

/// The dialog's line as typed, and what has the keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    text: String,
    /// The `Sorry!` alert's OK marks the line red until it's edited.
    invalid: bool,
    focus: Focus,
    focus_pending: bool,
    alert: Option<(Asking, usize)>,
}

/// What a step of the dialog asks of the app.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Save this line (`customInstantIntro`) and close.
    pub save: Option<String>,
    /// Close.
    pub close: bool,
}

impl State {
    /// The dialog as it opens: the saved line, or the default one, the input with the
    /// keyboard and Save selected.
    pub fn open(settings: &AccountSettings) -> Self {
        Self {
            text: settings.instant_intro().to_owned(),
            invalid: false,
            focus: Focus::Input,
            focus_pending: true,
            alert: None,
        }
    }

    /// An alert is up: it has the keyboard.
    pub fn alert_open(&self) -> bool {
        self.alert.is_some()
    }

    fn is_default(&self) -> bool {
        self.text == AccountSettings::INSTANT_INTRO
    }

    fn order(&self) -> Vec<Focus> {
        let mut order = vec![Focus::Input];
        if !self.is_default() {
            order.push(Focus::Reset);
        }
        order.extend([Focus::Cancel, Focus::Save]);
        order
    }

    fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
        self.focus_pending = focus == Focus::Input;
    }

    /// Carries out `event`.
    pub fn handle(&mut self, event: Event) -> Outcome {
        match event {
            Event::Tab(delta) => {
                let order = self.order();
                let at = order
                    .iter()
                    .position(|focus| *focus == self.focus)
                    .unwrap_or(0);
                self.set_focus(
                    order[(at as isize + delta).rem_euclid(order.len() as isize) as usize],
                );
            }
            Event::Enter => {
                return self.handle(Event::Click(if self.focus == Focus::Input {
                    Focus::Save
                } else {
                    self.focus
                }));
            }
            Event::Click(Focus::Save) => match validate(&self.text) {
                Some(text) => self.alert = Some((Asking::Invalid(text), 0)),
                None => {
                    return Outcome {
                        save: Some(self.text.clone()),
                        close: true,
                    };
                }
            },
            Event::Click(Focus::Cancel) => {
                return Outcome {
                    close: true,
                    ..Outcome::default()
                };
            }
            Event::Click(Focus::Reset) if !self.is_default() => {
                self.alert = Some((Asking::Reset, 0))
            }
            Event::Click(_) => {}
            Event::FocusInput => self.set_focus(Focus::Input),
            Event::AlertAction(index) => match self.alert.take() {
                // `OK` refocuses the input and marks it.
                Some((Asking::Invalid(_), _)) => {
                    self.invalid = true;
                    self.set_focus(Focus::Input);
                }
                Some((Asking::Reset, _)) => {
                    if index == 0 {
                        AccountSettings::INSTANT_INTRO.clone_into(&mut self.text);
                        self.invalid = false;
                    }
                    if self.focus == Focus::Reset {
                        self.set_focus(Focus::Input);
                    }
                }
                None => {}
            },
            Event::AlertConfirm => {
                if let Some((_, selected)) = &self.alert {
                    return self.handle(Event::AlertAction(*selected));
                }
            }
            Event::AlertCycle(step) => {
                if let Some((asking, selected)) = &mut self.alert {
                    let last = if *asking == Asking::Reset { 1 } else { 0 };
                    *selected = selected.saturating_add_signed(step).min(last);
                }
            }
            Event::AlertDismiss => {
                if let Some((asking, _)) = self.alert.take() {
                    // Escape in `Sorry!` is its only action, `OK`.
                    if asking != Asking::Reset {
                        self.invalid = true;
                        self.set_focus(Focus::Input);
                    }
                }
            }
        }
        Outcome::default()
    }
}

/// What's wrong with `text`, in the alert's words; their stray spaces and mixed quotes are meant.
fn validate(text: &str) -> Option<&'static str> {
    if text.is_empty() {
        return Some(
            "You can't have an empty Instant Intro. \n \n Please try \"Thanks {first_name}! (to BCC)\"",
        );
    }
    let variables: Vec<String> = variables(text).map(str::to_lowercase).collect();
    if !variables.iter().any(|variable| variable == "{first_name}") {
        Some(
            "You must include {first_name} in your Instant Intro. \n \n  For example: “Thanks {first_name}! (to BCC)\"",
        )
    } else if variables.iter().any(|variable| variable != "{first_name}") {
        Some(
            "Your Instant Intro can only contain {first_name}. \n \n  For example: “Thanks {first_name}! (to BCC)”",
        )
    } else {
        None
    }
}

/// The `{…}`s in `text` (`/\{([^}]+)\}/gi`).
fn variables(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        loop {
            let open = rest.find('{')?;
            let after = &rest[open + 1..];
            let close = after.find('}')?;
            let (start, end) = (open, open + 1 + close + 1);
            let found = &rest[start..end];
            rest = &rest[end..];
            if close > 0 {
                return Some(found);
            }
        }
    })
}

/// Draws the dialog over `area`, and its alert.
pub fn dialog(
    ctx: &Context,
    area: Rect,
    open: bool,
    state: &mut State,
) -> DialogResponse<Option<Event>> {
    let mut response = Dialog::new("instant-intro", "Instant Intro", BODY_HEIGHT).show(
        ctx,
        area,
        open,
        |ui, body| show(ui, body, state),
    );
    if let (Some(event), Some((asking, selected))) = (&mut response.inner, &state.alert) {
        let (title, text, actions): (&str, &str, &[&str]) = match asking {
            Asking::Invalid(text) => ("Sorry!", text, &["OK"]),
            Asking::Reset => (
                "Reset Intro",
                "This will reset your Instant Intro to: \n \n \"Thanks {first_name}! (to BCC)\"",
                &["OK", "Cancel"],
            ),
        };
        let alert = Alert {
            title,
            text,
            actions,
            selected: *selected,
        };
        let clicked = egui::Area::new(exo_ui::id(ctx, "instant-intro-alert"))
            .order(egui::Order::Tooltip)
            .fixed_pos(area.min)
            .show(ctx, |ui| {
                let backdrop = ui.allocate_rect(area, Sense::click());
                ui.painter()
                    .rect_filled(area, 0.0, Color32::from_black_alpha(128));
                match alert.show(ui, area) {
                    Some(index) => Some(Event::AlertAction(index)),
                    None => backdrop.clicked().then_some(Event::AlertDismiss),
                }
            });
        *event = clicked.inner.or(event.take());
        response.close = false;
    }
    response
}

fn show(ui: &mut Ui, body: Rect, state: &mut State) -> Option<Event> {
    let ctx = ui.ctx().clone();
    let painter = ui.painter().clone();
    let left = body.left();
    let mut event = None;
    TEXT.paint_line(
        &painter,
        pos2(left, body.top() + SUBHEADER_TOP + NUDGE),
        SUBHEADER,
        INK,
        None,
    );

    let id = exo_ui::id(&ctx, "instant-intro-input");
    let line = Rect::from_min_size(
        pos2(left, body.top() + INPUT_TOP + NUDGE),
        vec2(body.width(), TEXT.line_height),
    );
    let before = state.text.clone();
    let response = DialogInput::new(INPUT, PLACEHOLDER)
        .error(state.invalid)
        .show(ui, id, &mut state.text, line);
    if state.text != before {
        if state.text.chars().count() > MAX_LENGTH {
            state.text = state.text.chars().take(MAX_LENGTH).collect();
        }
        state.invalid = false;
    }
    if state.focus == Focus::Input && std::mem::take(&mut state.focus_pending) {
        let end = egui::text::CCursor::new(state.text.chars().count());
        exo_ui::focus_field(&ctx, id, egui::text::CCursorRange::one(end));
    }
    if response.gained_focus() && state.focus != Focus::Input {
        event = Some(Event::FocusInput);
    } else if state.focus != Focus::Input && response.has_focus() {
        response.surrender_focus();
    }

    // `To use Instant Intro, open a message and hit ⌘ shift i.`
    let footer = body.top() + FOOTER_TOP + NUDGE;
    // The caps' `margin-left: 4px` after the text, its trailing space laid out as nothing.
    let x = left
        + TEXT.paint_line(
            &painter,
            pos2(left, footer),
            FOOTER.trim_end(),
            FOOTER_INK,
            None,
        );
    let keys = Keycaps::new(
        &ctx,
        &["⌘", "shift", "i"],
        KeycapColors::shortcuts_pane(exo_ui::app_theme(&ctx).palette()),
    )
    .height(KEY_HEIGHT)
    .baseline(KEY_BASELINE);
    keys.paint(&painter, x, footer - 1.0);
    TEXT.paint_line(
        &painter,
        pos2(x + keys.width() + 1.0, footer),
        ".",
        FOOTER_INK,
        None,
    );

    // Reset at the left (12px out), Cancel and Save at the right (13px out), 8px apart; Save
    // selected unless a button has the keyboard.
    let top = body.top() + ACTIONS_TOP + NUDGE;
    let selected = if state.focus == Focus::Input {
        Focus::Save
    } else {
        state.focus
    };
    let mut right = body.right() + 13.0;
    for (focus, label) in [(Focus::Save, "Save"), (Focus::Cancel, "Cancel")] {
        // Each is filled (and glowing) only while it's the one selected.
        let action = DialogAction::new(label, ActionKind::Plain).focused(selected == focus);
        let width = action.width(&ctx);
        let rect = Rect::from_min_size(pos2(right - width, top), vec2(width, DIALOG_ACTION_HEIGHT));
        if action
            .show(
                ui,
                exo_ui::id(ui.ctx(), ("instant-intro-action", label)),
                rect,
            )
            .clicked()
        {
            event = Some(Event::Click(focus));
        }
        right = rect.left() - 8.0;
    }
    let reset = DialogAction::new("Reset", ActionKind::Plain)
        .enabled(!state.is_default())
        .focused(selected == Focus::Reset);
    let rect = Rect::from_min_size(
        pos2(left - 12.0, top),
        vec2(reset.width(&ctx), DIALOG_ACTION_HEIGHT),
    );
    if reset
        .show(
            ui,
            exo_ui::id(ui.ctx(), ("instant-intro-action", "Reset")),
            rect,
        )
        .clicked()
    {
        event = Some(Event::Click(Focus::Reset));
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_must_name_first_name_and_nothing_else() {
        assert!(validate("Thanks {first_name}! (to BCC)").is_none());
        assert!(
            validate("Merci {FIRST_NAME}").is_none(),
            "case doesn't matter"
        );
        assert!(
            validate("")
                .is_some_and(|text| text.starts_with("You can't have an empty Instant Intro."))
        );
        assert!(
            validate("Thanks!")
                .is_some_and(|text| text.starts_with("You must include {first_name}"))
        );
        assert!(
            validate("Thanks {first_name} and {last_name}").is_some_and(
                |text| text.starts_with("Your Instant Intro can only contain {first_name}.")
            )
        );
        assert!(
            validate("Thanks {} {first_name}").is_none(),
            "empty braces aren't a variable"
        );
    }

    #[test]
    fn save_reset_and_the_tab_order() {
        let mut state = State::open(&AccountSettings::default());
        assert_eq!(state.text, "Thanks {first_name}! (to BCC)");
        state.handle(Event::Tab(1));
        assert_eq!(
            state.focus,
            Focus::Cancel,
            "Reset is disabled on the default line"
        );
        state.text = "Hi {first_name}".into();
        state.handle(Event::Tab(-1));
        assert_eq!(state.focus, Focus::Reset, "enabled, it's before Cancel");
        state.handle(Event::Enter);
        state.handle(Event::AlertAction(0));
        assert_eq!(
            (state.text.as_str(), state.focus),
            ("Thanks {first_name}! (to BCC)", Focus::Input)
        );
        state.text = "Bye".into();
        assert_eq!(state.handle(Event::Enter), Outcome::default());
        assert!(state.alert_open());
        state.handle(Event::AlertAction(0));
        assert!(state.invalid && !state.alert_open());
        state.text = "Bye {first_name}".into();
        assert_eq!(
            state.handle(Event::Enter),
            Outcome {
                save: Some("Bye {first_name}".into()),
                close: true
            }
        );
    }
}
