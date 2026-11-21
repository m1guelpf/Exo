//! A message's calendar invite card: a photo picked by the event's title with the update's
//! status over it, then the RSVP (or `Add To Calendar`), When, Where, video, Who, About and
//! Files rows, read from the message's `.ics`. The buttons answer through the calendar.

use std::{collections::HashMap, path::Path, sync::LazyLock};

use egui::{
    Color32, CornerRadius, Rect, Sense, Stroke, TextureHandle, Ui, epaint::RectShape, pos2, vec2,
};
use exo_store::{Attachment, Contact, MessageDetail};
use exo_ui::{BoxShadow, Family, Font, Palette, TextStyle, Theme};
use jiff::{Timestamp, Zoned, civil::DateTime, tz::TimeZone};
use regex::Regex;

use crate::format::{
    dates,
    names::{NameFormat, Person, display_name},
};

/// The card's text: 14px/24px; the row names bold, `min-width: 92px`.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 24.0);
const BOLD: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 24.0);
const QUESTION: f32 = 92.0;
/// A row's value: `padding-left: 12px`.
const PROPERTY_PADDING: f32 = 12.0;
/// The photo: 176px tall.
const IMAGE_HEIGHT: f32 = 176.0;
/// The rows: `padding: 28px 24px 24px`, 16px apart.
const ROWS_TOP: f32 = 28.0;
const ROWS_SIDE: f32 = 24.0;
const ROWS_BOTTOM: f32 = 24.0;
const ROW_GAP: f32 = 16.0;
/// The card's `margin-bottom: 16px; top: 1px`.
pub(super) const MARGIN_BOTTOM: f32 = 16.0;
/// An RSVP button: 28px tall, `padding: 0 12px; margin-top: -4px`, 16px apart.
const BUTTON_HEIGHT: f32 = 28.0;
const BUTTON_PADDING: f32 = 12.0;
const BUTTON_GAP: f32 = 16.0;
/// A focused RSVP button's glow: `0 0 6px 1px #aeb1dd`, `0 0 6px 0 #54acdc` on the current
/// answer, then `0 2px 12px 0 rgba(174, 177, 221, .5)`.
const FOCUS_GLOW: BoxShadow =
    BoxShadow::new([0.0, 0.0], 6.0, 1.0, Color32::from_rgb(0xae, 0xb1, 0xdd));
const SELECTED_FOCUS_GLOW: BoxShadow =
    BoxShadow::new([0.0, 0.0], 6.0, 0.0, Color32::from_rgb(0x54, 0xac, 0xdc));
const FOCUS_HAZE: BoxShadow = BoxShadow::new(
    [0.0, 2.0],
    12.0,
    0.0,
    Color32::from_rgba_unmultiplied_const(0xae, 0xb1, 0xdd, 128),
);
/// Who lists the first 15 attendees.
const MAX_WHO: usize = 15;
/// About shows the first 8 lines until `Show more`.
const ABOUT_LINES: usize = 8;
/// About's `Show more`: 16px under the text.
const SHOW_MORE_GAP: f32 = 16.0;
/// The status over the photo: `top: 16px; left: 24px; margin-left: -7px; padding: 4px 7px
/// 0`, a 1px border, `max-width: calc(100% - 24px)`.
const STATUS_TOP: f32 = 16.0;
const STATUS_LEFT: f32 = 24.0 - 7.0;
const STATUS_PADDING_TOP: f32 = 4.0;
const STATUS_PADDING_X: f32 = 7.0;
/// Files: three columns `minmax(120px, 154.16px)`, 8px apart.
const FILE_WIDTH: f32 = 154.16;
const FILE_GAP: f32 = 8.0;
/// The guest's note: the email sans at 14px, `line-height: 1.714` (24px), `padding-bottom:
/// 16px`. Its `margin-top: 1px` collapses into the body's 20px.
const NOTE: TextStyle = TextStyle::new(Font::new(Family::SansEmail, 400), 14.0, 24.0);
const NOTE_BOTTOM: f32 = 16.0;
/// A title longer than this leads the description.
const LONG_SUMMARY: usize = 60;

/// An attendee's answer (`PARTSTAT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response {
    NeedsAction,
    Accepted,
    Tentative,
    Declined,
}

impl Response {
    /// Calendar's `responseStatus` for it.
    pub fn status(self) -> &'static str {
        match self {
            Response::NeedsAction => "needsAction",
            Response::Accepted => "accepted",
            Response::Tentative => "tentative",
            Response::Declined => "declined",
        }
    }

    /// The answer Calendar's `responseStatus` names.
    pub fn parse(status: &str) -> Option<Self> {
        [
            Response::NeedsAction,
            Response::Accepted,
            Response::Tentative,
            Response::Declined,
        ]
        .into_iter()
        .find(|response| response.status() == status)
    }
}

/// What a click on the card asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardAction {
    /// A link: the location's map or number, the video call, a file.
    Open(String),
    /// An answer other than the current one; `NeedsAction` adds the event to the calendar.
    Respond(Response),
    /// `Show more` / `Show Less`.
    ToggleAbout,
}

/// The video call an invite's description links to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meeting {
    /// `Meet` or `Teams`: the row's label.
    pub label: &'static str,
    pub url: String,
}

/// A file the invite attaches (`ATTACH` with a name and a type).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub name: String,
    pub mime: String,
    pub url: String,
}

/// What the card shows of an invite.
#[derive(Clone, Debug, PartialEq)]
pub struct Invite {
    /// `UID`, which the event on the calendar shares.
    pub uid: String,
    pub summary: String,
    /// The description, its `<url>`s made links and Teams' join lines too.
    pub description: String,
    pub location: String,
    pub start: Timestamp,
    pub end: Timestamp,
    /// The `METHOD` is `REQUEST` (an invitation, not a reply or a cancellation).
    pub request: bool,
    /// `STATUS:CANCELLED` or `METHOD:CANCEL`.
    pub cancelled: bool,
    /// `METHOD:COUNTER`: a guest proposes the time it gives (RFC 5546 §3.2.7).
    pub counter: bool,
    /// It repeats (`RRULE`, `EXRULE`, `RDATE` or `EXDATE`).
    pub recurring: bool,
    /// The calendar that made it (`PRODID`).
    pub product: String,
    /// Attendees: name, address, answer.
    pub attendees: Vec<(String, String, Response)>,
    /// The organizer: name, address.
    pub organizer: Option<(String, String)>,
    pub meeting: Option<Meeting>,
    pub files: Vec<File>,
    /// A guest's note: the one attendee's `X-RESPONSE-COMMENT`, else the event's
    /// `COMMENT`; empty without one.
    pub note: String,
    /// What I answered here, over the `.ics`'s answer.
    pub saved: Option<Response>,
}

impl Invite {
    /// Who I am on the invite: `me` (the mail account) when it's
    /// invited, else the first of my other calendar accounts, `accounts`, that is; `me` when
    /// none is.
    pub fn calendar_account<'a>(&'a self, me: &'a str, accounts: &[String]) -> &'a str {
        if self
            .attendees
            .iter()
            .any(|(_, email, _)| email.eq_ignore_ascii_case(me))
        {
            return me;
        }
        self.attendees
            .iter()
            .map(|(_, email, _)| email.as_str())
            .find(|email| {
                accounts
                    .iter()
                    .any(|account| account.eq_ignore_ascii_case(email))
            })
            .unwrap_or(me)
    }

    /// Whether I can answer it: I'm on it, and it isn't called off.
    pub fn can_respond(&self, me: &str) -> bool {
        !self.cancelled
            && self
                .attendees
                .iter()
                .any(|(_, email, _)| email.eq_ignore_ascii_case(me))
    }

    /// Whether `Add To Calendar` shows instead: not a series, an answer or a
    /// cancellation.
    pub fn can_add_to_calendar(&self, me: &str) -> bool {
        !self.can_respond(me) && !self.recurring && !self.is_response(me) && !self.cancelled
    }

