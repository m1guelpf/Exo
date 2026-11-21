//! What the sanitizer notes about a message as it walks it: one-click actions (schema.org
//! `ld+json` and microdata, and links it recognizes) and the facts in a Google Calendar
//! notification.

use std::sync::LazyLock;

use regex::Regex;

use crate::dom::{self, NodeRef};

/// A one-click action: what it says and where it goes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Action {
    /// `View order`, `Track Order`, `Open Document`…
    pub text: String,
    pub url: String,
    /// Where it was declared, for schema.org actions (`source`); `None` for a recognized link.
    pub source: Option<ActionSource>,
}

/// How a message declared an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionSource {
    /// A `<script type="application/ld+json">`.
    LdJson,
    /// `itemtype="http://schema.org/…"` markup.
    Microdata,
}

/// What a Google Calendar notification says about its event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CalendarData {
    /// The body is Google Calendar's (`source: "gcal"`): it carries
    /// `<meta itemprop="name" content="Google Calendar">`.
    pub gcal: bool,
    /// Its `eventStatus` microdata.
    pub event_status: Option<EventStatus>,
    /// A guest's note with their answer (the line after the info bar's `<br>`, unquoted).
    pub response_comment: Option<String>,
    /// What an update changed (the info bar's `Changed: …`, without `Changed:`).
    pub updated_fields: Option<String>,
}

/// schema.org's `EventStatusType`, the two that are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventStatus {
    Cancelled,
    Rescheduled,
}

/// The schema.org action types shown, and the text one without a name gets.
const SUPPORTED: [(&str, &str); 2] = [("ViewAction", "View on "), ("TrackAction", "Track on ")];

/// Links that manage an order.
static MANAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(manage|see)(\s)+((this|my|your|full)\s)?(Shipping|Shipment|Tracking|Package|Parcel|Order|Item){1}(\s)*(details)?").expect("valid regex")
});
/// Links that track one.
static TRACK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(track|see|view|where's)(\s)+((this|my|your)\s)?(shipping|shipment|tracking(\s)(number|details)|package|parcel|order|item)").expect("valid regex")
});
static ISSUE_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[a-z]+-[0-9]+").expect("valid regex"));
static DOMINOS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:[a-z]+/){3}tracker/").expect("valid regex"));
static USPS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"((\d{4}\s){5}\d{2})|(\d{34})").expect("valid regex"));
static UPS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)1Z[a-z0-9]{16}").expect("valid regex"));
static DIGITS_12: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{12}").expect("valid regex"));
static DIGITS_20: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{20}").expect("valid regex"));
static QUOTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^"([\s\S]*)"$"#).expect("valid regex"));

/// What the walk has found so far.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub actions: Vec<Action>,
    pub calendar: CalendarData,
    /// The walk has passed the root element. The calendar's facts are looked for under the root
    /// alone: whatever lies under a later element also lies under it.
    looked_for_calendar: bool,
}

impl Found {
    /// Looks at element `node` (`tag`, lowercased), before it's sanitized.
    pub(crate) fn visit(&mut self, node: &NodeRef, tag: &str) {
        if dom::has_attr(node, "parsed") {
            return;
        }
        if let Some(action) = action(node, tag) {
            self.actions.push(action);
        }
        if !self.looked_for_calendar {
            self.looked_for_calendar = true;
            self.calendar = calendar_data(node);
        }
        dom::set_attr(node, "parsed", "true");
    }
}

/// The action `node` declares or is.
fn action(node: &NodeRef, tag: &str) -> Option<Action> {
    if dom::attr(node, "type").as_deref() == Some("application/ld+json") {
        return ld_json(node);
    }
    if dom::attr(node, "itemtype").is_some_and(|kind| kind.starts_with("http://schema.org/")) {
        return microdata(node);
    }
    (tag == "a").then(|| link(node)).flatten()
}

