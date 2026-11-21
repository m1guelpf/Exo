//! The teardown alerts: the full-screen alert that replaces an account that can't go on, and the
//! one for an account whose store can't be opened (corrupt, on a full disk, locked, or the
//! default `Sorry!`).
//!
//! The alert covers the window with the full-screen alert backdrop (the photo under a 40% black
//! scrim), and Enter or Escape performs its first action. The backdrop is the day's Inbox Zero
//! photo. None of these reload by themselves, and a broken store is one account's: other
//! accounts are a `ctrl+1…9` away.

use egui::{Color32, Key, Modifiers, Rect, Ui, pos2};
use jiff::Zoned;

use crate::{account::OpenError, backdrop::Backdrop, chrome};

/// Paints `alert` over the full-screen alert backdrop, the day's photo, and the tab strip.
/// Returns the action clicked, if any.
pub(crate) fn paint(ui: &mut Ui, backdrop: &Backdrop, alert: exo_ui::Alert<'_>) -> Option<usize> {
    let window = ui.max_rect();
    let painter = ui.painter();
    backdrop.paint_photo(painter, window);
    painter.rect_filled(window, 0.0, Color32::from_black_alpha(102));
    // The alert's container is the page below the tab strip.
    let page = Rect::from_min_max(
        pos2(
            window.left(),
            window.top() + chrome::tab_bar_height(ui.ctx()),
        ),
        window.max,
    );
    let clicked = alert.show(ui, page);
    chrome::show(
        ui,
        chrome::Tab {
            title: "Exo",
            login_screen: false,
            account_color: None,
        },
        chrome::Backdrop::Photo,
    );
    clicked
}

/// Which teardown says why an account's store couldn't be opened, from the kind of database error
/// ([`exo_store::Unusable`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The file can't be used as it is (unreadable, from a schema this build
    /// has no upgrade from, an upgrade that failed). Clearing the cache fixes it.
    Corrupt,
    /// The disk is full: nothing to do in Exo but wait for space.
    DiskFull,
    /// Something else holds the file, which a reload may find let go.
    Locked,
    /// Anything else, such as the account's runtime not starting: the default teardown.
    Failed,
}

/// The title of each of these.
const TITLE: &str = "Sorry!";
const CLEAR_CACHE: &str = "Clear Cache and Restart";
const RELOAD: &str = "Reload";

impl From<exo_store::Unusable> for Reason {
    fn from(unusable: exo_store::Unusable) -> Self {
        match unusable {
            exo_store::Unusable::Corrupt => Self::Corrupt,
            exo_store::Unusable::DiskFull => Self::DiskFull,
            exo_store::Unusable::Locked => Self::Locked,
        }
    }
}

impl Reason {
    /// Why `error` stopped an account opening.
    #[must_use]
    pub fn of(error: &OpenError) -> Self {
        match error {
            OpenError::Store(error) => error.unusable().map_or(Self::Failed, Self::from),
            OpenError::Runtime(_) => Self::Failed,
        }
    }

    /// The alert's text and actions.
    #[must_use]
    pub fn alert(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Corrupt => (
                "The Exo cache is corrupted.\nTo fix this please clear your cache and restart the app.",
                &[CLEAR_CACHE],
            ),
            Self::DiskFull => (
                "Your disk is full. To fix this, please free up some disk space and restart Exo.",
                &[],
            ),
            Self::Locked => ("Please reload this page to continue.", &[RELOAD]),
            Self::Failed => (
                "We've encountered an error and will look into it right away.",
                &[RELOAD],
            ),
        }
    }
}

/// What the screen asks the app for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Open the account again (`Reload`).
    Reload,
    /// Clear its cache and open it again (`Clear Cache and Restart`).
    ClearCache,
    /// Show the account in this place in the list (ctrl+1…9).
    Switch(usize),
}

/// The screen for an account whose store couldn't be opened.
pub struct Unopened {
    email: String,
    reason: Reason,
    backdrop: Backdrop,
}

impl Unopened {
    /// The screen for `email`, over the photo for `now`. It's ready to draw.
    pub fn new(ctx: &egui::Context, now: &Zoned, email: String, reason: Reason) -> Self {
        let mut backdrop = Backdrop::new(ctx, now);
        backdrop.wait();
        Self {
            email,
            reason,
            backdrop,
        }
    }

    /// The account whose store couldn't be opened.
    pub fn email(&self) -> &str {
        &self.email
    }

    /// Why.
    pub fn reason(&self) -> Reason {
        self.reason
    }

    /// The photo is decoded. For screenshots and tests.
    pub fn is_idle(&self) -> bool {
        self.backdrop.is_loaded()
    }

    /// Draws a frame and handles its input. Returns what was asked for, if anything.
    pub fn show(&mut self, ui: &mut Ui) -> Option<Choice> {
        let (text, actions) = self.reason.alert();
        let first = match actions.first() {
            Some(&CLEAR_CACHE) => Some(Choice::ClearCache),
            Some(_) => Some(Choice::Reload),
            None => None,
        };
        let ctx = ui.ctx().clone();
        // Control on the Mac, Alt elsewhere.
        let switch = if ctx.os() == egui::os::OperatingSystem::Mac {
            Modifiers::CTRL
        } else {
            Modifiers::ALT
        };
        let (key, place) = ctx.input_mut(|input| {
            let key = first.is_some()
                && (input.consume_key(Modifiers::NONE, Key::Enter)
                    | input.consume_key(Modifiers::NONE, Key::Escape));
            let digits = [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
                Key::Num9,
            ];
            (
                key,
                digits
                    .iter()
                    .position(|digit| input.consume_key(switch, *digit)),
            )
        });
        let clicked = paint(
            ui,
            &self.backdrop,
            exo_ui::Alert {
                title: TITLE,
                text,
                actions,
                selected: 0,
            },
        )
        .is_some();
        if let Some(place) = place {
            return Some(Choice::Switch(place));
        }
        (clicked || key).then_some(first).flatten()
    }
}
