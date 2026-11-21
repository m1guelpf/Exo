//! The sign-in screen: the photo, a wordmark, and a white card with the tagline, "Sign in with
//! Google" (or "Sign in as {email}" for an account Google signed out, with "Use another account→"
//! under it), a spinner while the browser is out, and the footer's errors; and the round back
//! button when there's an account to go back to.
//!
//! Google is the only provider, so the card's fixed heights leave out a Microsoft button. The
//! wordmark says "EXO".

use std::path::Path;

use egui::{
    Color32, CornerRadius, Key, Modifiers, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2,
};
use exo_ui::{BoxShadow, Family, Font, KeyTooltip, Placement, TextStyle, motion};

use crate::{chrome, photo::Photo, sign_in::SignIn};

/// The column is 570px wide; the card is padded `0 37px`.
const WIDTH: f32 = 570.0;
const PADDING_X: f32 = 37.0;
/// The wordmark: white bold caps 12.5px tall, tracked out, in a 16px box 32px above the card.
const WORDMARK: TextStyle =
    TextStyle::new(Font::new(Family::SansEmail, 700), 17.5, 16.0).letter_spacing(3.0);
const WORDMARK_NUDGE: f32 = 2.0;
const WORDMARK_HEIGHT: f32 = 16.0;
const WORDMARK_GAP: f32 = 32.0;
/// The card: `#fafafa` at 98%, 4px corners, two soft shadows, `margin-bottom: 20px`; 320px tall
/// with the provider button, 392px with a footer too.
const CARD: Color32 = Color32::from_rgb(0xfa, 0xfa, 0xfa);
const CARD_OPACITY: f32 = 0.98;
const CARD_SHADOWS: [BoxShadow; 2] = [
    BoxShadow::new(
        [0.0, 15.0],
        20.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 10),
    ),
    BoxShadow::new(
        [0.0, 19.0],
        40.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 26),
    ),
];
const CARD_MARGIN_BOTTOM: f32 = 20.0;
/// The tagline: light 24px/32px, `top: 2px`, `rgba(0,0,0,.6)`, `margin-bottom: 4px`.
const PROMPT: TextStyle = TextStyle::new(Font::new(Family::Sans, 100), 24.0, 32.0);
const PROMPT_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 153);
const PROMPT_NUDGE: f32 = 2.0;
/// The button: `#8797cd`, white bold 14px on a 50px line, `padding: 2px 20px`, 315px wide,
/// 52px tall, 4px corners, `top: 4px`; it grows 1% hovered and shrinks 5% pressed, its two
/// shadows changing with it (`transition: all .5s`).
const BUTTON: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 50.0);
const BUTTON_FILL: Color32 = Color32::from_rgb(0x87, 0x97, 0xcd);
const BUTTON_SIZE: egui::Vec2 = egui::Vec2::new(315.0, 52.0);
const BUTTON_TEXT_TOP: f32 = 2.0;
const BUTTON_NUDGE: f32 = 4.0;
const BUTTON_RADIUS: u8 = 4;
/// The spinner: a 60px ring with a 4px border whose right side is clear, turning once a
/// second, in `rgba(40,45,53,.3)`.
const SPINNER: f32 = 60.0;
const SPINNER_BORDER: f32 = 4.0;
const SPINNER_INK: Color32 = Color32::from_rgba_premultiplied(12, 14, 16, 77);
/// The footer: 14px/20px nudged down 3px, centered, `rgba(0,0,0,.7)`.
const FOOTER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 20.0);
const FOOTER_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 179);
const FOOTER_NUDGE: f32 = 3.0;
/// Links: `#54acdc`; the footer link's arrow is nudged `top: 1px; left: 3px`.
const LINK_INK: Color32 = Color32::from_rgb(0x54, 0xac, 0xdc);
const ARROW_NUDGE: egui::Vec2 = egui::Vec2::new(3.0, 1.0);
/// The back button: a 40px white circle `top: 24px; left: 2rem`, its 14px arrow black at .3 (.5
/// hovered); `scale(1.05)` hovered and `scale(.9)` pressed, its shadows changing with it
/// (`transition: all .4s`).
const BACK: f32 = 40.0;
const BACK_TOP: f32 = 24.0;
const BACK_ICON: f32 = 14.0;
const BACK_SHADOWS: [BoxShadow; 3] = [
    BoxShadow::new(
        [0.0, 0.0],
        6.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 26),
    ),
    BoxShadow::new(
        [0.0, 2.0],
        6.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 38),
    ),
    BoxShadow::new(
        [0.0, 0.0],
        0.5,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 26),
    ),
];
const BACK_HOVER_SHADOW: BoxShadow = BoxShadow::new(
    [0.0, 2.0],
    10.0,
    0.0,
    Color32::from_rgba_premultiplied(0, 0, 0, 77),
);
const BACK_PRESS_SHADOWS: [BoxShadow; 2] = [
    BoxShadow::new(
        [0.0, 0.0],
        4.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 51),
    ),
    BoxShadow::new(
        [0.0, 1.0],
        2.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 77),
    ),
];
/// Its tooltip's box sits this far right of the button: a 10px offset and the tooltip's padding.
const BACK_TOOLTIP_GAP: f32 = 18.36;
/// The spinner gives way to the button after 30 s without an answer from the browser, though
/// the attempt goes on.
const LOADING_TIMEOUT: f64 = 30.0;

