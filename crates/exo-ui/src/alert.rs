//! The alert: a small centered card with a bold title, text and its actions in a row at the bottom
//! right.
//!
//! 340px wide, padded `12px 24px 20px 16px`, 4px corners, white in both themes; title and text
//! are 14px/20px Exo Sans nudged down 3px, the text 8px below the title; the actions sit in a 28px
//! row 8px below the text, floated right past the padding (`margin-right: -12px`), the second one
//! 8px left of the first.

use egui::{Color32, CornerRadius, Rect, Sense, Ui, pos2, vec2};

use crate::{BoxShadow, Family, Font, TextStyle, WBR, form, motion, pixel_snap};

const WIDTH: f32 = 340.0;
const PADDING_TOP: f32 = 12.0;
const PADDING_RIGHT: f32 = 24.0;
const PADDING_BOTTOM: f32 = 20.0;
const PADDING_LEFT: f32 = 16.0;
/// The right padding of an alert without actions.
const PADDING_RIGHT_NO_ACTIONS: f32 = 16.0;
const RADIUS: u8 = 4;
const TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 20.0);
const TEXT: TextStyle = ALERT_TEXT;
/// The text: 14px/20px Exo Sans.
pub const ALERT_TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 20.0);
/// The text box's width in an alert with actions: the card's less its padding.
pub const ALERT_TEXT_WIDTH: f32 = WIDTH - PADDING_LEFT - PADDING_RIGHT;
/// The title, text and actions all carry `position: relative; top: 3px`.
const NUDGE: f32 = 3.0;
/// The text's margin above and below, and between its paragraphs.
const TEXT_MARGIN: f32 = ALERT_PARAGRAPH_GAP;
/// How far apart an alert's `<p>`s are.
pub const ALERT_PARAGRAPH_GAP: f32 = 8.0;
/// The action row's height.
const ACTIONS_HEIGHT: f32 = 28.0;
/// An action's padding, `8px 12px 4px 12px`, and its `margin-right: -12px`.
const ACTION_PADDING: [f32; 3] = [8.0, 12.0, 4.0];
/// The second action's gap before the first.
const ACTION_GAP: f32 = 8.0;
/// The title: `rgba(0, 0, 0, .9)`; the text: `rgba(0, 0, 0, .75)`, in both themes.
const TITLE_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 230);
const TEXT_INK: Color32 = ALERT_TEXT_INK;
/// The text's `rgba(0, 0, 0, .75)`.
pub const ALERT_TEXT_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 191);
/// `0 15px 20px rgba(0,0,0,.04), 0 8px 20px rgba(0,0,0,.1)`.
const SHADOWS: [BoxShadow; 2] = [
    BoxShadow::new(
        [0.0, 15.0],
        20.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 10),
    ),
    BoxShadow::new(
        [0.0, 8.0],
        20.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 26),
    ),
];
/// A radio: 12px, 7px before its label.
const RADIO: f32 = 12.0;
const RADIO_GAP: f32 = 7.0;
/// A keycap in an alert's text: an 18px cap, at least 18px wide, `padding: 0 5px`, `margin: 0 1px`,
/// `#abb7c5` (`#555b64` in Carbon) with 3px corners; its label 12px semibold (13px only on a 1×
/// screen) in a 15px line, white (at `.8` in Carbon), 2.5px down the cap (centered in its 12px
/// content box, then `translateY(3px)`).
const KEY_HEIGHT: f32 = 18.0;
const KEY_MIN_WIDTH: f32 = 18.0;
const KEY_PADDING: f32 = 5.0;
const KEY_MARGIN: f32 = 1.0;
const KEY_FILL: Color32 = Color32::from_rgb(0xab, 0xb7, 0xc5);
const KEY_FILL_CARBON: Color32 = Color32::from_rgb(0x55, 0x5b, 0x64);
const KEY_LABEL: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 15.0);
const KEY_LABEL_TOP: f32 = 2.5;
/// Separates `<p>` paragraphs in [`Alert::text`].
pub const PARAGRAPH: char = '\u{2029}';

