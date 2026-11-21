//! One message of the thread: a collapsed row, or an expanded card with its header (compact, or
//! Shift-H's full one), body and attachments.

use egui::{Color32, CornerRadius, Rect, Sense, Ui, Vec2, pos2, vec2};
use exo_store::{Contact, MessageDetail, ThreadDetail};
use exo_ui::{
    BoxShadow, Family, Font, IconButton, KeyTooltip, Placement, ShortcutTooltip, TextStyle,
    TextTooltip, Theme,
};
use jiff::{Timestamp, Zoned};

use super::{Layout, attachments};
use crate::{
    body::{Bodies, Facts, Look},
    format::{
        dates, failure, friendly, measure,
        names::{self, NameFormat, Person},
    },
};

/// A collapsed row: 12px padding around a 16px line.
pub(super) const COLLAPSED_HEIGHT: f32 = 40.0;
/// `margin-bottom` of a collapsed and an expanded message.
pub(super) const COLLAPSED_MARGIN_BOTTOM: f32 = 16.0;
pub(super) const EXPANDED_MARGIN_BOTTOM: f32 = 28.0;
/// An expanded card's padding: `12px 2rem 4px`.
pub(crate) const PADDING_TOP: f32 = 12.0;
pub(crate) const PADDING_BOTTOM: f32 = 4.0;
/// The body's margin: `20px 0 8px`.
const BODY_MARGIN_TOP: f32 = 20.0;
const BODY_MARGIN_BOTTOM: f32 = 8.0;
/// A body still being laid out shows three placeholder lines.
const PLACEHOLDER_HEIGHT: f32 = 72.0;
/// The full header: 24px rows, `margin-top: -4px`, `top: 1px`.
pub(crate) const HEADER_ROW: f32 = 24.0;
/// The full header's labels: at least 60px wide.
const HEADER_LABEL: f32 = 60.0;
/// A collapsed row's sender: at least 85px wide, then 15px of padding.
pub(super) const FROM_MIN_WIDTH: f32 = 85.0;
pub(super) const FROM_PADDING: f32 = 15.0;
/// A sent message's date: `padding-left: 14px`; and the reply buttons' legacy glyphs: 13×9, 28px
/// apart, 3px below the line's top, the forward's right edge where the date's padding starts.
const DATE_PADDING: f32 = 14.0;
const ACTION_PITCH: f32 = 28.0;
const ACTION_SIZE: Vec2 = Vec2::new(13.0, 9.0);
/// A labelled reply button: `padding: 10px 8px`, `gap: 4px`, `margin-left: 8px`.
const LABEL_PADDING: Vec2 = Vec2::new(8.0, 10.0);
const LABEL_GAP: f32 = 4.0;
const LABEL_MARGIN_LEFT: f32 = 8.0;
/// `@container message-pane-header (max-width: 500px)`: a labelled header this narrow condenses
/// its participants.
const CONDENSED_WIDTH: f32 = 500.0;
/// The Shift-H chevron (`legacy/DEPRECATED_expand`), 4px after the names.
const CHEVRON: Vec2 = Vec2::new(10.0, 12.0);
const CHEVRON_GAP: f32 = 4.0;
/// The condensed participants' least widths (the sender 18px, the lead 42px) and the count's 4px
/// margin.
const CONDENSED_SENDER_MIN: f32 = 18.0;
const CONDENSED_LEAD_MIN: f32 = 42.0;
const CONDENSED_COUNT_GAP: f32 = 4.0;

/// The expanded header's collapse chevron.
const COLLAPSE: &str = "legacy/DEPRECATED_collapse";
/// The eye after From when read receipts were blocked: 8px after the sender, 1px under the
/// baseline in Classic (`top: 1px`).
const PIXEL_BLOCKING: &str = "legacy/DEPRECATED_pixelBlocking";
const PIXEL_BLOCKING_GAP: f32 = 8.0;
const PIXEL_BLOCKING_DROP: f32 = 1.0;
/// Its tooltip, its top left on the icon's bottom left: the box 14px under the icon and 9px left
/// of it.
const TRACKING_BLOCKED: &str = "Email tracking blocked";
const PIXEL_BLOCKING_TOOLTIP: Placement = Placement::BelowFrom {
    gap: 14.0,
    inset: -9.0,
};

/// The sender line: 14px/16px bold.
pub(super) const FROM: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 16.0);
/// The collapsed snippet: 14px/16px.
pub(super) const SNIPPET: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 16.0);
/// The date: 12px/16px, `letter-spacing: .017em`.
pub(super) const DATE: TextStyle =
    TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0).letter_spacing(0.017 * 12.0);
/// The full header: 14px/24px bold.
pub(crate) const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 24.0);
/// A failed or waiting send's status: bold, `letter-spacing: 0`, `#ef8e8e` and `#ce9ece` in Snow
/// and Carbon alike.
pub(super) const STATUS: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
const FAILED_INK: Color32 = Color32::from_rgb(0xef, 0x8e, 0x8e);
const OFFLINE_INK: Color32 = Color32::from_rgb(0xce, 0x9e, 0xce);
/// What a send waiting for the network says in its date's place.
const WHEN_ONLINE: &str = "Send: when online";
/// A failed send's tooltip: at most 340px wide.
const FAILED_TOOLTIP_WIDTH: f32 = 340.0;

/// The card's corners: 4px.
const RADIUS: u8 = 4;

/// How a message is shown.
pub(super) struct State {
    /// It's the focused message.
    pub focused: bool,
    /// The messages have the keyboard, so the focused one shows its bar.
    pub show_focus: bool,
    pub header_expanded: bool,
    pub collapsible: bool,
    /// It holds a draft's form.
    pub composing: bool,
    /// Gmail can't be reached.
    pub offline: bool,
    /// The header names the sender in full (`showSenderFullNames`).
    pub full_names: bool,
    /// Its reply buttons have labels.
    pub reply_labels: bool,
}

/// What a draft that's sending, scheduled or failed shows in its date's place, which opens
/// `RecoverDraft`.
#[derive(Clone, Copy)]
pub(super) enum Status {
    /// Its send failed.
    Failed(exo_store::SendFailure),
    /// It's scheduled for then: `#ce9ece` bold.
    Scheduled(Timestamp),
    /// It waits for the network.
    Offline,
}

impl Status {
    pub(super) fn of(message: &MessageDetail, state: &State) -> Option<Self> {
        match (message.failure, message.scheduled) {
            (Some(reason), _) => Some(Self::Failed(reason)),
            (None, Some(at)) => Some(Self::Scheduled(at)),
            (None, None) => (message.sending && state.offline).then_some(Self::Offline),
        }
    }

    /// The compact header's text: `Scheduled: ` with the `exact-friendly` date.
    pub(super) fn title(self, now: &Zoned) -> String {
        match self {
            Self::Failed(reason) => failure::information(reason).title.to_owned(),
            Self::Scheduled(at) => format!("Scheduled: {}", friendly::exact_friendly(at, now)),
            Self::Offline => WHEN_ONLINE.to_owned(),
        }
    }

    /// A collapsed row's text (`exact-friendly-omit-time` when scheduled).
    pub(super) fn collapsed_title(self, now: &Zoned) -> String {
        match self {
            Self::Failed(reason) => failure::collapsed_title(reason).to_owned(),
            Self::Scheduled(at) => {
                format!("Scheduled: {}", friendly::exact_friendly_omit_time(at, now))
            }
            Self::Offline => WHEN_ONLINE.to_owned(),
        }
    }

    /// The full header's text: just `Send failed`, or the long `expanded` date.
    pub(super) fn full_title(self, now: &Zoned) -> String {
        match self {
            Self::Failed(_) => "Send failed".to_owned(),
            Self::Scheduled(at) => format!("Scheduled: {}", dates::expanded(at, now)),
            Self::Offline => WHEN_ONLINE.to_owned(),
        }
    }

