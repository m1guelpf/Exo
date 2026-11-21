//! Where the app counts: marking done, sending, Get Me To Zero clearing, and reaching Inbox Zero;
//! and the frame's popups, animation and dialog.

use egui::{Context, Rect};
use exo_store::{Achievements, ListId, Statistics};
use jiff::civil::Date;

use super::{Reached, View, dialog, streak};
use crate::Mail;

impl Mail {
    /// Counts with `count` (on the account's statistics and this device's celebrated levels)
    /// and raises its popup, if it reached a level.
    fn celebrate<R>(
        &mut self,
        count: impl FnOnce(&mut Statistics, &mut Achievements, Date) -> R,
        now: f64,
    ) -> R
    where
        R: Into<Option<Reached>> + Copy,
    {
        let mut statistics = self.account.settings().statistics.clone();
        let mut seen = self.account.device_settings().achievements.clone();
        let counted = count(&mut statistics, &mut seen, self.now.date());
        self.account
            .update_settings(|settings| settings.statistics = statistics);
        self.account
            .update_device_settings(|device| device.achievements = seen);
        if let Some(reached) = counted.into() {
            self.achievements.popups.push(reached, now);
        }
        counted
    }

    /// `threads` conversations marked done.
    pub(crate) fn count_done(&mut self, threads: usize, now: f64) {
        self.celebrate(
            |statistics, seen, _| super::mark_done(statistics, seen, threads as u64),
            now,
        );
    }

    /// A message sent.
    pub(crate) fn count_send(&mut self, now: f64) {
        self.celebrate(super::sent, now);
    }

    /// Get Me To Zero's progress: what each of its bulk actions has marked done since last frame.
    pub(crate) fn count_cleared(&mut self, bulk: &[crate::bulk::Summary], now: f64) {
        for action in bulk
            .iter()
            .filter(|action| action.name == crate::bulk::GET_ME_TO_ZERO)
        {
            use crate::bulk::Status;
            let counted = self.achievements.cleared.entry(action.token).or_default();
            let cleared = action.done.saturating_sub(*counted);
            if !matches!(action.status, Status::Running | Status::Done) || cleared == 0 {
                continue;
            }
            *counted = action.done;
            self.celebrate(
                |statistics, seen, _| super::cleared(statistics, seen, cleared as u64),
                now,
            );
        }
    }

    /// Inbox Zero in Inbox or Important counts once a day and moves the streak on once a week: when
    /// the list empties or is switched to empty (`reached`), and when the window gets the keyboard
    /// back. week's first plays `Week N`.
    pub(crate) fn count_inbox_zero(&mut self, ctx: &Context, zero: bool, reached: bool, now: f64) {
        let focused = ctx.input(|input| input.focused);
        let refocused = focused && !self.achievements.focused;
        self.achievements.focused = focused;
        let inbox = matches!(self.list, ListId::Inbox | ListId::Important);
        if !(zero
            && inbox
            && (reached || refocused)
            && super::would_count(&self.account.settings().statistics, self.now.date()))
        {
            return;
        }
        let hit = self.celebrate(super::hit_inbox_zero, now);
        if hit.week {
            let streak = self
                .account
                .settings()
                .statistics
                .inbox_zero_streak_week_current;
            self.achievements.animation = Some(streak::Animation::new(streak, now));
        }
    }

    /// Opens the Achievements dialog on `view`, clearing the `New` badges of the view it leaves.
    pub(crate) fn show_achievements(&mut self, view: View) {
        let left = self.achievements.dialog.replace(view);
        self.leave_achievements(left);
    }

    /// Closes the dialog, clearing its view's `New` badges.
    pub(crate) fn close_achievements(&mut self) {
        let left = self.achievements.dialog.take();
        self.leave_achievements(left);
    }

    fn leave_achievements(&mut self, view: Option<View>) {
        if let Some(view) = view {
            let mut seen = self.account.device_settings().achievements.clone();
            super::leave(view, &mut seen);
            self.account
                .update_device_settings(|device| device.achievements = seen);
        }
    }

    /// Paints `Week N` over the photo (`window`), under the app; then the dim layer behind the
    /// popups at Inbox Zero (150ms).
    pub(crate) fn paint_behind_app(
        &mut self,
        ctx: &Context,
        painter: &egui::Painter,
        window: Rect,
        now: f64,
    ) {
        if let Some(animation) = self
            .achievements
            .animation
            .filter(|animation| !animation.is_over(now))
        {
            animation.paint(painter, window, now);
            ctx.request_repaint();
            return;
        }
        self.achievements.animation = None;
        let dim = exo_ui::motion::transition(
            ctx,
            exo_ui::id(ctx, "achievement-dim"),
            self.achievements.popups.any(),
            0.15,
            exo_ui::motion::CubicBezier::EASE,
        );
        if dim > 0.0 {
            painter.rect_filled(
                window,
                0.0,
                egui::Color32::from_black_alpha((51.0 * dim).round() as u8),
            );
        }
    }

    /// The dialog, then the popups over the page (below the tab strip), above the regular toast.
    pub(crate) fn show_achievement_overlays(
        &mut self,
        ctx: &Context,
        page: Rect,
        zero: bool,
        now: f64,
    ) {
        let shown = self.achievements.dialog;
        let statistics = self.account.settings().statistics.clone();
        let seen = self.account.device_settings().achievements.clone();
        let view = shown.unwrap_or(View::Main);
        let asked = dialog::show(
            ctx,
            page,
            shown.is_some(),
            &dialog::Shown {
                view,
                statistics: &statistics,
                seen: &seen,
                today: self.now.date(),
            },
        );
        match asked {
            Some(dialog::Asked::Close) => self.close_achievements(),
            Some(dialog::Asked::Show(view)) => self.show_achievements(view),
            None => {}
        }
        // First in the stack (`unshift`), so over the file notifications and the toast: 48px
        // cards 12px apart.
        let step = 48.0 + 12.0;
        let under = usize::from(self.commands.toast_text().is_some())
            + self.attachment_files.downloads().len();
        if let Some(view) = self.achievements.popups.show(
            ctx,
            page,
            self.commands.toast_bottom() + step * under as f32,
            zero,
            now,
        ) {
            self.show_achievements(view);
        }
    }
}
