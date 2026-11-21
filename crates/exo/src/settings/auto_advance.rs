//! The Auto Advance dialog: where triaging a conversation goes next. A choice applies at once.
//!
//! The dropdown has the keyboard from the start and lists its choice first, then the others by
//! name. Metrics at a 1280×800 window; in Carbon the card keeps Snow's colours.

use egui::{Color32, Context, Rect, Sense, Ui, pos2, vec2};
use exo_store::AutoAdvance;
use exo_ui::{Dialog, DialogResponse, Family, Font, Menu, TextStyle};

/// The subheader and the dropdown's label: 12px/20px, 3px down their lines.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
/// Below the body's top: the subheader's line (`padding-top: 24px`), the label's
/// (`margin-bottom: 16px`, the row's `padding: 4px 0`) and the dropdown's value; the body ends
/// with the row.
const SUBHEADER_TOP: f32 = 3.0 + 24.0;
const LABEL_TOP: f32 = 44.0 + 16.0 + 4.0 + 3.0;
const VALUE_TOP: f32 = 69.0;
const BODY_HEIGHT: f32 = 88.0;
/// The label's `margin-right: 4px`, and the value 1px into its button.
const VALUE_LEFT: f32 = 5.0;
/// The subheader `rgba(0,0,0,.85)`; the dropdown's label `.7`.
const SUBHEADER_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 217);
const LABEL_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 179);
/// The open list: 18px left of the value and 26px under the button.
const MENU_OFFSET: egui::Vec2 = vec2(-18.0, 26.0);
const SUBHEADER: &str =
    "Choose where you go after triaging a conversation (e.g. Mark Done, Remind Me, Trash).";
const LABEL: &str = "After triaging a conversation:";

/// Each choice's name, in order.
const CHOICES: [(AutoAdvance, &str); 3] = [
    (AutoAdvance::Down, "go down and show the older conversation"),
    (AutoAdvance::Up, "go up and show the newer conversation"),
    (AutoAdvance::Off, "go back to the list of conversations"),
];

/// Something done in the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A click on the dropdown, or Enter on it: the list opens or closes.
    ToggleMenu,
    /// ↓ (1) or ↑ (-1): the list opens, or its highlight moves round.
    Move(isize),
    /// Enter or Tab in the open list: its highlighted row.
    Enter,
    /// Escape, or a click elsewhere, with the list open: it closes.
    MenuClose,
    /// The pointer on a row of the list.
    Highlight(usize),
    /// A choice.
    Choose(AutoAdvance),
}

/// The open list's highlighted row, between frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    menu: Option<usize>,
}

impl State {
    /// Whether the list is open: its keys are the dialog's.
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Applies `event` with `current` chosen; returns a new choice to save.
    pub fn handle(&mut self, event: Event, current: AutoAdvance) -> Option<AutoAdvance> {
        let count = CHOICES.len() as isize;
        match event {
            // The list opens on the choice, which it lists first.
            Event::ToggleMenu => self.menu = if self.menu.is_some() { None } else { Some(0) },
            Event::Move(step) => {
                self.menu = Some(
                    self.menu
                        .map_or(0, |at| (at as isize + step).rem_euclid(count) as usize),
                )
            }
            Event::Enter => {
                let at = self.menu.take()?;
                return Some(order(current)[at].0).filter(|chosen| *chosen != current);
            }
            Event::MenuClose => self.menu = None,
            Event::Highlight(row) => self.menu = Some(row),
            Event::Choose(chosen) => {
                self.menu = None;
                return (chosen != current).then_some(chosen);
            }
        }
        None
    }
}

/// The choices as the list shows them: `current` first, then the rest by name.
fn order(current: AutoAdvance) -> Vec<(AutoAdvance, &'static str)> {
    let mut choices = CHOICES.to_vec();
    choices.sort_by_key(|(choice, name)| (*choice != current, *name));
    choices
}

