//! The Emoji dialog: the skin tone compose's `:` picker gives emoji (`emojiSkinTone`), six raised
//! hands in a row with a bar under the chosen one, which slides to the next. Opening it with no
//! tone set sets `NONE`.
//!
//! Metrics at a 1280×800 window.

use egui::{Color32, Context, CursorIcon, Rect, Sense, Ui, pos2, vec2};
use exo_store::EmojiSkinTone;
use exo_ui::{Dialog, DialogResponse, Family, Font, TextStyle, motion};

/// The subheader and footer: 12px/20px, 3px down.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const NUDGE: f32 = 3.0;
/// From the body's top: the subheader's line, the options (a 1px padding over 24px emoji with
/// `padding-bottom: 5px`), the bar under them, and the footer's line.
const SUBHEADER_TOP: f32 = 24.0;
const OPTIONS_TOP: f32 = 61.0;
const BAR_TOP: f32 = 88.0;
const FOOTER_TOP: f32 = 112.0;
const BODY_HEIGHT: f32 = 132.0;
/// A tone: 24px wide, 24px apart, its emoji 24px/24px.
const OPTION: f32 = 24.0;
const OPTION_GAP: f32 = 24.0;
const EMOJI: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 24.0, 24.0);
/// Chromium sets the color glyph lower in its line than egui does (measured).
const EMOJI_DOWN: f32 = 2.0;
/// The bar under the chosen tone: 2px `#aeb1dd`, sliding over 200ms
/// `cubic-bezier(0.19, 1, 0.22, 1)`; the hovered option's `box-shadow: 0 2px 0 0 #aeb1dd` too.
const BAR: f32 = 2.0;
const ACCENT: Color32 = Color32::from_rgb(0xae, 0xb1, 0xdd);
const SLIDE: f64 = 0.2;
const SLIDE_CURVE: motion::CubicBezier = motion::CubicBezier::new(0.19, 1.0, 0.22, 1.0);
/// The subheader's `rgba(0, 0, 0, .85)`; the footer's `.6`.
const INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 217);
const FOOTER_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 153);
/// The footer's key: an 18px `#abb7c5` cap with 3px corners, 4px either
/// side, 1px down; its white `:`.
const KEY: f32 = 18.0;
const KEY_MARGIN: f32 = 4.0;
const KEY_FILL: Color32 = Color32::from_rgb(0xab, 0xb7, 0xc5);
const OPTIONS: [&str; 6] = ["🙌", "🙌🏻", "🙌🏼", "🙌🏽", "🙌🏾", "🙌🏿"];

/// Something done in the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Tab or → (1), ⇧Tab or ← (-1): the next or previous tone, wrapping.
    Step(isize),
    /// A click on a tone.
    Choose(EmojiSkinTone),
}

/// Where the bar slides from, and since when (stamped as it's next drawn).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    from: Option<(usize, Option<f64>)>,
}

impl State {
    /// Carries out `event` from `current`, returning the tone to save.
    pub fn handle(&mut self, event: Event, current: EmojiSkinTone) -> EmojiSkinTone {
        let at = position(current);
        let next = match event {
            Event::Step(delta) => {
                EmojiSkinTone::ALL
                    [(at as isize + delta).rem_euclid(OPTIONS.len() as isize) as usize]
            }
            Event::Choose(tone) => tone,
        };
        if next != current {
            self.from = Some((at, None));
        }
        next
    }
}

fn position(tone: EmojiSkinTone) -> usize {
    EmojiSkinTone::ALL
        .iter()
        .position(|each| *each == tone)
        .unwrap_or(0)
}

/// Draws the dialog over `area` with `current` chosen.
pub fn dialog(
    ctx: &Context,
    area: Rect,
    open: bool,
    state: &mut State,
    current: EmojiSkinTone,
) -> DialogResponse<Option<Event>> {
    Dialog::new("emoji", "Emoji", BODY_HEIGHT)
        .show(ctx, area, open, |ui, body| show(ui, body, state, current))
}