/// An `ld+json` script's action (one with a `@context` of `http://schema.org…` and a
/// `potentialAction` or `action`).
fn ld_json(node: &NodeRef) -> Option<Action> {
    let text = dom::text_content(&dom::parse(&dom::text_content(node)));
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let valid = |item: &serde_json::Value| {
        item["@context"]
            .as_str()
            .is_some_and(|context| context.starts_with("http://schema.org"))
            && (truthy(&item["potentialAction"]) || truthy(&item["action"]))
    };
    let item = match &json {
        serde_json::Value::Array(items) => items.iter().find(|item| valid(item))?,
        item => item,
    };
    if !valid(item) {
        return None;
    }
    let action = if truthy(&item["potentialAction"]) {
        &item["potentialAction"]
    } else {
        &item["action"]
    };
    let url = [&action["target"], &action["url"], &action["handler"]["url"]]
        .into_iter()
        .find(|value| truthy(value))?
        .as_str()?;
    metadata_action(
        url,
        action["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map(str::to_owned),
        action["@type"].as_str()?,
        ActionSource::LdJson,
    )
}

/// A JavaScript truthy value, as `a || b` picks.
fn truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        serde_json::Value::String(text) => !text.is_empty(),
        _ => true,
    }
}

/// The first supported microdata action at or under `node`, marked so it isn't read again.
fn microdata(node: &NodeRef) -> Option<Action> {
    let (found, kind) = SUPPORTED.iter().find_map(|(kind, _)| {
        let wanted = format!("http://schema.org/{kind}");
        let matches = |n: &NodeRef| {
            dom::attr(n, "itemtype").as_deref() == Some(wanted.as_str())
                && dom::attr(n, "parsed").as_deref() != Some("true")
        };
        let found = if matches(node) {
            Some(node.clone())
        } else {
            dom::select_first(node, matches)
        };
        Some((found?, *kind))
    })?;
    let link = dom::select_first(&found, |n| {
        dom::is(n, "meta") && dom::attr(n, "itemprop").as_deref() == Some("url")
    })
    .or_else(|| dom::select_first(&found, |n| dom::is(n, "link")))
    .or_else(|| dom::select_first(&found, |n| dom::is(n, "a")));
    let url =
        link.and_then(|link| dom::attr(&link, "href").or_else(|| dom::attr(&link, "content")));
    let name = dom::select_first(&found, |n| {
        (dom::is(n, "meta") || dom::is(n, "span"))
            && dom::attr(n, "itemprop").as_deref() == Some("name")
    });
    let text = name.map(|name| {
        let text = dom::attr(&name, "content")
            .filter(|content| !content.is_empty())
            .unwrap_or_else(|| dom::text_content(&name));
        // lodash's `capitalize` on each word.
        text.split(' ')
            .map(|word| upper_first(&word.to_lowercase()))
            .collect::<Vec<_>>()
            .join(" ")
    });
    dom::set_attr(&found, "parsed", "true");
    metadata_action(
        &url?,
        text.filter(|text| !text.is_empty()),
        kind,
        ActionSource::Microdata,
    )
}

/// A supported action with a URL, named after its host when it has no name (`View on Google Docs`).
fn metadata_action(
    url: &str,
    text: Option<String>,
    kind: &str,
    source: ActionSource,
) -> Option<Action> {
    let (_, prefix) = SUPPORTED.iter().find(|(supported, _)| *supported == kind)?;
    if url.is_empty() {
        return None;
    }
    let text = match text {
        Some(text) => text,
        None => {
            let host = url::Url::parse(url).ok()?.host_str()?.to_owned();
            let host = if host == "docs.google.com" {
                "Google Docs".to_owned()
            } else {
                host
            };
            format!("{prefix}{host}")
        }
    };
    Some(Action {
        text,
        url: url.to_owned(),
        source: Some(source),
    })
}

