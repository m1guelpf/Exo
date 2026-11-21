//! The HTML a message is sent with, and the quoted parts built around the body.

use jiff::Zoned;

use crate::{
    editor::link_run,
    html::{self, as_document},
    model::{Block, Content, Document, Element, Piece, Run},
};

/// What goes around the body when it's sent.
#[derive(Clone, Copy, Debug, Default)]
pub struct Parts<'a> {
    /// The From alias's signature (Gmail `sendAs` HTML, already cleaned), if it has one.
    pub signature: Option<&'a str>,
    /// Whether the signature's content is included: always for new mail, and on replies unless
    /// `includeSignatureOnReplies` is off.
    pub include_signature_content: bool,
    /// No signature at all.
    pub skip_signature: bool,
    /// The quoted message of a reply or forward ([`reply_quote`], [`forward_quote`]).
    pub quote: Option<&'a str>,
    /// Whether the quote and signature are already in the body (revealed with "…", or deleted).
    pub inlined: bool,
}

/// Where a read-status pixel would go. Exo has no read statuses, so it's an empty `<div>`.
const PIXEL: &str = "<div></div>";

/// The message's HTML body: `body` cleaned for sending (bare URLs linked), then wrapped with the
/// signature and quote. The signature and quote go in as given, so they must already be
/// sanitized.
pub fn html(body: &Document, parts: &Parts<'_>) -> String {
    let mut body = body.clone();
    clean(&mut body);
    let body = format!(
        "<html><head></head><body>{}</body></html>",
        html::write(&body)
    );
    if parts.inlined {
        return format!("<div><div>{body}</div>{PIXEL}</div>");
    }
    let signature = if parts.skip_signature {
        String::new()
    } else {
        signature(parts.signature, parts.include_signature_content, None)
    };
    match parts.quote {
        Some(quote) => {
            let quote = html::as_sent_document(&remove_pixels(quote));
            format!(
                "<div><div><div>{body}</div>{PIXEL}<br/>{signature}</div><br/><div>{quote}</div><br/></div>"
            )
        }
        None => format!("<div><div>{body}</div>{PIXEL}<br/>{signature}</div>"),
    }
}

/// The signature block: `<div class="gmail_signature">` holding the alias's signature (written
/// back as a document) and a `<br/>`, or nothing when there's no signature. Exo adds no "Sent
/// via" line. `draft_id` is the `data-signature-draft-id` the editor's copy carries.
pub fn signature(content: Option<&str>, include_content: bool, draft_id: Option<&str>) -> String {
    let id = draft_id
        .map(|id| format!(" data-signature-draft-id=\"{}\"", escape(id)))
        .unwrap_or_default();
    let Some(content) = content.filter(|content| !content.is_empty()) else {
        return format!("<div class=\"gmail_signature\"{id}></div>");
    };
    let content = if include_content {
        format!("<div>{}</div>", as_document(content))
    } else {
        String::new()
    };
    format!("<div class=\"gmail_signature\"{id}>{content}<br/></div>")
}

/// The body with the quote and signature revealed, to load into the editor when "…" is opened.
/// `quote` is `None` for new mail, which reveals only the signature.
pub fn inlined(body: &str, signature: Option<&str>, quote: Option<&str>) -> String {
    let signature = signature
        .map(|signature| format!("<div class=\"sh-signature\">{signature}</div>"))
        .unwrap_or_default();
    match quote {
        Some(quote) => {
            let quote = as_document(&remove_pixels(quote));
            format!(
                "<div><div><div>{body}</div><br/>{signature}</div><br/><div class=\"sh-quoted-content\"><div>{quote}</div></div></div>"
            )
        }
        None => format!("<div><div>{body}</div><br/>{signature}</div>"),
    }
}

/// A reply's quote: the reply header and the quoted body. `body_attrs` are the original body
/// element's `class`, `style` and `id`; `body` its inner HTML.
pub fn reply_quote(
    sent: &Zoned,
    name: &str,
    email: &str,
    body_class: &str,
    body_style: &str,
    body_id: &str,
    body: &str,
) -> String {
    // `Tue, Nov 17, 2026`, then the en-US time without its seconds.
    let date = sent.strftime("%a, %b %d, %Y");
    let time = sent.strftime("%-I:%M %p");
    let (name, email) = (escape(name), escape(email));
    let class = if body_class.is_empty() {
        "gmail_quote".to_owned()
    } else {
        format!("gmail_quote {}", escape(body_class))
    };
    format!(
        "<div class=\"gmail_quote\">On {date} at {time}, {name} <span dir=\"ltr\">&lt;<a href=\"mailto:{email}\" target=\"_blank\">{email}</a>&gt;</span> wrote:<br/><blockquote class=\"gmail_quote\" style=\"margin:0 0 0 .8ex;border-left:1px #ccc solid;padding-left:1ex\"><div class=\"gmail_extra\"><div class=\"{class}\"{rest}>{body}</div></div></blockquote></div>",
        rest = body_attrs(body_style, body_id),
    )
}

/// A forwarded message's header fields, raw (`Name <email>`).
#[derive(Clone, Debug, Default)]
pub struct Forwarded<'a> {
    /// `From`.
    pub from: &'a str,
    /// `To`, each address.
    pub to: &'a [&'a str],
    /// `Cc`, each address.
    pub cc: &'a [&'a str],
    /// The subject.
    pub subject: &'a str,
}

