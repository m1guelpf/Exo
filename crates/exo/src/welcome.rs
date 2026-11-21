//! The welcome splash: the first time an account opens on this Mac, its photo covers the
//! window, under the tab strip as the sign-in screen's does, for at least five seconds and until
//! the mail is ready; then the first-run flow decides what comes next and it fades out over
//! 500 ms. The keys are its while it covers the window.
//!
//! Classic's look: the photo (`install-bg.jpg`) under a 50% black scrim, the welcome in 16px
//! bold over two 16px lines, centered, and `Getting ready…` with the spinner 56px from the
//! bottom.

use std::path::Path;

use egui::{Color32, Id, Rect, Sense, Ui, Vec2, pos2};
use exo_ui::{
    Family, Font, TextStyle,
    motion::{self, CubicBezier},
};

use crate::{chrome, photo::Photo};

/// It stays at least five seconds once its photo shows.
const MINIMUM: f64 = 5.0;
/// It fades out over 500ms (`ease`), and is gone 550ms after it starts to.
const FADE: f32 = 0.5;
const GONE_AFTER: f64 = 0.55;

/// The welcome: 16px/24px bold, white at .98, 24px over the body.
const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 16.0, 24.0);
const HEADER_INK: Color32 = Color32::from_rgba_premultiplied(250, 250, 250, 250);
const HEADER_GAP: f32 = 24.0;
/// The lines under it and `Getting ready…`: 16px/24px in the splash's `#ededed`.
const BODY: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
const INK: Color32 = Color32::from_rgb(0xed, 0xed, 0xed);
/// The bottom row: 56px from the bottom; the 16px spinner, 9px, then the text, padded
/// 4px at the top, so the row is 28px.
const LOWER_BOTTOM: f32 = 56.0;
const SPINNER: f32 = 16.0;
const SPINNER_GAP: f32 = 9.0;
const LOWER_PADDING: f32 = 4.0;

/// The splash's copy.
const WELCOME: &str = "Welcome to Exo!";
const BODY_LINES: [&str; 2] = [
    "Exo is the most productive email app ever made.",
    "You'll get through your inbox twice as fast as before.",
];
const GETTING_READY: &str = "Getting ready…";

/// Where the splash is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Covering the window, the keys its.
    Covering,
    /// It just gave way: the first-run flow decides now, and it fades.
    Decide,
    /// Fading out.
    Fading,
    /// Gone.
    Gone,
}

/// The splash over an account's mail.
pub(crate) struct Welcome {
    /// Its own, so its fades start afresh whichever account had one before.
    id: Id,
    photo: Photo,
    /// When its photo first showed.
    since: Option<f64>,
    /// When it began to fade.
    hiding: Option<f64>,
}

impl Welcome {
    /// The splash, its photo from `images`, decoding.
    pub(crate) fn new(ctx: &egui::Context, images: &Path) -> Self {
        let id = exo_ui::id(
            ctx,
            ("welcome-splash", ctx.input(|input| input.time).to_bits()),
        );
        Self {
            id,
            photo: Photo::load(ctx, images.join("install-bg.jpg")),
            since: None,
            hiding: None,
        }
    }

    /// It covers the window: the keys are its, and the tab is a sign-in screen's.
    pub(crate) fn covers(&self) -> bool {
        self.hiding.is_none()
    }

    /// The photo is decoded. For screenshots and tests.
    pub(crate) fn is_idle(&self) -> bool {
        self.photo.is_loaded()
    }

    /// Draws a frame over `window`, then the tab strip with `tab` on it, and says where it is.
    /// `name` is the account's name, whose first word greets it; `ready` says its mail is
    /// ready.
    pub(crate) fn show(
        &mut self,
        ui: &Ui,
        window: Rect,
        tab: &chrome::Tab<'_>,
        name: Option<&str>,
        ready: bool,
    ) -> Phase {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        let mut phase = Phase::Covering;
        if self.photo.is_loaded() {
            let since = *self.since.get_or_insert(now);
            if self.hiding.is_none() && ready && now - since >= MINIMUM {
                self.hiding = Some(now);
                phase = Phase::Decide;
            } else if self.hiding.is_none() {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                    (MINIMUM - (now - since)).max(0.0),
                ));
            }
        }
        if let Some(hiding) = self.hiding {
            if now - hiding >= GONE_AFTER {
                return Phase::Gone;
            }
            if phase != Phase::Decide {
                phase = Phase::Fading;
            }
        }
        let shown = 1.0
            - motion::transition(
                &ctx,
                self.id.with("hide"),
                self.hiding.is_some(),
                FADE,
                CubicBezier::EASE,
            );
        egui::Area::new(self.id)
            .order(egui::Order::Foreground)
            .fixed_pos(window.min)
            .show(&ctx, |ui| {
                ui.set_clip_rect(window);
                // It takes every click on the window while it's there.
                ui.interact(window, self.id.with("cover"), Sense::click_and_drag());
                let mut painter = ui.painter().clone();
                painter.set_opacity(shown);
                self.photo.paint(&painter, window, |free| free / 2.0);
                painter.rect_filled(window, 0.0, Color32::from_black_alpha(128));
                // The text fades in as the account's contact arrives.
                let text = motion::transition(
                    &ctx,
                    self.id.with("text"),
                    self.since.is_some(),
                    FADE,
                    CubicBezier::EASE,
                );
                paint_text(&painter, window, name, text);
                paint_lower(&painter, window, now);
                // The tab strip stays over the page: over the photo while it covers the window.
                let backdrop = if self.covers() {
                    chrome::Backdrop::Photo
                } else {
                    chrome::Backdrop::Page
                };
                let tab = chrome::Tab {
                    title: tab.title,
                    login_screen: self.covers() || tab.login_screen,
                    account_color: tab.account_color.filter(|_| !self.covers()),
                };
                let mut strip = ui.new_child(egui::UiBuilder::new().max_rect(window));
                chrome::show(&mut strip, tab, backdrop);
            });
        phase
    }
}