    pub(super) fn ink(self) -> Color32 {
        match self {
            Self::Failed(_) => FAILED_INK,
            Self::Scheduled(_) | Self::Offline => OFFLINE_INK,
        }
    }
}

/// A scheduled send's tooltip, online or off (Exo keeps the schedule itself).
const SCHEDULED_TOOLTIP: &str = "Message will send at this time";

/// What was clicked in an expanded message, besides its body.
pub(super) enum Hit {
    /// The header row: collapse the message.
    Header,
    /// Expand or fold the header (Shift-H's): its participants, or its chevron, after which the
    /// shortcut is suggested (`hint`).
    HeaderToggle { hint: bool },
    /// The attachment cards or their `Download all`.
    Files(attachments::Click),
    /// A reply button: Reply, Reply all or Forward.
    Reply(exo_store::DraftAction),
    /// A failed or waiting send's status.
    Recover,
}

/// The participants' names a header drew, and where: each has a tooltip and shows its contact
/// once the pointer settles on it.
pub(super) type Named = Vec<(Rect, Contact)>;

/// Whether the card shows as marketing mail: its new part is, or its quoted part
/// is and it's shown.
pub(super) fn marketing(facts: &Facts, show_quoted: bool) -> bool {
    facts.unquoted_marketing || (show_quoted && facts.quoted_marketing)
}

/// The full header's rows: From, To, Cc, Bcc (the empty ones left out), and the date.
fn header_rows(message: &MessageDetail) -> usize {
    1 + [
        !message.to.is_empty(),
        !message.cc.is_empty(),
        !message.bcc.is_empty(),
    ]
    .iter()
    .filter(|row| **row)
    .count()
        + 1
}

/// How far below the card's top the body starts.
pub(super) fn body_offset(header_expanded: bool, message: &MessageDetail) -> f32 {
    let header = if header_expanded {
        header_rows(message) as f32 * HEADER_ROW - 4.0
    } else {
        FROM.line_height
    };
    PADDING_TOP + header + BODY_MARGIN_TOP
}

/// An expanded card's height for a body `body` px tall (unknown yet: the placeholder's) under an
/// invite card `invite` px tall (with its margin).
pub(super) fn expanded_height(
    header_expanded: bool,
    message: Option<&MessageDetail>,
    invite: f32,
    body: Option<f32>,
    inner_width: f32,
) -> f32 {
    let Some(message) = message else {
        return COLLAPSED_HEIGHT;
    };
    let attachments = attachments::height(attachments::listed(message).count(), inner_width);
    body_offset(header_expanded, message)
        + invite
        + body.unwrap_or(PLACEHOLDER_HEIGHT)
        + attachments
        + BODY_MARGIN_BOTTOM
        + PADDING_BOTTOM
}

/// What an expanded card's header and focus bar are drawn in.
///
/// A marketing card in Carbon is white at 90% opacity and keeps Snow's dark ink. The card's
/// opacity applies to everything in it, so each color is composited over white and then at 90%
/// over the pane.
struct Ink {
    /// The participants.
    names: Color32,
    /// The date.
    date: Color32,
    /// Shift-H's labels, addresses and date.
    label: Color32,
    /// Shift-H's names.
    contact: Color32,
    /// The reply buttons and the expand chevron at rest.
    icon: Color32,
    /// The reply buttons and the expand chevron, hovered.
    icon_hover: Color32,
    /// The collapse chevron, which hovering doesn't change.
    collapse: Color32,
    /// A reply button's label, at rest and hovered.
    action_label: Color32,
    action_label_hover: Color32,
    /// The focus bar.
    focus: Color32,
}

impl Ink {
    fn new(ctx: &egui::Context, marketing: bool) -> Self {
        let palette = exo_ui::palette(ctx);
        let dark = exo_ui::theme(ctx) == Theme::Carbon;
        if dark && marketing {
            let on_card = |color: Color32| {
                let [r, g, b, a] = color.to_array();
                let over_white = |channel: u8| channel.saturating_add(255 - a);
                exo_ui::motion::mix(
                    palette.background_base_default,
                    Color32::from_rgb(over_white(r), over_white(g), over_white(b)),
                    0.9,
                )
            };
            // In Carbon a marketing card keeps Snow's date, label and name colours.
            Self {
                names: on_card(Color32::from_black_alpha(204)),
                date: on_card(Color32::from_black_alpha(128)),
                label: on_card(Color32::from_black_alpha(128)),
                contact: on_card(Color32::from_black_alpha(204)),
                icon: on_card(Color32::from_black_alpha(89)),
                icon_hover: on_card(Color32::from_black_alpha(125)),
                collapse: on_card(Color32::from_black_alpha(125)),
                // Its reply buttons' labels.
                action_label: on_card(Color32::from_black_alpha(153)),
                action_label_hover: on_card(Color32::from_black_alpha(204)),
                focus: on_card(FOCUS),
            }
        } else {
            // The full header's labels at `rgba(0,0,0,.5)`, its names at `.8` (white in Carbon).
            let (label, contact) = if dark {
                (
                    Color32::from_white_alpha(128),
                    Color32::from_white_alpha(204),
                )
            } else {
                (
                    Color32::from_black_alpha(128),
                    Color32::from_black_alpha(204),
                )
            };
            Self {
                // The participants.
                names: palette.ink(0.8, 0.8),
                date: palette.text_base_subdued,
                label,
                contact,
                // Icons filled at .7: the reply buttons and the expand chevron at opacity .5, .7
                // hovered, the collapse chevron at .7 either way.
                icon: palette.ink(0.35, 0.35),
                icon_hover: palette.ink(0.49, 0.49),
                collapse: palette.ink(0.49, 0.49),
                // Reply buttons' labels at `rgba(0,0,0,.6)`, `.8` hovered.
                action_label: palette.ink(0.6, 0.6),
                action_label_hover: palette.ink(0.8, 0.8),
                focus: FOCUS,
            }
        }
    }
}

/// Which of a message's boxes a card is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CardKind {
    /// A collapsed row, hovered or focused.
    Collapsed,
    /// An expanded card.
    Expanded,
    /// An expanded card that holds a draft's form.
    Composing,
}