    /// My answer: the one given here, else as the invite has it.
    pub fn answer_of(&self, me: &str) -> Option<Response> {
        self.saved.or_else(|| {
            self.attendees
                .iter()
                .find(|(_, email, _)| email.eq_ignore_ascii_case(me))
                .map(|(_, _, response)| *response)
        })
    }

    /// Records `response` as my answer.
    pub fn answer(&mut self, me: &str, response: Response) {
        self.saved = Some(response);
        for (_, email, answer) in &mut self.attendees {
            if email.eq_ignore_ascii_case(me) {
                *answer = response;
            }
        }
    }

    /// A guest's answer to an invitation: one attendee, not me, who
    /// answered; or a guest's proposal (`METHOD:COUNTER`).
    fn is_response(&self, me: &str) -> bool {
        let [(_, email, response)] = self.attendees.as_slice() else {
            return false;
        };
        !email.eq_ignore_ascii_case(me) && (self.counter || *response != Response::NeedsAction)
    }

    /// The description as About shows it: Calendar's boilerplate dropped,
    /// a long title first; `None` when there's nothing.
    fn about(&self) -> Option<String> {
        let summary = self.summary.trim();
        let long = summary.chars().count() > LONG_SUMMARY;
        if self.description.is_empty() && !long {
            return None;
        }
        let cleaned = crate::calendar::description::clean(&self.description);
        Some(if long {
            format!("{summary}\n\n{cleaned}")
        } else {
            cleaned
        })
    }
}

/// The message's invite attachment, if it carries one (not on a forward, `Fwd…`).
pub(crate) fn attachment(message: &MessageDetail) -> Option<&Attachment> {
    let subject = message.subject.trim_start().to_lowercase();
    if subject.starts_with("fwd") || subject.starts_with("fw:") {
        return None;
    }
    message
        .attachments
        .iter()
        .find(|attachment| super::attachments::is_invite(attachment))
}

/// Whether the card stands in for the message's body: Google Calendar's notification (its
/// microdata, or sent by `calendar-notification@google.com`), or
/// Outlook's when it isn't a reply.
pub(super) fn hides_body(invite: &Invite, message: &MessageDetail, gcal: bool) -> bool {
    let sender = message
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("sender"))
        .map(|header| gmail::mime::Mailbox::parse(&header.value).email);
    let google = gcal
        || sender
            .is_some_and(|sender| sender.eq_ignore_ascii_case("calendar-notification@google.com"));
    let outlook = ["Outlook", "Microsoft Exchange Server 2010"].contains(&invite.product.as_str())
        && message.in_reply_to.is_none();
    google || outlook
}

/// Reads the first `VEVENT` of an iCalendar file.
pub fn parse(ics: &str) -> Option<Invite> {
    // Lines fold with a leading space or tab (RFC 5545 §3.1).
    let unfolded = ics
        .replace("\r\n ", "")
        .replace("\r\n\t", "")
        .replace("\n ", "")
        .replace("\n\t", "");
    let mut in_event = false;
    let mut method = String::new();
    let mut status = String::new();
    let mut invite = Invite {
        uid: String::new(),
        summary: String::new(),
        description: String::new(),
        location: String::new(),
        start: Timestamp::UNIX_EPOCH,
        end: Timestamp::UNIX_EPOCH,
        request: false,
        cancelled: false,
        counter: false,
        recurring: false,
        product: String::new(),
        attendees: Vec::new(),
        organizer: None,
        meeting: None,
        files: Vec::new(),
        note: String::new(),
        saved: None,
    };
    let mut started = false;
    let (mut comment, mut response_comments) = (None, Vec::new());
    for line in unfolded.lines() {
        let Some((name, params, value)) = content_line(line) else {
            continue;
        };
        match name.as_str() {
            "METHOD" => method = value.trim().to_ascii_uppercase(),
            "PRODID" if !in_event => invite.product = value.trim().to_owned(),
            "BEGIN" if value.eq_ignore_ascii_case("VEVENT") => in_event = true,
            "END" if value.eq_ignore_ascii_case("VEVENT") => break,
            _ if !in_event => {}
            "UID" => invite.uid = value.trim().to_owned(),
            "SUMMARY" => invite.summary = unescape(value),
            "DESCRIPTION" => invite.description = unescape(value),
            "LOCATION" => invite.location = unescape(value),
            "COMMENT" if comment.is_none() => comment = Some(unescape(value)),
            "STATUS" => status = value.trim().to_ascii_uppercase(),
            "RRULE" | "EXRULE" | "RDATE" | "EXDATE" => invite.recurring = true,
            "DTSTART" => {
                invite.start = date_time(value, params.get("TZID"))?;
                started = true;
            }
            "DTEND" => invite.end = date_time(value, params.get("TZID"))?,
            "ORGANIZER" => {
                let email = value
                    .trim_start_matches("mailto:")
                    .trim_start_matches("MAILTO:")
                    .to_owned();
                invite.organizer = Some((params.get("CN").cloned().unwrap_or_default(), email));
            }
            "ATTENDEE" => {
                let email = value
                    .trim_start_matches("mailto:")
                    .trim_start_matches("MAILTO:")
                    .to_owned();
                let response = match params
                    .get("PARTSTAT")
                    .map(|status| status.to_ascii_uppercase())
                    .as_deref()
                {
                    Some("ACCEPTED") => Response::Accepted,
                    Some("TENTATIVE") => Response::Tentative,
                    Some("DECLINED") => Response::Declined,
                    _ => Response::NeedsAction,
                };
                invite.attendees.push((
                    params.get("CN").cloned().unwrap_or_default(),
                    email,
                    response,
                ));
                response_comments.push(
                    params
                        .get("X-RESPONSE-COMMENT")
                        .map_or("", |comment| comment.trim())
                        .to_owned(),
                );
            }
            // An attachment is listed only with both a name and a type.
            "ATTACH" => {
                if let (Some(name), Some(mime)) = (params.get("FILENAME"), params.get("FMTTYPE")) {
                    invite.files.push(File {
                        name: name.clone(),
                        mime: mime.clone(),
                        url: value.trim().to_owned(),
                    });
                }
            }
            _ => {}
        }
    }
    invite.request = method == "REQUEST";
    invite.counter = method == "COUNTER";
    invite.cancelled = status == "CANCELLED" || method == "CANCEL";
    let (description, meeting) = extract_meeting(&invite.description);
    (invite.description, invite.meeting) = (description, meeting);
    invite.note = match response_comments.as_slice() {
        [only] if !only.is_empty() => only.clone(),
        _ => comment.unwrap_or_default(),
    };
    started.then_some(invite)
}

/// A content line's upper-cased name, its parameters (upper-cased names, values unquoted and
/// RFC 6868's `^n`, `^'` and `^^` decoded) and its value (RFC 5545 §3.1): the first `:` outside
/// a quoted parameter value starts the value.
fn content_line(line: &str) -> Option<(String, HashMap<String, String>, &str)> {
    let mut quoted = false;
    let mut parts = Vec::new();
    let mut start = 0;
    for (at, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' | ':' if !quoted => {
                parts.push(&line[start..at]);
                start = at + 1;
                if c == ':' {
                    let name = parts.remove(0).to_ascii_uppercase();
                    let params = parts
                        .into_iter()
                        .filter_map(|param| param.split_once('='))
                        .map(|(key, value)| {
                            (
                                key.to_ascii_uppercase(),
                                decode_param(value.trim_matches('"')),
                            )
                        })
                        .collect();
                    return Some((name, params, &line[start..]));
                }
            }
            _ => {}
        }
    }
    None
}

