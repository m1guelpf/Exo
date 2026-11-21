//! Which part of the open thread has the keyboard: the focus regions under the thread pane. ↑
//! from the first message reaches the header's controls, ← the back gutter, and ↓ past the last
//! message the Show New Messages bar.

use exo_commands::Region;

use super::{Action, ThreadView, header::Control};

/// A part of the thread view that can have the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Zone {
    /// The messages (`THREAD_MESSAGES`).
    Messages,
    /// The header, with the control at this place in its tab order focused, or none.
    Header(Option<usize>),
    /// The back gutter; `from_header` when it took the keyboard from the header, where → goes
    /// back to.
    ActionPane { from_header: bool },
    /// The Show New Messages bar.
    NewMessages,
}

/// A key in the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderKey {
    /// Escape: back to the messages.
    Escape,
    /// ↑ / P: from the status line to the actions.
    Up,
    /// ↓ / N: from the actions to the status line, else to the messages.
    Down,
    /// ← / →: the previous or next control, not wrapping; ← with none focused goes to the back
    /// gutter.
    Step(isize),
    /// Enter: clicks the focused control.
    Enter,
}

/// A key in the back gutter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GutterKey {
    /// → / Tab: back to the messages, or the header if that's where the gutter took the keyboard
    /// from.
    Right,
}

impl ThreadView {
    /// The focus region the thread view's keyboard is in.
    pub fn region(&self) -> Region {
        match self.zone {
            Zone::Messages => Region::ThreadMessages,
            Zone::Header(_) => Region::ThreadMessagesHeader,
            Zone::ActionPane { .. } => Region::ActionPane,
            Zone::NewMessages => Region::ThreadMessagesNewMessages,
        }
    }

    /// The header's focused control, while the header has the keyboard.
    pub(super) fn header_focus(&self) -> Option<Control> {
        let Zone::Header(Some(at)) = self.zone else {
            return None;
        };
        self.header_controls().get(at).copied()
    }

    /// Gives the header the keyboard: the control whose tab index it last focused,
    /// else the one with tab index 1 (the status line), else none.
    fn enter_header(&mut self) {
        let controls = self.header_controls();
        let start = Control::tab_start(&controls);
        let with = |tab: u8| {
            controls
                .iter()
                .position(|control| control.tab_index(start) == tab)
        };
        self.zone = Zone::Header(with(self.header_tab).or_else(|| with(1)));
    }

    /// A key in the header. Returns what Enter clicked.
    pub(crate) fn header_key(&mut self, key: HeaderKey) -> Option<Action> {
        let Zone::Header(focused) = self.zone else {
            return None;
        };
        let controls = self.header_controls();
        let start = Control::tab_start(&controls);
        let status = controls
            .iter()
            .position(|control| *control == Control::Status);
        let control = focused.and_then(|at| controls.get(at).copied());
        match key {
            HeaderKey::Escape => self.zone = Zone::Messages,
            // ↑ from the status line: the actions take the keyboard on Done, and the tab index to
            // come back to becomes 1.
            HeaderKey::Up if control == Some(Control::Status) => {
                self.zone = Zone::Header(
                    controls
                        .iter()
                        .position(|control| *control == Control::Done),
                );
                self.header_tab = 1;
            }
            HeaderKey::Up => {}
            HeaderKey::Down => match (control, status) {
                (Some(control), Some(status)) if control != Control::Status => {
                    self.zone = Zone::Header(Some(status));
                    self.header_tab = 1;
                }
                _ => self.zone = Zone::Messages,
            },
            HeaderKey::Step(-1) if focused.is_none() => {
                self.zone = Zone::ActionPane { from_header: true }
            }
            HeaderKey::Step(delta) => {
                let to = focused.map_or(0, |at| at as isize + delta);
                if let Some(next) = usize::try_from(to).ok().filter(|to| *to < controls.len()) {
                    self.zone = Zone::Header(Some(next));
                    self.header_tab = controls[next].tab_index(start);
                }
            }
            HeaderKey::Enter => return control.map(|control| control.action(self.status.as_ref())),
        }
        None
    }

    /// A key in the back gutter.
    pub fn gutter_key(&mut self, key: GutterKey) {
        let GutterKey::Right = key;
        match self.zone {
            Zone::ActionPane { from_header: true } => self.enter_header(),
            Zone::ActionPane { from_header: false } => self.zone = Zone::Messages,
            _ => {}
        }
    }

    /// ← in the messages, pressed: scrolls a widened column left, and at its left
    /// edge gives the back gutter the keyboard. Held (`repeat`), it only scrolls.
    pub fn left(&mut self, repeat: bool) {
        if !self.scroll_sideways(-1)
            && !repeat
            && matches!(self.zone, Zone::Messages | Zone::NewMessages)
        {
            self.zone = Zone::ActionPane { from_header: false };
        }
    }

    /// Enter on the Show New Messages bar.
    pub fn new_messages_focused(&self) -> bool {
        self.zone == Zone::NewMessages
    }

    /// A click in the thread pane gives the messages the keyboard.
    pub(super) fn clicked_in_pane(&mut self) {
        self.zone = Zone::Messages;
    }

    /// ↑ / ↓ past the first or last message, not held: the header, or
    /// the Show New Messages bar if there is one. Returns whether the keyboard moved.
    pub(super) fn arrow_out(&mut self, delta: isize, at: usize) -> bool {
        if delta < 0 && self.zone == Zone::NewMessages {
            // Back onto the message it left from, unless that's the only one.
            if at > 0 {
                self.zone = Zone::Messages;
                self.focus_at(at, 0);
            }
            return true;
        }
        let next = self.step(at, delta);
        let first = |index: usize| index == 0;
        let last = |index: usize| index + 1 == self.items.len();
        if delta < 0 && first(next) && first(at) {
            self.enter_header();
            true
        } else if delta > 0 && last(next) && last(at) {
            if self.has_new {
                self.zone = Zone::NewMessages;
            }
            true
        } else {
            false
        }
    }
}
