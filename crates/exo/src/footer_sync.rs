//! The footer's sync line, first match wins: what's on its way to Gmail since it was last offline
//! or since launch (`Sending: 2 of 3`, `Uploading: 1 of 4`), what's waiting for the network
//! (`1 email to send`, `3 changes to upload`), a bulk action's progress, Gmail's history being
//! applied (`Updating: 12 of 40`), `Connecting…` before the session's first round of it, or
//! nothing. The store's count of what's unsent keeps the counts it counts from.

use egui::{Rect, Ui};
use exo_store::{SyncProgress, Unsent};

/// How long a round of history runs before it shows, but for the session's first.
const SHOW_SYNC_DELAY: f64 = 1.0;
/// The line fades in and out (opacity, 200ms ease-in-out); a change of what it says doesn't.
const FADE: f32 = 0.2;
/// The line before the session's first round of history.
const CONNECTING: &str = "Connecting…";

/// What the line remembers between frames.
#[derive(Default)]
pub(crate) struct FooterSync {
    /// What it counts progress against: what was waiting while offline, or at launch.
    from: Option<Unsent>,
    /// The counts were read once: what's waiting at launch is counted from.
    counted: bool,
    /// When the round of history running started.
    started: Option<f64>,
    /// The last line shown, which fades out.
    last: String,
}

impl FooterSync {
    /// The line for this frame, at `now` seconds: `uploading` is the snapshot's count of what's
    /// on its way to Gmail (`None` before it's read), `bulk` a running bulk action's status.
    pub(crate) fn line(
        &mut self,
        uploading: Option<Unsent>,
        online: bool,
        bulk: Option<String>,
        progress: SyncProgress,
        now: f64,
    ) -> Option<String> {
        // A round shows once it has run a second, or at once the first time.
        let running = progress
            .changes
            .filter(|(total, done)| *total > 0 && done < total);
        match running {
            Some(_) => {
                self.started.get_or_insert(now);
            }
            None => self.started = None,
        }
        let updating = running.filter(|_| {
            !progress.caught_up || self.started.is_some_and(|at| now - at > SHOW_SYNC_DELAY)
        });
        if let Some(text) = uploading.and_then(|pending| self.pending(pending, online)) {
            return Some(text);
        }
        if let Some(bulk) = bulk.filter(|_| online) {
            return Some(bulk);
        }
        if let Some((total, done)) = updating {
            return Some(format!(
                "Updating: {} of {}",
                crate::format::thousands(done + 1),
                crate::format::thousands(total)
            ));
        }
        (!progress.caught_up && online).then(|| CONNECTING.to_owned())
    }

    /// What's unsent: progress against what was waiting while online, the counts while
    /// offline.
    fn pending(&mut self, pending: Unsent, online: bool) -> Option<String> {
        let launch = !std::mem::replace(&mut self.counted, true);
        let total = pending.sends + pending.changes;
        if !online {
            if total == 0 {
                return None;
            }
            self.from = Some(pending);
            return Some(if pending.sends > 0 {
                format!("{} to send", plural(pending.sends, "email"))
            } else {
                format!("{} to upload", plural(pending.changes, "change"))
            });
        }
        if total == 0 || (self.from.is_none() && !launch) {
            self.from = None;
            return None;
        }
        let from = *self.from.get_or_insert(pending);
        let from_total = from.sends + from.changes;
        if pending.sends > 0 && pending.sends <= from.sends {
            Some(format!(
                "Sending: {} of {}",
                from.sends - pending.sends + 1,
                from.sends
            ))
        } else if total <= from_total {
            Some(format!(
                "Uploading: {} of {}",
                from_total - total + 1,
                from_total
            ))
        } else {
            None
        }
    }

    /// Paints `line` in the right pane's footer, fading it in when one appears and the last one
    /// out when it goes.
    pub(crate) fn paint(&mut self, ui: &Ui, pane: Rect, rem: f32, line: Option<String>) {
        let shown = line.is_some();
        if let Some(line) = line {
            self.last = line;
        }
        let opacity = exo_ui::motion::transition(
            ui.ctx(),
            exo_ui::id(ui.ctx(), "footer-sync-line"),
            shown,
            FADE,
            exo_ui::motion::CubicBezier::EASE_IN_OUT,
        );
        if opacity > 0.0 {
            crate::shell::sync_status(ui.painter(), pane, rem, &self.last, opacity);
        }
        // The second a round of history waits before it shows.
        if self.started.is_some() && !shown {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(SHOW_SYNC_DELAY));
        }
    }
}

