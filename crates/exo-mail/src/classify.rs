//! Telling marketing mail from personal mail, and the other judgments made about a rendered body.

use std::sync::LazyLock;

use regex::Regex;

use crate::{
    content,
    dom::{self, NodeRef, Style},
};

/// Banners mail systems put on outside mail.
static EXTERNAL_BANNER: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)\bsent from (?:outside|an external)\b",
        r"(?i)\boriginated (?:from )?outside\b",
        r"(?i)\[external(?:\s+(?:email|sender|message))?\]",
        r"(?im)^\s*ext\s*[\-:]",
        r"(?i)\bdo not click (?:on )?(?:any |the )?links?\b",
        r"(?i)\bverify the sender\b",
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).expect("valid regex"))
    .collect()
});

/// Whether a message part is marketing mail, which keeps its own design: centered content,
/// bordered boxes, button-like links, or the `Superhuman-marketing` marker class. Delivery-status
/// notifications never are.
pub(crate) fn is_marketing(root: &NodeRef, sender: &str) -> bool {
    if is_delivery_status_sender(sender) {
        return false;
    }
    let any = |matches: fn(&NodeRef) -> bool| dom::select_first(root, matches).is_some();
    let centered = any(|n| {
        dom::attr(n, "align").is_some_and(|a| a.eq_ignore_ascii_case("center")) && counts(n)
    });
    let bordered = any(|n| {
        if !dom::attr(n, "style").is_some_and(|s| s.contains("border:")) {
            return false;
        }
        let style = Style::of(n);
        let outlook =
            style.get("border-color") == "windowtext" && style.get("border-width") == "1pt";
        let border = style.get("border");
        !outlook
            && !is_excluded(n)
            && !border.is_empty()
            && border != "none"
            && !border_is_zero(border)
    });
    let marked = any(|n| dom::has_class(n, "Superhuman-marketing"));
    let centered_text = any(|n| {
        dom::attr(n, "style").is_some_and(|s| s.contains("text-align"))
            && Style::of(n).get("text-align") == "center"
            && counts(n)
    });
    let button = any(|n| {
        if !dom::is(n, "a")
            || !dom::attr(n, "style").is_some_and(|s| s.contains("display"))
            || is_excluded(n)
        {
            return false;
        }
        let style = Style::of(n);
        matches!(style.get("display"), "block" | "inline-block")
            && [
                style.get("padding"),
                style.get("height"),
                style.get("line-height"),
            ]
            .iter()
            .any(|v| has_positive_length(v))
    });
    centered || bordered || marked || (centered_text && button)
}

/// Whether a `border` shorthand's width is zero.
fn border_is_zero(border: &str) -> bool {
    border
        .split_whitespace()
        .any(|token| matches!(token, "0" | "0px" | "0pt" | "0em"))
}

fn has_positive_length(value: &str) -> bool {
    static LENGTH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^(\d+(?:\.\d+)?)(px|em|rem|pt)$").expect("valid regex"));
    value.split_whitespace().any(|token| {
        LENGTH
            .captures(token)
            .and_then(|found| found[1].parse::<f64>().ok())
            .is_some_and(|length| length > 0.0)
    })
}

fn is_delivery_status_sender(email: &str) -> bool {
    let user = email
        .split('@')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    user == "mailer-daemon" || user == "postmaster"
}

/// Whether an element counts as a signal: rules and hidden or banner content don't.
fn counts(node: &NodeRef) -> bool {
    !is_horizontal_rule(node) && !is_excluded(node)
}

fn is_excluded(node: &NodeRef) -> bool {
    is_hidden_by_display(node)
        || dom::closest(node, |n| dom::has_class(n, "exo-external-warning-banner")).is_some()
}

/// `display: none` on the element or an ancestor.
fn is_hidden_by_display(node: &NodeRef) -> bool {
    dom::closest(node, |n| Style::of(n).get("display") == "none").is_some()
}

/// An `<hr>`, or an element without text whose only children lead down to one.
fn is_horizontal_rule(node: &NodeRef) -> bool {
    if dom::is(node, "hr") {
        return true;
    }
    if !dom::text_content(node).trim().is_empty() {
        return false;
    }
    let mut node = node.clone();
    loop {
        let Ok([child]) = <[NodeRef; 1]>::try_from(dom::element_children(&node)) else {
            return false;
        };
        if dom::is(&child, "hr") {
            return true;
        }
        node = child;
    }
}

