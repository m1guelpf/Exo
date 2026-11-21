//! The popped-out draft: one draft at a time in a card anchored to the bottom of the window, right
//! of the thread (or over the list), while the rest of the app goes on. Its thread keeps a
//! placeholder under the message it answers.

use egui::{Color32, CornerRadius, Rect, Sense, pos2, vec2};
use exo_store::DraftAction;
use jiff::Timestamp;

use super::{Fields, Open, PendingReply, clean_up, factory, form, form::Surface};

/// The card's padding: `14px 2rem 4px`.
const CARD_PADDING_TOP: f32 = 14.0;
const CARD_PADDING_BOTTOM: f32 = 4.0;
/// In a window at least 1440px tall, the form is at most 70% (`max-height: 70vh`) of the page under
/// the strip, rather than [`form::MAX_BELOW_PAGE`].
const TALL_WINDOW: f32 = 1440.0;
const TALL_FORM_MAX: f32 = 0.7;
/// How long the pointer rests before it counts as still.
const STILL: f32 = 0.1;
/// The card's slide out of the contact pane's way: `transform 100ms ease`.
const SLIDE: f32 = 0.1;

impl crate::Mail {
    /// ⌘⇧P and the header button: the focused draft pops out, or the popped-out one pops back in.
    pub(super) fn compose_pop(&mut self) {
        match (self.compose.focus.clone(), self.compose.popped.clone()) {
            (Some(id), Some(popped)) if id == popped => self.pop_in(),
            (Some(id), _) => self.pop_out(&id),
            (None, Some(_)) => self.pop_in(),
            (None, None) => {}
        }
    }

    /// Pops draft `id` out, with the keyboard. A new message leaves its pane, which goes back to
    /// the list; a draft popped out before goes back to its thread.
    pub(super) fn pop_out(&mut self, id: &str) {
        if let Some(previous) = self.compose.popped.take().filter(|previous| previous != id) {
            self.put_back(&previous);
        }
        if self.compose.pane() == Some(id) {
            self.compose.pane = None;
        }
        self.compose.popped = Some(id.to_owned());
        self.compose.focus = Some(id.to_owned());
    }

    /// ⌘/ in a draft: it pops out unless it is, and search opens with an empty bar (only the
    /// filter's token), which has the keyboard.
    pub(super) fn pop_out_and_search(&mut self, now: f64) {
        let Some(id) = self
            .compose
            .focus
            .clone()
            .filter(|_| !self.compose.focused().is_some_and(Open::is_snippet))
        else {
            return;
        };
        if self.compose.popped.as_deref() != Some(id.as_str()) {
            self.pop_out(&id);
        }
        self.search_enter(None, now);
        let initial = crate::search::initial_query(
            &exo_store::ListId::Inbox,
            &[],
            [],
            self.filter_search_suffix(),
        );
        if let Some(search) = &mut self.search {
            *search = crate::search::Search::new(&initial, &self.snapshot.labels);
            search.contact = Some(None);
        }
        self.compose.focus = None;
    }

    /// Pops the popped-out draft back in: a reply into its thread, opened if it isn't, a new
    /// message into the new-message pane, keeping its field. Either takes the place of a snippet
    /// being edited, once `Save Snippet?` has asked.
    pub(super) fn pop_in(&mut self) {
        if self.compose.popped.is_some() && self.snippet_unsaved() {
            self.ask_leaving_snippet(Box::new(Self::pop_in));
            return;
        }
        let Some(id) = self.compose.popped.take() else {
            return;
        };
        let Some(open) = self.compose.drafts.iter().find(|open| open.id == id) else {
            return;
        };
        let (thread, anchor, new) = (
            open.thread.clone(),
            open.record.in_reply_to.clone(),
            open.is_new_email(),
        );
        match thread.filter(|_| !new) {
            Some(thread) => {
                if self.thread.as_ref().is_none_or(|view| view.id() != thread) {
                    self.open_thread(thread.clone());
                }
                self.compose.loaded = Some(thread);
                if let (Some(view), Some(anchor)) = (&mut self.thread, anchor) {
                    view.reveal_draft(&anchor);
                }
            }
            None => {
                self.leave_pane();
                self.close_thread();
                self.compose.pane = Some((id.clone(), None));
            }
        }
        self.compose.focus = Some(id);
    }

