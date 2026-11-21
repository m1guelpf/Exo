//! `Week N`: on the week's first Inbox Zero the chrome hides for 2 s and, over the photo,
//! `Inbox Zero Streak` / `Week N` sits over a huge `N - 1` that rolls down out of view as `N` rolls
//! in after it.

use egui::{Color32, Painter, Rect, pos2, vec2};
use exo_ui::{Family, Font, TextStyle, motion::CubicBezier};

/// The chrome hides for 2 s.
const CHROME_HIDDEN: f64 = 2.0;
/// The animation shows for 2.5 s; the layer hides at 2 s, fading over 150ms.
const SHOWN: f64 = 2.5;
const HIDES_AT: f64 = 2.0;
const FADE: f64 = 0.15;

/// The header: 16px/20px, `top: 2px`, uppercase in Classic.
const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 20.0);
/// The number: 440px semibold, `letter-spacing: -20px`.
const NUMBER: TextStyle =
    TextStyle::new(Font::new(Family::Sans, 600), 440.0, 440.0).letter_spacing(-20.0);

/// The animation, from when it started.
#[derive(Clone, Copy, Debug)]
pub struct Animation {
    since: f64,
    streak: u32,
}

impl Animation {
    /// The animation for a streak of `streak` weeks, starting at `now`.
    pub fn new(streak: u32, now: f64) -> Self {
        Self { since: now, streak }
    }

    /// It's over at `now`.
    pub fn is_over(&self, now: f64) -> bool {
        now - self.since >= SHOWN
    }

    /// The chrome (the list's header, Inbox Zero's button, the footer) is hidden at `now`.
    pub fn hides_chrome(&self, now: f64) -> bool {
        now - self.since < CHROME_HIDDEN
    }

    /// Paints it over `window` (the photo's), under the app.
    pub fn paint(&self, painter: &Painter, window: Rect, now: f64) {
        let elapsed = now - self.since;
        let opacity = if elapsed < HIDES_AT {
            1.0
        } else {
            1.0 - ((elapsed - HIDES_AT) / FADE).min(1.0) as f32
        };
        if opacity <= 0.0 {
            return;
        }
        let painter = {
            let mut painter = painter.clone();
            painter.multiply_opacity(opacity);
            painter
        };
        // The backdrop: `rgba(0,0,0,.2)`.
        painter.rect_filled(window, 0.0, Color32::from_black_alpha(51));
        // The content's top margin, on a 4px grid.
        let height = window.height();
        let inner = window.top() + 4.0 * ((height / 2.0 - 234.0 - 0.033 * height) / 4.0).ceil();
        let ink = Color32::from_white_alpha(191);
        let center = window.center().x;
        let mut y = inner + 2.0;
        for (i, text) in [
            "INBOX ZERO STREAK".to_owned(),
            format!("WEEK {}", self.streak),
        ]
        .iter()
        .enumerate()
        {
            let width = HEADER.width(painter.ctx(), text);
            HEADER.paint_line(&painter, pos2(center - width / 2.0, y), text, ink, None);
            y += HEADER.line_height;
            if i == 0 {
                // The line: 42px, `margin: 22px 0 25px`.
                painter.hline(
                    center - 21.0..=center + 21.0,
                    y + 22.0 + 0.5,
                    egui::Stroke::new(1.0, ink),
                );
                y += 22.0 + 1.0 + 25.0;
            }
        }
        // The numbers' baselines at 112 and 472 in the SVG, clipped to `y: 125, height: 360`,
        // both moving 360px down over 250ms after 250ms (`animation: 250ms ease-in-out 250ms
        // forwards move`).
        let roll =
            360.0 * CubicBezier::EASE_IN_OUT.at(((elapsed - 0.25) / 0.25).clamp(0.0, 1.0) as f32);
        let clip = Rect::from_min_size(
            pos2(window.left(), inner + 125.0),
            vec2(window.width(), 360.0),
        )
        .intersect(window);
        let numbers = painter.with_clip_rect(clip);
        let number_ink = Color32::from_white_alpha(128);
        let baseline = NUMBER.baseline(painter.ctx());
        for (value, at) in [(self.streak, 112.0), (self.streak.saturating_sub(1), 472.0)] {
            let text = value.to_string();
            // `text-anchor: middle` takes the trailing letter-spacing as part of the run.
            let width = NUMBER.width(painter.ctx(), &text);
            NUMBER.paint_line(
                &numbers,
                pos2(center - width / 2.0, inner + at + roll - baseline),
                &text,
                number_ink,
                None,
            );
        }
    }
}