/// A link recognized as an action: an issue, an order, a shipment, a Google file.
fn link(node: &NodeRef) -> Option<Action> {
    let href = dom::attr(node, "href")?;
    let mut text = dom::text_content(node).trim().to_owned();
    if text.is_empty() {
        text = dom::select(node, |n| dom::has_attr(n, "alt"))
            .iter()
            .filter_map(|n| dom::attr(n, "alt"))
            .collect::<String>()
            .trim()
            .to_owned();
    }
    if href.is_empty()
        || text.is_empty()
        || ["pixel", ".gif", ".png", ".jpg", ".jpeg", ".svg"]
            .iter()
            .any(|token| href.contains(token))
    {
        return None;
    }
    let action = |text: &str| {
        Some(Action {
            text: text.to_owned(),
            url: href.clone(),
            source: None,
        })
    };
    let google_content = [
        "Open",
        "Respond",
        "Open sharing settings",
        "Open in Keep",
        "call back",
        "play message",
    ]
    .contains(&text.as_str());
    let docs = href.starts_with("https://docs.google.com/")
        || href.starts_with("https://drive.google.com/");
    let account = href.starts_with("https://accounts.google.com/")
        && (text == "Check activity" || text == "Take action");
    if is_issue(&href, &text) {
        action("View Issue")
    } else if MANAGE.is_match(&text)
        || (href.contains("amazon.com") && text.contains("View or manage order"))
    {
        action("Manage Order")
    } else if (href.to_lowercase().contains("dominos.com") && DOMINOS.is_match(&text))
        || is_tracking(&href.to_lowercase(), &text)
    {
        action("Track Order")
    } else if docs
        && google_content
        && (href.contains("ownershiptransfer") || href.contains("userstoinvite="))
    {
        action("Open Sharing Settings")
    } else if href.starts_with("https://keep.google.com/")
        && href.find("note").is_some_and(|at| at > 0)
        && google_content
    {
        action("Open Note")
    } else if (href.contains("https://voice.google.com/") && google_content) || account {
        // Each word capitalized.
        action(
            &text
                .split_whitespace()
                .map(upper_first)
                .collect::<Vec<_>>()
                .join(" "),
        )
    } else if docs && google_content {
        if href.contains("comment_email_discussion") || href.contains("todo_email_discussion") {
            return action("Open Discussion");
        }
        let kind = match href.split('/').nth(3).unwrap_or_default() {
            "presentation" | "slides" => "Presentation",
            "spreadsheets" => "Spreadsheet",
            "forms" => "Form",
            "drive" => "Folder",
            "keep" => "Note",
            _ => "Document",
        };
        action(&format!("Open {kind}"))
    } else {
        None
    }
}

fn is_issue(href: &str, text: &str) -> bool {
    ((href.contains("linear.app/") && href.contains("/issue/"))
        || href.contains("atlassian.net/browse/"))
        && ISSUE_KEY.is_match(text)
}

/// Whether a link tracks a shipment; `href` is lowercased.
fn is_tracking(href: &str, text: &str) -> bool {
    if href.contains("linear.app/") {
        return false;
    }
    TRACK.is_match(text)
        || (href.contains("tracking") && text.contains("track"))
        || (href.contains("track") && text.contains("order"))
        || (href.contains("usps.com") && USPS.is_match(text))
        || (href.contains("ups.com") && UPS.is_match(text))
        || (href.contains("fedextrack") && DIGITS_12.is_match(text))
        || (href.contains("dhl.") && (DIGITS_20.is_match(text) || text.starts_with(" >> Click")))
        || ((href.contains("/track/") || href.contains("shipstation"))
            && (DIGITS_12.is_match(text) || DIGITS_20.is_match(text)))
        || (href.contains("apple.com") && href.contains("vieworder"))
        || href.starts_with("https://parcel-api.delivery-status.com")
}

