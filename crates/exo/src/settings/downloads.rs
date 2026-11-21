//! The Downloads dialog: where attachments download to, app-wide ([`crate::downloads`]). A line of
//! explanation, then `Download attachments to:` and a dropdown of `Downloads`, `Desktop`, the
//! folder chosen before (by its path) and `Other…`, which asks for a folder. A choice applies at
//! once; there's no Save. The dropdown has the keyboard from the start.

use std::path::{Path, PathBuf};

use egui::{Color32, Context, Rect, Sense, Ui, pos2, vec2};
use exo_ui::{Dialog, DialogDropdown, DialogResponse, Family, Font, Menu, TextStyle};

/// The subheader: 12px/20px, `padding-top: 24px`, 3px down, `margin-bottom: 16px`.
const LINE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const SUBHEADER_TOP: f32 = 27.0;
/// The dropdown's row (`padding: 4px 0`) 60px down, its label 3px down its line, the
/// dropdown's value 1px past the label's `margin-right: 4px` and 2px lower.
const LABEL_TOP: f32 = 67.0;
const VALUE_LEFT: f32 = 5.0;
const VALUE_TOP: f32 = 69.0;
const BODY_HEIGHT: f32 = 88.0;
/// The subheader's `rgba(0, 0, 0, .85)`, the label's `.7`.
const SUBHEADER_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 217);
const LABEL_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 179);
/// The open list: 18px left of the value and 26px under the button.
const MENU_OFFSET: egui::Vec2 = vec2(-18.0, 26.0);
const SUBHEADER: &str = "Choose where to download attachments.";
const LABEL: &str = "Download attachments to:";

/// A row of the dropdown (`{ name, id }`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// `Downloads`.
    Downloads,
    /// `Desktop`.
    Desktop,
    /// The folder chosen before, by its path (`current`).
    Current(PathBuf),
    /// `Other…`: a folder chooser (`pick`).
    Other,
}

impl Choice {
    fn name(&self) -> String {
        match self {
            Self::Downloads => "Downloads".into(),
            Self::Desktop => "Desktop".into(),
            Self::Current(path) => path.to_string_lossy().into_owned(),
            Self::Other => "Other…".into(),
        }
    }
}

/// The rows for the folder downloads go to (`chosen`, `None` for the Downloads folder), the one
/// it is first and the rest by name, as dialog dropdowns sort them.
pub fn choices(
    chosen: Option<&Path>,
    downloads: Option<&Path>,
    desktop: Option<&Path>,
) -> Vec<Choice> {
    let mut choices = vec![Choice::Downloads, Choice::Desktop];
    let selected = match chosen {
        None => Choice::Downloads,
        Some(folder) if Some(folder) == downloads => Choice::Downloads,
        Some(folder) if Some(folder) == desktop => Choice::Desktop,
        Some(folder) => {
            choices.push(Choice::Current(folder.to_owned()));
            Choice::Current(folder.to_owned())
        }
    };
    choices.push(Choice::Other);
    // The selected one first, then by name.
    choices.sort_by_cached_key(|choice| {
        format!(
            "{}{}",
            if *choice == selected { 'a' } else { 'b' },
            choice.name()
        )
    });
    choices
}

/// Something done in the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Open or close the list (a click on the dropdown, Enter or an arrow on it).
    ToggleMenu,
    /// ↓ (1) or ↑ (-1) in the open list.
    MenuMove(isize),
    /// Escape with the list open.
    MenuClose,
    /// The pointer on a row of the list.
    Highlight(usize),
    /// Enter or Tab on the open list: its highlighted row.
    MenuPick,
    /// A row, clicked or picked.
    Choose(Choice),
}

/// The open list's highlighted row, between frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    menu: Option<usize>,
}

impl State {
    /// Whether the list is open: Escape closes it, not the dialog.
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Applies `event` to the list, the dropdown's rows being `choices`; returns the row chosen.
    pub fn handle(&mut self, event: Event, choices: &[Choice]) -> Option<Choice> {
        match event {
            Event::ToggleMenu => self.menu = if self.menu.is_some() { None } else { Some(0) },
            Event::MenuMove(step) => {
                self.menu = self
                    .menu
                    .map(|at| (at as isize + step).rem_euclid(choices.len() as isize) as usize)
            }
            Event::MenuClose => self.menu = None,
            Event::Highlight(row) => self.menu = Some(row),
            Event::MenuPick => return self.menu.take().and_then(|at| choices.get(at).cloned()),
            Event::Choose(choice) => {
                self.menu = None;
                return Some(choice);
            }
        }
        None
    }
}