/// `text` for an address or domain in an alert: a `<wbr>` before each word that follows a word and
/// punctuation, so `daniel@brightmoor.co` can break after its `@` and `.`. Chromium lays a `<wbr>`
/// out as a zero-width space ([`WBR`]), and its glyphs either side don't kern; the alert does the
/// same.
pub fn wbrise(text: &str) -> String {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Kind {
        Whitespace,
        Punctuation,
        Other,
        Word,
    }
    const PUNCTUATION: &str = "!#$%&)*+,-./:=>?@\\]^_|}~";
    let is = |kind: Kind, c: char| match kind {
        Kind::Whitespace => c.is_whitespace(),
        Kind::Punctuation => PUNCTUATION.contains(c),
        Kind::Other => "({[\"'`".contains(c),
        Kind::Word => !c.is_whitespace() && !PUNCTUATION.contains(c),
    };
    let (mut pieces, mut current, mut kinds) = (Vec::new(), String::new(), Vec::<Kind>::new());
    let mut rest = text;
    while !rest.is_empty() {
        // Each round takes a run of each kind in turn.
        for kind in [Kind::Whitespace, Kind::Punctuation, Kind::Other, Kind::Word] {
            let end = rest.find(|c| !is(kind, c)).unwrap_or(rest.len());
            if end == 0 {
                continue;
            }
            let (run, after) = rest.split_at(end);
            if kind == Kind::Word && kinds.ends_with(&[Kind::Word, Kind::Punctuation]) {
                pieces.push(std::mem::replace(&mut current, run.to_owned()));
                kinds.push(kind);
            } else {
                current.push_str(run);
                if kinds.last() != Some(&kind) {
                    kinds.push(kind);
                }
            }
            rest = after;
        }
    }
    pieces.push(current);
    pieces.join(&WBR.to_string())
}

/// An alert with up to two actions, one of them selected (filled). Enter, Escape and the arrow
/// keys are the caller's to bind.
#[derive(Clone, Copy, Debug)]
pub struct Alert<'a> {
    /// The title (`Almost there…`).
    pub title: &'a str,
    /// The text: each `\n` is a `<br>` (so a blank line is an empty line), and runs of spaces
    /// collapse, leading ones included. [`PARAGRAPH`] separates `<p>`s, which sit 8px apart.
    pub text: &'a str,
    /// The actions' labels, right to left: the first is rightmost.
    pub actions: &'a [&'a str],
    /// The selected action (usually the first).
    pub selected: usize,
}