/// RFC 6868's parameter value escapes.
fn decode_param(value: &str) -> String {
    static ESCAPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\^['n^]").expect("valid regex"));
    ESCAPE
        .replace_all(value, |found: &regex::Captures| match &found[0] {
            "^n" => "\n",
            "^'" => "\"",
            _ => "^",
        })
        .into_owned()
}

/// `TEXT` value escapes (RFC 5545 §3.3.11).
fn unescape(value: &str) -> String {
    value
        .replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

static TEAMS_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([\n|]\s*)(.+?)<(https?://.+?)>").expect("valid regex"));
static ANGLE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(https?://.+?)>").expect("valid regex"));
static MEET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\n(?:Join|Join with Google Meet): (https?://(www\.)?meet\.google\.com/(.*))")
        .expect("valid regex")
});

/// The description with its `<url>`s (and Teams' `text<url>`
/// lines) as links, and the Meet (else Teams) call it joins.
fn extract_meeting(description: &str) -> (String, Option<Meeting>) {
    let mut text = description.to_owned();
    let mut teams = None;
    if description.contains("Microsoft Teams meeting") {
        let trimmed = text.trim().trim_matches('_').to_owned();
        if let Some(at) = trimmed.find("Microsoft Teams meeting") {
            let tail = TEAMS_LINE.replace_all(&trimmed[at..], |found: &regex::Captures| {
                if found[3].contains("teams.microsoft.com/l/meetup-join/") {
                    teams = Some(found[3].to_owned());
                }
                format!("{}<a href=\"{}\">{}</a>", &found[1], &found[3], &found[2])
            });
            text = format!("{}{tail}", &trimmed[..at]);
        }
    }
    let text = ANGLE_URL
        .replace_all(&text, " <a href=\"$1\">$1</a>")
        .into_owned();
    let meet = description
        .contains("meet.google.com")
        .then(|| MEET.captures(description).map(|found| found[1].to_owned()))
        .flatten();
    let meeting = meet.map(|url| Meeting { label: "Meet", url }).or_else(|| {
        teams.map(|url| Meeting {
            label: "Teams",
            url,
        })
    });
    (text, meeting)
}

/// A `DATE-TIME` (`20261125T140000Z`, or local with a `TZID`) or `DATE` (`20261125`).
fn date_time(value: &str, zone: Option<&String>) -> Option<Timestamp> {
    let value = value.trim();
    let utc = value.ends_with('Z');
    let digits = value.trim_end_matches('Z');
    let civil = if digits.len() == 8 {
        DateTime::strptime("%Y%m%d", digits)
            .or_else(|_| {
                jiff::civil::Date::strptime("%Y%m%d", digits)
                    .map(|date| date.to_datetime(jiff::civil::Time::midnight()))
            })
            .ok()?
    } else {
        DateTime::strptime("%Y%m%dT%H%M%S", digits).ok()?
    };
    let zone = if utc {
        TimeZone::UTC
    } else {
        zone.and_then(|name| TimeZone::get(name).ok())
            .unwrap_or_else(TimeZone::system)
    };
    civil.to_zoned(zone).ok().map(|zoned| zoned.timestamp())
}

/// Which photo heads the card.
pub(super) fn image_kind(invite: &Invite) -> &'static str {
    let attendees = invite.attendees.len();
    kind(&invite.summary, attendees)
        .or_else(|| kind(&invite.description, attendees))
        .unwrap_or("default")
}

fn kind(text: &str, attendees: usize) -> Option<&'static str> {
    const EDGE: &str = r#"(^|$|\s|-|—|[.;:'"()?!&_\/~])"#;
    static RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        [
            (r"beach|swim|surf|boat|jet(-|\s)?ski|snorkling|scuba(-|\s)?div|paddling|whale(-|\s)?watching|kayak|sail".to_owned(), "beach"),
            (r"birth\s?day|b'?day".to_owned(), "birthday"),
            (r"meeting|interview|planning|stand-?up|agenda|all(\s|-)?hand|on\-?boarding".to_owned(), "meeting"),
            (format!(r"{EDGE}call{EDGE}|phone"), "call"),
            (format!(r"coff?ee|{EDGE}caf(e|é){EDGE}|espresso|starbucks|cappuccino|mocha"), "coffee"),
            (r"dinner|restaurant|cook\-off|dining|dine|supper".to_owned(), "dinner"),
            (format!(r"drink|{EDGE}bar{EDGE}|happy\shour|cocktail|whiske?y|bourbon|vodka|{EDGE}gin{EDGE}|tequila|liquor|{EDGE}wine{EDGE}"), "drinks"),
            (format!(r"exc?er(c|s)i(s|z)e|workout|{EDGE}gym{EDGE}|treadmill|{EDGE}run{EDGE}"), "exercise"),
            (format!(r"flight|airport|flying|airplane|{EDGE}jet{EDGE}"), "flight"),
            (r"hike|hiking".to_owned(), "hike"),
            (r"breakfast|brunch".to_owned(), "breakfast"),
            (format!(r"lunch|{EDGE}eat{EDGE}|food|meal"), "lunch"),
            (r"party|rave|rager|celebration|gala".to_owned(), "party"),
            (format!(r"{EDGE}(tea|ocha|matcha){EDGE}"), "tea"),
            (format!(r"({EDGE}1(-|\s)?on(-|\s)?1{EDGE})|(1\s?(\:|\-|/)\s?1{EDGE})|(one(-|\s)?on(-|\s)?one)"), "1on1"),
        ]
        .into_iter()
        .map(|(pattern, kind)| (Regex::new(&format!("(?i){pattern}")).expect("valid regex"), kind))
        .collect()
    });
    static PAIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<(\s|-)?>").expect("valid regex"));
    // A call over video is a meeting, not a phone call.
    static VIDEO_CALL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)video\scall").expect("valid regex"));
    for (rule, kind) in RULES.iter() {
        if rule.is_match(text) && !(*kind == "call" && VIDEO_CALL.is_match(text)) {
            return Some(kind);
        }
    }
    PAIR.is_match(text)
        .then_some(if attendees == 2 { "1on1" } else { "meeting" })
}

/// The invite photos, decoded once.
#[derive(Default)]
pub struct Photos(HashMap<&'static str, Option<TextureHandle>>);

impl Photos {
    fn get(
        &mut self,
        ctx: &egui::Context,
        images: &Path,
        kind: &'static str,
    ) -> Option<&TextureHandle> {
        self.0
            .entry(kind)
            .or_insert_with(|| {
                let path = images.join(format!("invite-{kind}.jpg"));
                let decoded = std::fs::read(&path)
                    .ok()
                    .and_then(|bytes| image::load_from_memory(&bytes).ok());
                let Some(decoded) = decoded else {
                    log::warn!("no invite photo at {}", path.display());
                    return None;
                };
                let rgba = decoded.to_rgba8();
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [rgba.width() as usize, rgba.height() as usize],
                    rgba.as_raw(),
                );
                Some(ctx.load_texture(
                    format!("invite {kind}"),
                    color,
                    egui::TextureOptions::LINEAR,
                ))
            })
            .as_ref()
    }
}

/// What the card knows beyond the invite: whose card it is (the calendar account it's to), the
/// mail account (Who's `Me`), the message it came in, the notification's facts, and whether
/// About is opened up.
pub(super) struct Context<'a> {
    pub me: &'a str,
    pub user: &'a str,
    pub now: &'a Zoned,
    pub message: &'a MessageDetail,
    pub calendar: Option<&'a exo_mail::CalendarData>,
    pub about_open: bool,
}

/// The status over the photo: a cancellation or an update, else a guest's answer.
fn status(invite: &Invite, context: &Context) -> Option<String> {
    let calendar = context.calendar.cloned().unwrap_or_default();
    let updated = context.message.subject.contains("Updated Invitation:");
    if invite.cancelled || updated || calendar.event_status.is_some() {
        if invite.cancelled || calendar.event_status == Some(exo_mail::EventStatus::Cancelled) {
            return Some("Event has been canceled".to_owned());
        }
        if !(updated
            || calendar.event_status == Some(exo_mail::EventStatus::Rescheduled)
            || calendar.updated_fields.is_some())
        {
            return None;
        }
        return Some(match &calendar.updated_fields {
            Some(fields) => format!("Event has been changed - {fields}"),
            None => "Event has been changed".to_owned(),
        });
    }
    if !invite.is_response(context.me) {
        return None;
    }
    let (name, email, response) = &invite.attendees[0];
    if invite.counter {
        return Some(format!(
            "{} has proposed a new time",
            responder(name, email, context.message)
        ));
    }
    let answered = match response {
        Response::Accepted => "accepted",
        Response::Tentative => "responded maybe",
        Response::Declined => "declined",
        Response::NeedsAction => return None,
    };
    Some(format!(
        "{} has {answered}",
        responder(name, email, context.message)
    ))
}

