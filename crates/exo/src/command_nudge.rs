//! The palette's first-run nudge. For an account that joined after 2025-01-13 (first signed in
//! to Exo on this Mac), until it has seen the nudge or used a triage key, with conversations in
//! the list: 3s after the mail opens, the next palette shows `Try: label, folder, trash, or
//! unread` in a sliding gradient; leaving a conversation for the list opens the palette with
//! `Remember: use ⌘K to perform any action in seconds!`. Closing the palette with either up, or
//! triaging, marking done or replying, is the end of it.

use jiff::Timestamp;

/// The nudge's wait after the mail opens.
const DELAY: f64 = 3.0;
/// Accounts that joined before this never see it.
const SINCE: Timestamp = Timestamp::constant(1_736_726_400, 0);

/// Which nudge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nudge {
    /// The palette's first open: `Try: …`.
    FirstOpen,
    /// Back from a conversation to the list: `Remember: …`.
    LeftThread,
}

impl Nudge {
    fn placeholder(self) -> &'static str {
        match self {
            Self::FirstOpen => "Try: label, folder, trash, or unread",
            Self::LeftThread => "Remember: use ⌘K to perform any action in seconds!",
        }
    }
}

/// The nudge's state in a mail: when it opened, the nudge set, and whether the palette was open
/// last frame.
#[derive(Debug, Default)]
pub(crate) struct State {
    opened: Option<f64>,
    nudge: Option<Nudge>,
    palette_open: bool,
}

impl crate::Mail {
    /// Whether the nudge would show: not seen yet, a recent account, and mail in the list.
    fn command_nudge_due(&self) -> bool {
        !self.account.settings().has_shown_command_nudge
            && self.joined().is_some_and(|joined| joined > SINCE)
            && self.inbox.count() > 0
    }

    /// Once a frame, before the palette draws: the timer, the palette closing on a nudge, and
    /// the palette's placeholder.
    pub(crate) fn command_nudge_frame(&mut self, now: f64) {
        let due = self.command_nudge_due();
        let opened = *self.command_nudge.opened.get_or_insert(now);
        if due && self.command_nudge.nudge.is_none() && now - opened >= DELAY {
            self.command_nudge.nudge = Some(Nudge::FirstOpen);
        } else if due && self.command_nudge.nudge.is_none() {
            self.ctx
                .request_repaint_after_secs((opened + DELAY - now) as f32);
        }
        // The palette closing.
        let open = self.commands.palette_id().is_some();
        if self.command_nudge.palette_open && !open && due && self.command_nudge.nudge.is_some() {
            self.account
                .update_settings(|settings| settings.has_shown_command_nudge = true);
        }
        self.command_nudge.palette_open = open;
        let placeholder = self
            .command_nudge
            .nudge
            .filter(|_| self.command_nudge_due())
            .map(|nudge| nudge.placeholder().into());
        self.commands.set_palette_nudge(placeholder);
    }

    /// The main palette's placeholder from the nudge, while it would show (for tests).
    pub fn command_nudge(&self) -> Option<&'static str> {
        self.command_nudge
            .nudge
            .filter(|_| self.command_nudge_due())
            .map(Nudge::placeholder)
    }

    /// Records the nudge as seen: a triage key, Done, a reply or moving between conversations
    /// ends the nudge.
    pub(crate) fn dismiss_command_nudge(&mut self) {
        if self.command_nudge_due() {
            self.account
                .update_settings(|settings| settings.has_shown_command_nudge = true);
        }
        self.command_nudge.nudge = None;
    }

    /// Account setup ends on the palette's first-run nudge, without the 3 s wait.
    pub(crate) fn command_nudge_first_open(&mut self) {
        if self.command_nudge_due() {
            self.command_nudge.nudge = Some(Nudge::FirstOpen);
        }
    }

    /// Back from a conversation to the list, which with the nudge due opens the palette on its
    /// reminder.
    pub(crate) fn command_nudge_left_thread(&mut self) {
        if self.thread.is_none() && self.command_nudge_due() {
            self.command_nudge.nudge = Some(Nudge::LeftThread);
            self.commands.open_palette(None);
        }
    }
}