/// Tags external-sender banners so they neither count as marketing signals nor get restyled.
pub(crate) fn mark_external_banners(root: &NodeRef) {
    for table in dom::select(root, |n| {
        dom::is(n, "table") && dom::attr(n, "align").as_deref() == Some("left")
    }) {
        let yellow = dom::select(&table, |n| dom::is(n, "td"))
            .iter()
            .any(|cell| {
                let style = dom::attr(cell, "style")
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                style.contains("#fff8e5") || style.contains("rgb(255, 248, 229)")
            });
        if yellow {
            dom::add_class(&table, &["exo-external-warning-banner"]);
        }
    }
    for table in dom::select(root, |n| dom::is(n, "table")) {
        if dom::has_class(&table, "exo-external-warning-banner") {
            continue;
        }
        let text = dom::text_content(&table);
        let text = text.trim();
        if !text.is_empty()
            && text.chars().count() <= 600
            && EXTERNAL_BANNER.iter().any(|p| p.is_match(text))
        {
            dom::add_class(&table, &["exo-external-warning-banner"]);
        }
    }
}

/// Marks long `<pre>` blocks to keep their lines unwrapped.
pub(crate) fn mark_small_pre(root: &NodeRef) {
    for pre in dom::select(root, |n| dom::is(n, "pre")) {
        let text = dom::text_content(&pre);
        let large = text.encode_utf16().count() > 4000
            || text
                .split('\n')
                .any(|line| line.encode_utf16().count() > 500);
        if large {
            dom::remove_class(&pre, &["exo-small-pre"]);
        } else {
            dom::add_class(&pre, &["exo-small-pre"]);
        }
    }
}

/// Marks the body's only meaningful child, so its margins don't push the message down.
pub(crate) fn mark_flush_top(document: &NodeRef) {
    let Some(body) = dom::body(document).filter(|b| dom::is(b, "body")) else {
        return;
    };
    let meaningful: Vec<NodeRef> = dom::element_children(&body)
        .into_iter()
        .filter(is_meaningful)
        .collect();
    let [only] = meaningful.as_slice() else {
        return;
    };
    let tag = dom::tag(only).unwrap_or_default().to_ascii_lowercase();
    let replaced = matches!(
        tag.as_str(),
        "img"
            | "svg"
            | "video"
            | "canvas"
            | "picture"
            | "iframe"
            | "object"
            | "embed"
            | "audio"
            | "hr"
    );
    if !replaced && !is_layout_card(only) {
        dom::add_class(only, &["exo-flush-top"]);
    }
}

fn is_meaningful(node: &NodeRef) -> bool {
    let tag = dom::tag(node).unwrap_or_default().to_ascii_uppercase();
    if matches!(
        tag.as_str(),
        "STYLE" | "SCRIPT" | "META" | "LINK" | "TITLE" | "HEAD"
    ) || is_hidden_by_display(node)
        || content::is_hidden(node)
    {
        return false;
    }
    let media = |n: &NodeRef| {
        matches!(
            dom::tag(n),
            Some("img" | "picture" | "video" | "svg" | "canvas")
        )
    };
    !dom::text_content(node).trim().is_empty()
        || media(node)
        || dom::select_first(node, media).is_some()
}

/// A box with its own background or border.
fn is_layout_card(node: &NodeRef) -> bool {
    let style = Style::of(node);
    let paints = |value: &str| !value.is_empty() && value != "none" && value != "transparent";
    if paints(style.get("background-color"))
        || paints(style.get("background-image"))
        || paints(style.get("background"))
        || dom::attr(node, "bgcolor").is_some()
    {
        return true;
    }
    static ZERO: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^0(?:px)?(?:\s|$)").expect("valid regex"));
    [
        "border",
        "border-top",
        "border-right",
        "border-bottom",
        "border-left",
    ]
    .iter()
    .any(|side| {
        let value = style.get(side);
        !value.is_empty() && value != "none" && !ZERO.is_match(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marketing(html: &str) -> bool {
        is_marketing(&dom::parse(html), "news@example.com")
    }

    #[test]
    fn classifies_marketing_mail() {
        assert!(marketing(
            "<table><tr><td align=\"center\">Sale</td></tr></table>"
        ));
        assert!(marketing("<div style=\"border: 1px solid #ddd\">Hi</div>"));
        assert!(!marketing(
            "<div dir=\"ltr\"><div>Hi Ann,</div><div>Lunch?</div></div>"
        ));
        assert!(!marketing(
            "<blockquote style=\"border-left:1px solid #ccc\">old</blockquote>"
        ));
        assert!(!marketing("<div align=\"center\"><hr></div>"));
        assert!(!is_marketing(
            &dom::parse("<p align=center>x</p>"),
            "MAILER-DAEMON@x.com"
        ));
    }

    #[test]
    fn marks_the_only_wrapper_flush() {
        let document = dom::parse("<div dir=ltr><div>Hi</div></div>");
        mark_flush_top(&document);
        assert!(dom::outer_html(&document).contains("<div dir=\"ltr\" class=\"exo-flush-top\">"));
    }
}