/// A forward's quote: the forwarded-message header and body.
pub fn forward_quote(
    sent: &Zoned,
    header: &Forwarded<'_>,
    body_class: &str,
    body_style: &str,
    body_id: &str,
    body: &str,
) -> String {
    // `Tuesday, November 17 2026 at 3:45 PM PST`.
    let date = sent.strftime("%A, %B %-d %Y at %-I:%M %p %Z");
    let cc = if header.cc.is_empty() {
        String::new()
    } else {
        format!("<span>Cc: {}<br/></span>", escape(&header.cc.join(", ")))
    };
    let class = if body_class.is_empty() {
        "gmail_quote".to_owned()
    } else {
        format!("gmail_quote {}", escape(body_class))
    };
    format!(
        "<div><div>---------- Forwarded message ----------<br/>From: {from}<br/>Date: {date}<br/>Subject: {subject}<br/>To: {to}<br/>{cc}</div><br/><div><div class=\"{class}\"{rest}>{body}</div></div></div>",
        from = escape(header.from),
        subject = escape(header.subject),
        to = escape(&header.to.join(", ")),
        rest = body_attrs(body_style, body_id),
    )
}

/// The original body's `style` and `id`, each left out when it's empty.
fn body_attrs(style: &str, id: &str) -> String {
    let mut out = String::new();
    if !style.is_empty() {
        out.push_str(&format!(" style=\"{}\"", escape(style)));
    }
    if !id.is_empty() {
        out.push_str(&format!(" id=\"{}\"", escape(id)));
    }
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Drops the read-status pixel placeholders an earlier send left in `html`.
fn remove_pixels(html: &str) -> String {
    let mut out = html.to_owned();
    while let Some(start) = out.find("<!--{{pixel_placeholder_") {
        let Some(end) = out[start..].find("-->") else {
            break;
        };
        out.replace_range(start..start + end + 3, "");
    }
    out
}

/// The clean-up for sending: links bare URLs, points inline images at their Content-ID, drops
/// signature ids and the dark-mode colour tags, Exo's and the `sh-` ones a draft begun elsewhere
/// carries (leaving an empty `class` attribute, as a browser does).
fn clean(doc: &mut Document) {
    for block in &mut doc.blocks {
        for container in &mut block.path {
            clean_element(&mut container.element);
        }
        clean_element(&mut block.element);
        if let Content::Inline(runs) = &mut block.content {
            for run in runs.iter_mut() {
                run.marks.iter_mut().for_each(clean_element);
                if let Piece::Image(image) = &mut run.piece {
                    clean_image(image);
                }
            }
            linkify(runs);
        }
    }
}

fn clean_element(element: &mut Element) {
    element
        .attrs
        .retain(|(name, _)| name != "data-signature-draft-id");
    if let Some(class) = element.attr("class") {
        let colour_tag = |name: &str| {
            name.starts_with("color") || name.starts_with("original") || name == "modified-inline"
        };
        let kept: Vec<&str> = class
            .split_whitespace()
            .filter(|class| {
                !class
                    .strip_prefix("exo-")
                    .or_else(|| class.strip_prefix("sh-"))
                    .is_some_and(colour_tag)
            })
            .collect();
        let kept = kept.join(" ");
        element.set_attr("class", &kept);
    }
}

/// An image shown by Content-ID gets it as its URL, in the `src` it has (an image without one
/// gets one), and loses Exo's class; its `src-cid` stays.
fn clean_image(image: &mut Element) {
    for (attr, target) in [
        ("src-cid", "src"),
        ("poster-cid", "poster"),
        ("background-cid", "background"),
    ] {
        if let Some(cid) = image
            .attr(attr)
            .filter(|cid| !cid.is_empty())
            .map(str::to_owned)
        {
            image.set_attr(target, &format!("cid:{cid}"));
            if let Some(class) = image.attr("class") {
                let kept: Vec<&str> = class
                    .split_whitespace()
                    .filter(|class| *class != crate::image::CLASS)
                    .collect();
                let kept = kept.join(" ");
                image.set_attr("class", &kept);
            }
        }
    }
}

/// `Linkify.text` on every text run outside a link: `<a target="_blank" rel="noopener
/// noreferrer" href="…">`.
fn linkify(runs: &mut Vec<Run>) {
    let link = |href: &str| {
        Element::with(
            "a",
            &[
                ("target", "_blank"),
                ("rel", "noopener noreferrer"),
                ("href", href),
            ],
        )
    };
    *runs = std::mem::take(runs)
        .into_iter()
        .flat_map(|run| link_run(run, exo_mail::links, link))
        .collect();
}

/// Gives every element in `blocks` without a `class` an empty one, as every element that shows
/// text has in sent mail once its dark-mode classes are stripped: for a revealed quote and
/// signature.
pub(crate) fn add_empty_classes(blocks: &mut [Block]) {
    let mut seen = std::collections::HashSet::new();
    for block in blocks {
        for container in &mut block.path {
            if seen.insert(container.id) {
                empty_class(&mut container.element);
            }
        }
        if !block.element.tag.is_empty() {
            empty_class(&mut block.element);
        }
        if let Content::Inline(runs) = &mut block.content {
            for run in runs {
                run.marks.iter_mut().for_each(empty_class);
            }
        }
    }
}

fn empty_class(element: &mut Element) {
    if element.attr("class").is_none()
        && !matches!(element.tag.as_str(), "br" | "img" | "hr" | "wbr")
    {
        element.attrs.push(("class".to_owned(), String::new()));
    }
}