impl crate::Mail {
    /// The account just opened on this Mac for the first time: the welcome splash, until it's
    /// been past it (quitting before then shows it again).
    pub fn welcome_first_time(&mut self) {
        self.account
            .update_device_settings(|device| device.splash_pending = true);
        // A new account starts out with the tutorial to show.
        self.account
            .update_settings(|settings| settings.do_show_tutorial = true);
        self.resume_welcome();
    }

    /// The welcome splash, if the account hasn't been past it yet.
    pub(crate) fn resume_welcome(&mut self) {
        if self.account.device_settings().splash_pending && self.welcome.is_none() {
            self.welcome = Some(Welcome::new(&self.ctx, &self.images));
        }
    }

    /// The welcome splash covers the window.
    pub fn welcome_covers(&self) -> bool {
        self.welcome.as_ref().is_some_and(Welcome::covers)
    }

    /// Draws the welcome splash over `window`, with the tab strip on it (`title`, the
    /// account's color), once it's had its five seconds and the mail its first list.
    pub(crate) fn show_welcome(
        &mut self,
        ui: &Ui,
        window: Rect,
        title: &str,
        account_color: &'static str,
    ) {
        let Some(splash) = &mut self.welcome else {
            return;
        };
        let name = self
            .accounts
            .iter()
            .find(|account| account.email == self.account.email())
            .and_then(|account| account.name.as_deref());
        let tab = chrome::Tab {
            title,
            login_screen: false,
            account_color: Some(account_color),
        };
        match splash.show(ui, window, &tab, name, !self.snapshot.loading) {
            // The first-run flow starts under the splash as it fades.
            Phase::Decide => {
                self.account
                    .update_device_settings(|device| device.splash_pending = false);
                self.maybe_start_first_run();
            }
            Phase::Gone => self.welcome = None,
            Phase::Covering | Phase::Fading => {}
        }
    }
}

/// `{first}, welcome to Exo!` with the first word of the account's name, if it has one, else
/// the plain welcome.
fn welcome(name: Option<&str>) -> String {
    match name.and_then(|name| name.split_whitespace().next()) {
        Some(first) => format!("{first}, welcome to Exo!"),
        None => WELCOME.to_owned(),
    }
}

/// The welcome and the two lines under it, a column centered on `window`, at `opacity`.
fn paint_text(painter: &egui::Painter, window: Rect, name: Option<&str>, opacity: f32) {
    let ctx = painter.ctx();
    let block = HEADER.line_height + HEADER_GAP + BODY.line_height * BODY_LINES.len() as f32;
    let top = window.center().y - block / 2.0;
    let centered = |style: &TextStyle, text: &str, y: f32, ink: Color32| {
        let left = window.center().x - style.width(ctx, text) / 2.0;
        style.paint_line(
            painter,
            pos2(left, y),
            text,
            ink.gamma_multiply(opacity),
            None,
        );
    };
    centered(&HEADER, &welcome(name), top, HEADER_INK);
    for (line, text) in BODY_LINES.iter().enumerate() {
        centered(
            &BODY,
            text,
            top + HEADER.line_height + HEADER_GAP + BODY.line_height * line as f32,
            INK,
        );
    }
}

/// `Getting ready…` after the turning spinner, centered 56px from the bottom.
fn paint_lower(painter: &egui::Painter, window: Rect, now: f64) {
    let ctx = painter.ctx();
    let width = SPINNER + SPINNER_GAP + BODY.width(ctx, GETTING_READY);
    let row_height = LOWER_PADDING + BODY.line_height;
    let row = Rect::from_min_size(
        pos2(
            window.center().x - width / 2.0,
            window.bottom() - LOWER_BOTTOM - row_height,
        ),
        Vec2::new(width, row_height),
    );
    // The spinner: white at .98, centered on the row.
    let spinner = Rect::from_min_size(
        pos2(row.left(), row.center().y - SPINNER / 2.0),
        Vec2::splat(SPINNER),
    );
    exo_ui::paint_spinner(painter, spinner, now, HEADER_INK);
    BODY.paint_line(
        painter,
        pos2(spinner.right() + SPINNER_GAP, row.top() + LOWER_PADDING),
        GETTING_READY,
        INK,
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_greets_the_account_by_its_first_name() {
        assert_eq!(welcome(Some("Maya Okafor")), "Maya, welcome to Exo!");
        assert_eq!(welcome(Some("  ")), "Welcome to Exo!");
        assert_eq!(welcome(None), "Welcome to Exo!");
    }
}
