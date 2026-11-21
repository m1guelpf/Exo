//! The Touch Bar's tip: on a Mac whose Escape is on its Touch Bar, someone who keeps pressing
//! Escape is offered Backtick as Escape once, while the setting has never been turned on or off.
//! The fifth Escape of the run, 40 seconds or more after the account's first one (kept on the
//! device), opens the alert instead of doing what it does, unless another alert is up. `Turn On`
//! turns the setting on, `No Thanks` (Escape) off; either way the tip never comes back.

use std::sync::atomic::{AtomicU32, Ordering};

use egui::{Event, Key, Modifiers};
use exo_ui::{
    ALERT_PARAGRAPH_GAP, ALERT_TEXT, ALERT_TEXT_INK, KEYCAP_BASELINE, KeycapColors, Keycaps,
};

/// Escapes pressed this run, in any account (counted while the setting is unset).
static ESCAPES: AtomicU32 = AtomicU32::new(0);
/// The tip waits for this many Escapes in a run…
const ESCAPES_BEFORE: u32 = 5;
/// …and this long after the account's first.
const WAIT_MS: i64 = 40_000;

const TITLE: &str = "Tired of touching the Touch Bar?";
/// Right to left: `Turn On`, then `No Thanks`, Escape's.
const ACTIONS: [&str; 2] = ["Turn On", "No Thanks"];
const NO_THANKS: usize = 1;
/// The space above the keycaps' line, which holds the text.
const KEY_CODE_TOP: f32 = 6.0;

/// The tip while it's up: which action is selected.
#[derive(Debug)]
pub(crate) struct Tip {
    selected: usize,
}

impl crate::Mail {
    /// Counts this frame's Escapes and opens the tip in place of the one that earns it, or
    /// answers the tip's keys while it's up (Enter, Escape, ←/→ and Tab, as an alert's). Call
    /// before the commands take the frame's keys.
    pub(crate) fn touch_bar_tip_keys(&mut self, ctx: &egui::Context) {
        if self.touch_bar_tip.is_some() {
            self.tip_keys(ctx);
            return;
        }
        if self.account.settings().enable_backtick_as_escape.is_some()
            || !crate::desktop::escape_on_touch_bar()
        {
            return;
        }
        let alert_up = self.commands.region() == exo_commands::Region::Alert;
        let now = self.now.timestamp().as_millisecond();
        let earned = ctx.input_mut(|input| {
            let mut earned = false;
            input.events.retain(|event| {
                if earned
                    || !matches!(
                        event,
                        Event::Key {
                            key: Key::Escape,
                            pressed: true,
                            ..
                        }
                    )
                {
                    return true;
                }
                let escapes = ESCAPES.fetch_add(1, Ordering::Relaxed) + 1;
                earned = !alert_up
                    && escapes >= ESCAPES_BEFORE
                    && self
                        .account
                        .device_settings()
                        .first_checked_escape
                        .is_some_and(|first| now - first >= WAIT_MS);
                // The Escape that opens the tip does nothing else.
                !earned
            });
            earned
        });
        if self
            .account
            .device_settings()
            .first_checked_escape
            .is_none()
            && ESCAPES.load(Ordering::Relaxed) > 0
        {
            self.account
                .update_device_settings(|device| device.first_checked_escape = Some(now));
        }
        if earned {
            self.touch_bar_tip = Some(Tip { selected: 0 });
        }
    }

    fn tip_keys(&mut self, ctx: &egui::Context) {
        let pressed = |key, modifiers| ctx.input_mut(|input| input.consume_key(modifiers, key));
        if pressed(Key::Enter, Modifiers::NONE) {
            let selected = self.touch_bar_tip.as_ref().map_or(0, |tip| tip.selected);
            self.answer_tip(selected);
        } else if pressed(Key::Escape, Modifiers::NONE) {
            self.answer_tip(NO_THANKS);
        } else if let Some(tip) = &mut self.touch_bar_tip {
            if pressed(Key::ArrowLeft, Modifiers::NONE) || pressed(Key::Tab, Modifiers::SHIFT) {
                tip.selected = (tip.selected + 1).min(ACTIONS.len() - 1);
            } else if pressed(Key::ArrowRight, Modifiers::NONE)
                || pressed(Key::Tab, Modifiers::NONE)
            {
                tip.selected = tip.selected.saturating_sub(1);
            }
        }
    }