/// The screen's copy and the sign-in errors.
mod copy {
    /// The tagline, on one line (`ever made.` upright, like the rest).
    pub const PROMPT: &str = "The most productive email app ever made.";
    /// The wordmark: Exo's name.
    pub const WORDMARK: &str = "EXO";
    /// The button.
    pub const BUTTON: &str = "Sign in with Google";
    /// The button for an account Google signed out.
    pub fn sign_in_as(email: &str) -> String {
        format!("Sign in as {email}")
    }
    /// The footer's link to sign in another account, its arrow drawn on its own.
    pub const ANOTHER_ACCOUNT: &str = "Use another account";
    pub const ARROW: &str = "→";
    /// The back button's tooltip: back to the account the screen came from,
    /// or to another that's still signed in.
    pub const BACK: &str = "Back";
    pub const BACK_TO_SIGNED_IN: &str = "Back to signed-in account";
    /// Any other failure.
    pub const FAILED: &str = "Sorry, something went wrong! Please try again later.";
    /// Consent given without every permission Exo asks for.
    pub const MISSING_SCOPES: &str = "Please select all permissions when signing in with Google.\nExo requires these to manage your email.";
    /// The account has no Gmail mailbox.
    pub const NO_MAILBOX: &str = "This Google Account does not appear to have a Gmail mailbox.";
    /// Exo ships no OAuth client: the user saves their own at `path`.
    pub fn no_client(path: &str) -> String {
        format!("Please save your Google OAuth client (Desktop app) as {path}.")
    }
}

/// Where the back button leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Back {
    /// The account the screen was opened from: adding an account, or
    /// switching to one Google signed out. Its tooltip says `Back`.
    Previous(String),
    /// Another account still signed in: `Back to signed-in account`.
    SignedIn(String),
}

impl Back {
    /// The account it leads to.
    pub fn email(&self) -> &str {
        match self {
            Self::Previous(email) | Self::SignedIn(email) => email,
        }
    }

    fn tooltip(&self) -> &'static str {
        match self {
            Self::Previous(_) => copy::BACK,
            Self::SignedIn(_) => copy::BACK_TO_SIGNED_IN,
        }
    }
}

/// What the sign-in screen asks of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// This account signed in.
    SignedIn(String),
    /// The back button (or Escape): show this account.
    Back(String),
}

/// The sign-in screen.
pub struct LogIn {
    photo: Photo,
    /// When the spinner replaced the button.
    loading_since: Option<f64>,
    footer: Option<String>,
    /// The account Google signed out, which the button signs in; Google is asked to offer it
    /// even after "Use another account".
    hint: Option<String>,
    /// "Use another account" hasn't been clicked.
    show_hint: bool,
    back: Option<Back>,
}

impl LogIn {
    /// The screen with the photo from `images`, saying `footer` (say, that there's no OAuth
    /// client).
    pub(crate) fn new(ctx: &egui::Context, images: &Path, footer: Option<String>) -> Self {
        Self {
            photo: Photo::load(ctx, images.join("install-bg.jpg")),
            loading_since: None,
            footer,
            hint: None,
            show_hint: false,
            back: None,
        }
    }

    /// The screen for signing `email` in again: `Sign in as {email}`.
    pub(crate) fn with_hint(mut self, email: String) -> Self {
        self.hint = Some(email);
        self.show_hint = true;
        self
    }

