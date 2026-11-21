//! Answering an invitation from the palette, and RSVP + Mark Done: `RSVP: Yes`, `RSVP: Maybe` and
//! `RSVP: No` for the open conversation's invitation, which with `RSVP + Mark Done` on also mark
//! the conversation done, as the invite card's buttons then do.
//!
//! Exo reads invites only for the open conversation, so that's where the rows are.

use exo_commands::{CONVERSATION, Command, Commands};
use exo_store::Mutation;

use crate::{Mail, actionable, commands::Action, thread::invite::Response};

/// A row: its id, answer, title, aliases, operative word, icon and default score (`scale: 3`
/// each).
type Row = (
    &'static str,
    Response,
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
    f64,
);

const ROWS: [Row; 3] = [
    (
        "RSVP_ACCEPTED",
        Response::Accepted,
        "RSVP: Yes",
        &["Accept", "Going"],
        "Yes",
        "data/rsvp-accepted",
        20.0,
    ),
    (
        "RSVP_TENTATIVE",
        Response::Tentative,
        "RSVP: Maybe",
        &["Tentative"],
        "Maybe",
        "data/rsvp-tentative",
        19.0,
    ),
    (
        "RSVP_DECLINED",
        Response::Declined,
        "RSVP: No",
        &["Decline", "Not going"],
        "No",
        "data/rsvp-declined",
        18.0,
    ),
];

/// The toasts' word for `response`.
fn word(response: Response) -> &'static str {
    match response {
        Response::Accepted => "Yes",
        Response::Tentative => "Maybe",
        Response::Declined | Response::NeedsAction => "No",
    }
}

/// Registers the RSVP rows for one conversation with an invitation to answer, titled `… and
/// Mark Done` when an answer would mark it done.
pub(crate) fn register(commands: &mut Commands<Action>, marks_done: bool) {
    for (id, response, title, aliases, operative, icon, score) in ROWS {
        let title = if marks_done {
            format!("{title} and Mark Done")
        } else {
            title.to_owned()
        };
        commands.add(
            Command::new(id, title, Action::Rsvp(response))
                .aliases(aliases.iter().copied())
                .operative_word(operative)
                .icon(icon)
                .default_score(score)
                .scale(3.0)
                .regions(CONVERSATION),
        );
    }
}

impl Mail {
    /// Whether the palette's RSVP rows apply, and if so whether an answer marks the conversation
    /// done: the one actionable conversation is the open one, and it has an invitation to
    /// answer.
    pub(crate) fn rsvp_rows(&self) -> Option<bool> {
        let thread = self.thread.as_ref()?;
        thread.conversation_invite()?;
        let [_] = actionable(Some(thread), &self.inbox)[..] else {
            return None;
        };
        Some(self.rsvp_marks_done())
    }

    /// Whether answering marks the actionable conversations done: `RSVP + Mark Done` is on and
    /// they aren't all done already.
    pub(crate) fn rsvp_marks_done(&self) -> bool {
        let threads = actionable(self.thread.as_ref(), &self.inbox);
        self.account.settings().rsvp_and_done
            && !threads.is_empty()
            && !threads.iter().all(|thread| thread.done)
    }

    /// A palette row: answers the open conversation's invitation, even with the answer it has.
    pub(crate) fn rsvp_command(&mut self, response: Response) {
        let Some(invite) = self
            .thread
            .as_ref()
            .and_then(crate::thread::ThreadView::conversation_invite)
        else {
            return;
        };
        let mut answered = invite.clone();
        answered.answer(&self.calendar_invite_account(invite), response);
        self.calendar_respond_to_invite(answered, response, false);
    }

    /// The toast an answer gets, unless it marks done.
    pub(crate) fn rsvp_toast(&self, response: Response) -> String {
        match (self.snapshot.connection.is_online(), response) {
            (false, _) => "RSVP will be sent when online.".to_owned(),
            (true, Response::NeedsAction) => "Event added to calendar.".to_owned(),
            (true, response) => format!("Responded {}.", word(response)),
        }
    }

    /// RSVP + Mark Done's second half: the actionable conversations are marked done, with Undo and
    /// a toast that says the answer, and leave the list the way Auto Advance says.
    pub(crate) fn rsvp_mark_done(&mut self, response: Response) {
        let text = match (self.snapshot.connection.is_online(), response) {
            (false, Response::NeedsAction) => "Event will be added when online.".to_owned(),
            (false, _) => "RSVP will be sent when online.".to_owned(),
            (true, Response::NeedsAction) => "Event added to calendar and Marked Done.".to_owned(),
            (true, response) => format!("Responded {} and Marked Done.", word(response)),
        };
        let ids: Vec<String> = actionable(self.thread.as_ref(), &self.inbox)
            .iter()
            .map(|thread| thread.id.clone())
            .collect();
        let now = self.ctx.input(|input| input.time);
        self.triage_threads(&ids, Mutation::MarkDone, Some(text), 0, now);
    }
}