/// The responder's name: the attendee's first name (as the invite names them), else the sender's
/// name when it's them, else the address.
fn responder(name: &str, email: &str, message: &MessageDetail) -> String {
    if !name.is_empty() && !name.contains('@') {
        return display_name(
            &Person::new(name.to_owned(), email.to_owned()),
            false,
            NameFormat::First,
            false,
        );
    }
    message
        .from
        .as_ref()
        .filter(|from| from.email.eq_ignore_ascii_case(email))
        .and_then(|from| from.name.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| email.to_owned())
}

/// A piece of a row's value: text, a link when `link` is set, bold when `bold`, a participant's
/// name with their tooltip when `contact` is.
#[derive(Clone, Debug, Default, PartialEq)]
struct Piece {
    text: String,
    link: Option<String>,
    bold: bool,
    contact: Option<Contact>,
}

impl Piece {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    fn link(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            link: Some(url.into()),
            ..Self::default()
        }
    }
}

/// The card's rows: label and value.
struct Row {
    question: &'static str,
    value: Value,
}

enum Value {
    /// Lines of pieces, each line breaking where text breaks, indented by its first number.
    Text(Vec<(f32, Vec<Piece>)>),
    /// When: breaks only after ` at`, `,` and `– ` (`white-space: nowrap` with `<wbr>`s).
    When(String),
    Rsvp(Option<Response>),
    AddToCalendar(bool),
    /// About's lines, and `Show more` (false) or `Show Less` (true) under them.
    About(Vec<(f32, Vec<Piece>)>, Option<bool>),
    Files(Vec<File>),
}

fn rows(invite: &Invite, context: &Context) -> Vec<Row> {
    let me = context.me;
    let mut rows = Vec::new();
    // My answer given here, else the invite's.
    let answer = invite.answer_of(me);
    if invite.can_respond(me) {
        rows.push(Row {
            question: "RSVP",
            value: Value::Rsvp(answer.filter(|answer| *answer != Response::NeedsAction)),
        });
    } else if invite.can_add_to_calendar(me) {
        rows.push(Row {
            question: "",
            value: Value::AddToCalendar(invite.saved == Some(Response::NeedsAction)),
        });
    }
    let mut when = dates::calendar_range(invite.start, invite.end, context.now);
    if invite.recurring {
        when.push_str(" (recurring)");
    }
    rows.push(Row {
        question: "When",
        value: Value::When(when),
    });
    if let Some(location) = location(invite) {
        rows.push(Row {
            question: "Where",
            value: Value::Text(vec![(0.0, location)]),
        });
    }
    if let Some(meeting) = &invite.meeting {
        let shown = if meeting.url.contains("teams.microsoft.com/l/meetup-join/") {
            "https://teams.microsoft.com/l/meetup-join/…"
        } else {
            &meeting.url
        };
        rows.push(Row {
            question: meeting.label,
            value: Value::Text(vec![(0.0, vec![Piece::link(shown, &meeting.url)])]),
        });
    }
    if !invite.attendees.is_empty() {
        // Each name with its contact tooltip, ", " between.
        let mut who = Vec::new();
        for (at, (name, email, _)) in invite.attendees.iter().take(MAX_WHO).enumerate() {
            if at > 0 {
                who.push(Piece::text(", "));
            }
            // The mail account is `Me`; another calendar account of mine goes
            // by its name.
            let is_me = email.eq_ignore_ascii_case(context.user);
            let text = display_name(
                &Person::new(name.clone(), email.clone()),
                is_me,
                NameFormat::First,
                false,
            );
            let contact = Contact {
                name: Some(name.clone()).filter(|name| !name.is_empty()),
                email: email.clone(),
                is_me,
            };
            who.push(Piece {
                text,
                contact: Some(contact),
                ..Piece::default()
            });
        }
        let more = invite.attendees.len().saturating_sub(MAX_WHO);
        if more > 0 {
            who.push(Piece::text(format!(" and {more} more")));
        }
        rows.push(Row {
            question: "Who",
            value: Value::Text(vec![(0.0, who)]),
        });
    }
    if let Some(about) = invite.about() {
        let all = crate::calendar::description::lines(&about);
        if !all.is_empty() {
            let source: Vec<&str> = about
                .split("\r\n")
                .flat_map(|line| line.split(['\r', '\n']))
                .collect();
            let long = source.len() > ABOUT_LINES;
            let shown = if long && !context.about_open {
                crate::calendar::description::lines(&source[..ABOUT_LINES].join("\n"))
            } else {
                all
            };
            let lines = shown
                .into_iter()
                .map(|line| {
                    (
                        line.indent,
                        line.runs
                            .into_iter()
                            .map(|run| Piece {
                                text: run.text,
                                link: run.link,
                                bold: run.bold,
                                contact: None,
                            })
                            .collect(),
                    )
                })
                .collect();
            rows.push(Row {
                question: "About",
                value: Value::About(lines, long.then_some(context.about_open)),
            });
        }
    }
    if !invite.files.is_empty() {
        rows.push(Row {
            question: "Files",
            value: Value::Files(invite.files.clone()),
        });
    }
    rows
}

/// Where's pieces: nothing for a Teams call the video row has; Teams' own words as they are;
/// a phone number to call; the location's links; else a map search.
fn location(invite: &Invite) -> Option<Vec<Piece>> {
    let location = &invite.location;
    if location.is_empty() {
        return None;
    }
    let teams = location
        .trim()
        .eq_ignore_ascii_case("microsoft teams meeting");
    if teams
        && invite
            .meeting
            .as_ref()
            .is_some_and(|meeting| meeting.label == "Teams")
    {
        return None;
    }
    if teams {
        return Some(vec![Piece::text(location)]);
    }
    if let Some(tel) = phone_href(location) {
        return Some(vec![Piece::link(location, tel)]);
    }
    let links = exo_mail::links(location);
    if !links.is_empty() {
        let mut pieces = Vec::new();
        let mut at = 0;
        for (range, url) in links {
            pieces.push(Piece::text(&location[at..range.start]));
            pieces.push(Piece::link(&location[range.clone()], url));
            at = range.end;
        }
        pieces.push(Piece::text(&location[at..]));
        pieces.retain(|piece| !piece.text.is_empty());
        return Some(pieces);
    }
    let query: String = url::form_urlencoded::byte_serialize(location.as_bytes()).collect();
    Some(vec![Piece::link(
        location,
        format!(
            "https://maps.google.com/maps?q={}",
            query.replace('+', "%20")
        ),
    )])
}

/// A `tel:` link for a location that's only a phone number, of 7 to 15 digits.
fn phone_href(location: &str) -> Option<String> {
    static PHONE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\+?[\d\s\-().]+$").expect("valid regex"));
    let location = location.trim();
    if !PHONE.is_match(location) {
        return None;
    }
    let digits: String = location.chars().filter(char::is_ascii_digit).collect();
    (7..=15).contains(&digits.len()).then(|| {
        format!(
            "tel:{}{digits}",
            if location.starts_with('+') { "+" } else { "" }
        )
    })
}

/// Pieces broken into lines no wider than `width` at spaces, as Chromium breaks text.
fn wrap(ctx: &egui::Context, pieces: &[Piece], width: f32) -> Vec<Vec<(Piece, f32)>> {
    wrap_in(
        ctx,
        pieces,
        width,
        |piece| if piece.bold { BOLD } else { TEXT },
    )
}