impl Alert<'_> {
    /// Paints the alert centered in `area` (the window below the tab bar). Returns the action
    /// clicked, if any.
    pub fn show(&self, ui: &mut Ui, area: Rect) -> Option<usize> {
        let ctx = ui.ctx().clone();
        let paragraphs: Vec<Vec<String>> = self
            .text
            .split(PARAGRAPH)
            .map(|paragraph| pretty_wrap(&ctx, paragraph, self.content_width()))
            .collect();
        let lines: usize = paragraphs.iter().map(Vec::len).sum();
        let height = lines as f32 * TEXT.line_height
            + (paragraphs.len().saturating_sub(1)) as f32 * TEXT_MARGIN;
        self.paint(ui, area, height, |ui, text| {
            paint_paragraphs(ui.painter(), text.min, &paragraphs);
        })
    }

    /// [`Alert::show`] with a radio button per option in place of the text, one per line, as a
    /// recurring event's RSVP puts them (12px, `top: 2px`, 7px before its label); a click on an
    /// option's line selects it into `chosen`.
    pub fn show_options(
        &self,
        ui: &mut Ui,
        area: Rect,
        options: &[&str],
        chosen: &mut usize,
    ) -> Option<usize> {
        let height = options.len() as f32 * TEXT.line_height;
        let width = self.content_width();
        self.paint(ui, area, height, |ui, text| {
            for (index, label) in options.iter().enumerate() {
                let y = text.top() + index as f32 * TEXT.line_height;
                let line = Rect::from_min_size(pos2(text.left(), y), vec2(width, TEXT.line_height));
                if ui
                    .interact(
                        line,
                        ui.id().with(("alert-option", self.title, index)),
                        Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    *chosen = index;
                }
                paint_radio(ui.painter(), pos2(text.left(), y + 2.0), index == *chosen);
                TEXT.paint_line(
                    ui.painter(),
                    pos2(text.left() + RADIO + RADIO_GAP, y),
                    label,
                    TEXT_INK,
                    None,
                );
            }
        })
    }

    /// [`Alert::show`] whose text is one paragraph of `before`, an inline keycap labelled `key`
    /// (the follow-up hint's `H`) and `after`, wrapped at its spaces.
    pub fn show_with_key(
        &self,
        ui: &mut Ui,
        area: Rect,
        before: &str,
        key: &str,
        after: &str,
    ) -> Option<usize> {
        self.show_with_keys(ui, area, before, &[key], after)
    }

    /// [`Alert::show`] whose text is `before`, laid out as [`Alert::show`] lays text out, then a
    /// keycap for each of `keys` on its last line after a space (inline, side by side: Get Me To
    /// Zero's `⌘ K`), then a space and `after`, wrapped at its spaces.
    pub fn show_with_keys(
        &self,
        ui: &mut Ui,
        area: Rect,
        before: &str,
        keys: &[&str],
        after: &str,
    ) -> Option<usize> {
        let ctx = ui.ctx().clone();
        let width = self.content_width();
        let paragraphs: Vec<Vec<String>> = before
            .split(PARAGRAPH)
            .map(|paragraph| pretty_wrap(&ctx, paragraph, width))
            .collect();
        let last = paragraphs
            .last()
            .and_then(|lines| lines.last())
            .map_or(0.0, |line| TEXT.width(&ctx, line));
        let space = TEXT.width(&ctx, "a a") - TEXT.width(&ctx, "aa");
        let key_widths: Vec<f32> = keys
            .iter()
            .map(|key| (KEY_LABEL.width(&ctx, key) + 2.0 * KEY_PADDING).max(KEY_MIN_WIDTH))
            .collect();
        let keys_left = last + space;
        let prefix = keys_left
            + key_widths
                .iter()
                .map(|width| width + 2.0 * KEY_MARGIN)
                .sum::<f32>()
            + space;
        // Greedy, as Chromium breaks a paragraph whose last line keeps more than a word.
        let mut lines: Vec<String> = vec![String::new()];
        for word in after.split_whitespace() {
            let first = lines.len() == 1;
            let limit = if first { width - prefix } else { width };
            let line = lines.last_mut().expect("a line");
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if line.is_empty() && !first || TEXT.width(&ctx, &candidate) <= limit {
                *line = candidate;
            } else {
                lines.push(word.to_owned());
            }
        }
        let before_lines: usize = paragraphs.iter().map(Vec::len).sum();
        let gaps = paragraphs.len().saturating_sub(1) as f32 * TEXT_MARGIN;
        let height = (before_lines + lines.len() - 1) as f32 * TEXT.line_height + gaps;
        let (fill, ink) = match crate::app_theme(&ctx) {
            crate::Theme::Snow => (KEY_FILL, Color32::WHITE),
            crate::Theme::Carbon => (KEY_FILL_CARBON, Color32::WHITE.gamma_multiply(0.8)),
        };
        self.paint(ui, area, height, |ui, text| {
            let painter = ui.painter();
            // The keys and `after` continue the last line.
            let y = paint_paragraphs(painter, text.min, &paragraphs) - TEXT.line_height;
            let mut x = text.left() + keys_left;
            for (key, key_width) in keys.iter().zip(&key_widths) {
                let cap =
                    Rect::from_min_size(pos2(x + KEY_MARGIN, y), vec2(*key_width, KEY_HEIGHT));
                painter.rect_filled(cap, CornerRadius::same(3), fill);
                let label = KEY_LABEL.width(&ctx, key);
                KEY_LABEL.paint_line(
                    painter,
                    pos2(cap.center().x - label / 2.0, cap.top() + KEY_LABEL_TOP),
                    key,
                    ink,
                    None,
                );
                x += key_width + 2.0 * KEY_MARGIN;
            }
            for (index, line) in lines.iter().enumerate() {
                let x = if index == 0 {
                    text.left() + prefix
                } else {
                    text.left()
                };
                TEXT.paint_line(
                    painter,
                    pos2(x, y + index as f32 * TEXT.line_height),
                    line,
                    TEXT_INK,
                    None,
                );
            }
        })
    }

    /// [`Alert::show`] with `body` painting the text box ([`ALERT_TEXT_WIDTH`] wide with actions
    /// and `height` tall, its top already nudged down) itself: an alert whose children aren't plain
    /// text (bold runs, links, a checkbox). Paint in [`ALERT_TEXT`] and [`ALERT_TEXT_INK`] to match
    /// the plain text.
    pub fn show_body(
        &self,
        ui: &mut Ui,
        area: Rect,
        height: f32,
        body: impl FnOnce(&mut Ui, Rect),
    ) -> Option<usize> {
        self.paint(ui, area, height, body)
    }

    /// The text box's width: without actions there's no action row, and a narrower padding.
    fn content_width(&self) -> f32 {
        let padding_right = if self.actions.is_empty() {
            PADDING_RIGHT_NO_ACTIONS
        } else {
            PADDING_RIGHT
        };
        WIDTH - PADDING_LEFT - padding_right
    }

    /// [`Alert::show_body`] with `emoji` drawn as an image `size` big after the title and a space
    /// (`Welcome to Exo <img>`): the image stands on the title's baseline, and when it's taller
    /// than the baseline is low it raises the title's line box, pushing the rest down.
    pub fn show_body_with_title_emoji(
        &self,
        ui: &mut Ui,
        area: Rect,
        emoji: &str,
        size: egui::Vec2,
        height: f32,
        body: impl FnOnce(&mut Ui, Rect),
    ) -> Option<usize> {
        let ctx = ui.ctx().clone();
        let raised = (size.y - TITLE.baseline(&ctx)).max(0.0);
        let title = |painter: &egui::Painter, top_left: egui::Pos2| {
            let width = TITLE.paint_line(
                painter,
                top_left + vec2(0.0, raised),
                &format!("{} ", self.title),
                TITLE_INK,
                None,
            );
            let image = Rect::from_min_size(
                top_left + vec2(width, raised + TITLE.baseline(&ctx) - size.y),
                size,
            );
            crate::paint_emoji(painter, emoji, image);
        };
        self.paint_titled(ui, area, TITLE.line_height + raised, title, height, body)
    }

    fn paint(
        &self,
        ui: &mut Ui,
        area: Rect,
        text_height: f32,
        body: impl FnOnce(&mut Ui, Rect),
    ) -> Option<usize> {
        let title = |painter: &egui::Painter, top_left: egui::Pos2| {
            TITLE.paint_line(painter, top_left, self.title, TITLE_INK, None);
        };
        self.paint_titled(ui, area, TITLE.line_height, title, text_height, body)
    }

    /// The alert with its title's line box `title_height` tall, painted by `title` from its top
    /// left (nudged down already).
    fn paint_titled(
        &self,
        ui: &mut Ui,
        area: Rect,
        title_height: f32,
        title: impl FnOnce(&egui::Painter, egui::Pos2),
        text_height: f32,
        body: impl FnOnce(&mut Ui, Rect),
    ) -> Option<usize> {
        let ctx = ui.ctx().clone();
        let actions_height = if self.actions.is_empty() {
            0.0
        } else {
            ACTIONS_HEIGHT
        };
        let height = PADDING_TOP
            + title_height
            + TEXT_MARGIN
            + text_height
            + TEXT_MARGIN
            + actions_height
            + PADDING_BOTTOM;
        // Centering can land on a half pixel; Chromium paints the alert's layer at the whole
        // pixel below it, text and all.
        let card = pixel_snap(Rect::from_center_size(area.center(), vec2(WIDTH, height)));

        let painter = ui.painter();
        for shadow in SHADOWS {
            shadow.paint(painter, card, CornerRadius::same(RADIUS));
        }
        painter.rect_filled(card, CornerRadius::same(RADIUS), Color32::WHITE);
        let left = card.left() + PADDING_LEFT;
        let title_top = card.top() + PADDING_TOP;
        title(painter, pos2(left, title_top + NUDGE));
        let text_top = title_top + title_height + TEXT_MARGIN;
        body(
            ui,
            Rect::from_min_size(
                pos2(left, text_top + NUDGE),
                vec2(self.content_width(), text_height),
            ),
        );

        // The actions float right in their row, 12px into the padding.
        let [pad_top, pad_x, pad_bottom] = ACTION_PADDING;
        let mut right = card.right() - PADDING_RIGHT + pad_x;
        let row_top = text_top + text_height + TEXT_MARGIN;
        let mut clicked = None;
        for (index, label) in self.actions.iter().enumerate() {
            let size = vec2(
                form::action_width(&ctx, label),
                pad_top + TEXT.line_height + pad_bottom,
            );
            let button = Rect::from_min_size(pos2(right - size.x, row_top + NUDGE), size);
            right = button.left() - ACTION_GAP;
            let response = ui.interact(
                button,
                ui.id().with(("alert-action", self.title, index)),
                Sense::click(),
            );
            // Selected, hovered and pressed are filled; selected and hovered glow. Hover and press
            // transition 150ms ease.
            let selected = if index == self.selected { 1.0 } else { 0.0 };
            let hover = motion::transition(
                &ctx,
                response.id.with("hover"),
                response.hovered(),
                0.15,
                motion::CubicBezier::EASE,
            );
            let pressed = motion::transition(
                &ctx,
                response.id.with("pressed"),
                response.is_pointer_button_down_on(),
                0.15,
                motion::CubicBezier::EASE,
            );
            let (fill, ink) = form::filled(f32::max(selected, f32::max(hover, pressed)));
            form::paint_action(
                ui.painter(),
                button,
                label,
                fill,
                f32::max(selected, hover),
                ink,
            );
            if response.clicked() {
                clicked = Some(index);
            }
        }
        clicked
    }
}

