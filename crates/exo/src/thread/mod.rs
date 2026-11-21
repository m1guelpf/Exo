//! The open conversation: the back gutter, the header with the subject, labels and actions, and
//! the messages, collapsed or expanded, with their bodies from [`crate::body`].
//!
//! Keys aren't bound here; the operations they trigger are methods ([`ThreadView::next_message`],
//! [`ThreadView::toggle_current`], …) for the app's commands to call.

pub(crate) mod attachments;
pub(crate) mod dates;
mod focus;
mod footer;
mod header;
pub(crate) mod invite;
mod markers;
mod message;
mod new_messages;
mod overflow;
mod print;
mod selection;
mod tooltips;
mod wide;

pub(crate) use focus::{GutterKey, HeaderKey};
pub(crate) use message::{
    HEADER as HEADER_TEXT, HEADER_ROW, PADDING_BOTTOM as CARD_PADDING_BOTTOM,
    PADDING_TOP as CARD_PADDING_TOP, header_job,
};

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use egui::{Pos2, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use exo_commands::Region;
use exo_store::{Attachment, MessageDetail, ThreadDetail};
use exo_ui::motion::CubicBezier;
use jiff::Zoned;

use crate::{
    body::{self, Bodies, Event, Granularity, Hit},
    format::names::Person,
};

/// `SANDBOX_MAX_WIDTH`: the message column's widest content.
const SANDBOX_MAX_WIDTH: f32 = 640.0;
/// The messages' `padding-top: 28px; margin-top: -20px`.
const MESSAGES_PADDING_TOP: f32 = 28.0;
const MESSAGES_MARGIN_TOP: f32 = -20.0;
/// Each card's `margin-top: -12px`, collapsed or expanded.
const CARD_MARGIN_TOP: f32 = -12.0;
/// 44px (11 × 4px): a collapsed message's pitch, and the positioner's step.
const STEP: f32 = 44.0;
/// Four steps: where a message lands below the top when scrolled to, and how far ↑/↓ scroll
/// within one.
const IDEAL_TOP: f32 = 4.0 * STEP;
/// 16px: how far a message may poke out before ↑/↓ or N/P reposition.
const SLACK: f32 = 16.0;
/// The keyboard's scroll animation, an estimate.
const SCROLL_DURATION: f32 = 0.2;

/// What the app should do in response to the user.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    /// The back button, or Escape with nothing to collapse.
    Back,
    /// The header's up/down buttons: the previous (-1) or next (1) conversation.
    Navigate(isize),
    /// The header's Done button.
    ToggleDone,
    /// The header's ⌘K button.
    CommandPalette,
    /// The header's Remind Me button.
    Remind,
    /// A label chip: show that label's list.
    ShowLabel(String),
    /// Open a URL in the system browser (a link in a body, the Unsubscribe link).
    OpenUrl(String),
    /// Open a web link without bringing the browser to the front (a ⌘-click on a link).
    OpenUrlInBackground(String),
    /// The Show New Messages bar.
    ShowNewMessages,
    /// The third click in a row on headers that expanded (or collapsed) a message: suggest ⇧O.
    ExpandAllHint { expand: bool },
    /// The pointer settled on a participant or a `mailto:` link: show them in the right pane.
    ShowContact(Person),
    /// Text was selected in the subject or a full header's addresses: suggest ⌘K's copy row.
    CopyHint { subject: bool },
    /// A participant's address in their tooltip: a new message to them, popped out when ⇧ was held.
    ComposeTo(Person, bool),
    /// Something a copy tooltip put on the clipboard, for its `"…" copied to clipboard.`.
    Copied(String),
    /// The header's Unsubscribe.
    Unsubscribe,
    /// An attachment card: preview it in Quick Look, or only download it when ⌘ or Ctrl was held
    /// or the click was on its download icon.
    Download { attachment: Attachment, open: bool },
    /// An image or message card, or an image in a body: Exo's viewer, over `attachments` from
    /// `start`.
    Preview {
        attachments: Vec<Attachment>,
        start: usize,
    },
    /// `Download all` under message `message`'s cards.
    DownloadAll { message: String },
    /// A card dragged out of the window, from `at`.
    DragFile {
        attachment: Attachment,
        at: egui::Pos2,
    },
    /// Something for compose: a reply button, or a draft's own action.
    Compose(crate::compose::Action),
    /// A header chevron expanded (or folded) a message's header: suggest Shift-H.
    HeaderHint { expanded: bool },
    /// A click on a body's `…` showed (or hid) its quoted text: suggest O.
    QuoteHint { shown: bool },
    /// The draft being typed in scrolled out of view: suggest popping it out.
    PopOutHint,
    /// An invite card's RSVP: the invite, my answer recorded, and the answer.
    RespondToInvite(invite::Invite, invite::Response),
    /// The pointer settled on an invite's When or a detected date: that day in the right pane.
    ShowDay(jiff::civil::Date),
    /// An invite's Where that's a phone number: call it (`tel:` and digits, which the card
    /// builds itself).
    Dial(String),
    /// A click on a detected date: a new event with the conversation's people and subject, the
    /// form's day on that date.
    CreateEvent(jiff::civil::Date),
}

/// One message as the thread shows it.
#[derive(Clone, Debug)]
struct Item {
    id: String,
    expanded: bool,
    header_expanded: bool,
    show_quoted: bool,
    /// In Trash while the view doesn't show Trash: collapsed, it's the
    /// `This message was moved to Trash.` marker, which the cursor skips.
    trashed: bool,
}

/// One thing in the messages column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    /// Message `items[at]`, a card or a collapsed row.
    Message(usize),
    /// Message `items[at]`, in Trash and folded into its marker.
    Trashed(usize),
    /// `Reminder returned`.
    Reminder,
}

/// An entry where the last layout put it: its top in the column's content, its height (a
/// message's with its footer hint), and its card's.
#[derive(Clone, Copy, Debug)]
struct Placed {
    entry: Entry,
    top: f32,
    height: f32,
    card: f32,
}

/// What a selection in the header's text was in: the subject, or a full header's addresses.
const SELECTED_SUBJECT: u32 = 1;
const SELECTED_CONTACTS: u32 = 2;

/// The gap two adjoining vertical margins leave (CSS margin collapsing): the largest positive
/// one less the most negative one.
fn collapse(a: f32, b: f32) -> f32 {
    a.max(b).max(0.0) + a.min(b).min(0.0)
}

/// An animated scroll.
#[derive(Clone, Copy, Debug)]
struct ScrollAnimation {
    from: f32,
    to: f32,
    started: f64,
    duration: f32,
}

/// Opening a draft scrolls to it over 500ms.
const DRAFT_SCROLL_DURATION: f32 = 0.5;
/// While typing, a draft's bottom stays this far above the messages'.
const DRAFT_MARGIN_BOTTOM: f32 = 28.0;
/// Presses this close in time (macOS's default double-click interval) and place make a double
/// or triple click, as in the editor.
pub(crate) const MULTI_CLICK_DELAY: f64 = 0.5;
pub(crate) const MULTI_CLICK_DISTANCE: f32 = 4.0;

/// The open thread's state.
pub struct ThreadView {
    id: String,
    detail: Option<Arc<ThreadDetail>>,
    items: Vec<Item>,
    /// The focused message.
    focus: Option<String>,
    scroll: f32,
    animation: Option<ScrollAnimation>,
    /// The first layout scrolls to a new thread's starting position.
    positioned: bool,
    /// A message to bring into view on the next frame.
    reveal: Option<(String, isize)>,
    /// A message to focus once the thread has it (from a notification).
    wanted: Option<String>,
    /// The message whose draft to bring into view.
    reveal_draft: Option<String>,
    /// The draft being typed in showed when last looked at.
    draft_was_in_view: bool,
    /// Scroll to the end once everything is measured (a reply was just sent).
    reveal_end: bool,
    /// The message column's visible height and the content's, from the last frame.
    viewport: f32,
    content: f32,
    /// Each item's top in the content and its height, from the last frame.
    tops: Vec<(f32, f32)>,
    /// The message a drag selection started in.
    selecting: Option<String>,
    /// The last press in a body: when, where, and how many clicks in a row it made (at most 3).
    clicks: Option<(f64, Pos2, u8)>,
    /// The label chips the header shows.
    chips: Vec<String>,
    /// The body under the pointer this frame.
    hovered_body: Option<String>,
    /// The messages' scrollbar.
    scrollbar: exo_ui::Scrollbar,
    /// Calendar invites read from messages' `.ics` attachments, and the ones asked for.
    invites: HashMap<String, invite::Invite>,
    /// What the attachment cards show besides the messages.
    files: attachments::Files,
    asked_invites: HashSet<String>,
    photos: invite::Photos,
    /// Where the invite photos are (`assets/images`).
    images: PathBuf,
    /// The account's address, to find you among an invite's attendees.
    me: String,
    /// The account's other calendar accounts, who may be the invited one
    /// ([`invite::Invite::calendar_account`]), and those of them Google signed out, which sign
    /// in before they answer.
    calendar_accounts: Vec<String>,
    signed_out: Vec<String>,
    now: Zoned,
    /// The invite card's RSVP button with the keyboard.
    invite_focus: Option<usize>,
    /// The invite cards whose About shows every line (`Show Less`).
    about_open: HashSet<String>,
    /// The invite's When the pointer is settling on, by message.
    when_intent: exo_ui::HoverIntent<String>,
    /// The detected date the pointer is settling on, and the one whose `Create Event` is up.
    date_intent: exo_ui::HoverIntent<(String, String)>,
    date_tip: Option<selection::DateTip>,
    /// Where each expanded body was drawn this frame.
    body_origins: HashMap<String, Pos2>,
    /// Gmail can be reached: offline, a reply being sent says when it'll go.
    online: bool,
    /// Headers name senders in full (`showSenderFullNames`).
    full_names: bool,
    /// The bulk action the conversation waits on, as its header's status says it.
    pending: Option<String>,
    /// Your domains, whose mailing lists aren't unsubscribed from.
    own_domains: Vec<String>,
    /// Messages arrived that the thread doesn't show yet: the Show New
    /// Messages bar is up.
    has_new: bool,
    /// Messages to show as soon as a copy of the thread has them: replies sent from here.
    admitted: HashSet<String>,
    /// The view includes Trash (the Trash list, or a search), so trashed messages show as any
    /// other.
    trash_shown: bool,
    /// Which part of the thread has the keyboard.
    zone: focus::Zone,
    /// The modifiers held as the last press in a body went down, for what it hits.
    click_modifiers: egui::Modifiers,
    /// Clicks in a row on headers that expanded, or collapsed, a message: the third suggests ⇧O.
    clicks_in_a_row: (u8, u8),
    /// The header's status line, as last drawn.
    status: Option<header::Status>,
    /// The tab index the header last focused.
    header_tab: u8,
    /// The column's entries as last laid out, and where its content ends.
    entries: Vec<Placed>,
    end: f32,
    /// The intro hint's first name, under the only message.
    intro: Option<String>,
    /// The `mailto:` link the pointer is settling on, for hover intent.
    mailto_intent: exo_ui::HoverIntent<String>,
    /// A contact to show in the right pane, once this frame's done.
    contact: Option<Person>,
    /// The participants' names the headers drew this frame, the name the pointer is settling
    /// on, and the header's tooltips.
    named: message::Named,
    name_intent: exo_ui::HoverIntent<exo_store::Contact>,
    tooltips: tooltips::Tooltips,
    /// The selection in the header's text (the bodies keep their own).
    selection: exo_ui::TextSelection,
    /// Messages whose bodies are too wide for the column, with the width
    /// each is laid out at once the column widens for it.
    wide: HashMap<String, f32>,
    /// How far the widened column is scrolled sideways, and how far it could be last frame.
    scroll_x: f32,
    max_scroll_x: f32,
    /// The widened column's sideways scrollbar.
    scrollbar_x: exo_ui::Scrollbar,
    /// When the account first signed in to Exo on this Mac, if known.
    joined: Option<jiff::Timestamp>,
    /// Opened unread: it's marked read once its first copy is in, so the messages that were
    /// unread open expanded.
    read_on_open: bool,
}