/// The card's background and shadow. Beside a popped-out draft, with
/// `layout`'s, the wide shadows' blurs are `2rem`.
pub(super) fn card(ui: &Ui, rect: Rect, layout: &Layout, kind: CardKind, marketing: bool) {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter();
    let rect = exo_ui::pixel_snap(rect);
    let radius = CornerRadius::same(RADIUS);
    let dark = exo_ui::theme(ui.ctx()) == Theme::Carbon;
    let black = Color32::from_black_alpha;
    let wide = |px: f32| {
        if layout.popped_out() {
            2.0 * layout.rem()
        } else {
            px
        }
    };
    let shadows: [Option<BoxShadow>; 3] = match (kind, dark) {
        // `0 1px 4px rgba(0,0,0,.2), 0 0 .5px 0 rgba(0,0,0,.1)`.
        (CardKind::Collapsed, false) => [
            Some(BoxShadow::new([0.0, 1.0], 4.0, 0.0, black(51))),
            Some(BoxShadow::new([0.0, 0.0], 0.5, 0.0, black(26))),
            None,
        ],
        // `0 2px 20px 0 rgba(0,0,0,.2)` and the inset highlight below.
        (CardKind::Collapsed, true) => [
            Some(BoxShadow::new([0.0, 2.0], 20.0, 0.0, black(51))),
            None,
            None,
        ],
        // `0 2px 8px 0 rgba(0,0,0,.2), 0 0 .5px 0 rgba(0,0,0,.1)`.
        (CardKind::Expanded, false) => [
            Some(BoxShadow::new([0.0, 2.0], 8.0, 0.0, black(51))),
            Some(BoxShadow::new([0.0, 0.0], 0.5, 0.0, black(26))),
            None,
        ],
        // `0 0 20px 0 rgba(0,0,0,.03), 0 6px 32px 0 rgba(0,0,0,.1)`.
        (CardKind::Expanded, true) => [
            Some(BoxShadow::new([0.0, 0.0], 20.0, 0.0, black(8))),
            Some(BoxShadow::new([0.0, 6.0], wide(32.0), 0.0, black(26))),
            None,
        ],
        // `0 6px 15px 0 rgba(0,0,0,.1), 0 16px 45px rgba(0,0,0,.05), 0 0 .5px rgba(0,0,0,.1)`.
        (CardKind::Composing, false) => [
            Some(BoxShadow::new([0.0, 6.0], 15.0, 0.0, black(26))),
            Some(BoxShadow::new([0.0, 16.0], wide(45.0), 0.0, black(13))),
            Some(BoxShadow::new([0.0, 0.0], 0.5, 0.0, black(26))),
        ],
        // `0 0 12px 0 rgba(0,0,0,.03), 0 12px 24px 0 rgba(0,0,0,.1)`.
        (CardKind::Composing, true) => [
            Some(BoxShadow::new([0.0, 0.0], 12.0, 0.0, black(8))),
            Some(BoxShadow::new([0.0, 12.0], wide(24.0), 0.0, black(26))),
            None,
        ],
    };
    for shadow in shadows.iter().flatten() {
        shadow.paint(painter, rect, radius);
    }
    // A marketing card in Carbon: white at 90% opacity.
    let background = if marketing && dark {
        Color32::from_white_alpha(230)
    } else {
        palette.background_elevated_default
    };
    painter.rect_filled(rect, radius, background);
    if dark && !marketing {
        exo_ui::paint_carbon_highlight(painter, rect, radius, background);
    }
}

/// The focused card's bar: 3px of `rgb(190.2, 192.6, 227.8)`.
const FOCUS: Color32 = Color32::from_rgb(190, 193, 228);

/// The focus bar down the card's left edge.
fn focus_bar(ui: &Ui, rect: Rect, color: Color32) {
    let bar = exo_ui::pixel_snap(Rect::from_min_size(rect.min, vec2(3.0, rect.height())));
    ui.painter().rect_filled(
        bar,
        CornerRadius {
            nw: RADIUS,
            sw: RADIUS,
            ne: 0,
            se: 0,
        },
        color,
    );
}

/// A contact as message headers name them: the first name, `Me`,
/// or the lower-cased address.
fn name(contact: &Contact) -> String {
    named(contact, NameFormat::First)
}

/// [`name`] in `format`.
fn named(contact: &Contact, format: NameFormat) -> String {
    names::display_name(
        &Person::new(
            contact.name.clone().unwrap_or_default(),
            contact.email.clone(),
        ),
        contact.is_me,
        format,
        false,
    )
}

/// The people on the thread who aren't you.
fn others(thread: &ThreadDetail) -> usize {
    let mut seen: Vec<String> = Vec::new();
    for message in thread.messages.iter().filter(|message| !message.is_draft()) {
        for contact in message.from.iter().chain(&message.to).chain(&message.cc) {
            let email = contact.email.to_lowercase();
            if !contact.is_me && !seen.contains(&email) {
                seen.push(email);
            }
        }
    }
    seen.len()
}

/// The participants: the sender, and in an expanded message ` to ` its recipients when the thread
/// has more than one other person in it (or is all yours). With `full_names`
/// (`showSenderFullNames`) the sender is named in full, unless it's you; the recipients keep their
/// first names.
pub(super) fn participants(
    message: &MessageDetail,
    thread: &ThreadDetail,
    expanded: bool,
    full_names: bool,
) -> String {
    participant_pieces(message, thread, expanded, full_names)
        .into_iter()
        .map(|(text, _)| text)
        .collect()
}

/// [`participants`] in pieces: each name with its contact, and what's between them.
fn participant_pieces<'a>(
    message: &'a MessageDetail,
    thread: &ThreadDetail,
    expanded: bool,
    full_names: bool,
) -> Vec<(String, Option<&'a Contact>)> {
    let recipients: Vec<&Contact> = message.to.iter().chain(&message.cc).collect();
    let all_mine = thread
        .messages
        .iter()
        .filter(|message| !message.is_draft())
        .all(|message| message.from.as_ref().is_some_and(|from| from.is_me));
    let sender = message.from.as_ref();
    let everyone_me = sender.is_none_or(|from| from.is_me)
        && recipients
            .iter()
            .chain(&message.bcc.iter().collect::<Vec<_>>())
            .all(|contact| contact.is_me);
    if expanded && everyone_me {
        return vec![(
            if message.bcc.is_empty() {
                "Note to self".into()
            } else {
                "Me to Bcc: Me".into()
            },
            None,
        )];
    }
    let from = sender
        .map(|from| {
            named(
                from,
                if full_names {
                    NameFormat::Full
                } else {
                    NameFormat::First
                },
            )
        })
        .unwrap_or_default();
    let mut pieces = vec![(from, sender)];
    let show_recipients = expanded && !recipients.is_empty() && (others(thread) > 1 || all_mine);
    if !show_recipients {
        return pieces;
    }
    pieces.push((" to ".into(), None));
    join_pieces(&mut pieces, &recipients);
    // Whoever sent it: a message you got as Bcc says so too.
    if !message.bcc.is_empty() {
        pieces.push((". Bcc: ".into(), None));
        join_pieces(&mut pieces, &message.bcc.iter().collect::<Vec<_>>());
    }
    pieces
}

/// `Intersperse`'s pieces: each contact's name, `, ` between and ` & ` before the last.
fn join_pieces<'a>(pieces: &mut Vec<(String, Option<&'a Contact>)>, contacts: &[&'a Contact]) {
    for (at, contact) in contacts.iter().enumerate() {
        if at > 0 {
            pieces.push((
                if at + 1 == contacts.len() {
                    " & "
                } else {
                    ", "
                }
                .into(),
                None,
            ));
        }
        pieces.push((name(contact), Some(contact)));
    }
}

/// The condensed participants a header with reply labels shows when it's 500px wide or less:
/// `{sender} to {lead} +{n}`, the lead being you when you're among the recipients (Bcc included),
/// else the first of them.
struct Condensed<'a> {
    sender: (String, Option<&'a Contact>),
    lead: &'a Contact,
    more: usize,
}

/// The condensed participants, where the full list names recipients (`Note to self` doesn't).
fn condensed<'a>(
    message: &'a MessageDetail,
    thread: &ThreadDetail,
    full_names: bool,
) -> Option<Condensed<'a>> {
    let mut pieces = participant_pieces(message, thread, true, full_names);
    if pieces.len() < 3 {
        return None;
    }
    let recipients: Vec<&Contact> = message
        .to
        .iter()
        .chain(&message.cc)
        .chain(&message.bcc)
        .collect();
    let lead = recipients
        .iter()
        .find(|contact| contact.is_me)
        .or(recipients.first())?;
    Some(Condensed {
        sender: pieces.swap_remove(0),
        lead,
        more: recipients.len() - 1,
    })
}