fn upper_first(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// A Google Calendar notification's facts, from the root element of the message.
fn calendar_data(root: &NodeRef) -> CalendarData {
    let itemprop = |n: &NodeRef, name: &str| {
        dom::is(n, "meta") && dom::attr(n, "itemprop").as_deref() == Some(name)
    };
    if dom::select_first(root, |n| {
        itemprop(n, "name") && dom::attr(n, "content").as_deref() == Some("Google Calendar")
    })
    .is_none()
    {
        return CalendarData::default();
    }
    let event_status = dom::select_first(root, |n| itemprop(n, "eventStatus")).and_then(|meta| {
        match dom::attr(&meta, "content")?.as_str() {
            "http://schema.org/EventCancelled" => Some(EventStatus::Cancelled),
            "http://schema.org/EventRescheduled" => Some(EventStatus::Rescheduled),
            _ => None,
        }
    });
    let in_info_bar = |n: &NodeRef| {
        dom::parent(n).is_some_and(|parent| dom::has_class(&parent, "info-bar-inner"))
    };
    // `.info-bar-inner > br`, else `h4 > strong + br`.
    let br = dom::select_first(root, |n| dom::is(n, "br") && in_info_bar(n)).or_else(|| {
        dom::select_first(root, |n| {
            dom::is(n, "br")
                && dom::parent(n).is_some_and(|parent| dom::is(&parent, "h4"))
                && dom::previous_element_sibling(n)
                    .is_some_and(|previous| dom::is(&previous, "strong"))
        })
    });
    let response_comment = br.and_then(|br| {
        let siblings = dom::following_siblings(&br);
        // The whole text: the text node and the ones adjoining it.
        let text: String = siblings.iter().map_while(dom::text).collect();
        (!siblings.is_empty() && dom::is_text(&siblings[0]))
            .then(|| QUOTED.replace(&text, "$1").trim().to_owned())
    });
    let updated_fields =
        dom::select_first(root, |n| dom::is(n, "div") && in_info_bar(n)).map(|div| {
            dom::text_content(&div)
                .replacen("Changed:", "", 1)
                .trim()
                .to_owned()
        });
    // Empty text counts as none (`a || b`).
    let (response_comment, updated_fields) = (
        response_comment.filter(|text| !text.is_empty()),
        updated_fields.filter(|text| !text.is_empty()),
    );
    CalendarData {
        gcal: true,
        event_status,
        response_comment,
        updated_fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(html: &str) -> Found {
        let document = dom::parse(html);
        let mut found = Found::default();
        for node in dom::descendants(&document).filter(dom::is_element) {
            let tag = dom::tag(&node)
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();
            found.visit(&node, &tag);
        }
        found
    }

    fn texts(html: &str) -> Vec<(String, String)> {
        found(html)
            .actions
            .into_iter()
            .map(|action| (action.text, action.url))
            .collect()
    }

    #[test]
    fn reads_ld_json_actions() {
        let html = r#"<script type="application/ld+json">{"@context":"http://schema.org","@type":"EmailMessage","potentialAction":{"@type":"ViewAction","target":"https://shop.example/orders/42","name":"View order"}}</script>"#;
        let actions = found(html).actions;
        assert_eq!(
            actions,
            [Action {
                text: "View order".into(),
                url: "https://shop.example/orders/42".into(),
                source: Some(ActionSource::LdJson)
            }]
        );
        // Unnamed, it's named after the host; `https://schema.org` isn't recognized.
        let unnamed = r#"<script type="application/ld+json">[{"@context":"http://schema.org","action":{"@type":"TrackAction","url":"https://docs.google.com/x"}}]</script>"#;
        assert_eq!(
            texts(unnamed),
            [(
                "Track on Google Docs".to_owned(),
                "https://docs.google.com/x".to_owned()
            )]
        );
        assert!(texts(&unnamed.replace("http://schema", "https://schema")).is_empty());
    }

    #[test]
    fn reads_microdata_actions_once() {
        let html = r#"<div itemscope itemtype="http://schema.org/EmailMessage"><div itemprop="potentialAction" itemscope itemtype="http://schema.org/ViewAction"><link itemprop="target" href="https://github.com/x/pull/1"><meta itemprop="name" content="VIEW pull request"></div></div>"#;
        assert_eq!(
            texts(html),
            [(
                "View Pull Request".to_owned(),
                "https://github.com/x/pull/1".to_owned()
            )]
        );
    }

    #[test]
    fn recognizes_links() {
        let link = |href: &str, text: &str| {
            texts(&format!(r#"<a href="{href}">{text}</a>"#))
                .into_iter()
                .map(|(text, _)| text)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            link("https://linear.app/exo/issue/EXO-12/fix", "EXO-12 Fix it"),
            ["View Issue"]
        );
        assert_eq!(
            link("https://shop.example/o", "Manage your order"),
            ["Manage Order"]
        );
        assert_eq!(
            link("https://shop.example/tracking?id=1", "track package"),
            ["Track Order"]
        );
        assert_eq!(
            link("https://docs.google.com/spreadsheets/d/1", "Open"),
            ["Open Spreadsheet"]
        );
        assert_eq!(
            link(
                "https://docs.google.com/document/d/1?comment_email_discussion",
                "Open"
            ),
            ["Open Discussion"]
        );
        assert_eq!(
            link("https://accounts.google.com/x", "Check activity"),
            ["Check Activity"]
        );
        assert!(link("https://shop.example/track.png", "track order").is_empty());
        assert!(link("https://example.com", "Read more").is_empty());
    }

    #[test]
    fn reads_a_google_calendar_notification() {
        let html = r#"<html><head><meta itemprop="name" content="Google Calendar"><meta itemprop="eventStatus" content="http://schema.org/EventRescheduled"></head><body><div class="info-bar-inner"><div>Changed: time, location</div></div><h4><strong>Note</strong><br>"Running late"</h4></body></html>"#;
        let calendar = found(html).calendar;
        assert_eq!(
            calendar,
            CalendarData {
                gcal: true,
                event_status: Some(EventStatus::Rescheduled),
                response_comment: Some("Running late".into()),
                updated_fields: Some("time, location".into())
            }
        );
        assert_eq!(found("<p>Hi</p>").calendar, CalendarData::default());
    }
}