impl ThreadView {
    /// A view of thread `id` in account `me`, dated relative to `now`, with the invite photos in
    /// `images`; it shows the thread once [`Self::set_detail`] has it.
    pub fn new(id: String, now: Zoned, me: String, images: PathBuf) -> Self {
        Self {
            id,
            detail: None,
            items: Vec::new(),
            focus: None,
            scroll: 0.0,
            animation: None,
            positioned: false,
            reveal: None,
            wanted: None,
            reveal_draft: None,
            draft_was_in_view: false,
            reveal_end: false,
            viewport: 0.0,
            content: 0.0,
            tops: Vec::new(),
            selecting: None,
            clicks: None,
            chips: Vec::new(),
            hovered_body: None,
            scrollbar: exo_ui::Scrollbar::default(),
            invites: HashMap::new(),
            files: attachments::Files::default(),
            asked_invites: HashSet::new(),
            photos: invite::Photos::default(),
            invite_focus: None,
            about_open: HashSet::new(),
            when_intent: exo_ui::HoverIntent::default(),
            date_intent: exo_ui::HoverIntent::default(),
            date_tip: None,
            body_origins: HashMap::new(),
            images,
            me,
            calendar_accounts: Vec::new(),
            signed_out: Vec::new(),
            now,
            online: true,
            full_names: false,
            pending: None,
            own_domains: Vec::new(),
            has_new: false,
            admitted: HashSet::new(),
            trash_shown: false,
            zone: focus::Zone::Messages,
            click_modifiers: egui::Modifiers::NONE,
            clicks_in_a_row: (0, 0),
            status: None,
            header_tab: 1,
            entries: Vec::new(),
            end: 0.0,
            intro: None,
            mailto_intent: exo_ui::HoverIntent::default(),
            contact: None,
            named: Vec::new(),
            name_intent: exo_ui::HoverIntent::default(),
            tooltips: tooltips::Tooltips::default(),
            selection: exo_ui::TextSelection::default(),
            wide: HashMap::new(),
            scroll_x: 0.0,
            max_scroll_x: 0.0,
            scrollbar_x: exo_ui::Scrollbar::default(),
            joined: None,
            read_on_open: false,
        }
    }

    /// Whether the account is new: it joined in the last week, by Exo's first sign-in on this Mac.
    /// Reply buttons have labels then, on the focused message.
    fn new_account(&self) -> bool {
        let week_ago = self.now.checked_sub(jiff::Span::new().days(7));
        self.joined
            .zip(week_ago.ok())
            .is_some_and(|(joined, week_ago)| joined >= week_ago.timestamp())
    }

    /// The header's controls, in tab order.
    fn header_controls(&self) -> Vec<header::Control> {
        header::controls(self.status.as_ref())
    }

    /// Whether the view includes Trash (the Trash list, a search): its trashed messages show as
    /// any other. Set before the thread's first copy.
    pub fn set_trash_shown(&mut self, shown: bool) {
        self.trash_shown = shown;
    }

    /// What the header's status line needs from the rest of the app: the bulk action the
    /// conversation waits on (`Pending: Mark Done`), and your domains, whose lists it doesn't
    /// offer to unsubscribe from.
    pub fn set_status(&mut self, pending: Option<String>, own_domains: Vec<String>) {
        self.pending = pending;
        self.own_domains = own_domains;
    }

    /// Whether Gmail can be reached, as the offline bar says.
    pub fn set_online(&mut self, online: bool) {
        self.online = online;
    }

    /// Names senders in full in the message headers, or by first name (`showSenderFullNames`).
    pub fn set_full_names(&mut self, full_names: bool) {
        self.full_names = full_names;
    }

    /// Says the conversation was unread when it opened: [`Self::take_read_on_open`] says when to
    /// mark it read.
    pub fn set_read_on_open(&mut self) {
        self.read_on_open = true;
    }

    /// Whether to mark the conversation read now: once, after its first copy has laid out its
    /// messages.
    pub fn take_read_on_open(&mut self) -> bool {
        self.detail.is_some() && std::mem::take(&mut self.read_on_open)
    }

    /// Records when the account first signed in to Exo on this Mac, which decides whether reply
    /// buttons have labels.
    pub fn set_joined(&mut self, joined: Option<jiff::Timestamp>) {
        self.joined = joined;
    }

    /// Dates the messages relative to `now`.
    pub fn set_now(&mut self, now: Zoned) {
        self.now = now;
    }

    /// The attachment cards' state: downloads' progress, thumbnails, zips.
    pub(crate) fn files_mut(&mut self) -> &mut attachments::Files {
        &mut self.files
    }

    /// The image attachments whose card thumbnails to fetch: those of the expanded messages,
    /// whose cards show; each is asked for once.
    pub(crate) fn thumbnail_requests(&mut self) -> Vec<Attachment> {
        let Some(detail) = self.detail.clone() else {
            return Vec::new();
        };
        let expanded: HashSet<&str> = self
            .items
            .iter()
            .filter(|item| item.expanded)
            .map(|item| item.id.as_str())
            .collect();
        self.files.thumbnail_requests(
            Self::shown(&detail).filter(|message| expanded.contains(message.id.as_str())),
        )
    }

    /// The `.ics` attachments to fetch for invite cards, by message; each message is looked at
    /// once.
    pub fn invite_requests(&mut self) -> Vec<(String, Attachment)> {
        let Some(detail) = self.detail.clone() else {
            return Vec::new();
        };
        let mut requests = Vec::new();
        for message in Self::shown(&detail) {
            if self.asked_invites.contains(&message.id) {
                continue;
            }
            self.asked_invites.insert(message.id.clone());
            requests
                .extend(invite::attachment(message).map(|ics| (message.id.clone(), ics.clone())));
        }
        requests
    }

    /// Takes the `.ics` of message `id`.
    pub fn set_invite(&mut self, id: &str, ics: &[u8]) {
        if let Some(parsed) = invite::parse(&String::from_utf8_lossy(ics)) {
            self.invites.insert(id.to_owned(), parsed);
        }
    }