/// [`wrap`] with each piece in `style`'s face.
fn wrap_in(
    ctx: &egui::Context,
    pieces: &[Piece],
    width: f32,
    style: impl Fn(&Piece) -> TextStyle,
) -> Vec<Vec<(Piece, f32)>> {
    let mut lines: Vec<Vec<(Piece, f32)>> = vec![Vec::new()];
    let mut used = 0.0;
    for piece in pieces {
        for word in piece.text.split_inclusive(' ') {
            let word_piece = Piece {
                text: word.to_owned(),
                ..piece.clone()
            };
            let fit = style(piece).width(ctx, word.trim_end_matches(' '));
            let line = lines.last_mut().expect("a line");
            if !line.is_empty() && used + fit > width {
                lines.push(Vec::new());
                used = 0.0;
            }
            let advance = style(piece).width(ctx, word);
            lines
                .last_mut()
                .expect("a line")
                .push((word_piece, advance));
            used += advance;
        }
    }
    lines
}

/// When's lines, each a list of pieces: the time splits at ` at`, `,` and `– `, and a line breaks
/// only between pieces, each shaped on its own (no kerning across them).
fn when_lines(ctx: &egui::Context, when: &str, width: f32) -> Vec<Vec<String>> {
    static SPLIT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"( at|,|– )").expect("valid regex"));
    let mut pieces = Vec::new();
    let mut at = 0;
    for found in SPLIT.find_iter(when) {
        pieces.extend([&when[at..found.start()], found.as_str()]);
        at = found.end();
    }
    pieces.push(&when[at..]);
    let mut lines: Vec<Vec<String>> = vec![Vec::new()];
    let mut used = 0.0;
    for piece in pieces.into_iter().filter(|piece| !piece.is_empty()) {
        let advance = TEXT.width(ctx, piece);
        if !lines.last().is_some_and(Vec::is_empty) && used + advance > width {
            // A new line drops the spaces it starts with.
            let piece = piece.trim_start();
            used = TEXT.width(ctx, piece);
            lines.push(vec![piece.to_owned()]);
        } else {
            used += advance;
            lines.last_mut().expect("a line").push(piece.to_owned());
        }
    }
    lines
}

/// A row's value's height in a value column `width` wide.
fn value_height(ctx: &egui::Context, value: &Value, width: f32) -> f32 {
    let lines = |lines: &[(f32, Vec<Piece>)]| {
        lines
            .iter()
            .map(|(indent, pieces)| wrap(ctx, pieces, width - indent).len().max(1) as f32)
            .sum::<f32>()
            * TEXT.line_height
    };
    match value {
        Value::Text(text) => lines(text),
        Value::When(when) => when_lines(ctx, when, width).len() as f32 * TEXT.line_height,
        Value::Rsvp(_) | Value::AddToCalendar(_) => TEXT.line_height,
        Value::About(text, more) => {
            lines(text)
                + if more.is_some() {
                    SHOW_MORE_GAP + TEXT.line_height
                } else {
                    0.0
                }
        }
        Value::Files(files) => {
            let (size, _) = file_grid(width);
            files.len().div_ceil(3) as f32 * (size.y + FILE_GAP) - FILE_GAP
        }
    }
}

/// One file card's size, and the gap between, in a value column `width` wide.
fn file_grid(width: f32) -> (egui::Vec2, f32) {
    let card = ((width - 2.0 * FILE_GAP) / 3.0).clamp(120.0, FILE_WIDTH);
    let (base, _) = exo_ui::attachment_grid(4.0 * (FILE_WIDTH + 2.0 * exo_ui::CARD_MARGIN_X));
    (vec2(card, card * base.y / base.x), FILE_GAP)
}

/// The value column's width for a card `width` wide.
fn value_width(width: f32) -> f32 {
    width - 2.0 * ROWS_SIDE - QUESTION - PROPERTY_PADDING
}

/// The card's height for a column `width` wide.
pub(super) fn height(ui: &Ui, invite: &Invite, context: &Context, width: f32) -> f32 {
    let rows = rows(invite, context);
    let values: f32 = rows
        .iter()
        .map(|row| value_height(ui.ctx(), &row.value, value_width(width)))
        .sum();
    IMAGE_HEIGHT + ROWS_TOP + values + (rows.len().saturating_sub(1)) as f32 * ROW_GAP + ROWS_BOTTOM
}

/// The guest's note above the card: the notification's, else the `.ics`'s, read as HTML's text
/// with its spaces collapsed (`white-space: normal`). Present, if empty, for a note of only markup.
pub(super) fn note(invite: &Invite, calendar: Option<&exo_mail::CalendarData>) -> Option<String> {
    let note = calendar
        .and_then(|calendar| calendar.response_comment.as_deref())
        .filter(|note| !note.is_empty())
        .unwrap_or(&invite.note);
    (!note.is_empty()).then(|| {
        exo_mail::html_text(note)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    })
}

/// The note's lines in a column `width` wide, its URLs links (`Linkify`).
fn note_lines(ctx: &egui::Context, note: &str, width: f32) -> Vec<Vec<(Piece, f32)>> {
    if note.is_empty() {
        return Vec::new();
    }
    let mut pieces = Vec::new();
    let mut at = 0;
    for (range, url) in exo_mail::links(note) {
        pieces.extend([
            Piece::text(&note[at..range.start]),
            Piece::link(&note[range.clone()], url),
        ]);
        at = range.end;
    }
    pieces.push(Piece::text(&note[at..]));
    pieces.retain(|piece| !piece.text.is_empty());
    wrap_in(ctx, &pieces, width, |_| NOTE)
}

/// The height the note takes above the card in a column `width` wide.
pub(super) fn note_height(ctx: &egui::Context, note: &str, width: f32) -> f32 {
    note_lines(ctx, note, width).len() as f32 * NOTE.line_height + NOTE_BOTTOM
}

/// Paints the note from `top` (the body area's top) across the column; returns the link
/// clicked. Black in Snow (it inherits the pane's), white at .9 in Carbon.
pub(super) fn show_note(
    ui: &Ui,
    left: f32,
    top: f32,
    width: f32,
    note: &str,
    id: egui::Id,
) -> Option<String> {
    let palette = exo_ui::palette(ui.ctx());
    let mut clicked = None;
    let mut y = top;
    for line in note_lines(ui.ctx(), note, width) {
        let mut x = left;
        for (piece, advance) in line {
            // Links in `#54acdc`.
            let color = if piece.link.is_some() {
                palette.link_text_primary
            } else {
                palette.ink(1.0, 0.9)
            };
            NOTE.paint_line(ui.painter(), pos2(x, y), &piece.text, color, None);
            if let Some(url) = piece.link {
                let rect = Rect::from_min_size(pos2(x, y), vec2(advance, NOTE.line_height));
                if ui
                    .interact(
                        rect,
                        id.with(("note-link", x as i32, y as i32)),
                        Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    clicked = Some(url);
                }
            }
            x += advance;
        }
        y += NOTE.line_height;
    }
    clicked
}

/// The RSVP buttons' answers, in order.
pub(super) const ANSWERS: [Response; 3] =
    [Response::Accepted, Response::Tentative, Response::Declined];

/// What [`show`] found: what was clicked, whether the pointer is on When's text, and where
/// Who's names went, for their tooltips.
#[derive(Default)]
pub(super) struct Shown {
    pub clicked: Option<CardAction>,
    pub on_when: bool,
    pub names: super::message::Named,
}