/// The name of `current`, the first choice's when it has none.
fn name(current: AutoAdvance) -> &'static str {
    CHOICES
        .iter()
        .find(|(choice, _)| *choice == current)
        .map_or(CHOICES[0].1, |(_, name)| name)
}

/// Draws the dialog over `area` while `open` (and as it fades); returns what was done.
pub fn dialog(
    ctx: &Context,
    area: Rect,
    open: bool,
    state: &State,
    current: AutoAdvance,
) -> DialogResponse<Option<Event>> {
    Dialog::new("auto-advance", "Auto Advance", BODY_HEIGHT)
        .show(ctx, area, open, |ui, body| draw(ui, body, state, current))
}

fn draw(ui: &mut Ui, body: Rect, state: &State, current: AutoAdvance) -> Option<Event> {
    let painter = ui.painter().clone();
    let (left, top) = (body.left(), body.top());
    TEXT.paint_line(
        &painter,
        pos2(left, top + SUBHEADER_TOP),
        SUBHEADER,
        SUBHEADER_INK,
        None,
    );
    let label = TEXT.paint_line(
        &painter,
        pos2(left, top + LABEL_TOP),
        LABEL,
        LABEL_INK,
        None,
    );
    let button = exo_ui::paint_dialog_dropdown(
        ui,
        pos2(left + label + VALUE_LEFT, top + VALUE_TOP),
        name(current),
    );
    let response = ui
        .interact(
            button,
            exo_ui::id(ui.ctx(), "auto-advance-dropdown"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut event = response.clicked().then_some(Event::ToggleMenu);
    if let Some(highlighted) = state.menu {
        let choices = order(current);
        let names: Vec<&str> = choices.iter().map(|(_, name)| *name).collect();
        let list = Menu {
            items: &names,
            highlighted,
        }
        .show_fitted_in_dialog(
            ui,
            exo_ui::id(ui.ctx(), "auto-advance-list"),
            button.left_top() + MENU_OFFSET,
        );
        if let Some(clicked) = list.clicked {
            event = Some(Event::Choose(choices[clicked].0));
        } else if let Some(hovered) = list.hovered.filter(|hovered| *hovered != highlighted) {
            event = Some(Event::Highlight(hovered));
        } else if ui.input(|input| input.pointer.any_released())
            && !response.hovered()
            && !ui.rect_contains_pointer(list.rect)
        {
            event = Some(Event::MenuClose);
        }
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_puts_the_choice_first_then_the_rest_by_name() {
        let names = |current| {
            order(current)
                .into_iter()
                .map(|(choice, _)| choice)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(AutoAdvance::Down),
            [AutoAdvance::Down, AutoAdvance::Off, AutoAdvance::Up]
        );
        assert_eq!(
            names(AutoAdvance::Up),
            [AutoAdvance::Up, AutoAdvance::Off, AutoAdvance::Down]
        );
        assert_eq!(
            names(AutoAdvance::Off),
            [AutoAdvance::Off, AutoAdvance::Down, AutoAdvance::Up]
        );
    }

    #[test]
    fn arrows_open_the_list_and_go_round_and_enter_chooses() {
        let mut state = State::default();
        assert_eq!(state.handle(Event::Move(1), AutoAdvance::Down), None);
        assert_eq!(state.menu, Some(0), "↓ opens the list on the choice");
        state.handle(Event::Move(-1), AutoAdvance::Down);
        assert_eq!(state.menu, Some(2), "↑ wraps to the last row");
        assert_eq!(
            state.handle(Event::Enter, AutoAdvance::Down),
            Some(AutoAdvance::Up)
        );
        assert!(!state.menu_open());
        state.handle(Event::ToggleMenu, AutoAdvance::Down);
        assert_eq!(
            state.handle(Event::Enter, AutoAdvance::Down),
            None,
            "the choice again changes nothing"
        );
        assert_eq!(
            state.handle(Event::Choose(AutoAdvance::Off), AutoAdvance::Down),
            Some(AutoAdvance::Off)
        );
    }
}