    /// What message `id`'s invite card is drawn with, `calendar` the notification's facts.
    fn invite_context<'a>(
        &'a self,
        id: &str,
        calendar: Option<&'a exo_mail::CalendarData>,
    ) -> Option<invite::Context<'a>> {
        let me = self.invites.get(id).map_or(self.me.as_str(), |parsed| {
            parsed.calendar_account(&self.me, &self.calendar_accounts)
        });
        Some(invite::Context {
            me,
            user: &self.me,
            now: &self.now,
            message: self.message(id)?,
            calendar,
            about_open: self.about_open.contains(id),
        })
    }

    /// The height an invite card, and the guest's note above it, add above message `id`'s body.
    fn invite_block(&self, ui: &Ui, id: &str, width: f32, bodies: &Bodies) -> f32 {
        let calendar = bodies.facts(id).map(|facts| &facts.calendar);
        let (Some(parsed), Some(context)) =
            (self.invites.get(id), self.invite_context(id, calendar))
        else {
            return 0.0;
        };
        let note = invite::note(parsed, calendar)
            .map_or(0.0, |note| invite::note_height(ui.ctx(), &note, width));
        note + invite::height(ui, parsed, &context, width) + invite::MARGIN_BOTTOM
    }

    /// Whether message `id`'s body is the notification its invite card stands in for.
    fn hides_body(&self, id: &str, bodies: &Bodies) -> bool {
        let (Some(parsed), Some(message)) = (self.invites.get(id), self.message(id)) else {
            return false;
        };
        invite::hides_body(
            parsed,
            message,
            bodies.facts(id).is_some_and(|facts| facts.calendar.gcal),
        )
    }

    /// The focused message's invite, if it's expanded and I can answer it or add it to my
    /// calendar.
    fn focusable_invite(&self) -> Option<&invite::Invite> {
        let focused = self.focus.as_ref()?;
        let item = self
            .items
            .iter()
            .find(|item| &item.id == focused)
            .filter(|item| item.expanded)?;
        self.invites.get(&item.id).filter(|parsed| {
            parsed.can_respond(self.invite_me(parsed))
                || parsed.can_add_to_calendar(self.invite_me(parsed))
        })
    }

    /// Who I am on `parsed` ([`invite::Invite::calendar_account`]).
    fn invite_me<'a>(&'a self, parsed: &'a invite::Invite) -> &'a str {
        parsed.calendar_account(&self.me, &self.calendar_accounts)
    }

    /// The account's other calendar accounts, any of whom an invite may be to, and those of
    /// them signed out.
    pub fn set_calendar_accounts(&mut self, accounts: Vec<String>, signed_out: Vec<String>) {
        self.calendar_accounts = accounts;
        self.signed_out = signed_out;
    }

    /// Whether `parsed` is to a signed-out calendar account, whose answer waits for it to sign
    /// in: the card doesn't show it as given meanwhile.
    fn needs_sign_in(&self, parsed: &invite::Invite) -> bool {
        self.signed_out
            .iter()
            .any(|email| email == self.invite_me(parsed))
    }

    /// `tab` in the messages: the keyboard to the focused message's invite card,
    /// on its first button. Returns whether there was one.
    pub fn focus_invite(&mut self) -> bool {
        let found = self.focusable_invite().is_some();
        if found {
            self.invite_focus = Some(0);
        }
        found
    }

    /// Whether `tab` would take the keyboard to an invite card.
    pub fn can_focus_invite(&self) -> bool {
        self.invite_focus.is_none() && self.focusable_invite().is_some()
    }

    /// Whether the invite card has the keyboard.
    pub fn invite_focused(&self) -> bool {
        self.invite_focus.is_some() && self.focusable_invite().is_some()
    }

    /// → / ↓ / tab and ← / ↑ in the card: the next or previous button, round.
    /// `Add To Calendar` is alone, and stays.
    pub fn move_invite_focus(&mut self, delta: isize) {
        let answerable = self
            .focusable_invite()
            .is_some_and(|parsed| parsed.can_respond(self.invite_me(parsed)));
        if let Some(focus) = self.invite_focus.as_mut().filter(|_| answerable) {
            *focus = (*focus as isize + delta).rem_euclid(invite::ANSWERS.len() as isize) as usize;
        }
    }

    /// Escape in the card: back to the messages.
    pub fn blur_invite(&mut self) {
        self.invite_focus = None;
    }

    /// Enter or space in the card: the focused button's answer, unless it's the
    /// current one. Returns the invite with it recorded, and the answer.
    pub fn submit_invite(&mut self) -> Option<(invite::Invite, invite::Response)> {
        let focus = self.invite_focus?;
        let id = self.focus.clone()?;
        let parsed = self.focusable_invite()?;
        let me = self.invite_me(parsed).to_owned();
        let response = if parsed.can_respond(&me) {
            invite::ANSWERS[focus]
        } else {
            invite::Response::NeedsAction
        };
        if parsed.answer_of(&me) == Some(response) {
            return None;
        }
        if self.needs_sign_in(parsed) {
            return Some((parsed.clone(), response));
        }
        let mut answered = parsed.clone();
        answered.answer(&me, response);
        self.invites.insert(id, answered.clone());
        Some((answered, response))
    }

    /// The calendar's answer to the invite with UID `uid`, shown on its cards: what was answered
    /// before, from here or elsewhere, rather than the `.ics`'s.
    pub fn show_calendar_answer(&mut self, uid: &str, response: invite::Response) {
        for parsed in self.invites.values_mut().filter(|parsed| parsed.uid == uid) {
            let me = parsed
                .calendar_account(&self.me, &self.calendar_accounts)
                .to_owned();
            if parsed.answer_of(&me) != Some(response) {
                parsed.answer(&me, response);
            }
        }
    }

    /// The conversation's invitation you can answer: the
    /// invite of the message whose id is the thread's, else of its first message.
    pub fn conversation_invite(&self) -> Option<&invite::Invite> {
        let first = self.detail.as_ref()?.messages.first()?;
        self.invites
            .get(&self.id)
            .or_else(|| self.invites.get(&first.id))
            .filter(|parsed| parsed.can_respond(self.invite_me(parsed)))
    }

    /// A new event for this conversation: its subject, and everyone on its messages but me, once
    /// each.
    pub fn event_prefill(&self) -> (String, Vec<(Option<String>, String)>) {
        let Some(detail) = &self.detail else {
            return Default::default();
        };
        let mut guests: Vec<(Option<String>, String)> = Vec::new();
        for contact in detail
            .messages
            .iter()
            .flat_map(|message| message.from.iter().chain(&message.to).chain(&message.cc))
        {
            if !contact.is_me
                && !contact.email.eq_ignore_ascii_case(&self.me)
                && !guests
                    .iter()
                    .any(|(_, email)| email.eq_ignore_ascii_case(&contact.email))
            {
                guests.push((contact.name.clone(), contact.email.clone()));
            }
        }
        (detail.subject.clone(), guests)
    }

    /// The one-click actions of messages `ids`.
    fn actions_of(&self, ids: &[&str], bodies: &Bodies) -> Vec<exo_mail::Action> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        crate::email_actions::of(detail, ids, &self.own_domains, |id| {
            bodies.facts(id).map(|facts| facts.actions.clone())
        })
    }

    /// The conversation's last one-click action, which the header's status line and the
    /// palette offer.
    pub fn conversation_action(&self, bodies: &Bodies) -> Option<exo_mail::Action> {
        let ids: Vec<&str> = self
            .detail
            .as_ref()?
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect();
        self.actions_of(&ids, bodies).pop()
    }

    /// The focused message's one-click actions, which ⌘O lists first.
    pub fn focused_actions(&self, bodies: &Bodies) -> Vec<exo_mail::Action> {
        self.focus
            .as_deref()
            .map(|id| self.actions_of(&[id], bodies))
            .unwrap_or_default()
    }

    /// The UIDs of the thread's invites.
    pub fn invite_uids(&self) -> Vec<String> {
        self.invites
            .values()
            .map(|parsed| parsed.uid.clone())
            .collect()
    }

    /// The invite the right pane's calendar shows: the focused message's,
    /// or with none focused, the first message's that has one.
    pub fn invite(&self) -> Option<&invite::Invite> {
        match &self.focus {
            Some(focused) => self.invites.get(focused),
            None => self
                .detail
                .as_ref()?
                .messages
                .iter()
                .find_map(|message| self.invites.get(&message.id)),
        }
    }

    /// The message whose body is under the pointer.
    pub fn hovered_body(&self) -> Option<&str> {
        self.hovered_body.as_deref()
    }

    /// The link under the pointer, for the link preview.
    pub fn hovered_link(&self, bodies: &Bodies) -> Option<String> {
        match bodies.hovered(self.hovered_body.as_deref()?)? {
            Hit::Link { href } => Some(href.clone()),
            _ => None,
        }
    }

    /// Sets the header's label chips.
    pub fn set_chips(&mut self, chips: Vec<String>) {
        self.chips = chips;
    }

    /// The thread's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The focused message's id.
    pub fn focused(&self) -> Option<&str> {
        self.focus.as_deref()
    }

    /// Whether each message is expanded, oldest first.
    pub fn expanded(&self) -> Vec<bool> {
        self.items.iter().map(|item| item.expanded).collect()
    }

    /// How far the messages are scrolled, in px.
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    /// Whether the focused message shows its full header.
    pub fn header_expanded(&self) -> bool {
        self.focus_index()
            .is_some_and(|at| self.items[at].header_expanded)
    }

    /// The thread, once loaded.
    pub fn detail(&self) -> Option<&ThreadDetail> {
        self.detail.as_deref()
    }

    /// The thread, once loaded, shared.
    pub fn shared_detail(&self) -> Option<Arc<ThreadDetail>> {
        self.detail.clone()
    }

    /// The messages the thread shows, oldest first: everything but drafts being written, which
    /// belong to compose. A failed or scheduled draft is a message.
    fn shown(detail: &ThreadDetail) -> impl Iterator<Item = &MessageDetail> {
        detail.messages.iter().filter(|message| {
            !message.is_draft() || message.failure.is_some() || message.scheduled.is_some()
        })
    }

    /// Takes the latest copy of the thread. The first one
    /// shows every message: the unread ones expanded, and always the last that isn't in Trash.
    /// Later copies keep what the user expanded, and hold messages that arrived back behind the
    /// Show New Messages bar, but for failed or scheduled sends and replies sent from here.
    pub fn set_detail(&mut self, detail: Arc<ThreadDetail>) {
        if self
            .detail
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &detail))
        {
            return;
        }
        let first = self.detail.is_none();
        self.detail = Some(detail);
        self.update_items(first);
        if let Some(wanted) = self.wanted.take() {
            self.focus_message(&wanted);
        }
    }

    /// Lists the thread's messages, adding the ones it doesn't show yet when `add_new`.
    fn update_items(&mut self, add_new: bool) {
        let Some(detail) = self.detail.clone() else {
            return;
        };
        let mut items = Vec::new();
        self.has_new = false;
        for message in Self::shown(&detail) {
            let trashed = message.has_label("TRASH") && !self.trash_shown;
            let item = match self.items.iter().find(|item| item.id == message.id) {
                Some(item) => Item {
                    trashed,
                    ..item.clone()
                },
                None if add_new || message.is_draft() || self.admitted.remove(&message.id) => {
                    Item {
                        id: message.id.clone(),
                        expanded: !trashed && message.is_unread(),
                        header_expanded: false,
                        show_quoted: false,
                        trashed,
                    }
                }
                None => {
                    self.has_new = true;
                    continue;
                }
            };
            items.push(item);
        }
        if let Some(last) = items.iter_mut().rev().find(|item| !item.trashed) {
            last.expanded = true;
        }
        if self
            .focus
            .as_ref()
            .is_none_or(|focus| !items.iter().any(|item| &item.id == focus))
        {
            self.focus = items
                .iter()
                .find(|item| item.expanded)
                .map(|item| item.id.clone());
        }
        self.items = items;
        if !self.has_new && self.zone == focus::Zone::NewMessages {
            self.zone = focus::Zone::Messages;
        }
    }

    /// Whether messages arrived that the Show New Messages bar holds back.
    pub fn has_new_messages(&self) -> bool {
        self.has_new
    }

    /// Shows the messages the bar held back and focuses the first of
    /// them. Returns whether there were any.
    pub fn show_new_messages(&mut self) -> bool {
        if !self.has_new {
            return false;
        }
        let known: HashSet<String> = self.items.iter().map(|item| item.id.clone()).collect();
        self.update_items(true);
        self.zone = focus::Zone::Messages;
        if let Some(at) = self.items.iter().position(|item| !known.contains(&item.id)) {
            self.items[at].expanded = true;
            self.focus_at(at, 1);
        }
        true
    }

    /// ⇧N: shows the new messages, or else focuses the last message.
    pub fn show_new_or_last(&mut self) -> bool {
        if self.show_new_messages() {
            return true;
        }
        let last = self.items.len().saturating_sub(1);
        self.focus_at(last, 1);
        false
    }

    /// Focuses message `id`, expanded and in view, now or once a copy of the thread has it.
    pub fn focus_message(&mut self, id: &str) {
        // One the Show New Messages bar holds back shows with the rest.
        if self.has_new
            && !self.items.iter().any(|item| item.id == id)
            && self.message(id).is_some()
        {
            self.update_items(true);
        }
        match self.items.iter().position(|item| item.id == id) {
            Some(at) => {
                self.items[at].expanded = true;
                self.focus_at(at, 1);
            }
            None => self.wanted = Some(id.to_owned()),
        }
    }

    fn message(&self, id: &str) -> Option<&MessageDetail> {
        self.detail
            .as_deref()?
            .messages
            .iter()
            .find(|message| message.id == id)
    }

    fn focus_index(&self) -> Option<usize> {
        let focus = self.focus.as_deref()?;
        self.items.iter().position(|item| item.id == focus)
    }

    /// The focused message's sender, for the contact pane: none when it's you.
    pub fn focused_sender(&self) -> Option<Person> {
        let message = self.message(self.focus.as_deref()?)?;
        let from = message.from.as_ref().filter(|from| !from.is_me)?;
        Some(Person::new(
            from.name.clone().unwrap_or_default(),
            from.email.clone(),
        ))
    }

    /// The messages the thread shows, oldest first, once it's loaded.
    pub fn messages(&self) -> Vec<&MessageDetail> {
        self.detail
            .as_deref()
            .map(|detail| Self::shown(detail).collect())
            .unwrap_or_default()
    }

    /// Whether message `index` can collapse: it isn't the last message that
    /// isn't in Trash. A trashed one always can, back into its marker.
    fn collapsible(&self, index: usize) -> bool {
        self.items
            .iter()
            .rposition(|item| !item.trashed)
            .is_none_or(|last| index != last)
    }

    /// Escape: collapses the focused message if it can, and says whether it did.
    pub fn collapse_current(&mut self) -> bool {
        match self.focus_index() {
            Some(at) if self.items[at].expanded && self.collapsible(at) => {
                self.items[at].expanded = false;
                self.items[at].show_quoted = false;
                true
            }
            _ => false,
        }
    }

    /// Whether the focused message is collapsed (Enter expands it).
    pub fn focus_is_collapsed(&self) -> bool {
        self.focus_index()
            .is_some_and(|at| !self.items[at].expanded)
    }

    /// Enter on a collapsed message: expands it.
    pub fn expand_current(&mut self) {
        if let Some(at) = self.focus_index() {
            self.items[at].expanded = true;
        }
    }

    /// O: cycles the focused message: collapsed → expanded → quoted text shown → collapsed. A
    /// message that can't collapse only toggles its quoted text.
    pub fn toggle_current(&mut self, bodies: &Bodies) {
        let Some(at) = self.focus_index() else { return };
        let has_quoted = bodies
            .facts(&self.items[at].id)
            .is_some_and(|facts| facts.has_quoted);
        let collapsible = self.collapsible(at);
        let item = &mut self.items[at];
        if !item.expanded {
            item.expanded = true;
        } else if has_quoted && !item.show_quoted {
            item.show_quoted = true;
        } else if collapsible {
            item.expanded = false;
            item.show_quoted = false;
        } else {
            item.show_quoted = false;
        }
    }

    /// ⇧O: expands every message, or collapses every one that can when all are expanded.
    pub fn toggle_all(&mut self) {
        let expand = self.items.iter().any(|item| !item.expanded);
        for at in 0..self.items.len() {
            let expanded = expand || !self.collapsible(at);
            let item = &mut self.items[at];
            item.expanded = expanded;
            if !item.expanded {
                item.show_quoted = false;
            }
        }
    }

    /// ⇧H: shows or hides the focused message's full header.
    pub fn toggle_header(&mut self) {
        if let Some(at) = self.focus_index() {
            self.items[at].header_expanded = !self.items[at].header_expanded;
        }
    }

    /// N / P: focuses the next (1) or previous (-1) message and scrolls to it.
    pub fn next_message(&mut self, delta: isize) {
        let Some(at) = self.focus_index() else { return };
        let to = self.step(at, delta);
        self.focus_at(to, delta);
    }

    /// The message `delta` (±1) from `at` that the cursor can land on: a
    /// trashed one only if it's expanded. Past the end it's the last; past the start, `at`.
    fn step(&self, at: usize, delta: isize) -> usize {
        let lands = |item: &Item| !(item.trashed && !item.expanded);
        let mut to = at;
        loop {
            match to
                .checked_add_signed(delta)
                .filter(|to| *to < self.items.len())
            {
                Some(next) if lands(&self.items[next]) => return next,
                Some(next) => to = next,
                None if delta > 0 => {
                    return self
                        .items
                        .len()
                        .checked_sub(1)
                        .filter(|last| lands(&self.items[*last]))
                        .unwrap_or(at);
                }
                None => return at,
            }
        }
    }

    fn focus_at(&mut self, at: usize, direction: isize) {
        if let Some(item) = self.items.get(at) {
            self.focus = Some(item.id.clone());
            self.reveal = Some((item.id.clone(), direction));
        }
    }

    /// ↓ / ↑: scrolls within the focused message, and past its edge moves
    /// to the next or previous message; pressed, not held (`repeat`), past the first message to
    /// the header and past the last to the Show New Messages bar.
    pub fn arrow(&mut self, delta: isize, repeat: bool) {
        let Some(at) = self.focus_index() else { return };
        let Some(&(top, height)) = self.tops.get(at) else {
            return;
        };
        let (view_top, view_bottom) = (self.scroll, self.scroll + self.viewport);
        let target = if delta > 0 && top + height > view_bottom {
            Some(self.scroll + IDEAL_TOP)
        } else if delta < 0 && top < view_top {
            Some(self.scroll - IDEAL_TOP)
        } else {
            None
        };
        match target {
            Some(to) if self.clamp_scroll(to) != self.scroll => self.animate_to(to),
            _ if !repeat && self.arrow_out(delta, at) => {}
            _ => self.next_message(delta),
        }
    }

    /// Space / ⇧Space: a page down or up, then focus the message nearest the middle.
    pub fn page(&mut self, direction: isize) {
        let page = self.viewport - MESSAGES_PADDING_TOP - SLACK;
        let to = self.clamp_scroll(self.scroll + direction as f32 * page);
        self.animate_to(to);
        let middle = to + self.viewport / 2.0;
        let central = self
            .tops
            .iter()
            .position(|&(top, height)| top + height + 16.0 > middle);
        let at = match (direction, central) {
            _ if to <= 0.0 => 0,
            _ if to >= self.max_scroll() => self.items.len().saturating_sub(1),
            (_, Some(at)) => at,
            (_, None) => self.items.len().saturating_sub(1),
        };
        if let Some(item) = self.items.get(at) {
            self.focus = Some(item.id.clone());
        }
    }

    /// `g g` / ⌘↑ and ⇧G / ⌘↓: the first or last message, scrolled to the top or bottom.
    pub fn jump(&mut self, to_end: bool) {
        let at = if to_end {
            self.items.len().saturating_sub(1)
        } else {
            0
        };
        if let Some(item) = self.items.get(at) {
            self.focus = Some(item.id.clone());
        }
        let to = if to_end { self.max_scroll() } else { 0.0 };
        self.animate_to(to);
    }

    fn max_scroll(&self) -> f32 {
        (self.content - self.viewport).max(0.0)
    }

    fn clamp_scroll(&self, to: f32) -> f32 {
        to.clamp(0.0, self.max_scroll())
    }

    fn animate_to(&mut self, to: f32) {
        self.animate_over(to, SCROLL_DURATION);
    }

    fn animate_over(&mut self, to: f32, duration: f32) {
        let to = self.clamp_scroll(to);
        self.animation = Some(ScrollAnimation {
            from: self.scroll,
            to,
            started: f64::NAN,
            duration,
        });
    }

    /// Focuses message `id` and scrolls its new draft into view on the next frame.
    pub fn reveal_draft(&mut self, id: &str) {
        self.focus = Some(id.to_owned());
        self.reveal_draft = Some(id.to_owned());
    }

    /// Focuses message `id`, a reply just sent that the next copy of the thread has, and scrolls
    /// to the end once it's measured.
    pub fn reveal_sent(&mut self, id: &str) {
        self.focus = Some(id.to_owned());
        self.reveal_end = true;
        // Shown at once, not behind the Show New Messages bar.
        self.admitted.insert(id.to_owned());
    }

    /// Message `from` is now `to` (Gmail's copy of a sent reply took its place): it keeps its
    /// expansion and the focus.
    pub fn rename_message(&mut self, from: &str, to: &str) {
        for item in self.items.iter_mut().filter(|item| item.id == from) {
            to.clone_into(&mut item.id);
        }
        if self.focus.as_deref() == Some(from) {
            self.focus = Some(to.to_owned());
        }
    }

    /// Paints the thread view in `rect` (the content pane) and handles the mouse. `region` is
    /// where the keyboard is: the messages' focus bar shows while it's theirs.
    /// `drafts` are the thread's drafts, drawn under the messages they answer; `typing` says
    /// one of them has the keyboard, so its bottom is kept in view.
    #[expect(
        clippy::too_many_arguments,
        reason = "one screen, drawn from the app's state; a struct would only rename them"
    )]
    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        rem: f32,
        window_width: f32,
        bodies: &mut Bodies,
        region: Region,
        events: &[Event],
        mut drafts: Option<&mut crate::compose::Inline<'_>>,
        typing: bool,
    ) -> Vec<Action> {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        let palette = exo_ui::palette(&ctx);
        ui.painter()
            .rect_filled(rect, 0.0, palette.background_base_default);
        let mut actions = Vec::new();
        // A press anywhere in the pane gives the messages the keyboard.
        if ui.input(|input| input.pointer.primary_pressed()) && ui.rect_contains_pointer(rect) {
            self.clicked_in_pane();
        }
        self.selection.begin();
        let mut anchored = None;
        for event in events {
            match event {
                Event::Clicked { id, hit } => self.clicked(id, hit, bodies, &mut actions),
                Event::Anchored { id, top } => anchored = Some((id.clone(), *top)),
                _ => {}
            }
        }
        let Some(detail) = self.detail.clone() else {
            return actions;
        };
        let popped_out = drafts.as_ref().is_some_and(|drafts| drafts.popped_out());
        let mut layout = Layout::new(rect, rem, window_width, popped_out);

        // The header's height depends on how the subject wraps, and the messages start under it.
        self.status = header::status(
            &detail,
            bodies,
            self.pending.as_deref(),
            self.conversation_action(bodies),
            &self.own_domains,
        );
        let header_height =
            header::height(ui, &layout, &detail, &self.chips, self.status.is_some());
        let messages_top = layout.header.top() + header_height + MESSAGES_MARGIN_TOP;

        // Find scrolls to a match out of view, at once (Chromium's `ScrollRectToVisible`): down
        // the messages, and across a widened column.
        let find = ui.id().with("thread-find");
        let reveal = exo_ui::find::reveal(&ctx, find);

        // A wide body widens the column, which the wheel scrolls sideways as one (a body that
        // still scrolls on its own keeps the gesture). Not beside a popped-out draft, which
        // a widened column would run under.
        self.find_wide(bodies);
        if let Some(widest) = self.widest().filter(|_| !popped_out) {
            let thread_right = container(rect, rem, window_width, popped_out).right();
            if let Some(reveal) = reveal {
                self.scroll_x = (self.scroll_x + reveal.x).round();
            }
            layout.widen(rect, thread_right, widest, self.scroll_x);
            let body_scrolls = self
                .hovered_body
                .as_deref()
                .is_some_and(|id| bodies.overflow(id) > 0.0);
            let delta = ui.input(|input| input.smooth_scroll_delta.x);
            if delta != 0.0
                && !body_scrolls
                && ui.rect_contains_pointer(layout.viewport(messages_top, rect.bottom()))
            {
                // Whole CSS px, so the bodies' subpixel phase (and their layout) stays put.
                self.scroll_x = (self.scroll_x - delta).round();
                layout.widen(rect, thread_right, widest, self.scroll_x);
            }
        }
        (self.scroll_x, self.max_scroll_x) = layout.scroll_x();
        let messages = layout.viewport(messages_top, rect.bottom());
        self.viewport = messages.height();

        // The back gutter: all of its clickable goes back, and hovering it lights the back
        // button, with a pointer cursor.
        let gutter_rect = layout.gutter_target();
        let gutter = ui
            .interact(
                gutter_rect,
                ui.id().with("thread-back-gutter"),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let compact_clicked =
            header::back_button(ui, &layout, gutter.hovered(), region == Region::ActionPane);
        if gutter.clicked() || compact_clicked {
            actions.push(Action::Back);
        }

        let scale = ctx.pixels_per_point();
        let theme = exo_ui::theme(&ctx);
        // A message with a draft under it is expanded.
        let with_drafts = drafts
            .as_ref()
            .map(|drafts| drafts.messages())
            .unwrap_or_default();
        for item in &mut self.items {
            if with_drafts.contains(&item.id) {
                item.expanded = true;
            }
        }
        // The intro hint goes under the only message, without a draft.
        self.intro = match self.items.as_slice() {
            [only] if !with_drafts.contains(&only.id) => {
                self.message(&only.id).and_then(footer::intro)
            }
            _ => None,
        };
        let order = self.order(&detail);
        let mut draft_heights = HashMap::new();
        let mut entries = Vec::with_capacity(order.len());
        let mut tops = vec![(0.0, 0.0); self.items.len()];
        let mut y = MESSAGES_PADDING_TOP;
        let mut previous_margin = None::<f32>;
        let mut collapsed_before = 0;
        let mut seen_expanded = false;
        for (k, entry) in order.iter().enumerate() {
            let next = order.get(k + 1);
            let after_marker = k
                .checked_sub(1)
                .is_some_and(|previous| !matches!(order[previous], Entry::Message(_)));
            let (height, card, margin_top, margin_bottom) = match *entry {
                Entry::Message(at) => {
                    let item = &self.items[at];
                    let card = if item.expanded {
                        let layout = self.item_layout(&layout, at);
                        let marketing = bodies
                            .facts(&item.id)
                            .is_some_and(|facts| message::marketing(facts, item.show_quoted));
                        let body = bodies.height(
                            &item.id,
                            layout.look(
                                scale,
                                theme,
                                rem,
                                item.show_quoted,
                                marketing,
                                self.hides_body(&item.id, bodies),
                            ),
                        );
                        let invite = self.invite_block(ui, &item.id, layout.inner.width(), bodies);
                        let draft = match drafts.as_mut().filter(|_| with_drafts.contains(&item.id))
                        {
                            Some(drafts) => drafts.height(ui, &item.id, layout.inner.width()),
                            None => 0.0,
                        };
                        draft_heights.insert(item.id.clone(), draft);
                        message::expanded_height(
                            item.header_expanded,
                            self.message(&item.id),
                            invite,
                            body,
                            layout.inner.width(),
                        ) + draft
                    } else {
                        message::COLLAPSED_HEIGHT
                    };
                    let margin = if item.expanded {
                        message::EXPANDED_MARGIN_BOTTOM
                    } else {
                        message::COLLAPSED_MARGIN_BOTTOM
                    };
                    if !item.expanded && !seen_expanded {
                        collapsed_before += 1;
                    }
                    seen_expanded |= item.expanded;
                    if self.intro.is_some() {
                        // The hint's -16px margin under the card's 28px, then the hint.
                        (
                            card + collapse(margin, footer::MARGIN_TOP) + footer::HEIGHT,
                            card,
                            CARD_MARGIN_TOP,
                            0.0,
                        )
                    } else if matches!(next, Some(Entry::Trashed(_))) {
                        (
                            card,
                            card,
                            CARD_MARGIN_TOP,
                            margin.max(markers::MESSAGE_MARGIN_BEFORE),
                        )
                    } else {
                        (card, card, CARD_MARGIN_TOP, margin)
                    }
                }
                Entry::Trashed(_) | Entry::Reminder => {
                    if !seen_expanded {
                        collapsed_before += usize::from(matches!(entry, Entry::Trashed(_)));
                    }
                    let margin_top = if after_marker {
                        markers::MARGIN_TOP_AFTER_MARKER
                    } else {
                        markers::MARGIN_TOP
                    };
                    // The reminder's marker is wrapped in its hover intent's `div`, which no
                    // sibling rule sees through.
                    let before_message = matches!(entry, Entry::Trashed(_))
                        && matches!(next, Some(Entry::Message(_)));
                    let margin_bottom = if before_message {
                        markers::MARGIN_BOTTOM_BEFORE_MESSAGE
                    } else {
                        markers::MARGIN_BOTTOM
                    };
                    (markers::HEIGHT, markers::HEIGHT, margin_top, margin_bottom)
                }
            };
            y += previous_margin.map_or(margin_top, |previous| collapse(previous, margin_top));
            if let Entry::Message(at) | Entry::Trashed(at) = *entry {
                tops[at] = (y, height);
            }
            entries.push(Placed {
                entry: *entry,
                top: y,
                height,
                card,
            });
            y += height;
            previous_margin = Some(margin_bottom);
        }
        self.tops = tops;
        self.entries = entries;
        // Where what's added after the messages goes: past the last
        // one's margin.
        self.end = y + previous_margin.unwrap_or(0.0);
        // The table grows so the collapsed messages
        // above the first expanded one can scroll away, and the end can scroll a little past.
        let table_top = MESSAGES_PADDING_TOP + CARD_MARGIN_TOP;
        let natural = y - table_top;
        let collapsed_scroll = if collapsed_before > 4 {
            (collapsed_before - 4) as f32 * STEP
        } else {
            0.0
        };
        let bottom_scroll = (natural - self.viewport + 2.0 * MESSAGES_PADDING_TOP).max(0.0);
        let max_top = collapsed_scroll.max(bottom_scroll);
        self.content = if max_top > 0.0 {
            table_top + natural.max(self.viewport + max_top - 4.0)
        } else {
            self.end
        };

        // Scrolling: the first layout with every expanded body measured lands where a newly
        // opened thread starts; later, keys animate and the wheel scrolls.
        let measured = self
            .items
            .iter()
            .all(|item| !item.expanded || bodies.height_known(&item.id));
        if !self.positioned && measured {
            self.positioned = true;
            self.scroll = self.initial_scroll(collapsed_scroll);
        }
        if let Some((id, direction)) = self.reveal.take() {
            self.scroll_to_message(&id, direction);
        }
        if let Some(id) = self.reveal_draft.take().filter(|_| measured) {
            self.scroll_to_draft(&id);
        }
        if self.reveal_end && measured {
            self.reveal_end = false;
            self.animate_to(self.max_scroll());
        }
        if let Some((id, top)) = anchored {
            self.scroll_to_anchor(ui, &layout, &id, top, bodies);
        }
        if typing && self.animation.is_none() {
            self.keep_draft_in_view(&draft_heights);
        }
        // The draft being typed in scrolled away: suggest popping it out.
        if typing {
            let in_view = self.draft_in_view(&draft_heights);
            if self.draft_was_in_view && !in_view {
                actions.push(Action::PopOutHint);
            }
            self.draft_was_in_view = in_view;
        }
        if let Some(animation) = &mut self.animation {
            if animation.started.is_nan() {
                animation.started = now;
            }
            let progress = ((now - animation.started) as f32 / animation.duration).min(1.0);
            self.scroll =
                animation.from + (animation.to - animation.from) * CubicBezier::EASE.at(progress);
            if progress >= 1.0 {
                self.animation = None;
            } else {
                ctx.request_repaint();
            }
        }
        if ui.rect_contains_pointer(messages) {
            let delta = ui.input(|input| input.smooth_scroll_delta.y);
            let pointer = ui.input(|input| input.pointer.hover_pos());
            // A reply at its max height scrolls its own body first.
            let form = pointer
                .zip(drafts.as_deref())
                .is_some_and(|(at, drafts)| drafts.takes_wheel(at, delta));
            if delta != 0.0 && !form {
                self.animation = None;
                self.scroll -= delta;
            }
        }
        if let Some(delta) = reveal.map(|reveal| reveal.y).filter(|delta| *delta != 0.0) {
            self.animation = None;
            self.scroll += delta;
        }
        self.scroll = self.clamp_scroll(self.scroll);
        // Scroll offsets land on device pixels, as Chromium's do.
        self.scroll = (self.scroll * scale).round() / scale;

        // The header comes before the messages on the page, though it's painted over them.
        let header_slot = exo_ui::find::slot(&ctx);
        // Everything in the messages is clipped to them, under the header.
        let mut list = ui.new_child(UiBuilder::new().max_rect(messages));
        list.set_clip_rect(messages.intersect(ui.clip_rect()));
        let show_focus = matches!(
            region,
            Region::ThreadMessages | Region::ThreadMessagesNewMessages
        );
        let scope = exo_ui::find::Scope::begin(&ctx, find, messages, self.scroll);
        self.paint_messages(
            &mut list,
            &layout,
            &detail,
            messages,
            rem,
            show_focus,
            bodies,
            &mut actions,
            drafts,
            &draft_heights,
        );
        if layout.is_wide() {
            layout.paint_edges(ui, messages.top(), messages.bottom());
            self.scrollbar_x.show_horizontal(
                &list,
                list.id().with("thread-scrollbar-x"),
                messages,
                self.scroll_x,
                messages.width() + self.max_scroll_x,
            );
        }
        if self.has_new {
            let target = messages.top() - self.scroll + self.end;
            if new_messages::show(
                &list,
                &layout,
                messages,
                target,
                region == Region::ThreadMessagesNewMessages,
            ) {
                actions.push(Action::ShowNewMessages);
            }
        }
        drop(scope);
        let scrolled = self.scroll > 0.0;
        let focus = if region == Region::ThreadMessagesHeader {
            self.header_focus()
        } else {
            None
        };
        let header = exo_ui::find::in_slot(&ctx, header_slot, || {
            header::show(
                ui,
                &layout,
                &detail,
                &self.chips,
                header_height,
                self.status.as_ref(),
                scrolled,
                typing,
                focus,
                &mut self.selection,
                &self.now,
            )
        });
        actions.extend(header);
        self.select_header_text(ui, bodies, region, &mut actions);
        self.header_tooltips(ui, &layout, &detail, messages, &mut actions);
        if let Some(person) = self.contact.take() {
            actions.push(Action::ShowContact(person));
        }
        actions
    }

    /// A press and drag in the header's text selects it, and lets go of the bodies' selection.
    /// Letting go on the subject with text selected suggests Copy Subject, and letting go with the
    /// selection in a full header's addresses Copy Recipients (with the messages' keyboard).
    fn select_header_text(
        &mut self,
        ui: &Ui,
        bodies: &Bodies,
        region: Region,
        actions: &mut Vec<Action>,
    ) {
        let (started, released) = self.selection.interact(ui, ui.id().with("header-text"));
        if started {
            bodies.clear_selection();
        }
        let Some(released) = released else { return };
        let subject = released.on_one_piece
            && released.anchor == SELECTED_SUBJECT
            && released.focus == SELECTED_SUBJECT;
        if subject {
            actions.push(Action::CopyHint { subject: true });
        } else if region == Region::ThreadMessages
            && (released.anchor == SELECTED_CONTACTS || released.focus == SELECTED_CONTACTS)
        {
            actions.push(Action::CopyHint { subject: false });
        }
    }

    /// The text selected in the header, if any, for ⌘C.
    pub fn selected_text(&self) -> Option<String> {
        self.selection.text()
    }

    /// The participants' and the subject's tooltips, and the contact of a name the pointer
    /// settles on.
    fn header_tooltips(
        &mut self,
        ui: &Ui,
        layout: &Layout,
        detail: &ThreadDetail,
        messages: Rect,
        actions: &mut Vec<Action>,
    ) {
        // Names scrolled under the header aren't there to hover.
        let named: Vec<(Rect, exo_store::Contact)> = self
            .named
            .iter()
            .filter_map(|(rect, contact)| {
                Some((rect.intersect(messages), contact.clone()))
                    .filter(|(rect, _)| rect.is_positive())
            })
            .collect();
        let pointer = ui
            .ctx()
            .pointer_hover_pos()
            .filter(|at| ui.ctx().layer_id_at(*at) == Some(ui.layer_id()));
        let over = named
            .iter()
            .find(|(rect, _)| pointer.is_some_and(|at| rect.contains(at)))
            .map(|(_, contact)| contact.clone());
        if let Some(contact) = self.name_intent.update(ui.ctx(), over) {
            self.contact = Some(Person::new(
                contact.name.clone().unwrap_or_default(),
                contact.email.clone(),
            ));
        }
        let subject = header::subject_rect(ui, layout, detail, &self.chips)
            .map(|rect| (rect, header::title(detail)));
        match self
            .tooltips
            .show(ui, &named, subject, layout.column.x_range())
        {
            Some(tooltips::Clicked::ComposeTo(person, pop_out)) => {
                actions.push(Action::ComposeTo(person, pop_out))
            }
            Some(tooltips::Clicked::Copied(text)) => actions.push(Action::Copied(text)),
            None => {}
        }
    }

    /// What the messages column lists, in order: the messages, a trashed one folded into its
    /// marker, and `Reminder returned` after the last message that came before it.
    fn order(&self, detail: &ThreadDetail) -> Vec<Entry> {
        let mut order: Vec<Entry> = self
            .items
            .iter()
            .enumerate()
            .map(|(at, item)| {
                if item.trashed && !item.expanded {
                    Entry::Trashed(at)
                } else {
                    Entry::Message(at)
                }
            })
            .collect();
        if let Some(exo_store::ReminderStatus::Returned { at, .. }) = detail.reminder {
            let date = |entry: &Entry| match *entry {
                Entry::Message(item) | Entry::Trashed(item) => self
                    .message(&self.items[item].id)
                    .map(|message| message.date),
                Entry::Reminder => None,
            };
            let after = order
                .iter()
                .rposition(|entry| date(entry).is_some_and(|date| date <= at))
                .map_or(0, |last| last + 1);
            order.insert(after, Entry::Reminder);
        }
        order
    }

    /// Paints the visible messages and markers into `ui` (clipped to the message column) and
    /// handles their clicks.
    #[expect(
        clippy::too_many_arguments,
        reason = "the frame's layout, handed down from show"
    )]
    fn paint_messages(
        &mut self,
        ui: &mut Ui,
        layout: &Layout,
        detail: &ThreadDetail,
        messages: Rect,
        rem: f32,
        focused: bool,
        bodies: &mut Bodies,
        actions: &mut Vec<Action>,
        mut drafts: Option<&mut crate::compose::Inline<'_>>,
        draft_heights: &HashMap<String, f32>,
    ) {
        let scale = ui.ctx().pixels_per_point();
        let theme = exo_ui::theme(ui.ctx());
        let origin = pos2(layout.card.left(), messages.top() - self.scroll);
        let pointer = ui.input(|input| input.pointer.hover_pos());
        self.hovered_body = None;
        self.named.clear();
        self.body_origins.clear();
        // Failed drafts' cards, whose open tooltips go over every card.
        let mut failed = Vec::new();
        // The body holding the selection, whose hint goes over every card.
        let mut selected = None;
        // The invite whose When is under the pointer: its message, and when it starts.
        let mut on_when = None;
        // Every message counts, out of view or not: find searches them all.
        let searching = exo_ui::find::searching(ui.ctx());
        for placed in self.entries.clone() {
            let layout = &match placed.entry {
                Entry::Message(at) => self.item_layout(layout, at),
                Entry::Trashed(_) | Entry::Reminder => Self::spanning(layout),
            };
            let card = Rect::from_min_size(
                origin + vec2(0.0, placed.top),
                vec2(layout.card.width(), placed.card),
            );
            if !searching
                && !Rect::from_min_size(card.min, vec2(card.width(), placed.height))
                    .expand(32.0)
                    .intersects(messages)
            {
                continue;
            }
            let at = match placed.entry {
                Entry::Message(at) => at,
                Entry::Trashed(at) => {
                    let id = ui.id().with(("trashed", &self.items[at].id));
                    if markers::trashed(ui, card, layout, id) {
                        // Expands it, then focuses it and scrolls to it.
                        self.items[at].expanded = true;
                        self.count_toggle(true, actions);
                        self.focus_at(at, 0);
                    }
                    continue;
                }
                Entry::Reminder => {
                    if let Some(exo_store::ReminderStatus::Returned { at, .. }) = detail.reminder {
                        markers::reminder(
                            ui,
                            card,
                            layout,
                            at,
                            &self.now,
                            ui.id().with("reminder-date"),
                        );
                    }
                    continue;
                }
            };
            let item = self.items[at].clone();
            let Some(message) = detail.messages.iter().find(|message| message.id == item.id) else {
                continue;
            };
            let is_focus = self.focus.as_deref() == Some(&item.id);
            let state = message::State {
                focused: is_focus,
                show_focus: focused,
                header_expanded: item.header_expanded,
                collapsible: self.collapsible(at),
                composing: drafts
                    .as_ref()
                    .is_some_and(|drafts| drafts.has_form(&item.id)),
                offline: !self.online,
                full_names: self.full_names,
                reply_labels: is_focus && self.new_account(),
            };
            let response = ui.interact(
                card.intersect(messages),
                ui.id().with(("message", &item.id)),
                Sense::click_and_drag(),
            );
            if item.expanded {
                let marketing = bodies
                    .facts(&item.id)
                    .is_some_and(|facts| message::marketing(facts, item.show_quoted));
                let look = layout.look(
                    scale,
                    theme,
                    rem,
                    item.show_quoted,
                    marketing,
                    self.hides_body(&item.id, bodies),
                );
                let invite_block = self.invite_block(ui, &item.id, layout.inner.width(), bodies);
                let content_top = card.top() + message::body_offset(item.header_expanded, message);
                let body = bodies.height(&item.id, look).map(|height| {
                    Rect::from_min_size(
                        pos2(layout.inner.left(), content_top + invite_block),
                        vec2(layout.inner.width(), height),
                    )
                });
                let pointed = body.map(|body| overflow::scroll(ui, bodies, &item.id, body));
                let hit = message::expanded(
                    ui,
                    card,
                    layout,
                    message,
                    detail,
                    marketing,
                    &state,
                    look,
                    bodies,
                    &self.files,
                    invite_block,
                    &self.now,
                    response.hovered(),
                    &mut self.named,
                    &mut self.selection,
                );
                if message.failure.is_some() && !item.header_expanded {
                    failed.push((card, at));
                }
                if let Some(parsed) = self.invites.get(&item.id) {
                    let id = ui.id().with(("invite", &item.id));
                    let focused = self
                        .invite_focus
                        .filter(|_| self.focus.as_deref() == Some(item.id.as_str()));
                    let calendar = bodies.facts(&item.id).map(|facts| &facts.calendar);
                    let context = invite::Context {
                        me: parsed.calendar_account(&self.me, &self.calendar_accounts),
                        user: &self.me,
                        now: &self.now,
                        message,
                        calendar,
                        about_open: self.about_open.contains(&item.id),
                    };
                    let mut card_top = content_top;
                    if let Some(note) = invite::note(parsed, calendar) {
                        actions.extend(
                            invite::show_note(
                                ui,
                                layout.inner.left(),
                                card_top,
                                layout.inner.width(),
                                &note,
                                id,
                            )
                            .map(Action::OpenUrl),
                        );
                        card_top += invite::note_height(ui.ctx(), &note, layout.inner.width());
                    }
                    let shown = invite::show(
                        ui,
                        layout.inner.left(),
                        card_top,
                        layout.inner.width(),
                        parsed,
                        &context,
                        &mut self.photos,
                        &self.images,
                        id,
                        focused,
                    );
                    if shown.on_when {
                        on_when = Some((item.id.clone(), parsed.start));
                    }
                    self.named.extend(shown.names);
                    match shown.clicked {
                        Some(invite::CardAction::Open(url)) if url.starts_with("tel:") => {
                            actions.push(Action::Dial(url))
                        }
                        Some(invite::CardAction::Open(url)) => actions.push(Action::OpenUrl(url)),
                        Some(invite::CardAction::Respond(response))
                            if self.needs_sign_in(parsed) =>
                        {
                            actions.push(Action::RespondToInvite(parsed.clone(), response))
                        }
                        Some(invite::CardAction::Respond(response)) => {
                            let mut answered = parsed.clone();
                            answered.answer(
                                parsed.calendar_account(&self.me, &self.calendar_accounts),
                                response,
                            );
                            actions.push(Action::RespondToInvite(answered.clone(), response));
                            self.invites.insert(item.id.clone(), answered);
                        }
                        Some(invite::CardAction::ToggleAbout) => {
                            if !self.about_open.remove(&item.id) {
                                self.about_open.insert(item.id.clone());
                            }
                        }
                        None => {}
                    }
                }
                if let (Some(body), Some(pointed)) = (body, pointed) {
                    self.body_origins.insert(item.id.clone(), body.min);
                    overflow::scrollbar(ui, bodies, &item.id, body);
                    self.body_pointer(ui, &response, &item.id, pointed, pointer, bodies);
                    if bodies
                        .selection_bounds()
                        .is_some_and(|(id, _)| id == item.id)
                    {
                        selected = Some((item.id.clone(), body.min));
                    }
                }
                let draft = draft_heights.get(&item.id).copied().unwrap_or(0.0);
                if draft > 0.0
                    && let Some(drafts) = drafts.as_mut()
                {
                    let top = card.bottom() - message::PADDING_BOTTOM - draft;
                    let form = Rect::from_min_size(
                        pos2(layout.inner.left(), top),
                        vec2(layout.inner.width(), draft),
                    );
                    actions.extend(
                        drafts
                            .show(ui, &item.id, form, card, layout.column.x_range())
                            .into_iter()
                            .map(Action::Compose),
                    );
                }
                if let Some(name) = &self.intro {
                    footer::paint_intro(
                        ui,
                        layout,
                        card.bottom()
                            + collapse(message::EXPANDED_MARGIN_BOTTOM, footer::MARGIN_TOP),
                        name,
                    );
                }
                match hit {
                    Some(message::Hit::Header) if state.collapsible && draft == 0.0 => {
                        self.items[at].expanded = false;
                        self.count_toggle(false, actions);
                    }
                    Some(message::Hit::HeaderToggle { hint }) => {
                        self.items[at].header_expanded = !item.header_expanded;
                        if hint {
                            actions.push(Action::HeaderHint {
                                expanded: !item.header_expanded,
                            });
                        }
                    }
                    Some(message::Hit::Files(attachments::Click::Open {
                        attachment,
                        preview,
                    })) => {
                        actions.push(open_file(attachments::listed(message), attachment, preview))
                    }
                    Some(message::Hit::Files(attachments::Click::DownloadAll { message })) => {
                        actions.push(Action::DownloadAll { message })
                    }
                    Some(message::Hit::Files(attachments::Click::Drag { attachment, at })) => {
                        actions.push(Action::DragFile { attachment, at })
                    }
                    Some(message::Hit::Reply(action)) => {
                        self.focus = Some(item.id.clone());
                        actions.push(Action::Compose(crate::compose::Action::Reply(action)));
                    }
                    Some(message::Hit::Recover) => {
                        self.focus = Some(item.id.clone());
                        actions.push(Action::Compose(crate::compose::Action::Recover(
                            item.id.clone(),
                        )));
                    }
                    Some(message::Hit::Header) | None => {}
                }
                if response.clicked() || response.drag_started() {
                    self.focus = Some(item.id.clone());
                }
            } else {
                message::collapsed(
                    ui,
                    card,
                    layout,
                    message,
                    detail,
                    bodies.facts(&item.id),
                    &state,
                    &self.now,
                    response.hovered(),
                    &mut self.named,
                );
                // A pointer cursor over a collapsed row.
                let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.clicked() {
                    self.items[at].expanded = true;
                    self.focus = Some(item.id.clone());
                    self.count_toggle(true, actions);
                }
            }
        }
        // In a layer of their own, so neither the column nor its cards cut their
        // shadows; under the thread's header.
        let mut tooltips = ui.painter().clone();
        tooltips.set_clip_rect(Rect::from_x_y_ranges(
            ui.ctx().content_rect().x_range(),
            messages.y_range(),
        ));
        for (card, at) in failed {
            let Some(message) = detail
                .messages
                .iter()
                .find(|message| message.id == self.items[at].id)
            else {
                continue;
            };
            if let Some(url) = message::failed_tooltip(ui, &tooltips, layout, card, message) {
                actions.push(Action::OpenUrl(url.to_owned()));
            }
        }
        if let Some((id, body)) = selected {
            self.selection_hint(ui, &Self::spanning(layout), messages, body, bodies, &id);
        }
        if let Some(id) = self.when_intent.update(
            ui.ctx(),
            on_when
                .as_ref()
                .map(|(id, _): &(String, jiff::Timestamp)| id.clone()),
        ) && let Some((_, start)) = on_when.as_ref().filter(|(over, _)| over == id)
        {
            actions.push(Action::ShowDay(
                start.to_zoned(self.now.time_zone().clone()).date(),
            ));
        }
        self.hover_date(ui, &Self::spanning(layout), messages, bodies, actions);
        // Hovering a `mailto:` link shows its address's contact, with hover intent.
        let mailto = self
            .hovered_body
            .as_deref()
            .and_then(|id| match bodies.hovered(id) {
                Some(Hit::Link { href })
                    if href
                        .get(..7)
                        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("mailto:")) =>
                {
                    Some(href.clone())
                }
                _ => None,
            });
        self.hover_mailto(ui.ctx(), mailto);
        self.scrollbar.show(
            ui,
            ui.id().with("thread-scrollbar"),
            messages,
            self.scroll,
            self.content,
        );
    }

    /// A click on a header expanded (or folded) a message: the third
    /// in a row each way suggests ⇧O.
    fn count_toggle(&mut self, expanded: bool, actions: &mut Vec<Action>) {
        let (expands, collapses) = &mut self.clicks_in_a_row;
        let (counted, other) = if expanded {
            (expands, collapses)
        } else {
            (collapses, expands)
        };
        *other = 0;
        *counted = counted.saturating_add(1);
        if *counted >= 3 {
            actions.push(Action::ExpandAllHint { expand: expanded });
        }
    }

    /// The pointer over an expanded body (`body`, on screen): hover hit-tests and the cursor
    /// they pick, clicks, drag selections.
    fn body_pointer(
        &mut self,
        ui: &Ui,
        response: &egui::Response,
        id: &str,
        body: Rect,
        pointer: Option<Pos2>,
        bodies: &mut Bodies,
    ) {
        let local = |at: Pos2| (at - body.min).to_pos2();
        match pointer.filter(|at| body.contains(*at)) {
            Some(at) if response.hovered() => {
                self.hovered_body = Some(id.to_owned());
                // Every frame, not only while the pointer moves: scrolling, a header unfolding or
                // a relayout moves the body under a still pointer.
                bodies.hover(id, local(at));
                // Chromium's `cursor: auto`: the I-beam over text, the arrow over nothing.
                match bodies.hovered(id) {
                    Some(Hit::Text) => ui.ctx().set_cursor_icon(egui::CursorIcon::Text),
                    Some(Hit::Link { .. } | Hit::QuoteToggle | Hit::InlineImage { .. }) => {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand)
                    }
                    // A detected date takes a pointer cursor when it names a day.
                    Some(Hit::Date { text, .. }) => {
                        ui.ctx()
                            .set_cursor_icon(if self.date_on(id, text).is_some() {
                                egui::CursorIcon::PointingHand
                            } else {
                                egui::CursorIcon::Text
                            })
                    }
                    None => {}
                }
            }
            _ => bodies.unhover(id),
        }
        // A double or triple click selects the word or paragraph as the button goes down, as in
        // Chromium (and a fourth click in a row is another triple click, as macOS counts them).
        let (pressed, time) = ui.input(|input| (input.pointer.primary_pressed(), input.time));
        if pressed
            && response.hovered()
            && let Some(at) = pointer.filter(|at| body.contains(*at))
        {
            let count = match self.clicks {
                Some((then, place, count))
                    if time - then < MULTI_CLICK_DELAY
                        && place.distance(at) < MULTI_CLICK_DISTANCE =>
                {
                    (count + 1).min(3)
                }
                _ => 1,
            };
            self.clicks = Some((time, at, count));
            let granularity = match count {
                1 => None,
                2 => Some(Granularity::Word),
                _ => Some(Granularity::Paragraph),
            };
            if let Some(granularity) = granularity {
                bodies.select_from(id, local(at), granularity);
                self.selecting = Some(id.to_owned());
                self.selection.clear();
            }
        }
        let multi_click = self.clicks.is_some_and(|(.., count)| count > 1);
        // A selection starts where the button went down, not where the drag was recognized; after
        // a double or triple click, the drag extends that word or paragraph.
        if response.drag_started() && !multi_click {
            self.selecting = None;
            if let Some(at) = ui
                .input(|input| input.pointer.press_origin())
                .filter(|at| body.contains(*at))
            {
                bodies.select_from(id, local(at), Granularity::Character);
                self.selecting = Some(id.to_owned());
                self.selection.clear();
            }
        }
        if response.dragged()
            && self.selecting.as_deref() == Some(id)
            && let Some(at) = response.interact_pointer_pos()
        {
            bodies.select_to(id, local(at));
        }
        // A right-click: the shell's menu for what it's on.
        if response.secondary_clicked()
            && let Some(at) = response
                .interact_pointer_pos()
                .filter(|at| body.contains(*at))
        {
            bodies.context_menu(id, local(at), at);
        }
        if response.clicked()
            && !multi_click
            && let Some(at) = response
                .interact_pointer_pos()
                .filter(|at| body.contains(*at))
        {
            bodies.clear_selection();
            // ⌘ on a link opens it in the background; what the click hit comes back later.
            self.click_modifiers = ui.input(|input| input.modifiers);
            bodies.click(id, local(at));
        }
    }

    /// Where message `id`'s body was drawn this frame.
    fn body_origin(&self, id: &str) -> Option<Pos2> {
        self.body_origins.get(id).copied()
    }

    /// What a click in a body landed on.
    fn clicked(&mut self, id: &str, hit: &Hit, bodies: &Bodies, actions: &mut Vec<Action>) {
        match hit {
            // `#id` scrolls to the anchor; `mailto:` opens compose (the app's), and ⌘ opens a
            // web link in the background.
            Hit::Link { href } if href.starts_with('#') => bodies.find_anchor(id, &href[1..]),
            Hit::Link { href } if self.click_modifiers.command => {
                actions.push(Action::OpenUrlInBackground(href.clone()))
            }
            Hit::Link { href } => actions.push(Action::OpenUrl(href.clone())),
            Hit::Text => {}
            Hit::Date { text, .. } => {
                if let Some(date) = self.date_on(id, text) {
                    actions.push(Action::CreateEvent(date));
                }
            }
            Hit::QuoteToggle => {
                if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
                    item.show_quoted = !item.show_quoted;
                    actions.push(Action::QuoteHint {
                        shown: item.show_quoted,
                    });
                }
            }
            Hit::InlineImage { cid } => {
                // The viewer over all the message's images and messages.
                let message = self.message(id);
                let attachment = message.and_then(|message| {
                    message
                        .attachments
                        .iter()
                        .find(|attachment| attachment.content_id.as_deref() == Some(cid))
                        .cloned()
                });
                if let (Some(message), Some(attachment)) = (message, attachment) {
                    actions.push(open_file(message.attachments.iter(), attachment, true));
                }
            }
        }
    }

    /// Where a newly opened thread scrolls: past all but four of the collapsed
    /// messages above the first expanded one, or, when the only expanded message is the last,
    /// with it `IDEAL_TOP` below the top.
    fn initial_scroll(&self, collapsed_scroll: f32) -> f32 {
        let first_expanded = self.items.iter().position(|item| item.expanded);
        let target = match first_expanded {
            Some(at) if at + 1 == self.items.len() => self.tops[at].0 - IDEAL_TOP,
            _ => collapsed_scroll,
        };
        self.clamp_scroll(target)
    }

    /// Scrolls to what an in-message link pointed at, `top` CSS px down message `id`'s body, so
    /// that it starts just under the scrolled header.
    fn scroll_to_anchor(&mut self, ui: &Ui, layout: &Layout, id: &str, top: f32, bodies: &Bodies) {
        let Some(at) = self.items.iter().position(|item| item.id == id) else {
            return;
        };
        let Some(message) = self.message(id) else {
            return;
        };
        let body = message::body_offset(self.items[at].header_expanded, message);
        let invite = self.invite_block(ui, id, self.item_layout(layout, at).inner.width(), bodies);
        // The messages start under the header's bottom padding; once scrolled, the header ends
        // `SCROLLED_INSET` above its bottom.
        let under_header = -MESSAGES_MARGIN_TOP - header::SCROLLED_INSET;
        self.animation = None;
        self.scroll = self.clamp_scroll(self.tops[at].0 + body + invite + top - under_header);
    }

    /// Brings a draft into view: a card that fits is scrolled up to its top; otherwise its draft's
    /// bottom is brought into view.
    fn scroll_to_draft(&mut self, id: &str) {
        let Some(at) = self.items.iter().position(|item| item.id == id) else {
            return;
        };
        let Some(&(top, height)) = self.tops.get(at) else {
            return;
        };
        let fits = height + MESSAGES_PADDING_TOP <= self.viewport;
        let to = if fits && top < self.scroll + MESSAGES_PADDING_TOP {
            top - MESSAGES_PADDING_TOP
        } else if top + height > self.scroll + self.viewport - DRAFT_MARGIN_BOTTOM {
            if fits {
                top - MESSAGES_PADDING_TOP
            } else {
                top + height - self.viewport + DRAFT_MARGIN_BOTTOM
            }
        } else {
            return;
        };
        self.animate_over(to, DRAFT_SCROLL_DURATION);
    }

    /// Keeps the focused draft's bottom 28px above the messages' bottom.
    fn keep_draft_in_view(&mut self, drafts: &HashMap<String, f32>) {
        let bottom = self
            .items
            .iter()
            .zip(&self.tops)
            .filter(|(item, _)| drafts.get(&item.id).is_some_and(|height| *height > 0.0))
            .map(|(_, (top, height))| top + height)
            .fold(f32::NEG_INFINITY, f32::max);
        if bottom.is_finite() && bottom > self.scroll + self.viewport - DRAFT_MARGIN_BOTTOM {
            self.scroll = self.clamp_scroll(bottom - self.viewport + DRAFT_MARGIN_BOTTOM);
        }
    }

    /// Whether a draft (its form, at the bottom of its message's card) shows in the viewport.
    fn draft_in_view(&self, drafts: &HashMap<String, f32>) -> bool {
        let (view_top, view_bottom) = (self.scroll, self.scroll + self.viewport);
        self.items
            .iter()
            .zip(&self.tops)
            .any(|(item, (top, height))| {
                drafts.get(&item.id).is_some_and(|draft| {
                    *draft > 0.0 && top + height - draft < view_bottom && top + height > view_top
                })
            })
    }

    /// N moving onto a message just below the fold scrolls a step; a
    /// message that's out of view scrolls to `IDEAL_TOP` below the top.
    fn scroll_to_message(&mut self, id: &str, direction: isize) {
        let Some(at) = self.items.iter().position(|item| item.id == id) else {
            return;
        };
        let Some(&(top, height)) = self.tops.get(at) else {
            return;
        };
        let (view_top, view_bottom) = (self.scroll, self.scroll + self.viewport);
        if direction > 0 && top > view_bottom - IDEAL_TOP / 2.0 && top < view_bottom + IDEAL_TOP {
            self.animate_to(self.scroll + IDEAL_TOP);
        } else if top + SLACK > view_bottom || top + height - SLACK < view_top {
            self.animate_to(top - IDEAL_TOP);
        }
    }
}