/// Paints the card at `top` (the body area's top) across the column, `focused` the button with
/// the keyboard.
#[expect(
    clippy::too_many_arguments,
    reason = "one card, drawn from the message and the app's state"
)]
pub(super) fn show(
    ui: &Ui,
    left: f32,
    top: f32,
    width: f32,
    invite: &Invite,
    context: &Context,
    photos: &mut Photos,
    images: &Path,
    id: egui::Id,
    focused: Option<usize>,
) -> Shown {
    let palette = exo_ui::palette(ui.ctx());
    // The card's text: black at .75, white at .85.
    let text_color = palette.ink(0.75, 0.85);
    let painter = ui.painter();
    // `top: 1px`.
    let card = Rect::from_min_size(
        pos2(left, top + 1.0),
        vec2(width, height(ui, invite, context, width)),
    );
    painter.rect_filled(
        exo_ui::pixel_snap(card),
        CornerRadius::same(4),
        palette.fill_base_accent,
    );
    // The photo covers 640×176, centred (`background-size: cover; background-position-x: 50%`).
    let photo = Rect::from_min_size(card.min, vec2(width, IMAGE_HEIGHT));
    if let Some(texture) = photos.get(ui.ctx(), images, image_kind(invite)) {
        let [w, h] = texture.size().map(|side| side as f32);
        let scale = (photo.width() / w).max(photo.height() / h);
        let (u, v) = (photo.width() / (w * scale), photo.height() / (h * scale));
        let uv = Rect::from_min_max(pos2((1.0 - u) / 2.0, 0.0), pos2((1.0 + u) / 2.0, v));
        painter.add(
            RectShape::filled(
                exo_ui::pixel_snap(photo),
                CornerRadius {
                    nw: 4,
                    ne: 4,
                    sw: 0,
                    se: 0,
                },
                Color32::WHITE,
            )
            .with_texture(texture.id(), uv),
        );
    }
    if let Some(status) = status(invite, context) {
        paint_status(ui, photo, &status);
    }

    let value_left = card.left() + ROWS_SIDE + QUESTION + PROPERTY_PADDING;
    let value_width = value_width(width);
    let mut row_top = photo.bottom() + ROWS_TOP;
    let mut shown = Shown::default();
    let pointer = ui
        .ctx()
        .pointer_hover_pos()
        .filter(|_| ui.ui_contains_pointer());
    let link = |rect: Rect, key: &str| {
        ui.interact(
            rect,
            id.with(("link", key, rect.min.x as i32, rect.min.y as i32)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    };
    for row in rows(invite, context) {
        if !row.question.is_empty() {
            BOLD.paint_line(
                painter,
                pos2(card.left() + ROWS_SIDE, row_top),
                row.question,
                text_color,
                None,
            );
        }
        let height = value_height(ui.ctx(), &row.value, value_width);
        match &row.value {
            Value::Text(lines) | Value::About(lines, _) => {
                let mut y = row_top;
                for (indent, pieces) in lines {
                    for line in wrap(ui.ctx(), pieces, value_width - indent) {
                        let mut x = value_left + indent;
                        for (piece, advance) in line {
                            let (style, color) = (
                                if piece.bold { BOLD } else { TEXT },
                                if piece.link.is_some() {
                                    palette.link_text_primary
                                } else {
                                    text_color
                                },
                            );
                            style.paint_line(painter, pos2(x, y), &piece.text, color, None);
                            if let Some(url) = &piece.link
                                && link(
                                    Rect::from_min_size(
                                        pos2(x, y),
                                        vec2(advance, TEXT.line_height),
                                    ),
                                    row.question,
                                )
                            {
                                shown.clicked = Some(CardAction::Open(url.clone()));
                            }
                            // A contact name's box is its text's 17px content area, 3px down the line
                            // (a name's words, with the spaces between them).
                            if let Some(contact) = piece.contact {
                                shown.names.push((
                                    Rect::from_min_size(pos2(x, y + 3.0), vec2(advance, 17.0)),
                                    contact,
                                ));
                            }
                            x += advance;
                        }
                        y += TEXT.line_height;
                    }
                }
                if let Value::About(_, Some(open)) = row.value {
                    // `Show more`: bold, `rgba(0, 0, 0, .35)`, Carbon white at .45.
                    let label = if open { "Show Less" } else { "Show more" };
                    let top = row_top + height - TEXT.line_height;
                    let width = BOLD.paint_line(
                        painter,
                        pos2(value_left, top),
                        label,
                        palette.ink(0.35, 0.45),
                        None,
                    );
                    if ui
                        .interact(
                            Rect::from_min_size(
                                pos2(value_left, top),
                                vec2(width, TEXT.line_height),
                            ),
                            id.with("show-more"),
                            Sense::click(),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        shown.clicked = Some(CardAction::ToggleAbout);
                    }
                }
            }
            Value::When(when) => {
                let mut y = row_top;
                for line in when_lines(ui.ctx(), when, value_width) {
                    let mut x = value_left;
                    for piece in line {
                        TEXT.paint_line(painter, pos2(x, y), &piece, text_color, None);
                        x += TEXT.width(ui.ctx(), &piece);
                    }
                    shown.on_when |= pointer.is_some_and(|at| {
                        Rect::from_x_y_ranges(value_left..=x, y..=y + TEXT.line_height).contains(at)
                    });
                    y += TEXT.line_height;
                }
            }
            Value::Rsvp(answer) => {
                let mut x = card.left() + ROWS_SIDE + QUESTION;
                // Upper-cased labels in Classic.
                for (index, (label, response)) in
                    ["YES", "MAYBE", "NO"].into_iter().zip(ANSWERS).enumerate()
                {
                    let selected = *answer == Some(response);
                    let (button, clicked) = rsvp_button(
                        ui,
                        palette,
                        id.with(label),
                        pos2(x, row_top),
                        label,
                        selected,
                        focused == Some(index),
                    );
                    // Answering again with the current answer does nothing.
                    if clicked && !selected {
                        shown.clicked = Some(CardAction::Respond(response));
                    }
                    x = button.right() + BUTTON_GAP;
                }
            }
            Value::AddToCalendar(selected) => {
                let (_, clicked) = rsvp_button(
                    ui,
                    palette,
                    id.with("add"),
                    pos2(card.left() + ROWS_SIDE + QUESTION, row_top),
                    "ADD TO CALENDAR",
                    *selected,
                    focused == Some(0),
                );
                if clicked && !selected {
                    shown.clicked = Some(CardAction::Respond(Response::NeedsAction));
                }
            }
            Value::Files(files) => {
                let (size, gap) = file_grid(value_width);
                for (at, file) in files.iter().enumerate() {
                    let min = pos2(
                        value_left + (at % 3) as f32 * (size.x + gap),
                        row_top + (at / 3) as f32 * (size.y + gap),
                    );
                    let rect = Rect::from_min_size(min, size);
                    let response = ui
                        .interact(rect, id.with(("file", at)), Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    let hover = exo_ui::motion::transition(
                        ui.ctx(),
                        response.id.with("hover"),
                        response.hovered(),
                        0.25,
                        exo_ui::motion::CubicBezier::EASE,
                    );
                    exo_ui::paint_attachment_card(ui, rect, &file.name, &file.mime, hover);
                    if response.clicked() {
                        shown.clicked = Some(CardAction::Open(file.url.clone()));
                    }
                }
            }
        }
        row_top += height + ROW_GAP;
    }
    shown
}

/// The status over the photo: white at .95 with a hairline border, the
/// text bold at .75 in both themes.
fn paint_status(ui: &Ui, photo: Rect, status: &str) {
    let painter = ui.painter();
    let max = photo.width() - 24.0 - 2.0 * STATUS_PADDING_X - 2.0;
    let lines = BOLD.wrap(ui.ctx(), status, max);
    let width = lines
        .iter()
        .map(|line| BOLD.width(ui.ctx(), line))
        .fold(0.0, f32::max);
    let rect = Rect::from_min_size(
        photo.min + vec2(STATUS_LEFT, STATUS_TOP),
        vec2(
            width + 2.0 * STATUS_PADDING_X + 2.0,
            1.0 + STATUS_PADDING_TOP + lines.len() as f32 * BOLD.line_height + 1.0,
        ),
    );
    // The background runs under the border (`background-clip: border-box`).
    let snapped = exo_ui::pixel_snap(rect);
    painter.rect_filled(
        snapped,
        CornerRadius::same(4),
        Color32::from_white_alpha(242),
    );
    painter.rect_stroke(
        snapped,
        CornerRadius::same(4),
        Stroke::new(1.0, Color32::from_black_alpha(20)),
        egui::StrokeKind::Inside,
    );
    for (at, line) in lines.iter().enumerate() {
        BOLD.paint_line(
            painter,
            rect.min
                + vec2(
                    1.0 + STATUS_PADDING_X,
                    1.0 + STATUS_PADDING_TOP + at as f32 * BOLD.line_height,
                ),
            line,
            Color32::from_black_alpha(191),
            None,
        );
    }
}

/// One RSVP button with its row's line box at `at`: returns its box and whether it was clicked.
fn rsvp_button(
    ui: &Ui,
    palette: &Palette,
    id: egui::Id,
    at: egui::Pos2,
    label: &str,
    selected: bool,
    focused: bool,
) -> (Rect, bool) {
    let painter = ui.painter();
    let text = BOLD
        .single_line(painter, label, Color32::PLACEHOLDER, None)
        .size()
        .x;
    let button = Rect::from_min_size(
        pos2(at.x, at.y - 4.0 + 1.0),
        vec2(text + 2.0 * BUTTON_PADDING, BUTTON_HEIGHT),
    );
    // A pointer cursor over the button.
    let response = ui
        .interact(button, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let color = paint_rsvp_button(
        painter,
        palette,
        exo_ui::pixel_snap(button),
        response.hovered(),
        selected,
        focused,
    );
    // The label sits a pixel below the row's other text (measured).
    BOLD.paint_line(
        painter,
        pos2(button.left() + BUTTON_PADDING, at.y + 1.0),
        label,
        color,
        None,
    );
    (button, response.clicked())
}

/// Paints an RSVP button's box at `rect` and returns its label's color. Hovered, focused and the
/// current answer fill with the link blue; hovered ones cast a gray shadow and focused ones glow,
/// unless they're the answer, which only glows when focused, and takes a blue label.
fn paint_rsvp_button(
    painter: &egui::Painter,
    palette: &Palette,
    rect: Rect,
    hovered: bool,
    selected: bool,
    focused: bool,
) -> Color32 {
    let radius = CornerRadius::same(2);
    if focused {
        for shadow in [
            if selected {
                SELECTED_FOCUS_GLOW
            } else {
                FOCUS_GLOW
            },
            FOCUS_HAZE,
        ] {
            shadow.paint(painter, rect, radius);
        }
    } else if hovered && !selected {
        // `box-shadow: rgba(0, 0, 0, .2) 0 0 8px`, `.5` in Carbon.
        BoxShadow::new(
            [0.0, 0.0],
            8.0,
            0.0,
            Color32::from_black_alpha(if palette.theme == Theme::Carbon {
                128
            } else {
                51
            }),
        )
        .paint(painter, rect, radius);
    }
    if hovered || selected || focused {
        painter.rect_filled(rect, radius, palette.button_ghost_fill_active);
    }
    if selected {
        // The answer's label: `rgba(84, 172, 220, .9)`, `.8` in Carbon.
        Color32::from_rgba_unmultiplied(
            0x54,
            0xac,
            0xdc,
            if palette.theme == Theme::Carbon {
                204
            } else {
                230
            },
        )
    } else if hovered || focused {
        // Hovered or focused: `.35` in both.
        palette.ink(0.35, 0.35)
    } else {
        palette.button_ghost_content_default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICS: &str = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nDTSTART:20261125T140000Z\r\nDTEND:20261125T144500Z\r\nORGANIZER;CN=Grace Liu:mailto:grace@juniperpartners.vc\r\nATTENDEE;PARTSTAT=ACCEPTED;CN=Grace Liu:mailto:grace@juniperpartners.vc\r\nATTENDEE;PARTSTAT=NEEDS-ACTION;CN=Maya Okafor:mailto:maya@brightmoor.co\r\nLOCATION:Harborlight Coffee\\, 5th Ave\r\nSUMMARY:Coffee with Grace\r\n DESCRIPTION-folded\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn context<'a>(message: &'a MessageDetail, now: &'a Zoned) -> Context<'a> {
        Context {
            me: "maya@brightmoor.co",
            user: "maya@brightmoor.co",
            now,
            message,
            calendar: None,
            about_open: false,
        }
    }

    fn message(subject: &str) -> MessageDetail {
        MessageDetail {
            id: "m".into(),
            labels: Vec::new(),
            date: Timestamp::UNIX_EPOCH,
            from: None,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: subject.to_owned(),
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

    #[test]
    fn an_invite_is_to_the_mail_account_or_else_the_calendar_account_on_it() {
        let mut invite = parse(ICS).expect("an event");
        let others = ["maya.lin@example.com".to_owned()];
        assert_eq!(
            invite.calendar_account("maya@brightmoor.co", &others),
            "maya@brightmoor.co"
        );
        let mine = invite
            .attendees
            .iter_mut()
            .find(|(_, email, _)| email == "maya@brightmoor.co")
            .expect("on it");
        mine.1 = "Maya.Lin@example.com".into();
        assert_eq!(
            invite.calendar_account("maya@brightmoor.co", &others),
            "Maya.Lin@example.com",
            "as the invite spells it"
        );
        assert_eq!(
            invite.calendar_account("maya@brightmoor.co", &[]),
            "maya@brightmoor.co",
            "none of mine: the mail account"
        );
    }

    #[test]
    fn reads_an_invitation() {
        let invite = parse(ICS).expect("an event");
        assert!(invite.request);
        assert_eq!(
            invite.summary, "Coffee with GraceDESCRIPTION-folded",
            "folded lines join"
        );
        assert_eq!(invite.location, "Harborlight Coffee, 5th Ave");
        assert_eq!(invite.attendees.len(), 2);
        assert_eq!(invite.attendees[1].2, Response::NeedsAction);
        let now: Zoned = "2026-11-17T09:00:00[America/New_York]"
            .parse()
            .expect("valid");
        assert_eq!(
            dates::calendar_range(invite.start, invite.end, &now),
            "Wednesday, November 25th, 2026 at 9:00 AM – 9:45 AM EST"
        );
        assert_eq!(image_kind(&invite), "coffee");
        assert!(
            invite.can_respond("maya@brightmoor.co")
                && !invite.can_add_to_calendar("maya@brightmoor.co")
        );
        assert!(
            !invite.can_respond("ana@example.com") && invite.can_add_to_calendar("ana@example.com"),
            "an invite I'm not on adds to my calendar"
        );
    }

    #[test]
    fn photos_follow_the_title() {
        assert_eq!(kind("Weekly planning", 3), Some("meeting"));
        assert_eq!(kind("Quick call", 2), Some("call"));
        assert_eq!(kind("Video call with Ana", 2), None);
        assert_eq!(kind("Maya <> Yusuf", 2), Some("1on1"));
        assert_eq!(kind("Board prep", 4), None);
    }

    #[test]
    fn every_kind_has_its_photo() {
        let titles = [
            "Swim",
            "Birthday",
            "Standup",
            "Quick call",
            "Coffee",
            "Dinner",
            "Drinks",
            "Gym",
            "Flight",
            "Hike",
            "Brunch",
            "Lunch",
            "Party",
            "Matcha",
            "1:1",
        ];
        let kinds: std::collections::HashSet<&str> = titles
            .iter()
            .map(|title| kind(title, 3).expect("a kind"))
            .chain(["default"])
            .collect();
        assert_eq!(kinds.len(), 16, "one title for each kind");
        let images = crate::assets_dir().join("images");
        for kind in kinds {
            assert!(
                images.join(format!("invite-{kind}.jpg")).is_file(),
                "no photo for {kind}"
            );
        }
    }

    #[test]
    fn the_status_says_what_the_notification_is() {
        let now: Zoned = "2026-11-17T09:00:00[America/New_York]"
            .parse()
            .expect("valid");
        let invite = parse(&ICS.replace("METHOD:REQUEST", "METHOD:CANCEL")).expect("an event");
        let plain = message("Invitation: Coffee");
        assert_eq!(
            status(&invite, &context(&plain, &now)).as_deref(),
            Some("Event has been canceled")
        );
        assert!(
            !invite.can_respond("maya@brightmoor.co")
                && !invite.can_add_to_calendar("maya@brightmoor.co")
        );

        let invite = parse(ICS).expect("an event");
        let updated = message("Updated Invitation: Coffee");
        assert_eq!(
            status(&invite, &context(&updated, &now)).as_deref(),
            Some("Event has been changed")
        );
        let calendar = exo_mail::CalendarData {
            gcal: true,
            updated_fields: Some("time".into()),
            ..Default::default()
        };
        let with_fields = Context {
            calendar: Some(&calendar),
            ..context(&updated, &now)
        };
        assert_eq!(
            status(&invite, &with_fields).as_deref(),
            Some("Event has been changed - time")
        );
        // What changed alone isn't an update.
        assert_eq!(
            status(
                &invite,
                &Context {
                    calendar: Some(&calendar),
                    ..context(&plain, &now)
                }
            ),
            None
        );
        assert_eq!(status(&invite, &context(&plain, &now)), None);

        let reply = "BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nBEGIN:VEVENT\r\nDTSTART:20261125T140000Z\r\nDTEND:20261125T144500Z\r\nATTENDEE;PARTSTAT=TENTATIVE;CN=Grace Liu:mailto:grace@juniperpartners.vc\r\nUID:x\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let invite = parse(reply).expect("an event");
        assert_eq!(
            status(&invite, &context(&plain, &now)).as_deref(),
            Some("Grace has responded maybe")
        );
        assert!(
            !invite.can_add_to_calendar("maya@brightmoor.co"),
            "an answer isn't added"
        );
        let counter = parse(&reply.replace("METHOD:REPLY", "METHOD:COUNTER")).expect("an event");
        assert_eq!(
            status(&counter, &context(&plain, &now)).as_deref(),
            Some("Grace has proposed a new time")
        );
    }

    #[test]
    fn a_guests_note_is_their_response_comment_else_the_events_comment() {
        let reply = "BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nBEGIN:VEVENT\r\nDTSTART:20261125T140000Z\r\nDTEND:20261125T144500Z\r\nCOMMENT:Agenda\\, attached\r\nATTENDEE;CN=Grace Liu;X-RESPONSE-COMMENT=\"Late: ^'5 min^' ^^_^^ \";PARTSTAT=ACCEPTED:mailto:grace@juniperpartners.vc\r\nUID:x\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let invite = parse(reply).expect("an event");
        assert_eq!(
            invite.note, "Late: \"5 min\" ^_^",
            "quoted, with its `:` and RFC 6868's escapes"
        );
        assert_eq!(
            invite.attendees[0],
            (
                "Grace Liu".to_owned(),
                "grace@juniperpartners.vc".to_owned(),
                Response::Accepted
            )
        );
        let without = parse(&reply.replace(";X-RESPONSE-COMMENT=\"Late: ^'5 min^' ^^_^^ \"", ""))
            .expect("an event");
        assert_eq!(without.note, "Agenda, attached");
        // With more than one attendee, only the event's comment counts.
        let two = parse(&reply.replace(
            "UID:x",
            "ATTENDEE;CN=Maya:mailto:maya@brightmoor.co\r\nUID:x",
        ))
        .expect("an event");
        assert_eq!(two.note, "Agenda, attached");

        // The notification's own comment wins, read as HTML's text, its spaces collapsed.
        let calendar = exo_mail::CalendarData {
            response_comment: Some("See  you\nthere &amp; then".into()),
            ..Default::default()
        };
        assert_eq!(
            note(&invite, Some(&calendar)).as_deref(),
            Some("See you there & then")
        );
        assert_eq!(note(&invite, None).as_deref(), Some("Late: \"5 min\" ^_^"));
        assert_eq!(note(&parse(ICS).expect("an event"), None), None);
    }

    #[test]
    fn finds_the_video_call_and_links_in_the_description() {
        let description = "Agenda <https://docs.example/a>\n\nJoin with Google Meet: https://meet.google.com/abc-defg-hij\nOr dial: +1 555";
        let (text, meeting) = extract_meeting(description);
        assert_eq!(
            meeting,
            Some(Meeting {
                label: "Meet",
                url: "https://meet.google.com/abc-defg-hij".into()
            })
        );
        assert!(
            text.starts_with(
                "Agenda  <a href=\"https://docs.example/a\">https://docs.example/a</a>"
            ),
            "{text}"
        );
        let teams = "________________\nMicrosoft Teams meeting\nJoin on your computer\nClick here to join the meeting<https://teams.microsoft.com/l/meetup-join/19%3a>\n________________";
        let (text, meeting) = extract_meeting(teams);
        assert_eq!(meeting.map(|meeting| meeting.label), Some("Teams"));
        assert!(text.contains("<a href=\"https://teams.microsoft.com/l/meetup-join/19%3a\">Click here to join the meeting</a>"), "{text}");
    }

    #[test]
    fn where_is_a_map_a_number_or_its_links() {
        let mut invite = parse(ICS).expect("an event");
        let link = |invite: &Invite| {
            location(invite).map(|pieces| {
                pieces
                    .into_iter()
                    .filter_map(|piece| piece.link)
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(
            link(&invite),
            Some(vec![
                "https://maps.google.com/maps?q=Harborlight%20Coffee%2C%205th%20Ave".to_owned()
            ])
        );
        invite.location = "+1 (555) 010-2030".into();
        assert_eq!(link(&invite), Some(vec!["tel:+15550102030".to_owned()]));
        invite.location = "Zoom: https://zoom.us/j/1".into();
        assert_eq!(link(&invite), Some(vec!["https://zoom.us/j/1".to_owned()]));
        invite.location = "Microsoft Teams Meeting".into();
        assert_eq!(link(&invite), Some(Vec::new()));
        invite.meeting = Some(Meeting {
            label: "Teams",
            url: "https://teams.microsoft.com/l/meetup-join/x".into(),
        });
        assert_eq!(link(&invite), None, "the video row has it");
    }

    #[test]
    fn whos_names_carry_their_contacts_for_tooltips() {
        let now: Zoned = "2026-11-17T09:00:00[America/New_York]"
            .parse()
            .expect("valid");
        let invite = parse(ICS).expect("an event");
        let plain = message("Invitation: Coffee");
        let who = rows(&invite, &context(&plain, &now))
            .into_iter()
            .find_map(|row| match row.value {
                Value::Text(lines) if row.question == "Who" => Some(
                    lines
                        .into_iter()
                        .flat_map(|(_, pieces)| pieces)
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            });
        let who = who.expect("a Who row");
        let names: Vec<_> = who
            .iter()
            .map(|piece| {
                (
                    piece.text.as_str(),
                    piece.contact.as_ref().map(|contact| contact.email.as_str()),
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                ("Grace", Some("grace@juniperpartners.vc")),
                (", ", None),
                ("Me", Some("maya@brightmoor.co"))
            ]
        );
        assert_eq!(
            who[0]
                .contact
                .as_ref()
                .and_then(|contact| contact.name.as_deref()),
            Some("Grace Liu"),
            "the tooltip's full name"
        );
    }

    #[test]
    fn about_shows_eight_lines_until_show_more() {
        let now: Zoned = "2026-11-17T09:00:00[America/New_York]"
            .parse()
            .expect("valid");
        let mut invite = parse(ICS).expect("an event");
        invite.description = (1..=10)
            .map(|n| format!("Line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let plain = message("Invitation: Coffee");
        let about = |open: bool| {
            rows(
                &invite,
                &Context {
                    about_open: open,
                    ..context(&plain, &now)
                },
            )
            .into_iter()
            .find_map(|row| match row.value {
                Value::About(lines, more) => Some((lines.len(), more)),
                _ => None,
            })
        };
        assert_eq!(about(false), Some((8, Some(false))));
        assert_eq!(about(true), Some((10, Some(true))));
    }
}