    /// Escape or the cross: the pop-out closes and the keyboard goes back to the app. The draft
    /// stays in its thread, saved (or, untouched, dropped).
    pub(super) fn close_popout(&mut self) {
        let Some(id) = self.compose.popped.take() else {
            return;
        };
        if self.compose.focus.as_deref() == Some(id.as_str()) {
            self.compose.focus = None;
        }
        self.put_back(&id);
    }

    /// A draft that's no longer popped out: it stays open under its thread when that's showing,
    /// and is otherwise saved (or dropped) and closed.
    fn put_back(&mut self, id: &str) {
        let shown = self.thread.as_ref().map(|view| view.id().to_owned());
        let Some(at) = self.compose.drafts.iter().position(|open| open.id == id) else {
            return;
        };
        let open = &self.compose.drafts[at];
        if open.thread.is_some() && open.thread == shown && !open.is_new_email() {
            return;
        }
        let mut open = self.compose.drafts.remove(at);
        clean_up(&mut open, &self.account, &self.snapshot.aliases, &self.now);
        if self.compose.focus.as_deref() == Some(id) {
            self.compose.focus = None;
        }
    }

    /// ⇧C (`Pop Out Compose`): a new message, popped out over whatever is showing.
    pub(crate) fn compose_new_popped(&mut self, fields: Fields) {
        let record = factory::compose(&self.compose_account(), fields, Timestamp::now());
        let mut open = Open::new(factory::new_id(), None, record);
        self.with_auto_bcc(&mut open);
        let id = open.id.clone();
        self.compose.drafts.push(open);
        self.pop_out(&id);
    }

    /// ⇧R, ⇧A, ⇧F: a reply, reply all or forward, popped out once its draft exists.
    pub(super) fn compose_reply_popped(&mut self, action: DraftAction) {
        self.compose_reply(action, None);
        if let Some(pending) = &mut self.compose.pending {
            pending.pop_out = true;
        } else if let Some(id) = self.compose.focus.clone() {
            self.pop_out(&id);
        }
    }

    /// Pops out the reply just created, if it asked to be.
    pub(super) fn pop_out_created(&mut self, pending: &PendingReply, id: &str) {
        if pending.pop_out {
            self.pop_out(id);
        }
    }

    /// Whether a draft is popped out, and if so whether it has the keyboard (⌘D's sections).
    pub(crate) fn popout_focus(&self) -> Option<bool> {
        let popped = self.compose.popped.as_ref()?;
        Some(self.compose.focus.as_ref() == Some(popped))
    }

    /// Puts the keyboard on the popped-out draft, or takes it off.
    pub(crate) fn focus_popout(&mut self, on: bool) {
        let Some(popped) = self.compose.popped.clone() else {
            return;
        };
        if on {
            self.compose.focus = Some(popped);
        } else if self.compose.focus.as_ref() == Some(&popped) {
            self.compose.focus = None;
        }
    }

    /// ⌘D: the keyboard moves between the app and the pop-out, which keeps its field. Out of it,
    /// its text field lets go too, so what's typed next is the app's.
    pub(super) fn cycle_focus(&mut self) {
        let Some(popped) = self.compose.popped.clone() else {
            return;
        };
        if self.compose.focus.as_deref() == Some(popped.as_str()) {
            self.compose.focus = None;
            self.ctx.memory_mut(|memory| {
                if let Some(focused) = memory.focused() {
                    memory.surrender_focus(focused);
                }
            });
        } else {
            self.compose.focus = Some(popped);
        }
    }

    /// The pop-out's card as last drawn: what's under it doesn't have the pointer there.
    pub(crate) fn popout_card(&self) -> Option<Rect> {
        self.compose.popout_card
    }

