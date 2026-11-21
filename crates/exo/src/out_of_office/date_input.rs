//! The dialog's date fields: the shared [`exo_ui::DateInput`] and its popup, the input drawn as the
//! dialog styles it.

use egui::{Color32, Id, Pos2, Rect, Sense, Ui, pos2, vec2};
use exo_ui::{DateInput, DatePick, DatePicker, Family, Font, TextStyle};
use jiff::civil::{Date, Weekday};

/// The input's text: the UI face at 600 12px/16px, `top: 1px` (the dialog's light look doesn't
/// reach it).
pub const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 16.0);
/// Its placeholder is at normal weight.
const PLACEHOLDER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// `padding-bottom: 3px; border-bottom: 1px`, `2px` of each while focused.
const RULE_GAP: f32 = 3.0;
/// `min-width: 20px` (`field-sizing: content`).
const MIN_WIDTH: f32 = 20.0;
/// Classic's colours: the text `rgba(0,0,0,.9)`, `.4` disabled; the placeholder `.55`, `.2`
/// disabled; the rule `.1`, the focused one `#aeb1dd`, the wrong one `#f67b7b`.
const INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 230);
const INK_DISABLED: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 102);
const PLACEHOLDER_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 140);
const PLACEHOLDER_DISABLED: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 51);
const RULE: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 26);
const RULE_FOCUSED: Color32 = Color32::from_rgb(0xae, 0xb1, 0xdd);
const RULE_ERROR: Color32 = Color32::from_rgb(0xf6, 0x7b, 0x7b);

/// What happened in a date field this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateEvent {
    /// It was clicked or focused: it has the keyboard, and its popup opens.
    Focused,
    /// A day was picked in the popup.
    Picked(Date),
    /// What's typed changed; an empty field that may be empty clears the date.
    Cleared,
    /// A popup chevron was clicked: the field keeps the keyboard.
    MonthChanged,
}

/// How a date field draws this frame.
pub struct Field<'a> {
    /// Its id, for the text edit and the popup.
    pub id: Id,
    /// Its state.
    pub input: &'a mut DateInput,
    /// It has the keyboard.
    pub focused: bool,
    /// The form is off (`disabled`).
    pub disabled: bool,
    /// `Select date`, for the last day.
    pub placeholder: &'a str,
    /// Days before this can't be picked.
    pub min: Date,
    /// Today, which the popup marks.
    pub today: Date,
    /// Its spelling (the input is spellchecked).
    pub spelling: &'a mut exo_editor::FieldSpelling,
}

impl Field<'_> {
    /// The input's width: its text's, or its placeholder's while empty (`field-sizing:
    /// content`).
    pub fn width(&self, ctx: &egui::Context) -> f32 {
        let width = if self.input.text.is_empty() {
            PLACEHOLDER.width(ctx, self.placeholder)
        } else {
            TEXT.width(ctx, &self.input.text)
        };
        width.max(MIN_WIDTH)
    }

    /// Draws the input with its line box's top left at `at`, and returns what happened.
    pub fn show(&mut self, ui: &mut Ui, at: Pos2) -> Option<DateEvent> {
        let width = self.width(ui.ctx());
        let line = Rect::from_min_size(at, vec2(width, TEXT.line_height));
        let mut event = None;
        let ink = if self.disabled { INK_DISABLED } else { INK };
        if self.input.text.is_empty() {
            let placeholder = if self.disabled {
                PLACEHOLDER_DISABLED
            } else {
                PLACEHOLDER_INK
            };
            PLACEHOLDER.paint_line(ui.painter(), line.min, self.placeholder, placeholder, None);
        }
        let hit = Rect::from_min_max(line.min, pos2(line.right(), line.bottom() + RULE_GAP + 1.0));
        if self.disabled {
            TEXT.paint_line(ui.painter(), line.min, &self.input.text, ink, None);
        } else {
            let before = self.input.text.clone();
            let selection = ui.visuals().selection.bg_fill;
            let response = self.spelling.show(
                ui,
                self.id,
                &mut self.input.text,
                line,
                &TEXT.without_ligatures(),
                ink,
                selection,
            );
            if response.clicked() || (response.gained_focus() && !self.focused) {
                event = Some(DateEvent::Focused);
            } else {
                super::dialog::hold_focus(ui, self.focused, self.id);
            }
            if self.input.text != before {
                event = Some(DateEvent::Cleared);
            }
            let pointer = ui
                .interact(hit, self.id.with("hit"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::Text);
            if pointer.clicked() {
                event = Some(DateEvent::Focused);
            }
        }
        // The rule: 1px, 3px under the text; 2px, 2px under it while focused.
        let (gap, thickness) = if self.focused {
            (RULE_GAP - 1.0, 2.0)
        } else {
            (RULE_GAP, 1.0)
        };
        let color = if self.input.error {
            RULE_ERROR
        } else if self.focused {
            RULE_FOCUSED
        } else {
            RULE
        };
        let rule = Rect::from_min_size(
            pos2(line.left(), line.bottom() + gap),
            vec2(width, thickness),
        );
        ui.painter()
            .rect_filled(exo_ui::pixel_snap(rule), 0.0, color);
        if let Some(mut month) = self.input.month.filter(|_| self.focused && !self.disabled) {
            // Tethered to the input's box: its line, padding and rule.
            let at = pos2(line.left(), line.bottom() + RULE_GAP + 1.0) + exo_ui::DATE_PICKER_OFFSET;
            let picker = DatePicker {
                id: self.id.with("popup"),
                month: &mut month,
                selected: self.input.parsed,
                today: self.today,
                min: Some(self.min),
                week_start: Weekday::Sunday,
            };
            match picker.show(ui.ctx(), at) {
                Some(DatePick::Picked(day)) => event = Some(DateEvent::Picked(day)),
                Some(DatePick::MonthChanged) => {
                    self.input.month = Some(month);
                    event = Some(DateEvent::MonthChanged);
                }
                None => {}
            }
        }
        event
    }
}