    /// `Turn On` (0) or `No Thanks`: Backtick as Escape on or off, and the tip goes.
    fn answer_tip(&mut self, action: usize) {
        self.touch_bar_tip = None;
        self.account
            .update_settings(|settings| settings.enable_backtick_as_escape = Some(action == 0));
    }

    /// The tip while it's up, over `page` dimmed: `Top tip! Instead of [esc] tap backtick [`]`
    /// and `You can always turn this off in the settings.`, its keys as the shortcuts sheet's.
    pub(crate) fn show_touch_bar_tip(&mut self, ui: &mut egui::Ui, page: egui::Rect) {
        let Some(tip) = &self.touch_bar_tip else {
            return;
        };
        // The overlay: the page dimmed to half, nothing behind it clickable.
        ui.painter()
            .rect_filled(page, 0.0, egui::Color32::from_black_alpha(128));
        let _ = ui.interact(
            page,
            ui.id().with("touch-bar-tip-overlay"),
            egui::Sense::click(),
        );
        let ctx = ui.ctx().clone();
        let colors = KeycapColors::shortcuts_pane(exo_ui::palette(&ctx));
        let height = KEY_CODE_TOP + 2.0 * ALERT_TEXT.line_height + ALERT_PARAGRAPH_GAP;
        let alert = exo_ui::Alert {
            title: TITLE,
            text: "",
            actions: &ACTIONS,
            selected: tip.selected,
        };
        let clicked = alert.show_body(ui, page, height, |ui, text| {
            let painter = ui.painter();
            let top = text.top() + KEY_CODE_TOP;
            // A cap's label sits on the text's baseline.
            let cap_top = top + ALERT_TEXT.baseline(&ctx) - KEYCAP_BASELINE;
            let mut x = text.left();
            for (words, key) in [("Top tip! Instead of", "esc"), (" tap backtick", "`")] {
                x +=
                    ALERT_TEXT.paint_line(painter, egui::pos2(x, top), words, ALERT_TEXT_INK, None);
                let caps = Keycaps::new(&ctx, &[key], colors);
                caps.paint(painter, x, cap_top);
                x += caps.width();
            }
            let second = top + ALERT_TEXT.line_height + ALERT_PARAGRAPH_GAP;
            ALERT_TEXT.paint_line(
                painter,
                egui::pos2(text.left(), second),
                "You can always turn this off in the settings.",
                ALERT_TEXT_INK,
                None,
            );
        });
        if let Some(action) = clicked {
            self.answer_tip(action);
        }
    }
}

/// Whether `model` (`hw.model`) is a MacBook Pro whose Escape is on its Touch Bar: the 2016–2019
/// ones. Later Touch Bars have an Escape key of their own (the 16-inch, the 2020 13-inch and
/// Apple's own chips), as do Macs without one.
pub(crate) fn escape_on_touch_bar(model: &str) -> bool {
    const MODELS: [&str; 8] = [
        "MacBookPro13,2",
        "MacBookPro13,3",
        "MacBookPro14,2",
        "MacBookPro14,3",
        "MacBookPro15,1",
        "MacBookPro15,2",
        "MacBookPro15,3",
        "MacBookPro15,4",
    ];
    MODELS.contains(&model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_touch_bars_with_escape_on_them() {
        assert!(escape_on_touch_bar("MacBookPro15,2"), "13-inch, 2018");
        assert!(escape_on_touch_bar("MacBookPro13,3"), "15-inch, 2016");
        assert!(
            !escape_on_touch_bar("MacBookPro16,1"),
            "16-inch, 2019: its own Escape key"
        );
        assert!(!escape_on_touch_bar("MacBookPro17,1"), "13-inch M1");
        assert!(
            !escape_on_touch_bar("MacBookPro14,1"),
            "13-inch, 2017, no Touch Bar"
        );
        assert!(!escape_on_touch_bar("Mac14,7"), "13-inch M2");
        assert!(!escape_on_touch_bar("Mac15,3"));
    }
}