impl Condensed<'_> {
    /// Paints the condensed list on the line at `at` within `room`, its names cut with an
    /// ellipsis as [`condense`] shrinks them. Adds where each name went to `named` and returns the
    /// list's width.
    fn paint(
        &self,
        painter: &egui::Painter,
        at: egui::Pos2,
        color: Color32,
        room: f32,
        named: &mut Named,
    ) -> f32 {
        let width = |text: &str| {
            FROM.single_line(painter, text, Color32::PLACEHOLDER, None)
                .size()
                .x
        };
        let lead_text = name(self.lead);
        let count = (self.more > 0).then(|| format!("+{}", self.more));
        let (sender_me, lead_me) = (
            self.sender.1.is_some_and(|contact| contact.is_me),
            self.lead.is_me,
        );
        let to = width(" to ");
        let count_width = count
            .as_deref()
            .map_or(0.0, |count| CONDENSED_COUNT_GAP + width(count));
        let (sender, lead) = condense(
            width(&self.sender.0),
            width(&lead_text),
            to + count_width,
            room,
            sender_me,
            lead_me,
        );
        let row = |x: f32, w: f32| {
            Rect::from_min_max(pos2(at.x + x, at.y - 1.0), pos2(at.x + x + w, at.y + 16.0))
        };
        FROM.paint_line(painter, at, &self.sender.0, color, Some(sender));
        if let Some(contact) = self.sender.1 {
            named.push((row(0.0, sender), contact.clone()));
        }
        FROM.paint_line(painter, at + vec2(sender, 0.0), " to ", color, None);
        FROM.paint_line(
            painter,
            at + vec2(sender + to, 0.0),
            &lead_text,
            color,
            Some(lead),
        );
        named.push((row(sender + to, lead), self.lead.clone()));
        if let Some(count) = &count {
            FROM.paint_line(
                painter,
                at + vec2(sender + to + lead + CONDENSED_COUNT_GAP, 0.0),
                count,
                color,
                None,
            );
        }
        sender + to + lead + count_width
    }
}

/// The condensed list's flex row: the widths the sender and
/// the lead get, `sender` and `lead` px wide, beside `rest` px that don't shrink, in `room`. The
/// sender shrinks almost alone (`flex: 0 1000 auto`, weighted by its width), down to 18px, then
/// the lead (at most half the room) down to 42px; yours never shrink, nor are capped.
fn condense(
    sender: f32,
    lead: f32,
    rest: f32,
    room: f32,
    sender_me: bool,
    lead_me: bool,
) -> (f32, f32) {
    let mut sender = sender;
    let mut lead = if lead_me { lead } else { lead.min(0.5 * room) };
    let overflow = sender + lead + rest - room;
    let (sender_weight, lead_weight) = (
        if sender_me { 0.0 } else { 1000.0 * sender },
        if lead_me { 0.0 } else { lead },
    );
    if overflow > 0.0 && sender_weight + lead_weight > 0.0 {
        let total = sender_weight + lead_weight;
        let mut from_sender = overflow * sender_weight / total;
        let mut from_lead = overflow * lead_weight / total;
        // A share that would take the sender under its least width goes to the lead.
        let short = CONDENSED_SENDER_MIN - (sender - from_sender);
        if !sender_me && short > 0.0 {
            from_sender -= short;
            if !lead_me {
                from_lead += short;
            }
        }
        sender -= from_sender.max(0.0);
        lead -= from_lead;
    }
    let sender = if sender_me {
        sender
    } else {
        sender.max(CONDENSED_SENDER_MIN)
    };
    let lead = if lead_me {
        lead
    } else {
        lead.max(CONDENSED_LEAD_MIN)
    };
    (sender, lead)
}

/// A reply button's tooltip: its key, and with ⇧ to pop the draft out.
fn reply_tooltip(action: exo_store::DraftAction) -> [KeyTooltip<'static>; 2] {
    match action {
        exo_store::DraftAction::Forward => [
            KeyTooltip {
                text: "Forward",
                keys: &["F"],
            },
            KeyTooltip {
                text: "Forward, Pop Out Draft",
                keys: &["shift", "F"],
            },
        ],
        exo_store::DraftAction::ReplyAll => [
            KeyTooltip {
                text: "Reply All",
                keys: &["enter"],
            },
            KeyTooltip {
                text: "Reply All, Pop Out Draft",
                keys: &["shift", "enter"],
            },
        ],
        _ => [
            KeyTooltip {
                text: "Reply",
                keys: &["R"],
            },
            KeyTooltip {
                text: "Reply, Pop Out Draft",
                keys: &["shift", "R"],
            },
        ],
    }
}

/// Paints the participants' `pieces` on the line at `at`, cut at `room`, and adds where each
/// name went to `named` (the 17px of its text, from 1px above the line).
fn paint_participants(
    painter: &egui::Painter,
    at: egui::Pos2,
    pieces: &[(String, Option<&Contact>)],
    color: Color32,
    room: f32,
    named: &mut Named,
) {
    let text: String = pieces.iter().map(|(text, _)| text.as_str()).collect();
    FROM.paint_line(painter, at, &text, color, Some(room));
    let mut x = 0.0;
    let mut prefix = String::new();
    for (piece, contact) in pieces {
        prefix.push_str(piece);
        let end = FROM
            .single_line(painter, &prefix, Color32::PLACEHOLDER, None)
            .size()
            .x;
        if let Some(contact) = contact.filter(|_| x < room) {
            named.push((
                Rect::from_min_max(
                    pos2(at.x + x, at.y - 1.0),
                    pos2(at.x + end.min(room), at.y + 16.0),
                ),
                (*contact).clone(),
            ));
        }
        x = end;
    }
}

/// Where a date's hover target is: its text's 15px on the line, from 1px down.
fn date_target(left: f32, width: f32, line: f32) -> Rect {
    Rect::from_min_size(pos2(left, line + 1.0), vec2(width, 15.0))
}

/// Whether a reply all would copy someone besides the sender.
fn has_reply_all(message: &MessageDetail) -> bool {
    let mut people: Vec<String> = Vec::new();
    for contact in message.from.iter().chain(&message.to).chain(&message.cc) {
        let email = contact.email.to_lowercase();
        if !contact.is_me && !people.contains(&email) {
            people.push(email);
        }
    }
    people.len() > 1
}

/// The snippet up to a signature line or a quote header.
fn clean_snippet(text: &str) -> &str {
    use std::sync::LazyLock;
    static CUTS: LazyLock<[regex::Regex; 4]> = LazyLock::new(|| {
        [r"[^a-zA-Z0-9]+You are receiving this", r"Sent (using|with|from|via) ", r"On ((Mon|Tue|Wed|Thu|Fri|Sat|Sun), )?(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)", r"\s?\{#HS:[\d-]+#\}\s?"].map(|pattern| regex::Regex::new(pattern).expect("valid regex"))
    });
    let mut text = text
        .split("----")
        .next()
        .unwrap_or_default()
        .split("____")
        .next()
        .unwrap_or_default();
    for cut in CUTS.iter() {
        if let Some(found) = cut.find(text) {
            text = &text[..found.start()];
        }
    }
    text
}