    /// The screen with a back button to `back`, if there's somewhere to go back to.
    pub(crate) fn with_back(mut self, back: Option<Back>) -> Self {
        self.back = back;
        self
    }

    /// The account the button signs in, while it says `Sign in as`.
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref().filter(|_| self.show_hint)
    }

    /// Where the back button leads, if there is one.
    pub fn back(&self) -> Option<&Back> {
        self.back.as_ref()
    }

    /// Blocks until the photo is decoded, so the screen is ready to draw.
    pub(crate) fn wait(&mut self) {
        self.photo.wait();
    }

    /// The footer's text, if it shows one.
    pub fn footer(&self) -> Option<&str> {
        self.footer.as_deref()
    }

    /// The spinner is showing: a sign-in is waiting for the browser.
    pub fn is_loading(&self) -> bool {
        self.loading_since.is_some()
    }

    /// The photo is decoded. For screenshots and tests.
    pub fn is_idle(&self) -> bool {
        self.photo.is_loaded()
    }

    /// The screen failed to open the account that just signed in.
    pub(crate) fn fail(&mut self) {
        self.footer = Some(copy::FAILED.to_owned());
    }

    /// Draws a frame and handles its input. Returns what it asks of the app, if anything.
    pub(crate) fn show(&mut self, ui: &mut Ui, sign_in: &mut SignIn) -> Option<Outcome> {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        if let Some(outcome) = sign_in.poll() {
            self.loading_since = None;
            match outcome {
                Ok(account) => return Some(Outcome::SignedIn(account.email)),
                Err(error) => {
                    log::warn!("signing in: {error}");
                    self.footer = footer(&error);
                }
            }
        }
        if self
            .loading_since
            .is_some_and(|since| now - since >= LOADING_TIMEOUT)
        {
            self.loading_since = None;
        }
        // Enter signs in, Escape clicks the back button; Tab moves nothing.
        let (enter, escape) = ctx.input_mut(|input| {
            input.consume_key(Modifiers::NONE, Key::Tab);
            (
                input.consume_key(Modifiers::NONE, Key::Enter),
                input.consume_key(Modifiers::NONE, Key::Escape),
            )
        });

        let window = ui.max_rect();
        let painter = ui.painter();
        // The photo centered, under the shell's two shades: 40%
        // black fading to 4% over the top 176px (gone by 216px), 10% to 1% from the bottom.
        self.photo.paint(painter, window, |free| free / 2.0);
        painter.add(shade(window, true, 0.4, 0.04));
        painter.add(shade(window, false, 0.1, 0.01));

        // The column is centered in the page below the tab strip.
        let page = Rect::from_min_max(
            pos2(window.left(), window.top() + chrome::tab_bar_height(&ctx)),
            window.max,
        );
        let footer_lines = self
            .footer
            .as_deref()
            .map(|text| FOOTER.wrap(&ctx, text, WIDTH - 2.0 * PADDING_X))
            .unwrap_or_default();
        // The footer shows an error, else the link back to the default buttons.
        let link = footer_lines.is_empty() && self.hint().is_some();
        let card = match (self.hint().is_some(), !footer_lines.is_empty() || link) {
            (true, _) => Card::SINGLE,
            (false, false) => Card::DEFAULT,
            (false, true) => Card::DEFAULT_WITH_FOOTER,
        };
        let layout = Layout::new(page, &card, self.is_loading());
        let center = layout.card.center().x;

        let width = WORDMARK.width(&ctx, copy::WORDMARK) - WORDMARK.letter_spacing;
        WORDMARK.paint_line(
            painter,
            pos2(center - width / 2.0, layout.wordmark + WORDMARK_NUDGE),
            copy::WORDMARK,
            Color32::WHITE,
            None,
        );

        let card = painter.clone();
        for shadow in CARD_SHADOWS {
            shadow.paint(&card, layout.card, CornerRadius::same(4));
        }
        card.rect_filled(
            layout.card,
            CornerRadius::same(4),
            CARD.gamma_multiply(CARD_OPACITY),
        );
        let width = PROMPT.width(&ctx, copy::PROMPT);
        PROMPT.paint_line(
            painter,
            pos2(center - width / 2.0, layout.prompt + PROMPT_NUDGE),
            copy::PROMPT,
            PROMPT_INK,
            None,
        );

        let mut clicked = false;
        if let Some(since) = self.loading_since {
            let spinner = Rect::from_min_size(
                pos2(center - SPINNER / 2.0, layout.actions),
                vec2(SPINNER, SPINNER),
            );
            painter.add(spinner_ring(
                spinner,
                ((now - since).fract() * std::f64::consts::TAU) as f32,
                SPINNER_INK,
            ));
            ctx.request_repaint();
        } else {
            // `transform: perspective(1px)` gives the button a layer, which Chromium puts on whole pixels.
            let button = exo_ui::pixel_snap(Rect::from_min_size(
                pos2(center - BUTTON_SIZE.x / 2.0, layout.actions + BUTTON_NUDGE),
                BUTTON_SIZE,
            ));
            let response = ui
                .interact(button, ui.id().with("log-in-button"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let hover = motion::transition(
                &ctx,
                response.id.with("hover"),
                response.hovered(),
                0.5,
                motion::CubicBezier::EASE,
            );
            let press = motion::transition(
                &ctx,
                response.id.with("press"),
                response.is_pointer_button_down_on(),
                0.5,
                motion::CubicBezier::EASE,
            );
            // `scale(1.01)` hovered, `scale(.95)` pressed (`:active` wins over `:hover`).
            let lerp = |from: f32, to: f32, t: f32| from + (to - from) * t;
            let scale = lerp(lerp(1.0, 1.01, hover), 0.95, press);
            let scaled = Rect::from_center_size(button.center(), button.size() * scale);
            // `0 14px 34px -10px rgba(0,0,0,.15)` twice; hovered both `40px … .25`; pressed the
            // first `20px … .35`.
            let lift = hover.max(press);
            let shadows = [
                (
                    lerp(lerp(34.0, 40.0, hover), 20.0, press),
                    lerp(lerp(0.15, 0.25, hover), 0.35, press),
                ),
                (lerp(34.0, 40.0, lift), lerp(0.15, 0.25, lift)),
            ];
            for (blur, alpha) in shadows {
                BoxShadow::new(
                    [0.0, 14.0],
                    blur,
                    -10.0,
                    Color32::from_black_alpha((alpha * 255.0).round() as u8),
                )
                .paint(ui.painter(), scaled, CornerRadius::same(BUTTON_RADIUS));
            }
            ui.painter()
                .rect_filled(scaled, CornerRadius::same(BUTTON_RADIUS), BUTTON_FILL);
            let label = self
                .hint()
                .map_or_else(|| copy::BUTTON.to_owned(), copy::sign_in_as);
            let width = BUTTON.width(&ctx, &label);
            BUTTON.paint_line(
                ui.painter(),
                pos2(
                    button.center().x - width / 2.0,
                    button.top() + BUTTON_TEXT_TOP,
                ),
                &label,
                Color32::WHITE,
                None,
            );
            clicked = response.clicked();
        }
        for (i, line) in footer_lines.iter().enumerate() {
            let width = FOOTER.width(&ctx, line);
            FOOTER.paint_line(
                ui.painter(),
                pos2(
                    center - width / 2.0,
                    layout.footer + FOOTER_NUDGE + i as f32 * FOOTER.line_height,
                ),
                line,
                FOOTER_INK,
                None,
            );
        }
        if link && another_account_link(ui, center, layout.footer + FOOTER_NUDGE) {
            // "Use another account": the default buttons, with no footer.
            self.show_hint = false;
        }
        let back = self
            .back
            .as_ref()
            .and_then(|back| back_button(ui, window, back).then(|| back.email().to_owned()));

        let backdrop = chrome::Backdrop::Photo;
        chrome::show(
            ui,
            chrome::Tab {
                title: "Exo",
                login_screen: true,
                account_color: None,
            },
            backdrop,
        );
        if let Some(email) = back.or_else(|| {
            self.back
                .as_ref()
                .filter(|_| escape)
                .map(|back| back.email().to_owned())
        }) {
            return Some(Outcome::Back(email));
        }
        // Enter works while the spinner shows too, starting over.
        if clicked || enter {
            self.start(&ctx, sign_in, now);
        }
        None
    }

    fn start(&mut self, ctx: &egui::Context, sign_in: &mut SignIn, now: f64) {
        match sign_in.start(ctx, self.hint.as_deref()) {
            Ok(()) => {
                self.footer = None;
                self.loading_since = Some(now);
            }
            Err(error) => {
                log::warn!("signing in: {error}");
                self.footer = footer(&error);
            }
        }
    }
}

/// The card in one of its states: its height, and the prompt's, the button's, the spinner's and
/// the footer's top margins.
struct Card {
    height: f32,
    prompt: f32,
    button: f32,
    spinner: f32,
    footer: f32,
}

impl Card {
    /// The provider button.
    const DEFAULT: Self = Self {
        height: 320.0,
        prompt: 64.0,
        button: 40.0,
        spinner: 72.0,
        footer: 44.0,
    };
    /// The provider button and a footer.
    const DEFAULT_WITH_FOOTER: Self = Self {
        height: 392.0,
        prompt: 96.0,
        button: 32.0,
        spinner: 72.0,
        footer: 44.0,
    };
    /// One button, `Sign in as`: the card's own height and margins.
    const SINGLE: Self = Self {
        height: 300.0,
        prompt: 88.0,
        button: 40.0,
        spinner: 40.0,
        footer: 40.0,
    };
}

/// Where each part of the column is, top to bottom.
struct Layout {
    wordmark: f32,
    card: Rect,
    prompt: f32,
    /// The button's or the spinner's top.
    actions: f32,
    footer: f32,
}

impl Layout {
    /// The column for `page`, centered in it, its card in state `card`.
    fn new(page: Rect, card: &Card, loading: bool) -> Self {
        let height = WORDMARK_HEIGHT + WORDMARK_GAP + card.height + CARD_MARGIN_BOTTOM;
        let top = page.center().y - height / 2.0;
        // The card (`position: relative; z-index: 1`) paints in a layer of its own, which
        // Chromium puts on a whole pixel when centering lands it on a half.
        let rect = exo_ui::pixel_snap(Rect::from_min_size(
            pos2(
                page.center().x - WIDTH / 2.0,
                top + WORDMARK_HEIGHT + WORDMARK_GAP,
            ),
            vec2(WIDTH, card.height),
        ));
        let prompt = rect.top() + card.prompt;
        let below_prompt = prompt + PROMPT.line_height;
        let actions = below_prompt + if loading { card.spinner } else { card.button };
        // The button's `top: 4px` moves only the button.
        let footer = actions
            + if loading {
                SPINNER
            } else {
                BUTTON_SIZE.y + card.footer
            };
        Self {
            wordmark: top,
            card: rect,
            prompt,
            actions,
            footer,
        }
    }
}

/// What the footer says after `error`; nothing when the user declined, or when the browser
/// never came back (the spinner had given up long before).
pub(crate) fn footer(error: &exo_auth::Error) -> Option<String> {
    use exo_auth::Error;
    Some(match error {
        // Declined consent is swallowed.
        Error::AccessDenied | Error::TimedOut => return None,
        Error::MissingScopes(_) => copy::MISSING_SCOPES.to_owned(),
        Error::NoMailbox => copy::NO_MAILBOX.to_owned(),
        Error::MissingClient { path, .. } | Error::InvalidClient { path, .. } => {
            copy::no_client(&tilde(path))
        }
        _ => copy::FAILED.to_owned(),
    })
}

/// `path` with the home folder written `~`.
fn tilde(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// "Use another account→", centered on `center` with its line box at
/// `top`. Returns whether it was clicked.
fn another_account_link(ui: &Ui, center: f32, top: f32) -> bool {
    let ctx = ui.ctx();
    let (text, arrow) = (
        FOOTER.width(ctx, copy::ANOTHER_ACCOUNT),
        FOOTER.width(ctx, copy::ARROW),
    );
    let left = center - (text + arrow) / 2.0;
    let rect = Rect::from_min_size(
        pos2(left, top),
        vec2(text + arrow + ARROW_NUDGE.x, FOOTER.line_height),
    );
    let response = ui
        .interact(rect, ui.id().with("another-account"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    FOOTER.paint_line(
        ui.painter(),
        pos2(left, top),
        copy::ANOTHER_ACCOUNT,
        LINK_INK,
        None,
    );
    FOOTER.paint_line(
        ui.painter(),
        pos2(left + text, top) + ARROW_NUDGE,
        copy::ARROW,
        LINK_INK,
        None,
    );
    response.clicked()
}

/// The back button at the top left of `window`'s page, with its tooltip and
/// `esc` keycap to the right while hovered. Returns whether it was clicked.
fn back_button(ui: &Ui, window: Rect, back: &Back) -> bool {
    let ctx = ui.ctx();
    let rem = exo_ui::rem(window.width(), false);
    let rect = Rect::from_min_size(
        pos2(
            window.left() + 2.0 * rem,
            window.top() + chrome::tab_bar_height(ctx) + BACK_TOP,
        ),
        vec2(BACK, BACK),
    );
    let response = ui
        .interact(rect, ui.id().with("log-in-back"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let hover = motion::transition(
        ctx,
        response.id.with("hover"),
        response.hovered(),
        0.4,
        motion::CubicBezier::EASE,
    );
    let press = motion::transition(
        ctx,
        response.id.with("press"),
        response.is_pointer_button_down_on(),
        0.4,
        motion::CubicBezier::EASE,
    );
    let lerp = |from: f32, to: f32, t: f32| from + (to - from) * t;
    // `:active` wins over `:hover`.
    let scaled = Rect::from_center_size(
        rect.center(),
        rect.size() * lerp(lerp(1.0, 1.05, hover), 0.9, press),
    );
    let radius = CornerRadius::same((scaled.width() / 2.0).round() as u8);
    // The shadows cross-fade between the states' lists.
    let (rest, hovered) = ((1.0 - hover) * (1.0 - press), hover * (1.0 - press));
    let painter = ui.painter();
    for (shadows, weight) in [
        (&BACK_SHADOWS[..], rest),
        (&[BACK_HOVER_SHADOW][..], hovered),
        (&BACK_PRESS_SHADOWS[..], press),
    ] {
        for shadow in shadows.iter().filter(|_| weight > 0.0) {
            shadow.with_opacity(weight).paint(painter, scaled, radius);
        }
    }
    painter.circle_filled(scaled.center(), scaled.width() / 2.0, Color32::WHITE);
    // The arrow scales with the button (the transform is on the button).
    let icon = Rect::from_center_size(
        scaled.center(),
        vec2(BACK_ICON, BACK_ICON) * scaled.width() / BACK,
    );
    let name = "legacy/DEPRECATED_backArrow";
    exo_ui::paint_icon(
        ui,
        name,
        exo_ui::icon_rect(ctx, name, icon),
        Color32::from_black_alpha(if response.hovered() { 128 } else { 77 }),
    );
    KeyTooltip {
        text: back.tooltip(),
        keys: &["esc"],
    }
    .show_flat_at(ui, &response, Placement::Right(BACK_TOOLTIP_GAP));
    response.clicked()
}

/// One of the shell's shades over the photo: black at `edge` opacity at the window's top (or
/// bottom, `from_top` false), `inner` 176px in, and nothing from 216px.
fn shade(window: Rect, from_top: bool, edge: f32, inner: f32) -> Shape {
    let black = |alpha: f32| Color32::from_black_alpha((alpha * 255.0).round() as u8);
    let mut mesh = egui::Mesh::default();
    for (i, (offset, alpha)) in [(0.0, edge), (176.0, inner), (216.0, 0.0)]
        .into_iter()
        .enumerate()
    {
        let y = if from_top {
            window.top() + offset
        } else {
            window.bottom() - offset
        };
        mesh.colored_vertex(pos2(window.left(), y), black(alpha));
        mesh.colored_vertex(pos2(window.right(), y), black(alpha));
        if i > 0 {
            let base = 2 * i as u32;
            mesh.add_triangle(base - 2, base, base + 1);
            mesh.add_triangle(base - 2, base + 1, base - 1);
        }
    }
    Shape::mesh(mesh)
}

/// The spinner's ring in `rect`, turned `turn` radians clockwise: a 4px border whose
/// right side (between the 45° corner joins) is transparent.
fn spinner_ring(rect: Rect, turn: f32, color: Color32) -> Shape {
    const POINTS: usize = 91;
    let radius = (SPINNER - SPINNER_BORDER) / 2.0;
    let (start, sweep) = (
        std::f32::consts::FRAC_PI_4 + turn,
        1.5 * std::f32::consts::PI,
    );
    let points: Vec<Pos2> = (0..POINTS)
        .map(|i| {
            let angle = start + sweep * i as f32 / (POINTS - 1) as f32;
            rect.center() + radius * vec2(angle.cos(), angle.sin())
        })
        .collect();
    Shape::line(points, Stroke::new(SPINNER_BORDER, color))
}