/// A 12px radio at `at`: a ring of black at 40% 5.5px out, and when selected a `#6FBEEA` dot of
/// 3px.
pub fn paint_radio(painter: &egui::Painter, at: egui::Pos2, selected: bool) {
    let center = at + vec2(RADIO / 2.0, RADIO / 2.0);
    painter.circle_stroke(
        center,
        5.5,
        egui::Stroke::new(1.0, Color32::from_black_alpha(102)),
    );
    if selected {
        painter.circle_filled(center, 3.0, Color32::from_rgb(0x6f, 0xbe, 0xea));
    }
}

/// Paints `paragraphs`' lines from `at` down, 8px between paragraphs; returns the bottom of the
/// last line. Each piece between `<wbr>`s ([`wbrise`]) is shaped on its own, as Chromium's are:
/// no kerning across them.
fn paint_paragraphs(painter: &egui::Painter, at: egui::Pos2, paragraphs: &[Vec<String>]) -> f32 {
    let mut y = at.y;
    for (index, paragraph) in paragraphs.iter().enumerate() {
        if index > 0 {
            y += TEXT_MARGIN;
        }
        for line in paragraph {
            let mut x = at.x;
            for piece in line.split(WBR) {
                x += TEXT.paint_line(painter, pos2(x, y), piece, TEXT_INK, None);
            }
            y += TEXT.line_height;
        }
    }
    y
}

/// Wraps `paragraph` as an alert's text wraps: at `<br>`s and spaces, with whitespace collapsed
/// (`white-space: normal`), and `text-wrap: pretty`'s rule against a last line of one word: the
/// line above gives up its last word when it can.
fn pretty_wrap(ctx: &egui::Context, paragraph: &str, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for line in paragraph.split('\n') {
        let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        lines.extend(TEXT.wrap_pretty(ctx, &collapsed, width));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::wbrise;

    #[test]
    fn wbrise_breaks_before_words_that_follow_punctuation() {
        assert_eq!(
            wbrise("daniel@brightmoor.co"),
            "daniel@\u{200B}brightmoor.\u{200B}co"
        );
        assert_eq!(wbrise("shop.example"), "shop.\u{200B}example");
        assert_eq!(wbrise("a.b-c d"), "a.\u{200B}b-\u{200B}c d");
        assert_eq!(wbrise("plain"), "plain");
    }
}