/// Draws the dialog over `area` with `choices` (the first the folder downloads go to); returns
/// what was done.
pub fn dialog(
    ctx: &Context,
    area: Rect,
    open: bool,
    state: &State,
    choices: &[Choice],
) -> DialogResponse<Option<Event>> {
    Dialog::new("downloads", "Downloads", BODY_HEIGHT)
        .show(ctx, area, open, |ui, body| draw(ui, body, state, choices))
}

fn draw(ui: &mut Ui, body: Rect, state: &State, choices: &[Choice]) -> Option<Event> {
    let painter = ui.painter().clone();
    let (left, top) = (body.left(), body.top());
    LINE.paint_line(
        &painter,
        pos2(left, top + SUBHEADER_TOP),
        SUBHEADER,
        SUBHEADER_INK,
        None,
    );
    let label = LINE.paint_line(
        &painter,
        pos2(left, top + LABEL_TOP),
        LABEL,
        LABEL_INK,
        None,
    );
    let names: Vec<String> = choices.iter().map(Choice::name).collect();
    // The dropdown has the keyboard, so its caret shows.
    let palette = exo_ui::palette(ui.ctx());
    let style = DialogDropdown {
        value: palette.ink(0.7, 0.7),
        caret: palette.ink(0.5, 0.5),
        gap: 10.0,
    };
    let selected = names.first().map_or("Downloads", String::as_str);
    let button = exo_ui::paint_dialog_dropdown_with(
        ui,
        pos2(left + label + VALUE_LEFT, top + VALUE_TOP),
        selected,
        style,
    );
    let response = ui
        .interact(
            button,
            exo_ui::id(ui.ctx(), "downloads-location"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut event = response.clicked().then_some(Event::ToggleMenu);
    if let Some(highlighted) = state.menu {
        let items: Vec<&str> = names.iter().map(String::as_str).collect();
        let list = Menu {
            items: &items,
            highlighted,
        }
        .show_in_dialog(
            ui,
            exo_ui::id(ui.ctx(), "downloads-location-list"),
            button.left_top() + MENU_OFFSET,
        );
        if let Some(clicked) = list.clicked {
            event = Some(Event::Choose(choices[clicked].clone()));
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

/// `Could not change download folder` over the dialog in `area`; returns whether it was
/// dismissed (`OK`, or a click beside it).
pub fn alert(ctx: &Context, area: Rect) -> bool {
    let alert = exo_ui::Alert {
        title: "Could not change download folder",
        text: "Your download folder could not be changed. Please try again.",
        actions: &["OK"],
        selected: 0,
    };
    egui::Area::new(exo_ui::id(ctx, "downloads-alert"))
        .order(egui::Order::Tooltip)
        .fixed_pos(area.min)
        .show(ctx, |ui| {
            // The alert's backdrop, over the dialog's.
            let backdrop = ui.allocate_rect(area, Sense::click());
            ui.painter()
                .rect_filled(area, 0.0, Color32::from_black_alpha(128));
            alert.show(ui, area).is_some() || backdrop.clicked()
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_in_use_comes_first_and_the_rest_by_name() {
        let (downloads, desktop) = (
            Path::new("/Users/maya/Downloads"),
            Path::new("/Users/maya/Desktop"),
        );
        let names = |chosen: Option<&Path>| {
            choices(chosen, Some(downloads), Some(desktop))
                .iter()
                .map(Choice::name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(None), ["Downloads", "Desktop", "Other…"]);
        assert_eq!(names(Some(downloads)), ["Downloads", "Desktop", "Other…"]);
        assert_eq!(names(Some(desktop)), ["Desktop", "Downloads", "Other…"]);
        assert_eq!(
            names(Some(Path::new("/Volumes/Work/Inbox"))),
            ["/Volumes/Work/Inbox", "Desktop", "Downloads", "Other…"]
        );
    }

    #[test]
    fn the_keyboard_opens_moves_through_and_picks_from_the_list() {
        let rows = choices(None, None, None);
        let mut state = State::default();
        assert_eq!(state.handle(Event::ToggleMenu, &rows), None);
        assert_eq!(state.menu, Some(0), "on the folder in use");
        state.handle(Event::MenuMove(-1), &rows);
        assert_eq!(state.menu, Some(2), "wraps");
        assert_eq!(state.handle(Event::MenuPick, &rows), Some(Choice::Other));
        assert!(!state.menu_open());
        state.handle(Event::ToggleMenu, &rows);
        assert_eq!(state.handle(Event::MenuClose, &rows), None);
        assert!(!state.menu_open());
    }
}