    /// Takes the wheel this frame from what's drawn under the pop-out, while the pointer is over
    /// its card: the card isn't in the list's or the thread's scroll chain. Returns it, for
    /// [`Self::show_compose_popout`] to have back.
    pub(crate) fn take_wheel_over_popout(&self, ctx: &egui::Context) -> Option<egui::Vec2> {
        let card = self.compose.popout_card?;
        ctx.pointer_hover_pos().filter(|at| card.contains(*at))?;
        Some(ctx.input_mut(|input| std::mem::take(&mut input.smooth_scroll_delta)))
    }

    /// Draws the pop-out over the content pane `content` (and the right pane beside it).
    pub(crate) fn show_compose_popout(
        &mut self,
        ui: &mut egui::Ui,
        content: Rect,
        rem: f32,
        window_width: f32,
        now: f64,
    ) {
        self.compose.popout_card = None;
        let Some(id) = self.compose.popped.clone() else {
            return;
        };
        let creating_event = self.calendar_pane_form();
        let aliases = self.snapshot.aliases.clone();
        let me = self.account.email().to_owned();
        let focused = self.compose.focus.as_deref() == Some(id.as_str())
            && self.compose.alert.is_none()
            && !self.compose.palette_open;
        let thread_people = self.compose.thread_people.as_ref().map(|(_, index)| index);
        let book = self.compose.book.as_ref().map(|(_, index)| index);
        let Some(open) = self.compose.drafts.iter_mut().find(|open| open.id == id) else {
            self.compose.popped = None;
            return;
        };
        let thread_reminder = self
            .thread
            .as_ref()
            .and_then(crate::thread::ThreadView::detail)
            .filter(|detail| open.thread.as_deref() == Some(detail.id.as_str()))
            .and_then(|detail| super::set_reminder(detail.reminder.as_ref()));
        let now_zoned = &self.now;
        let send_and_done = self.compose.send_and_done;
        let window = ui.ctx().content_rect();
        let max_height = if window.height() >= TALL_WINDOW {
            TALL_FORM_MAX * (window.height() - crate::chrome::tab_bar_height(ui.ctx()))
        } else {
            content.height() - form::MAX_BELOW_PAGE
        };
        let mut cx = form::Context {
            aliases: &aliases,
            me: &me,
            surface: Surface::Popout,
            focused,
            book,
            thread_people,
            pin: egui::Rangef::EVERYTHING,
            drop: None,
            now: now_zoned,
            thread_reminder,
            send_and_done,
            max_height,
            availability_tip: self.compose.availability_tip,
        };
        // The pop-out's column, its card inset 2rem each side with `padding: 14px 2rem 4px`,
        // anchored to the window's bottom.
        let (left, width) = crate::thread::popout_column(content, rem, window_width);
        let form_width = width - 8.0 * rem;
        let height = form::height(ui, open, form_width, &cx);
        let card = Rect::from_min_max(
            pos2(
                left + 2.0 * rem,
                content.bottom() - CARD_PADDING_BOTTOM - height - CARD_PADDING_TOP,
            ),
            pos2(left + width - 2.0 * rem, content.bottom()),
        );
        // Out of the contact pane's way while the pointer rests there (`transform 100ms ease`).
        let right_pane = Rect::from_min_max(
            pos2(content.right(), content.top()),
            pos2(window.right(), content.bottom()),
        );
        let slid = self.compose.still_over_right_pane || creating_event;
        let slide = crate::thread::popout_slide(content, rem, window_width)
            * exo_ui::motion::transition(
                ui.ctx(),
                exo_ui::id(ui.ctx(), "compose-popout-slide"),
                slid,
                SLIDE,
                exo_ui::motion::CubicBezier::EASE,
            );
        let card = card.translate(vec2(slide, 0.0));
        still_over_right_pane(
            ui,
            &mut self.compose.still_over_right_pane,
            right_pane,
            card,
        );
        self.compose.popout_card = Some(card);
        paint_card(ui, card, rem);
        // The file drop zone: the card's width, from 16px above it to the window's bottom.
        cx.drop = Some(Rect::from_min_max(
            pos2(card.left(), card.top() - 16.0),
            pos2(card.right(), content.bottom()),
        ));
        // The card takes the clicks its form doesn't.
        let _ = ui.interact(
            card,
            exo_ui::id(ui.ctx(), ("compose-popout", &id)),
            Sense::click(),
        );
        let form_rect = Rect::from_min_size(
            pos2(card.left() + 2.0 * rem, card.top() + CARD_PADDING_TOP),
            vec2(form_width, 0.0),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(card)
                .id_salt(("compose-popout-form", &id)),
        );
        child.set_clip_rect(card);
        let response = form::show(&mut child, open, form_rect, &cx);
        let pressed = ui.input(|input| {
            input
                .pointer
                .any_pressed()
                .then(|| input.pointer.press_origin())
                .flatten()
        });
        if response.clicked_inside || pressed.is_some_and(|at| card.contains(at)) {
            // The ⌘D hint, as the press gives it the keyboard.
            if self.compose.focus.as_deref() != Some(id.as_str()) {
                self.commands.hint(
                    exo_commands::Hint::new()
                        .text("Next time, hit")
                        .keys(["⌘", "D"])
                        .text("to instantly move to your draft!"),
                );
            }
            self.compose.pressed_inside = true;
            self.compose.focus = Some(id);
        }
        for action in response.actions {
            self.compose_run(action, now);
        }
    }
}