/// What the new-message pane holds: its title, the form's height, and the room kept above the
/// card (a snippet's name sits there, 36px).
pub(crate) struct PaneCard<'a> {
    pub title: &'a str,
    pub height: f32,
    pub above: f32,
    /// The pop-out hint goes under the card: a new message started in a conversation, untouched
    /// yet.
    pub pop_out_hint: bool,
    /// The card holds no form: a read-only
    /// snippet.
    pub read_only: bool,
}

/// The thread pane for a new message: the back gutter, the title
/// (`New Message`) with the conversation arrows, and one card holding the compose form, which
/// `form` draws into the card's content column (left, top, width) and whose height the card says;
/// it's also given the message column's extent, which tooltips stay inside, and the card.
pub(crate) fn new_message_pane(
    ui: &mut Ui,
    rect: Rect,
    rem: f32,
    window_width: f32,
    popped_out: bool,
    pane: &PaneCard<'_>,
    form: impl FnOnce(&mut Ui, Rect, egui::Rangef, Rect) -> Vec<crate::compose::Action>,
) -> Vec<Action> {
    let (mut actions, form) = card_pane(ui, rect, rem, window_width, popped_out, pane, form);
    actions.splice(0..0, form.into_iter().map(Action::Compose));
    actions
}

/// [`new_message_pane`] around any card's content: what `content` returns comes back beside the
/// pane's own actions.
pub(crate) fn card_pane<T>(
    ui: &mut Ui,
    rect: Rect,
    rem: f32,
    window_width: f32,
    popped_out: bool,
    pane: &PaneCard<'_>,
    content: impl FnOnce(&mut Ui, Rect, egui::Rangef, Rect) -> T,
) -> (Vec<Action>, T) {
    let height = pane.height;
    let palette = exo_ui::palette(ui.ctx());
    ui.painter()
        .rect_filled(rect, 0.0, palette.background_base_default);
    let layout = Layout::new(rect, rem, window_width, popped_out);
    let mut actions = Vec::new();
    let gutter = ui
        .interact(
            layout.gutter_target(),
            ui.id().with("thread-back-gutter"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let compact_clicked = header::back_button(ui, &layout, gutter.hovered(), false);
    if gutter.clicked() || compact_clicked {
        actions.push(Action::Back);
    }
    let messages_top = layout.header.top() + header::title_height() + MESSAGES_MARGIN_TOP;
    let card = Rect::from_min_size(
        pos2(
            layout.card.left(),
            messages_top + MESSAGES_PADDING_TOP + CARD_MARGIN_TOP + pane.above,
        ),
        vec2(
            layout.card.width(),
            message::PADDING_TOP + height + message::PADDING_BOTTOM,
        ),
    );
    let clip = Rect::from_min_max(
        pos2(layout.column.left(), messages_top),
        pos2(layout.column.right(), rect.bottom()),
    );
    let mut list = ui.new_child(UiBuilder::new().max_rect(clip));
    list.set_clip_rect(clip.intersect(ui.clip_rect()));
    message::card(
        &list,
        card,
        &layout,
        if pane.read_only {
            message::CardKind::Expanded
        } else {
            message::CardKind::Composing
        },
        false,
    );
    if pane.pop_out_hint {
        footer::paint_pop_out(
            &list,
            &layout,
            card.bottom()
                + collapse(
                    message::EXPANDED_MARGIN_BOTTOM,
                    footer::MARGIN_TOP_UNDER_DRAFT,
                ),
        );
    }
    let content_rect = Rect::from_min_size(
        pos2(layout.inner.left(), card.top() + message::PADDING_TOP),
        vec2(layout.inner.width(), height),
    );
    let shown = content(&mut list, content_rect, layout.column.x_range(), card);
    actions.extend(header::show_title(ui, &layout, pane.title));
    (actions, shown)
}

/// The width of a message card's content column, which a new message's form fills.
pub(crate) fn content_width(rect: Rect, rem: f32, window_width: f32, popped_out: bool) -> f32 {
    Layout::new(rect, rem, window_width, popped_out)
        .inner
        .width()
}

/// The left edge of a message card's content column.
pub(crate) fn content_left(rect: Rect, rem: f32, window_width: f32, popped_out: bool) -> f32 {
    Layout::new(rect, rem, window_width, popped_out)
        .inner
        .left()
}

/// Where a popped-out draft goes beside the thread: the left edge and width of its column, in a
/// content pane `rect`.
pub(crate) fn popout_column(rect: Rect, rem: f32, window_width: f32) -> (f32, f32) {
    let popout = Popout::new(
        container(rect, rem, window_width, true),
        rect.left(),
        window_width,
        rem,
    );
    (popout.left, popout.width)
}

/// How far the pop-out slides left, out of the contact pane's way, while the pointer rests over
/// that pane: in a content pane `rect` too narrow for both
/// columns, until the pop-out ends where the thread's does. Zero when they fit.
pub(crate) fn popout_slide(rect: Rect, rem: f32, window_width: f32) -> f32 {
    let container = container(rect, rem, window_width, true);
    let columns = container.width();
    if columns >= 2.0 * (500.0 + 7.0 * rem) - 2.0 * rem {
        return 0.0;
    }
    let popout = Popout::new(container, rect.left(), window_width, rem);
    // `Popout` has added a rem to the width and taken it off the left.
    rect.left() + columns - (popout.left + popout.width)
}

/// A popped-out draft's columns, for the thread's container `container`, with the left
/// rail and pane `rail` wide in a window `window_width` wide.
struct Popout {
    /// Both columns' width.
    width: f32,
    /// How far the thread's column moves right.
    gutter: f32,
    /// The pop-out column's left edge.
    left: f32,
}

impl Popout {
    fn new(container: Rect, rail: f32, window_width: f32, rem: f32) -> Self {
        let columns = container.width();
        let widest = SANDBOX_MAX_WIDTH + 7.0 * rem;
        let narrowest = 500.0 + 7.0 * rem;
        let (mut width, mut left, mut gutter) = (widest, rail + columns / 2.0, 0.0);
        if columns > 2.0 * widest {
            gutter = (columns - 2.0 * widest) / 2.0;
        } else if columns > 2.0 * narrowest {
            width = columns / 2.0;
        } else {
            width = (columns - rem).min(narrowest);
            left = (rail + width).min(window_width - width);
        }
        Self {
            width: width + rem,
            gutter,
            left: left - rem,
        }
    }
}

/// The thread's container in a content pane `rect`.
fn container(rect: Rect, rem: f32, window_width: f32, popped_out: bool) -> Rect {
    container_and_gutter(rect, rem, window_width, popped_out).0
}

/// The thread's container and the back gutter beside it in a content pane `rect`, and the list
/// layout's left pane width. The gutter is `flex: 5.25 0 0; max-width: clamp(300px, 416px, …)`
/// and the thread `flex: 1 1 calc(640px + 8rem)` with `max-width: calc(100% - 4rem)`, centred:
/// spare room grows both, and whatever the thread can't take goes to the gutter. Beside a
/// popped-out draft the thread's basis is `calc(2 * (640px + 7rem))` and it may take the whole
/// pane.
fn container_and_gutter(
    rect: Rect,
    rem: f32,
    window_width: f32,
    popped_out: bool,
) -> (Rect, Rect, f32) {
    let (basis, thread_max) = if popped_out {
        (2.0 * (SANDBOX_MAX_WIDTH + 7.0 * rem), rect.width())
    } else {
        (SANDBOX_MAX_WIDTH + 8.0 * rem, rect.width() - 4.0 * rem)
    };
    // The list layout's left pane: `clamp(300px, 416px, 23.61vw + 33*(1440px - 100vw)/480)`.
    let left_pane =
        (0.2361 * window_width + 33.0 * (1440.0 - window_width) / 480.0).clamp(300.0, 416.0);
    let free = rect.width() - basis;
    let (mut action, mut thread) = if free >= 0.0 {
        (free * 5.25 / 6.25, basis + free / 6.25)
    } else {
        (0.0, basis + free)
    };
    if thread > thread_max {
        action += thread - thread_max;
        thread = thread_max;
    }
    if action > left_pane {
        thread = (thread + action - left_pane).min(thread_max);
        action = left_pane;
    }
    let spare = (rect.width() - action - thread).max(0.0);
    let action_pane = Rect::from_min_size(
        pos2(rect.left() + spare / 2.0, rect.top()),
        vec2(action, rect.height()),
    );
    let container = Rect::from_min_size(
        pos2(action_pane.right(), rect.top()),
        vec2(thread, rect.height()),
    );
    (container, action_pane, left_pane)
}

/// The widest window whose thread keeps 4px more room at the right, and the room.
const NARROW_WINDOW: f32 = 1024.0;
const NARROW_RIGHT: f32 = 4.0;

/// Where everything in the thread view goes for one window size.
#[derive(Clone, Debug)]
pub(crate) struct Layout {
    rem: f32,
    popped_out: bool,
    /// The list layout's left pane width, which the back gutter's hover gradient spans 75% of.
    left_pane: f32,
    /// The left back gutter.
    action_pane: Rect,
    /// The header, full height.
    header: Rect,
    /// The subject line's box, inside the header's padding.
    label: Rect,
    /// The message column.
    column: Rect,
    /// A message card's horizontal extent.
    card: Rect,
    /// A message's content inside the card's padding (the body's column).
    inner: Rect,
    /// The column widened for a wide body, else `None`.
    wide: Option<wide::Column>,
}

impl Layout {
    fn new(rect: Rect, rem: f32, window_width: f32, popped_out: bool) -> Self {
        let (container, action_pane, left_pane) =
            container_and_gutter(rect, rem, window_width, popped_out);
        let basis = SANDBOX_MAX_WIDTH + 8.0 * rem;
        // The header and the messages: `max-width: calc(640px + 8rem)`, centred; beside a
        // popped-out draft (`align-items: flex-start`) the header is up to twice as wide and the
        // messages take the pop-out's width, both at the left.
        let (header, column) = if popped_out {
            let header_width = container.width().min(2.0 * (SANDBOX_MAX_WIDTH + 7.0 * rem));
            let popout = Popout::new(container, rect.left(), window_width, rem);
            let header = Rect::from_min_size(container.min, vec2(header_width, rect.height()));
            (
                header,
                Rect::from_min_size(
                    pos2(container.left() + popout.gutter, rect.top()),
                    vec2(popout.width, rect.height()),
                ),
            )
        } else {
            let column_width = container.width().min(basis);
            let column_left = container.left() + (container.width() - column_width) / 2.0;
            let column = Rect::from_min_size(
                pos2(column_left, rect.top()),
                vec2(column_width, rect.height()),
            );
            (column, column)
        };
        // In a window up to 1024px wide, the header's padding and the messages' margin are 4px
        // wider at the right (`calc(4rem + 4px)`, `calc(2rem + 4px)`).
        let narrow = if window_width <= NARROW_WINDOW {
            NARROW_RIGHT
        } else {
            0.0
        };
        let label = Rect::from_min_size(
            pos2(
                header.left() + 4.0 * rem,
                header.top() + header::PADDING_TOP,
            ),
            vec2(header.width() - 8.0 * rem - narrow, header::SUBJECT_LINE),
        );
        let card = Rect::from_min_size(
            pos2(column.left() + 2.0 * rem, rect.top()),
            vec2(column.width() - 4.0 * rem - narrow, 0.0),
        );
        let inner = Rect::from_min_size(
            pos2(card.left() + 2.0 * rem, rect.top()),
            vec2(card.width() - 4.0 * rem, 0.0),
        );
        Self {
            rem,
            popped_out,
            left_pane,
            action_pane,
            header,
            label,
            column,
            card,
            inner,
            wide: None,
        }
    }

    pub(super) fn rem(&self) -> f32 {
        self.rem
    }

    /// The pane is beside a popped-out draft.
    pub(super) fn popped_out(&self) -> bool {
        self.popped_out
    }

    /// How a body in this layout is shown.
    fn look(
        &self,
        scale: f32,
        theme: exo_ui::Theme,
        rem: f32,
        show_quoted: bool,
        marketing: bool,
        hide_original: bool,
    ) -> body::Look {
        let palette = theme.palette();
        // A marketing card in Carbon is white.
        let card = if marketing && theme == exo_ui::Theme::Carbon {
            egui::Color32::WHITE
        } else {
            palette.background_elevated_default
        };
        body::Look {
            width: self.inner.width(),
            scale,
            theme,
            rem,
            show_quoted,
            hide_original,
            phase: self.inner.left() - self.inner.left().floor(),
            background: card.to_array(),
        }
    }
}

/// What a click on `attachment`, one of `among`, asks for: Exo's viewer over the images and
/// messages among them when `preview` and it's one, else Quick Look or a download.
fn open_file<'a>(
    among: impl Iterator<Item = &'a Attachment>,
    attachment: Attachment,
    preview: bool,
) -> Action {
    if !(preview && crate::attachment_files::is_previewable(&attachment)) {
        return Action::Download {
            attachment,
            open: preview,
        };
    }
    let attachments: Vec<Attachment> = among
        .filter(|candidate| crate::attachment_files::is_previewable(candidate))
        .cloned()
        .collect();
    let start = attachments
        .iter()
        .position(|candidate| {
            candidate.message_id == attachment.message_id && candidate.part_id == attachment.part_id
        })
        .unwrap_or(0);
    Action::Preview { attachments, start }
}
