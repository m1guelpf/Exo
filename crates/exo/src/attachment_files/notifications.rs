//! The file notifications: a card per download over the toast, newest on top. While the file is
//! fetched it reads its name and `{n}%`, with `Show` greyed; once saved, `Show` reveals it and its
//! name previews it; when it fails, the failure and `Settings`. A finished one fades out after 6 s
//! and goes at 8 s.

use std::path::PathBuf;

use egui::{Context, Rect};
use exo_ui::{ToastActionView, ToastKind, ToastView, motion};

use super::{AttachmentFiles, State};

/// A notification is 48px tall with 12px above it: each card sits this far above the one below it.
pub(crate) const STEP: f32 = 60.0;

/// What a click on a file notification asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Click {
    /// `Show`: the saved file in Finder.
    Show(PathBuf),
    /// `Settings`, on a failure: the Downloads dialog.
    Settings,
    /// The saved file's name: a preview of it.
    Preview(PathBuf, String),
}

impl AttachmentFiles {
    /// Draws the downloads' notifications at the bottom left of `area`, the lowest `bottom` px
    /// above its bottom edge (over the toast when one shows), and returns what was clicked. A
    /// click on an action or the × lets that notification go.
    pub(crate) fn show_notifications(
        &mut self,
        ctx: &Context,
        area: Rect,
        bottom: f32,
        now: f64,
    ) -> Option<Click> {
        let mut clicked = None;
        let mut cleared = None;
        for (index, download) in self.downloads.iter().enumerate() {
            let id = exo_ui::id(ctx, ("exo::file-notification", &download.key));
            let hover_id = id.with("pointer");
            let hovered = ctx
                .data(|data| data.get_temp::<bool>(hover_id))
                .unwrap_or(false);
            // A finished card fades over 2 s `ease-in`; the pointer on it brings it back over .2 s.
            let out = download.fading(now) && !hovered;
            let opacity = 1.0
                - motion::transition(
                    ctx,
                    id.with("out"),
                    out,
                    if out { 2.0 } else { 0.2 },
                    motion::CubicBezier::EASE_IN,
                );
            let (text, percentage, kind) = match &download.state {
                // The share shows while it's under 100 and above 0.
                State::Running => (
                    download.name.clone(),
                    Some(download.progress.floor())
                        .filter(|percent| *percent > 0.0 && *percent < 100.0)
                        .map(|percent| percent as u32),
                    ToastKind::Info,
                ),
                State::Saved(_) => (download.name.clone(), None, ToastKind::Info),
                State::Failed(failure) => (failure.text(&download.name), None, ToastKind::Error),
            };
            let action = match &download.state {
                State::Failed(_) => ToastActionView {
                    name: "Settings",
                    keys: &[],
                    tooltip: None,
                    hidden: false,
                    disabled: false,
                },
                state => ToastActionView {
                    name: "Show",
                    keys: &[],
                    tooltip: None,
                    hidden: false,
                    disabled: !matches!(state, State::Saved(_)),
                },
            };
            let view = ToastView {
                text: &text,
                kind,
                actions: &[action],
                divided: false,
                percentage,
                clickable: matches!(download.state, State::Saved(_)),
            };
            let response = view.show(ctx, id, area, bottom + STEP * index as f32, opacity);
            ctx.data_mut(|data| data.insert_temp(hover_id, response.hovered));
            if response.closed {
                cleared = Some(download.key.clone());
            }
            match (&download.state, response.clicked, response.text_clicked) {
                (State::Saved(path), Some(_), _) => {
                    clicked = Some(Click::Show(path.clone()));
                    cleared = Some(download.key.clone());
                }
                (State::Failed(_), Some(_), _) => {
                    clicked = Some(Click::Settings);
                    cleared = Some(download.key.clone());
                }
                (State::Saved(path), None, true) => {
                    clicked = Some(Click::Preview(path.clone(), download.name.clone()))
                }
                _ => {}
            }
        }
        if let Some(key) = cleared {
            self.clear(&key);
        }
        clicked
    }
}