/// A collapsed row.
#[expect(
    clippy::too_many_arguments,
    reason = "one row, drawn from the thread's state"
)]
pub(super) fn collapsed(
    ui: &Ui,
    rect: Rect,
    layout: &Layout,
    message: &MessageDetail,
    thread: &ThreadDetail,
    facts: Option<&Facts>,
    state: &State,
    now: &Zoned,
    hovered: bool,
    named: &mut Named,
) {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter();
    let focused = state.focused && state.show_focus;
    if hovered || focused {
        card(ui, rect, layout, CardKind::Collapsed, false);
    }
    if focused {
        focus_bar(ui, rect, FOCUS);
    }
    let (left, right) = (layout.inner.left(), layout.inner.right());
    let line = rect.top() + PADDING_TOP;

    let pieces = participant_pieces(message, thread, false, state.full_names);
    let from: String = pieces.iter().map(|(text, _)| text.as_str()).collect();
    let from_width = FROM
        .single_line(painter, &from, Color32::PLACEHOLDER, None)
        .size()
        .x;
    let from_box = (from_width + FROM_PADDING).max(FROM_MIN_WIDTH);
    // The sender and snippet: `rgba(0, 0, 0, .6)`.
    paint_participants(
        painter,
        pos2(left, line),
        &pieces,
        palette.ink(0.6, 0.6),
        from_box - FROM_PADDING,
        named,
    );

    // A failed or waiting send says so; `top: 1px`, the
    // date's trailing letter-spacing inside the right-aligned box.
    let date_left = match Status::of(message, state) {
        Some(status) => {
            let title = status.collapsed_title(now);
            let left = right
                - STATUS
                    .single_line(painter, &title, Color32::PLACEHOLDER, None)
                    .size()
                    .x;
            STATUS.paint_line(painter, pos2(left, line + 1.0), &title, status.ink(), None);
            left
        }
        None => {
            let date = dates::row(message.date, now);
            let date_width = DATE
                .single_line(painter, &date, Color32::PLACEHOLDER, None)
                .size()
                .x;
            let left = right - date_width - DATE.letter_spacing;
            DATE.paint_line(
                painter,
                pos2(left, line + 1.0),
                &date,
                palette.text_base_subdued,
                None,
            );
            super::markers::date_tooltip(
                ui,
                layout,
                date_target(left, date_width + DATE.letter_spacing, line),
                message.date,
                now,
                ui.id().with(("date", &message.id)),
            );
            left
        }
    };

    // The snippet: `flex: 1 1 0; margin-right: .5rem`, cut by [`snippet`].
    let snippet_left = left + from_box;
    let snippet_right = date_left - 0.5 * layout.rem;
    let available = (snippet_right - snippet_left).max(0.0);
    let text = snippet(facts, available, |text| {
        SNIPPET
            .single_line(painter, text, Color32::PLACEHOLDER, None)
            .size()
            .x
            > available
    });
    SNIPPET.paint_line(
        painter,
        pos2(snippet_left, line),
        &text,
        palette.ink(0.6, 0.6),
        Some(available),
    );
}

/// A collapsed row's snippet: cleaned, its whitespace collapsed, cut at a word with ` …` past 200
/// characters or when `overflows` says it's wider than `available` px. The cut is estimated from
/// the text's measured width, then an ellipsis trims what still overflows.
pub(super) fn snippet(
    facts: Option<&Facts>,
    available: f32,
    overflows: impl FnOnce(&str) -> bool,
) -> String {
    let text = facts
        .map(|facts| measure::normalize_whitespace(clean_snippet(&facts.snippet)))
        .unwrap_or_default();
    if measure::utf16_len(&text) >= 200 {
        format!("{} …", measure::truncate(&text, 200, true))
    } else if overflows(&text) {
        let fits = measure::max_characters(&text, f64::from(available) - measure::width(" …"));
        format!("{} …", measure::truncate(&text, fits, true))
    } else {
        text
    }
}

/// An expanded card (`marketing` for a marketing email's): header, body,
/// attachments. Returns what was clicked besides the body, which the caller hit-tests.
#[expect(
    clippy::too_many_arguments,
    reason = "one card, drawn from the thread's state"
)]
pub(super) fn expanded(
    ui: &Ui,
    rect: Rect,
    layout: &Layout,
    message: &MessageDetail,
    thread: &ThreadDetail,
    marketing: bool,
    state: &State,
    look: Look,
    bodies: &mut Bodies,
    files: &attachments::Files,
    invite: f32,
    now: &Zoned,
    hovered: bool,
    named: &mut Named,
    selection: &mut exo_ui::TextSelection,
) -> Option<Hit> {
    card(
        ui,
        rect,
        layout,
        if state.composing {
            CardKind::Composing
        } else {
            CardKind::Expanded
        },
        marketing,
    );
    let ink = Ink::new(ui.ctx(), marketing);
    if state.focused && state.show_focus {
        focus_bar(ui, rect, ink.focus);
    }
    let id = ui.id().with(("message-header", &message.id));
    let mut hit = if state.header_expanded {
        full_header(
            ui,
            rect,
            layout,
            message,
            now,
            Status::of(message, state),
            bodies.tracking_blocked(&message.id),
            &ink,
            selection,
        )
    } else {
        compact_header(
            ui, rect, layout, message, thread, state, now, hovered, id, &ink, named,
        )
    };

    let body_top = rect.top() + body_offset(state.header_expanded, message) + invite;
    let origin = pos2(layout.inner.left(), body_top);
    match bodies.height(&message.id, look) {
        Some(height) => {
            // The body clips what overflows it (it scrolls sideways); the scissor lands on
            // device pixels, as Chromium snaps clips.
            let clip = ui
                .clip_rect()
                .intersect(Rect::from_min_size(origin, vec2(look.width, height)));
            let opacity = if marketing && exo_ui::theme(ui.ctx()) == Theme::Carbon {
                // The card's 90% applies to the body as one group with the card, so the body
                // shows the pane through it, not the card: lay the pane back down under it (on
                // the device row its bands start on; they cover the column's width).
                let ppp = ui.ctx().pixels_per_point();
                let under = Rect::from_min_size(
                    pos2(origin.x, (origin.y * ppp).round() / ppp),
                    vec2(look.width, height),
                );
                ui.painter().with_clip_rect(clip).rect_filled(
                    under,
                    0.0,
                    exo_ui::palette(ui.ctx()).background_base_default,
                );
                0.9
            } else {
                1.0
            };
            bodies.paint(ui.painter(), &message.id, origin, clip, opacity);
            bodies.add_matches(ui.ctx(), &message.id, origin);
            let list_top = body_top + height;
            hit = hit
                .or(attachments::show(ui, layout.inner, list_top, message, files).map(Hit::Files));
        }
        None => placeholder(ui, origin, look.width),
    }
    hit
}

/// Three shimmer bars while a body is laid out (8px, `margin: 8px 0`).
fn placeholder(ui: &Ui, origin: egui::Pos2, width: f32) {
    for line in 0..3 {
        let bar = Rect::from_min_size(
            origin + vec2(0.0, 8.0 + 24.0 * line as f32),
            vec2(width, 8.0),
        );
        ui.painter()
            .rect_filled(bar, 0.0, Color32::from_black_alpha(13));
    }
}