fn show(ui: &Ui, body: Rect, state: &mut State, current: EmojiSkinTone) -> Option<Event> {
    let ctx = ui.ctx().clone();
    let painter = ui.painter().clone();
    let left = body.left();
    let mut event = None;
    TEXT.paint_line(
        &painter,
        pos2(left, body.top() + SUBHEADER_TOP + NUDGE),
        "Choose a default skin tone for your emoji.",
        INK,
        None,
    );
    let x = |at: usize| left + at as f32 * (OPTION + OPTION_GAP);
    for (at, emoji) in OPTIONS.into_iter().enumerate() {
        let option = Rect::from_min_size(
            pos2(x(at), body.top() + OPTIONS_TOP),
            vec2(OPTION, BAR_TOP - OPTIONS_TOP),
        );
        let response = ui
            .interact(
                option,
                exo_ui::id(ui.ctx(), ("emoji-skin-tone", at)),
                Sense::click(),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.hovered() {
            painter.rect_filled(
                Rect::from_min_size(
                    pos2(option.left(), option.bottom() - 1.0),
                    vec2(OPTION, BAR),
                ),
                0.0,
                ACCENT,
            );
        }
        EMOJI.paint_line(
            &painter,
            option.min + vec2(0.0, EMOJI_DOWN),
            emoji,
            INK,
            None,
        );
        if response.clicked() {
            event = Some(Event::Choose(EmojiSkinTone::ALL[at]));
        }
    }
    // The bar under the chosen tone, sliding from the last one (none as the dialog opens).
    let target = x(position(current));
    let now = ctx.input(|input| input.time);
    if let Some((from, None)) = state.from {
        state.from = Some((from, Some(now)));
    }
    let bar_x = match state.from {
        Some((from, Some(since))) if now - since < SLIDE => {
            ctx.request_repaint();
            let progress = SLIDE_CURVE.at(((now - since) / SLIDE) as f32);
            x(from) + (target - x(from)) * progress
        }
        _ => target,
    };
    painter.rect_filled(
        Rect::from_min_size(pos2(bar_x, body.top() + BAR_TOP), vec2(OPTION, BAR)),
        0.0,
        ACCENT,
    );

    // `To use an emoji, just type [:] then the emoji's name!` (a flex row: the spaces either side
    // of the cap go).
    let line = body.top() + FOOTER_TOP + NUDGE;
    let lead = TEXT.paint_line(
        &painter,
        pos2(left, line),
        "To use an emoji, just type",
        FOOTER_INK,
        None,
    );
    let cap = Rect::from_min_size(pos2(left + lead + KEY_MARGIN, line + 1.0), vec2(KEY, KEY));
    painter.rect_filled(exo_ui::pixel_snap(cap), 3.0, KEY_FILL);
    let colon = TEXT.width(&ctx, ":");
    TEXT.paint_line(
        &painter,
        pos2(cap.center().x - colon / 2.0, line),
        ":",
        Color32::WHITE,
        None,
    );
    TEXT.paint_line(
        &painter,
        pos2(cap.right() + KEY_MARGIN, line),
        "then the emoji's name!",
        FOOTER_INK,
        None,
    );
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_wrap_round_the_six_tones() {
        let mut state = State::default();
        assert_eq!(
            state.handle(Event::Step(-1), EmojiSkinTone::None),
            EmojiSkinTone::Dark
        );
        assert_eq!(
            state.handle(Event::Step(1), EmojiSkinTone::Dark),
            EmojiSkinTone::None
        );
        assert_eq!(
            state.handle(Event::Step(1), EmojiSkinTone::Light),
            EmojiSkinTone::MediumLight
        );
        assert_eq!(
            state.handle(Event::Choose(EmojiSkinTone::Medium), EmojiSkinTone::Light),
            EmojiSkinTone::Medium
        );
        assert_eq!(
            state.from,
            Some((1, None)),
            "the bar slides from where it was"
        );
    }
}