/// `{n} email`/`{n} emails`.
fn plural(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n > 1 { "s" } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONLINE: bool = true;
    const OFFLINE: bool = false;

    fn waiting(sends: usize, changes: usize) -> Option<Unsent> {
        Some(Unsent { sends, changes })
    }

    fn caught_up() -> SyncProgress {
        SyncProgress {
            caught_up: true,
            changes: None,
        }
    }

    #[test]
    fn offline_it_counts_what_waits_and_back_online_counts_it_going() {
        let mut footer = FooterSync::default();
        assert_eq!(
            footer.line(waiting(0, 0), ONLINE, None, caught_up(), 0.0),
            None
        );
        assert_eq!(
            footer.line(waiting(0, 1), ONLINE, None, caught_up(), 0.0),
            None,
            "online changes go without a word"
        );
        assert_eq!(
            footer
                .line(waiting(0, 1), OFFLINE, None, caught_up(), 0.0)
                .as_deref(),
            Some("1 change to upload")
        );
        assert_eq!(
            footer
                .line(waiting(0, 3), OFFLINE, None, caught_up(), 0.0)
                .as_deref(),
            Some("3 changes to upload")
        );
        assert_eq!(
            footer
                .line(waiting(2, 3), OFFLINE, None, caught_up(), 0.0)
                .as_deref(),
            Some("2 emails to send"),
            "sends first"
        );
        assert_eq!(
            footer
                .line(waiting(1, 3), ONLINE, None, caught_up(), 0.0)
                .as_deref(),
            Some("Sending: 2 of 2")
        );
        assert_eq!(
            footer
                .line(waiting(0, 3), ONLINE, None, caught_up(), 0.0)
                .as_deref(),
            Some("Uploading: 3 of 5")
        );
        assert_eq!(
            footer
                .line(waiting(0, 1), ONLINE, None, caught_up(), 0.0)
                .as_deref(),
            Some("Uploading: 5 of 5")
        );
        assert_eq!(
            footer.line(waiting(0, 0), ONLINE, None, caught_up(), 0.0),
            None,
            "done"
        );
        assert_eq!(
            footer.line(waiting(0, 2), ONLINE, None, caught_up(), 0.0),
            None,
            "counting starts over"
        );
    }

    #[test]
    fn what_waits_at_launch_is_counted_going() {
        let mut footer = FooterSync::default();
        assert_eq!(
            footer.line(None, ONLINE, None, caught_up(), 0.0),
            None,
            "not read yet"
        );
        assert_eq!(
            footer
                .line(waiting(1, 1), ONLINE, None, caught_up(), 0.0)
                .as_deref(),
            Some("Sending: 1 of 1")
        );
        assert_eq!(
            footer
                .line(waiting(0, 1), ONLINE, None, caught_up(), 0.0)
                .as_deref(),
            Some("Uploading: 2 of 2")
        );
        assert_eq!(
            footer.line(waiting(0, 3), ONLINE, None, caught_up(), 0.0),
            None,
            "more than it counted from"
        );
    }

    #[test]
    fn history_shows_at_once_the_first_time_then_after_a_second() {
        let mut footer = FooterSync::default();
        let first = SyncProgress {
            caught_up: false,
            changes: None,
        };
        assert_eq!(
            footer
                .line(waiting(0, 0), ONLINE, None, first, 0.0)
                .as_deref(),
            Some("Connecting…")
        );
        assert_eq!(
            footer.line(waiting(0, 0), OFFLINE, None, first, 0.0),
            None,
            "offline, nothing connects"
        );
        let running = |done| SyncProgress {
            caught_up: false,
            changes: Some((1200, done)),
        };
        assert_eq!(
            footer
                .line(waiting(0, 0), ONLINE, None, running(0), 0.0)
                .as_deref(),
            Some("Updating: 1 of 1,200")
        );
        assert_eq!(
            footer.line(waiting(0, 0), ONLINE, None, caught_up(), 0.5),
            None
        );
        let again = |done| SyncProgress {
            caught_up: true,
            changes: Some((40, done)),
        };
        assert_eq!(
            footer.line(waiting(0, 0), ONLINE, None, again(0), 1.0),
            None,
            "not yet a second"
        );
        assert_eq!(
            footer
                .line(waiting(0, 0), ONLINE, None, again(11), 2.5)
                .as_deref(),
            Some("Updating: 12 of 40")
        );
    }

    #[test]
    fn what_goes_to_gmail_comes_before_a_bulk_action_and_history() {
        let mut footer = FooterSync::default();
        let bulk = || Some("Updating: 120 of 4,000".to_owned());
        let running = SyncProgress {
            caught_up: false,
            changes: Some((5, 0)),
        };
        assert_eq!(
            footer
                .line(waiting(0, 2), OFFLINE, bulk(), running, 0.0)
                .as_deref(),
            Some("2 changes to upload")
        );
        assert_eq!(
            footer
                .line(waiting(0, 0), OFFLINE, bulk(), running, 0.0)
                .as_deref(),
            Some("Updating: 1 of 5"),
            "a bulk action only shows online"
        );
        assert_eq!(
            footer.line(waiting(0, 0), ONLINE, bulk(), running, 0.0),
            bulk()
        );
    }
}