/// The compact header: participants on the left; on the right the reply buttons (on the focused
/// message, or while hovered) and the date. A click on the participants expands the header, as
/// Shift-H does; anywhere else on the row collapses the message.
#[expect(
    clippy::too_many_arguments,
    reason = "one header row, drawn from the card's state"
)]
fn compact_header(
    ui: &Ui,
    rect: Rect,
    layout: &Layout,
    message: &MessageDetail,
    thread: &ThreadDetail,
    state: &State,
    now: &Zoned,
    hovered: bool,
    id: egui::Id,
    ink: &Ink,
    named: &mut Named,
) -> Option<Hit> {
    let painter = ui.painter();
    let (left, right) = (layout.inner.left(), layout.inner.right());
    let line = rect.top() + PADDING_TOP;
    if let Some(status) = Status::of(message, state) {
        return status_header(
            ui,
            layout,
            rect,
            message,
            thread,
            status,
            now,
            id,
            ink,
            state.full_names,
        );
    }

    let date = dates::row(message.date, now);
    let date_width = DATE
        .single_line(painter, &date, Color32::PLACEHOLDER, None)
        .size()
        .x;
    let date_left = right - date_width - DATE.letter_spacing;
    DATE.paint_line(painter, pos2(date_left, line + 1.0), &date, ink.date, None);
    let date_box = date_left - DATE_PADDING;
    // The tooltip's target is the `time`, its padding included.
    super::markers::date_tooltip(
        ui,
        layout,
        date_target(date_box, right - date_box, line),
        message.date,
        now,
        id.with("date"),
    );

    // Reply, Reply all (when there's someone to copy), Forward, right to left from the date.
    use exo_store::DraftAction;
    let mut actions: Vec<(&str, &str, DraftAction)> = vec![(
        "legacy/DEPRECATED_forwardMessage",
        "Forward",
        DraftAction::Forward,
    )];
    if has_reply_all(message) {
        actions.push((
            "legacy/DEPRECATED_replyAllMessage",
            "Reply all",
            DraftAction::ReplyAll,
        ));
    }
    actions.push((
        "legacy/DEPRECATED_replyMessage",
        "Reply",
        DraftAction::Reply,
    ));
    let mut replied = None;
    let from_right = if state.reply_labels {
        // With labels, always shown: the glyph and its label 4px apart in Classic's `padding:
        // 10px 8px`, each button's margins (`-9px` right, `8px` left) overlapping the next by 1px,
        // the forward's right edge 8px into the date's padding; no transitions.
        let mut right = date_box + LABEL_MARGIN_LEFT;
        for (k, (icon, label, action)) in actions.iter().enumerate() {
            let label_width = DATE
                .single_line(painter, label, Color32::PLACEHOLDER, None)
                .size()
                .x
                + DATE.letter_spacing;
            let width = LABEL_PADDING.x + ACTION_SIZE.x + LABEL_GAP + label_width + LABEL_PADDING.x;
            let button = Rect::from_min_max(
                pos2(right - width, line + 1.0 - LABEL_PADDING.y),
                pos2(right, line + 1.0 + DATE.line_height + LABEL_PADDING.y),
            );
            let icon_rect = Rect::from_min_size(
                pos2(
                    button.left() + LABEL_PADDING.x,
                    button.center().y - ACTION_SIZE.y / 2.0,
                ),
                ACTION_SIZE,
            );
            let response = IconButton::new(icon).colors(ink.icon, ink.icon_hover).show(
                ui,
                id.with(("action", k)),
                button,
                icon_rect,
            );
            let color = if response.hovered() {
                ink.action_label_hover
            } else {
                ink.action_label
            };
            DATE.paint_line(
                painter,
                pos2(icon_rect.right() + LABEL_GAP, line + 1.0),
                label,
                color,
                None,
            );
            ShortcutTooltip {
                rows: &reply_tooltip(*action),
                note: None,
            }
            .show_at(
                ui,
                &response,
                Placement::Below {
                    gap: 8.0,
                    shift: 0.0,
                },
            );
            if response.clicked() {
                replied = Some(Hit::Reply(*action));
            }
            right = button.left() + 1.0;
        }
        right - 1.0 - LABEL_MARGIN_LEFT
    } else {
        let shown = state.focused || hovered;
        let opacity = exo_ui::motion::transition(
            ui.ctx(),
            id.with("actions"),
            shown,
            0.2,
            exo_ui::motion::CubicBezier::LINEAR,
        );
        if shown || opacity > 0.0 {
            for (k, (icon, _, action)) in actions.iter().enumerate() {
                let at = pos2(
                    date_box - ACTION_SIZE.x - k as f32 * ACTION_PITCH,
                    line + 3.0,
                );
                let icon_rect = Rect::from_min_size(at, ACTION_SIZE);
                let hit_rect = icon_rect.expand(9.0);
                let response = IconButton::new(icon)
                    .colors(
                        ink.icon.gamma_multiply(opacity),
                        ink.icon_hover.gamma_multiply(opacity),
                    )
                    .show(ui, id.with(("action", k)), hit_rect, icon_rect);
                ShortcutTooltip {
                    rows: &reply_tooltip(*action),
                    note: None,
                }
                .show_at(
                    ui,
                    &response,
                    Placement::Below {
                        gap: 8.0,
                        shift: 0.0,
                    },
                );
                if response.clicked() {
                    replied = Some(Hit::Reply(*action));
                }
            }
            date_box - ACTION_SIZE.x - (actions.len() - 1) as f32 * ACTION_PITCH
        } else {
            date_box
        }
    };

    // With labels, a header 500px wide or less names one recipient and counts the rest, and the
    // Shift-H chevron after the names always takes its room: 4px after them, 26px of clickable
    // (8px inside it), 12px after; condensed 18px (4px inside), 4px after.
    let condensed = condensed(message, thread, state.full_names)
        .filter(|_| state.reply_labels && layout.inner.width() <= CONDENSED_WIDTH);
    let (chevron_inset, chevron_after) = match (state.reply_labels, condensed.is_some()) {
        (false, _) => (9.0, 0.0),
        (true, false) => (8.0, 12.0),
        (true, true) => (4.0, 4.0),
    };
    let chevron_room = if state.reply_labels {
        CHEVRON_GAP + 2.0 * chevron_inset + CHEVRON.x + chevron_after
    } else {
        0.0
    };
    let pieces = participant_pieces(message, thread, true, state.full_names);
    let text: String = pieces.iter().map(|(text, _)| text.as_str()).collect();
    let room = (from_right - left - chevron_room).max(0.0);
    let names = match &condensed {
        Some(condensed) => condensed.paint(painter, pos2(left, line), ink.names, room, named),
        None => {
            paint_participants(painter, pos2(left, line), &pieces, ink.names, room, named);
            FROM.single_line(painter, &text, Color32::PLACEHOLDER, None)
                .size()
                .x
                .min(room)
        }
    };
    // A pointer cursor over the row, and on it the participants' label.
    let row = ui
        .interact(
            Rect::from_min_max(pos2(left, line), pos2(from_right, line + FROM.line_height)),
            id.with("row"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let names_rect = Rect::from_min_size(pos2(left, line), vec2(names, FROM.line_height));
    let label = ui
        .interact(names_rect, id.with("names"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);

    // The Shift-H chevron after the names: without labels it takes room and clicks while the
    // message is hovered; it fades in while the names or the chevron are (`opacity .2s linear`).
    let mut hit = None;
    // Level with the line's top: the 10×12 chevron, in a clickable
    // 36px tall with labels (`padding: 10px`, `margin: -10px`).
    let chevron = Rect::from_min_size(
        pos2(left + names + CHEVRON_GAP + chevron_inset, line),
        CHEVRON,
    );
    let clickable = if state.reply_labels {
        Rect::from_min_max(
            pos2(chevron.left() - chevron_inset, line - 10.0),
            pos2(
                chevron.right() + chevron_inset,
                line + FROM.line_height + 10.0,
            ),
        )
    } else {
        chevron.expand(9.0)
    };
    let lit =
        hovered && (ui.rect_contains_pointer(names_rect) || ui.rect_contains_pointer(clickable));
    let chevron_opacity = exo_ui::motion::transition(
        ui.ctx(),
        id.with("expand"),
        lit,
        0.2,
        exo_ui::motion::CubicBezier::LINEAR,
    );
    if hovered || state.reply_labels || chevron_opacity > 0.0 {
        let (rest, hover) = (
            ink.icon.gamma_multiply(chevron_opacity),
            ink.icon_hover.gamma_multiply(chevron_opacity),
        );
        let response = IconButton::new("legacy/DEPRECATED_expand")
            .colors(rest, hover)
            .show(ui, id.with("expand"), clickable, chevron);
        // 8px under the icon, not its clickable.
        KeyTooltip {
            text: "Expand Message Header",
            keys: &["shift", "H"],
        }
        .show_at(
            ui,
            &response,
            Placement::Below {
                gap: 8.0 - (clickable.bottom() - chevron.bottom()),
                shift: 0.0,
            },
        );
        if response.clicked() {
            hit = Some(Hit::HeaderToggle { hint: true });
        }
    }
    if hit.is_none() && label.clicked() {
        hit = Some(Hit::HeaderToggle { hint: false });
    }
    if hit.is_none() && row.clicked() {
        hit = Some(Hit::Header);
    }
    replied.or(hit)
}

/// Where `text` sits in the date's place of the compact header of the card at `card`.
fn status_slot(ui: &Ui, layout: &Layout, card: Rect, text: &str) -> Rect {
    let width = STATUS
        .single_line(ui.painter(), text, Color32::PLACEHOLDER, None)
        .size()
        .x;
    Rect::from_min_size(
        pos2(layout.inner.right() - width, card.top() + PADDING_TOP + 1.0),
        vec2(width, STATUS.line_height),
    )
}

/// The compact header of a failed or waiting send: the participants, and in the date's place its
/// status, which opens `RecoverDraft`. No reply buttons. A failed one's tooltip is
/// [`failed_tooltip`]'s.
#[expect(
    clippy::too_many_arguments,
    reason = "one header row, drawn from the card's state"
)]
fn status_header(
    ui: &Ui,
    layout: &Layout,
    card: Rect,
    message: &MessageDetail,
    thread: &ThreadDetail,
    status: Status,
    now: &Zoned,
    id: egui::Id,
    ink: &Ink,
    full_names: bool,
) -> Option<Hit> {
    let painter = ui.painter();
    let (left, line) = (layout.inner.left(), card.top() + PADDING_TOP);
    let title = status.title(now);
    let slot = status_slot(ui, layout, card, &title);
    STATUS.paint_line(painter, slot.min, &title, status.ink(), None);
    let recover = ui
        .interact(slot, id.with("status"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if let Status::Scheduled(_) = status {
        // Under the date, centred, past the tooltip's 8px.
        let row = KeyTooltip {
            text: SCHEDULED_TOOLTIP,
            keys: &[],
        };
        ShortcutTooltip {
            rows: std::slice::from_ref(&row),
            note: None,
        }
        .show_below(ui, &recover, 8.0, false, layout.column.x_range());
    }
    let text = participants(message, thread, true, full_names);
    let room = (slot.left() - left).max(0.0);
    FROM.paint_line(painter, pos2(left, line), &text, ink.names, Some(room));
    let row = ui
        .interact(
            Rect::from_min_max(pos2(left, line), pos2(slot.left(), line + FROM.line_height)),
            id.with("row"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if recover.clicked() {
        Some(Hit::Recover)
    } else {
        row.clicked().then_some(Hit::Header)
    }
}

/// The tooltip under the reason in failed draft `message`'s compact header (card `card`), which
/// stays open, painted with `painter`: over the messages and under everything above them. Under
/// the title's span (its 15px content area ends 1px above the line box), past the tooltip's 8px,
/// pinned to the scroll container; where it settles (one shown while the bodies above still load
/// stays where they left it until the window resizes or the thread scrolls). Returns where
/// `Learn more` goes, once clicked.
pub(super) fn failed_tooltip(
    ui: &Ui,
    painter: &egui::Painter,
    layout: &Layout,
    card: Rect,
    message: &MessageDetail,
) -> Option<&'static str> {
    let info = failure::information(message.failure?);
    let slot = status_slot(ui, layout, card, info.title);
    let tooltip = TextTooltip {
        text: info.description,
        max_width: FAILED_TOOLTIP_WIDTH,
        learn_more: info.doc_link.is_some(),
    };
    let id = ui.id().with(("failed-tooltip", &message.id));
    if tooltip.show_open(ui, painter, id, slot, 7.0, layout.column.x_range()) {
        info.doc_link
    } else {
        None
    }
}

/// A full header row's people: each by name and ` <address>`, or
/// the address alone, comma-separated, after the label's column, wrapped at `width`; names in
/// `contact`, the rest in `label`.
pub(crate) fn header_job(
    contacts: &[Contact],
    width: f32,
    label: Color32,
    contact: Color32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for (at, person) in contacts.iter().enumerate() {
        if at > 0 {
            job.append(", ", 0.0, HEADER.format(label));
        }
        match person.name.as_deref().filter(|name| !name.is_empty()) {
            Some(name) => {
                job.append(name, 0.0, HEADER.format(contact));
                job.append(&format!(" <{}>", person.email), 0.0, HEADER.format(label));
            }
            None => job.append(&person.email, 0.0, HEADER.format(contact)),
        }
    }
    if let Some(first) = job.sections.first_mut() {
        first.leading_space = HEADER_LABEL;
    }
    job.wrap = egui::text::TextWrapping::wrap_at_width(width);
    job
}

/// Shift-H's full header: From, To, Cc, Bcc with names and addresses, then the full date (or a
/// failed or waiting send's status) and the chevron that folds it back. Its text is selectable
/// (`cursor: text`); only the chevron folds it. With `tracking_blocked`, an eye follows From.
#[expect(
    clippy::too_many_arguments,
    reason = "one header, drawn from the card's state"
)]
fn full_header(
    ui: &Ui,
    rect: Rect,
    layout: &Layout,
    message: &MessageDetail,
    now: &Zoned,
    status: Option<Status>,
    tracking_blocked: bool,
    ink: &Ink,
    selection: &mut exo_ui::TextSelection,
) -> Option<Hit> {
    let id = ui.id().with(("message-header", &message.id));
    let painter = ui.painter();
    let left = layout.inner.left();
    let width = layout.inner.width();
    let first = rect.top() + PADDING_TOP - 4.0 + 1.0;
    let mut top = first;
    let rows: [(&str, &[Contact]); 4] = [
        ("From", message.from.as_slice()),
        ("To", &message.to),
        ("Cc", &message.cc),
        ("Bcc", &message.bcc),
    ];
    // Where the eye's baseline is, after From.
    let mut eye = None;
    for (label, contacts) in rows {
        if contacts.is_empty() {
            continue;
        }
        // `::before` content, which find doesn't search either.
        exo_ui::find::unsearchable(ui.ctx(), || {
            HEADER.paint_line(painter, pos2(left, top), label, ink.label, None)
        });
        let galley = painter.layout_job(header_job(contacts, width, ink.label, ink.contact));
        let lines = galley.rows.len().max(1) as f32;
        let end = galley
            .rows
            .last()
            .map_or(HEADER_LABEL, |row| row.rect().right());
        // The addresses are selectable text (`cursor: text`); the labels are `::before` content,
        // which isn't.
        selection.add(
            painter,
            pos2(left, top),
            galley.clone(),
            HEADER_ROW,
            super::SELECTED_CONTACTS,
            "\n",
            exo_ui::palette(ui.ctx()).text_selection_default,
        );
        exo_ui::paint_text(painter, pos2(left, top), galley, &HEADER);
        if label == "From" && tracking_blocked {
            let line = top + HEADER_ROW * (lines - 1.0);
            eye = Some(pos2(
                left + end + PIXEL_BLOCKING_GAP,
                line + HEADER.baseline(ui.ctx()),
            ));
        }
        top += HEADER_ROW * lines;
    }
    // The header's text takes an I-beam.
    ui.interact(
        Rect::from_min_max(
            pos2(left, first - 1.0),
            pos2(left + width, top + HEADER_ROW - 1.0),
        ),
        id.with("text"),
        Sense::hover(),
    )
    .on_hover_cursor(egui::CursorIcon::Text);
    // Over the text, which it takes the pointer from.
    if let Some(baseline) = eye {
        pixel_blocking(ui, baseline, id.with("pixel-blocking"));
    }
    let date_width = match status {
        // The 12px/16px slot as an inline block on the header's 24px line, by baselines, then
        // `top: 1px`: just `Send failed` (and no chevron), or `Send: when online`.
        Some(status) => {
            // The long `expanded` date when scheduled, and no tooltip.
            let text = status.full_title(now);
            let y = top + HEADER.baseline(ui.ctx()) - STATUS.baseline(ui.ctx()) + 1.0;
            let width = STATUS.paint_line(painter, pos2(left, y), &text, status.ink(), None);
            let slot = Rect::from_min_size(pos2(left, y), vec2(width, STATUS.line_height));
            let response = ui
                .interact(slot, id.with("status"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() || matches!(status, Status::Failed(_)) {
                return response.clicked().then_some(Hit::Recover);
            }
            width
        }
        None => HEADER.paint_line(
            painter,
            pos2(left, top),
            &dates::expanded(message.date, now),
            ink.label,
            None,
        ),
    };
    // The collapse chevron 12px after the date, 3px down.
    let size = exo_ui::icon_size(ui.ctx(), COLLAPSE).unwrap_or(vec2(10.0, 12.0));
    let chevron = Rect::from_min_size(pos2(left + date_width + 12.0, top + 3.0), size);
    let response = IconButton::new(COLLAPSE)
        .colors(ink.collapse, ink.collapse)
        .show(ui, id.with("collapse"), chevron.expand(9.0), chevron);
    // 14px under the icon.
    KeyTooltip {
        text: "Collapse Message Header",
        keys: &["shift", "H"],
    }
    .show_at(
        ui,
        &response,
        Placement::Below {
            gap: 14.0 - 9.0,
            shift: 0.0,
        },
    );
    response
        .clicked()
        .then_some(Hit::HeaderToggle { hint: true })
}

/// The `Email tracking blocked` eye, its bottom edge `PIXEL_BLOCKING_DROP` under `baseline`
/// (an inline SVG sits on the baseline), in `rgba(0, 0, 0, .45)` (white in
/// Carbon), with its tooltip.
fn pixel_blocking(ui: &Ui, baseline: egui::Pos2, id: egui::Id) {
    let size = exo_ui::icon_size(ui.ctx(), PIXEL_BLOCKING).unwrap_or(vec2(13.0, 10.0));
    let icon = Rect::from_min_size(baseline + vec2(0.0, PIXEL_BLOCKING_DROP - size.y), size);
    exo_ui::paint_icon(
        ui,
        PIXEL_BLOCKING,
        icon,
        exo_ui::palette(ui.ctx()).ink(0.45, 0.45),
    );
    // A default cursor over the icon, not the header text's I-beam.
    let response = ui
        .interact(icon, id, Sense::hover())
        .on_hover_cursor(egui::CursorIcon::Default);
    KeyTooltip {
        text: TRACKING_BLOCKED,
        keys: &[],
    }
    .show_at(ui, &response, PIXEL_BLOCKING_TOOLTIP);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(name: &str, email: &str, is_me: bool) -> Contact {
        Contact {
            name: Some(name.into()),
            email: email.into(),
            is_me,
        }
    }

    fn message(from: Contact, to: Vec<Contact>) -> MessageDetail {
        MessageDetail {
            id: "m".into(),
            labels: Vec::new(),
            date: jiff::Timestamp::UNIX_EPOCH,
            from: Some(from),
            to,
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: String::new(),
            snippet: String::new(),
            rfc822_id: None,
            in_reply_to: None,
            references: Vec::new(),
            list_id: None,
            list_unsubscribe: None,
            list_unsubscribe_post: None,
            headers: Vec::new(),
            body: exo_store::Body::default(),
            attachments: Vec::new(),
            failure: None,
            scheduled: None,
            sending: false,
        }
    }

    fn thread(messages: Vec<MessageDetail>) -> ThreadDetail {
        ThreadDetail {
            id: exo_store::ThreadId::new("t"),
            subject: String::new(),
            labels: Vec::new(),
            muted: false,
            reminder: None,
            messages,
        }
    }

    #[test]
    fn headers_name_recipients_when_the_thread_has_company() {
        let me = contact("Maya Okafor", "maya@brightmoor.co", true);
        let tom = contact("Tom Becker", "tom@brightmoor.co", false);
        let priya = contact("Priya Raman", "priya@brightmoor.co", false);
        let crit = message(tom.clone(), vec![me.clone(), priya]);
        assert_eq!(
            participants(&crit, &thread(vec![crit.clone()]), true, false),
            "Tom to Me & Priya"
        );
        assert_eq!(
            participants(&crit, &thread(vec![crit.clone()]), false, false),
            "Tom"
        );
        let grace = message(
            contact("Grace Liu", "grace@juniperpartners.vc", false),
            vec![me.clone()],
        );
        assert_eq!(
            participants(&grace, &thread(vec![grace.clone()]), true, false),
            "Grace",
            "one other person: no recipients"
        );
        let note = message(me.clone(), vec![me]);
        assert_eq!(
            participants(&note, &thread(vec![note.clone()]), true, false),
            "Note to self"
        );
    }

    #[test]
    fn full_names_name_only_the_sender_in_full() {
        let me = contact("Maya Okafor", "maya@brightmoor.co", true);
        let tom = contact("Tom Becker", "tom@brightmoor.co", false);
        let priya = contact("Priya Raman", "priya@brightmoor.co", false);
        let crit = message(tom, vec![me.clone(), priya]);
        assert_eq!(
            participants(&crit, &thread(vec![crit.clone()]), true, true),
            "Tom Becker to Me & Priya"
        );
        assert_eq!(
            participants(&crit, &thread(vec![crit.clone()]), false, true),
            "Tom Becker",
            "collapsed too"
        );
        let mine = message(
            me.clone(),
            vec![contact("Grace Liu", "grace@juniperpartners.vc", false)],
        );
        assert_eq!(
            participants(&mine, &thread(vec![mine.clone()]), false, true),
            "Me",
            "your own name isn't written in full"
        );
    }

    #[test]
    fn a_marketing_card_in_dark_mode_keeps_dark_ink() {
        // Morning Ledger's header in Carbon: dark text on the 90% white card (its sender at
        // (50, 50, 51) and its date at (118, 118, 119)).
        let ctx = egui::Context::default();
        exo_ui::set_theme(&ctx, Theme::Carbon);
        let ink = Ink::new(&ctx, true);
        assert_eq!(ink.names, Color32::from_rgb(50, 50, 50));
        assert_eq!(ink.date, Color32::from_rgb(118, 118, 119));
        assert_eq!(ink.label, ink.date);
        assert_eq!(ink.contact, ink.names);
        assert_eq!(
            Ink::new(&ctx, false).names,
            Color32::from_white_alpha(204),
            "other cards keep the theme's"
        );
    }

    #[test]
    fn condensed_participants_shrink_the_sender_first_then_the_lead() {
        // Room enough: nothing shrinks; a short lead still takes its least 42px.
        assert_eq!(
            condense(30.0, 20.0, 40.0, 300.0, false, false),
            (30.0, 42.0)
        );
        // The sender gives up nearly all of it, down to 18px; then the lead.
        let (sender, lead) = condense(100.0, 100.0, 40.0, 200.0, false, false);
        assert!(
            (sender + lead - 160.0).abs() < 0.01 && lead > 99.0 && lead <= 100.0,
            "{sender} {lead}"
        );
        let (sender, lead) = condense(100.0, 100.0, 40.0, 100.0, false, false);
        assert!(
            (sender - 18.0).abs() < 0.01 && (lead - 42.0).abs() < 0.01,
            "{sender} {lead}"
        );
        // Yours stays whole, and the lead is at most half the room.
        assert_eq!(
            condense(20.0, 200.0, 40.0, 400.0, true, false),
            (20.0, 200.0)
        );
        assert_eq!(
            condense(20.0, 300.0, 40.0, 400.0, false, false),
            (20.0, 200.0)
        );
    }

    #[test]
    fn snippets_stop_at_quote_headers() {
        assert_eq!(
            clean_snippet("Sounds good. On Mon, Nov 16 Tom wrote: …"),
            "Sounds good. "
        );
        assert_eq!(clean_snippet("Thanks!\n----\nMaya"), "Thanks!\n");
    }
}