/// Once the pointer has rested 100ms, whether it's over the contact pane (not the card over it:
/// what's under the pointer is the card), or over the card it slid out of that pane's way; a press
/// in the card lets it slide back.
fn still_over_right_pane(ui: &egui::Ui, still: &mut bool, right_pane: Rect, card: Rect) {
    let (at, rested, pressed) = ui.input(|input| {
        (
            input.pointer.hover_pos(),
            input.pointer.time_since_last_movement(),
            input.pointer.any_pressed(),
        )
    });
    if pressed && at.is_some_and(|at| card.contains(at)) {
        *still = false;
        return;
    }
    if rested < STILL {
        ui.ctx().request_repaint_after_secs(STILL - rested);
        return;
    }
    *still = at.is_some_and(|at| {
        if card.contains(at) {
            *still
        } else {
            right_pane.contains(at)
        }
    });
}

/// The card: the elevated background, rounded at the top, with its shadows (clipped at the window's
/// bottom).
fn paint_card(ui: &egui::Ui, card: Rect, rem: f32) {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter();
    let rect = exo_ui::pixel_snap(card);
    let radius = CornerRadius {
        nw: 4,
        ne: 4,
        sw: 0,
        se: 0,
    };
    let shadows: [([f32; 2], f32, Color32); 3] = match exo_ui::theme(ui.ctx()) {
        // `0 6px 15px 0 rgba(0,0,0,.1), 0 16px 2rem rgba(0,0,0,.05), 0 0 .5px rgba(0,0,0,.1)`.
        exo_ui::Theme::Snow => [
            ([0.0, 6.0], 15.0, Color32::from_black_alpha(26)),
            ([0.0, 16.0], 2.0 * rem, Color32::from_black_alpha(13)),
            ([0.0, 0.0], 0.5, Color32::from_black_alpha(26)),
        ],
        // `0 0 12px 0 rgba(0,0,0,.03), 0 12px 2rem 0 rgba(0,0,0,.1)`, and the inset highlight.
        exo_ui::Theme::Carbon => [
            ([0.0, 0.0], 12.0, Color32::from_black_alpha(8)),
            ([0.0, 12.0], 2.0 * rem, Color32::from_black_alpha(26)),
            ([0.0, 0.0], 0.0, Color32::TRANSPARENT),
        ],
    };
    for (offset, blur, color) in shadows {
        if color != Color32::TRANSPARENT {
            exo_ui::BoxShadow::new(offset, blur, 0.0, color).paint(painter, rect, radius);
        }
    }
    painter.rect_filled(rect, radius, palette.background_elevated_default);
    if palette.theme == exo_ui::Theme::Carbon {
        exo_ui::paint_carbon_highlight(painter, rect, radius, palette.background_elevated_default);
    }
}
