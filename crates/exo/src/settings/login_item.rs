//! Open at Login, a setting of Exo's own: Exo as a login item, through fastframe-autostart
//! (`SMAppService.mainApp`). It's off until it's turned on.
//!
//! macOS keeps the only record: the setting reads it back each time the menu opens, so a change
//! made in System Settings › General › Login Items shows. Both reading and changing it can
//! block, so they run off the UI thread.

use std::sync::{Arc, Mutex, PoisonError};

use fastframe_autostart::{Autostart, Start, State};

/// Exo's login item, as macOS last said.
#[derive(Debug)]
pub struct LoginItem {
    /// On or off; `None` until read, and where Exo can't be a login item (a bare binary, as
    /// `cargo run` starts it).
    on: Arc<Mutex<Option<bool>>>,
    /// Repainted when macOS answers.
    ctx: egui::Context,
}

impl LoginItem {
    /// Reads the login item for `ctx`'s app.
    pub fn new(ctx: &egui::Context) -> Self {
        let item = Self {
            on: Arc::default(),
            ctx: ctx.clone(),
        };
        item.read();
        item
    }

    /// On or off, or `None` where there's no setting to show.
    pub fn on(&self) -> Option<bool> {
        *self.on.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads it again.
    pub fn read(&self) {
        self.spawn(|autostart| Ok(autostart.state()));
    }

    /// Turns it on when off and off when on. The setting shows the change at once.
    pub fn toggle(&self) {
        let Some(on) = self.on() else { return };
        *self.on.lock().unwrap_or_else(PoisonError::into_inner) = Some(!on);
        self.spawn(move |autostart| autostart.set((!on).then_some(Start::Window)));
    }

    /// Runs `call` on a thread of its own, then shows the state it leaves.
    fn spawn(&self, call: impl FnOnce(&Autostart) -> std::io::Result<State> + Send + 'static) {
        let (on, ctx) = (Arc::clone(&self.on), self.ctx.clone());
        std::thread::spawn(move || {
            // macOS goes by the bundle; the name is for other systems' entries.
            let autostart = Autostart::new("exo", "Exo");
            let state = call(&autostart).unwrap_or_else(|error| {
                log::warn!("changing Exo's login item: {error}");
                autostart.state()
            });
            if let State::NeedsApproval(_) = state {
                log::warn!(
                    "Exo's login item waits for approval in System Settings › General › Login Items"
                );
            }
            *on.lock().unwrap_or_else(PoisonError::into_inner) = shown(state);
            ctx.request_repaint();
        });
    }
}

/// What the setting shows for `state`: on once registered, approved or not.
fn shown(state: State) -> Option<bool> {
    match state {
        State::Unsupported => None,
        State::Off => Some(false),
        State::On(_) | State::NeedsApproval(_) => Some(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_binary_has_no_setting() {
        assert_eq!(shown(State::Unsupported), None);
        assert_eq!(shown(State::Off), Some(false));
        assert_eq!(shown(State::NeedsApproval(Start::Window)), Some(true));
    }
}
