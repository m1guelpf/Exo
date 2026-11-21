//! Sanitizing, the message pipeline's first step: DOMPurify with its default allowlists plus
//! hooks that fix up links and image sources.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::LazyLock,
};

use regex::Regex;

use crate::{
    actions::Found,
    content::is_space,
    dom::{self, Namespace, NodeRef, Style},
    net, purify,
};

/// App class names a sender's HTML may not use.
const APP_CLASSES: &[&str] = &[
    "ShadowHTML",
    "Theme-snow",
    "Theme-carbon",
    "Theme-light",
    "Theme-dark",
    "Exo-quoting-ellipsis",
];

/// How deep elements nest: deeper ones become siblings, as Chromium's parser
/// (`kMaximumHTMLParserDOMTreeDepth`) makes them. It also keeps every later pass, and Blitz, off a
/// deep stack.
const MAX_DEPTH: usize = 512;

/// Elements whose text a parser reads as markup in other contexts (DOMPurify's raw-text list).
const RAW_TEXT: &[&str] = &[
    "iframe",
    "noembed",
    "noframes",
    "noscript",
    "plaintext",
    "script",
    "style",
    "xmp",
];
/// MathML elements whose children are HTML.
const MATHML_TEXT_INTEGRATION_POINTS: &[&str] = &["mi", "mn", "mo", "ms", "mtext"];
/// Elements HTML shares with SVG.
const COMMON_SVG_AND_HTML: &[&str] = &["a", "font", "script", "style", "title"];

static EMAIL_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<([^\s&>"']{0,255}@[a-z0-9\-.]+)>"#).expect("valid regex"));
static TITLE_OPEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<title\b[^>]*>[^<]*").expect("valid regex"));
static TITLE_CLOSE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</title\b[^>]*>").expect("valid regex"));
static SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[a-z][a-z0-9.\-+]*:").expect("valid regex"));
static ALLOWED_URI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:(?:(?:f|ht)tps?|mailto|tel|callto|sms|cid|xmpp|matrix):|[^a-z]|[a-z+.\-]+(?:[^a-z+.\-:]|$))")
        .expect("valid regex")
});
static SCRIPT_OR_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:[0-9A-Za-z_]+script|data):").expect("valid regex"));
static UNSAFE_IN_ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)((--!?|])>)|</(style|script|title|xmp|textarea|noscript|iframe|noembed|noframes)",
    )
    .expect("valid regex")
});
static MARKUP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[/0-9A-Za-z_!]").expect("valid regex"));
static DATA_ATTRIBUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^data-[\-\w.\x{00B7}-\x{FFFF}]+$").expect("valid regex"));
static ARIA_ATTRIBUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^aria-[\-\w]+$").expect("valid regex"));
static GOOMOJI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([0-9a-f]+)@goomoji\.gmail$").expect("valid regex"));
static OUTLOOK_EMOJI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https://a\.gfx\.ms/emoji_([0-9a-f]+)\.png").expect("valid regex")
});
static HOSTED_EMOJI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https://emojis\.superhuman\.com/([0-9A-F-]+)").expect("valid regex")
});
static READ_RECEIPT_PIXEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:https?://mail.superhuman.com|mail.staging-superhuman.com|mail.pp-sh.io|localhost.superhumanapp.com:8080)/~backend/read/.*/.*\.gif")
        .expect("valid regex")
});

/// An email address.
pub(crate) static VALID_EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[a-z0-9!#$%&'*+/=?^_`{|}~-]+(?:\.[a-z0-9!#$%&'*+/=?^_`{|}~-]+)*@(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9-]*[a-z0-9])$")
        .expect("valid regex")
});

/// Parses and sanitizes a message's HTML into a whole document.
pub(crate) fn sanitize(html: &str) -> NodeRef {
    clean(html).0
}

/// [`sanitize`], and what the walk noted on the way (actions, Google Calendar's facts).
pub(crate) fn clean(html: &str) -> (NodeRef, Found) {
    let document = dom::parse(&preprocess(html));
    let mut janitor = Janitor::new(Mode::Message);
    janitor.clean(&document);
    (document, janitor.found)
}

/// Sanitizes HTML going into the compose editor, when it loads a draft and when it inserts a
/// paste: DOMPurify with its default allowlists, unknown URL schemes allowed and the `src-cid`,
/// `poster-cid` and `background-cid` attributes kept, and none of the hooks the message pipeline
/// adds (so links keep no `target`, which DOMPurify doesn't allow). Nesting is capped at 512
/// elements, as in Chromium's parser. Returns the body's content.
pub fn sanitize_fragment(html: &str) -> String {
    let document = dom::parse(html);
    Janitor::new(Mode::Fragment).clean(&document);
    dom::body(&document).map_or_else(String::new, |body| dom::inner_html(&body))
}

/// Sanitizes a calendar event's description for its event details: the message pass, with
/// `<style>` dropped. Returns the body's content.
pub fn sanitize_description(html: &str) -> String {
    let document = sanitize(html);
    dom::detach_all(&dom::select(&document, |node| dom::is(node, "style")));
    dom::body(&document).map_or_else(String::new, |body| dom::inner_html(&body))
}

/// Which of the two DOMPurify configurations a pass applies.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// A message: DOMPurify plus the pipeline's hooks.
    Message,
    /// The compose editor: DOMPurify alone, with a few attributes added.
    Fragment,
}

impl Mode {
    /// Attributes allowed on top of DOMPurify's.
    fn added_attributes(self) -> &'static [&'static str] {
        match self {
            Self::Message => &[],
            Self::Fragment => &["src-cid", "poster-cid", "background-cid"],
        }
    }
}

/// Drops `<title>` text and escapes `<someone@example.com>` written as a tag, unless the
/// message also closes it.
fn preprocess(html: &str) -> String {
    let html = TITLE_OPEN.replace_all(html, "");
    let html = TITLE_CLOSE.replace_all(&html, "");
    // Where each address tag still open starts, and the starts of those closed later.
    let mut open: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut closed = HashSet::new();
    for found in EMAIL_TAG.captures_iter(&html) {
        let (Some(tag), Some(name)) = (found.get(0), found.get(1)) else {
            continue;
        };
        match name.as_str().strip_prefix('/') {
            Some(name) => closed.extend(open.get_mut(name).and_then(Vec::pop)),
            None => open.entry(name.as_str()).or_default().push(tag.start()),
        }
    }
    EMAIL_TAG
        .replace_all(&html, |found: &regex::Captures| {
            let start = found.get(0).map_or(0, |m| m.start());
            if found[1].starts_with('/') || closed.contains(&start) {
                found[0].to_owned()
            } else {
                format!("&lt;{}&gt;", &found[1])
            }
        })
        .into_owned()
}

/// DOMPurify's walk, with the pipeline's hooks in [`Mode::Message`].
struct Janitor {
    mode: Mode,
    /// The first absolute `<base href>`, which root-relative URLs resolve against.
    base_url: Option<url::Url>,
    /// Actions and Google Calendar's facts, noted before each element is sanitized.
    found: Found,
}

/// What becomes of a node.
enum Verdict {
    Keep,
    Remove,
    /// Its children take its place, and are visited next.
    Unwrap,
    /// Text takes its place (emoji drawn as images).
    Replace(String),
}

/// The children of one element, being visited.
struct Level {
    parent: NodeRef,
    pending: VecDeque<NodeRef>,
    kept: Vec<NodeRef>,
}

impl Level {
    fn new(parent: &NodeRef) -> Self {
        Self {
            parent: parent.clone(),
            pending: dom::take_children(parent).into(),
            kept: Vec::new(),
        }
    }

    /// Makes `node`'s children this level's next ones.
    fn hoist(&mut self, node: &NodeRef) {
        for child in dom::take_children(node).into_iter().rev() {
            self.pending.push_front(child);
        }
    }
}

impl Janitor {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            base_url: None,
            found: Found::default(),
        }
    }

    /// Visits every node in document order, like DOMPurify's node iterator, rebuilding each child
    /// list in one go.
    fn clean(&mut self, document: &NodeRef) {
        let mut levels = vec![Level::new(document)];
        loop {
            let depth = levels.len();
            let Some(level) = levels.last_mut() else {
                break;
            };
            let Some(node) = level.pending.pop_front() else {
                dom::set_children(&level.parent, std::mem::take(&mut level.kept));
                levels.pop();
                continue;
            };
            match self.node(&node, &level.parent) {
                Verdict::Keep => {
                    level.kept.push(node.clone());
                    if depth > MAX_DEPTH {
                        level.hoist(&node);
                    } else {
                        levels.push(Level::new(&node));
                    }
                }
                Verdict::Remove => {}
                Verdict::Unwrap => level.hoist(&node),
                Verdict::Replace(text) => level.kept.push(dom::text_node(&text)),
            }
        }
    }

    /// DOMPurify's element pass, then its attribute pass, for a child of `parent`.
    fn node(&mut self, node: &NodeRef, parent: &NodeRef) -> Verdict {
        let Some(tag) = dom::tag(node).map(str::to_ascii_lowercase) else {
            // Comments, doctypes and processing instructions aren't allowed.
            return if dom::is_text(node) {
                Verdict::Keep
            } else {
                Verdict::Remove
            };
        };
        let hooks = self.mode == Mode::Message;
        if hooks
            && tag == "style"
            && (dom::has_attr(node, "amp4email-boilerplate") || dom::has_attr(node, "amp-custom"))
        {
            return Verdict::Remove;
        }
        if hooks {
            self.before_sanitize(node, &tag);
            dom::remove_class(node, APP_CLASSES);
        }
        if hides_markup(node, &tag) {
            return Verdict::Remove;
        }
        if !purify::TAGS.contains(tag.as_str()) {
            return if purify::FORBID_CONTENTS.contains(tag.as_str()) {
                Verdict::Remove
            } else {
                Verdict::Unwrap
            };
        }
        if !has_valid_namespace(node, &tag, parent) {
            return Verdict::Remove;
        }
        sanitize_attributes(node, &tag, self.mode);
        if hooks {
            self.after_sanitize(node, &tag)
        } else {
            Verdict::Keep
        }
    }

    /// The hook before the element pass: unwraps CSS from HTML comments, resolves URLs against the
    /// message's `<base>`, and notes actions and Google Calendar's facts.
    fn before_sanitize(&mut self, node: &NodeRef, tag: &str) {
        if tag == "style" {
            let css = dom::text_content(node);
            if !css.is_empty() {
                let css = css
                    .trim_start_matches(is_space)
                    .strip_prefix("<!--")
                    .unwrap_or(&css);
                let css = css
                    .trim_end_matches(is_space)
                    .strip_suffix("-->")
                    .unwrap_or(css);
                dom::set_children(node, vec![dom::text_node(css)]);
            }
        }
        if tag == "base"
            && self.base_url.is_none()
            && let Some(href) = dom::attr(node, "href")
            && (href.starts_with("https://") || href.starts_with("http://"))
            && let Ok(url) = url::Url::parse(&href)
            && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "0.0.0.0"))
        {
            self.base_url = Some(url);
        }
        if let Some(base) = &self.base_url {
            for name in ["href", "src", "background", "poster"] {
                if let Some(value) = dom::attr(node, name)
                    && value.starts_with('/')
                    && let Ok(resolved) = base.join(&value)
                {
                    dom::set_attr(node, name, resolved.as_str());
                }
            }
        }
        self.found.visit(node, tag);
    }

    /// The hook after the attribute pass: CSS that hides content, links, and image sources.
    fn after_sanitize(&mut self, node: &NodeRef, tag: &str) -> Verdict {
        sanitize_css(node, tag);
        open_links_in_browser(node, tag);
        let verdict = self.fix_sources(node);
        if tag == "img" && dom::has_attr(node, "srcset") {
            dom::set_attr(node, "sizes", "640px");
        }
        verdict
    }

    /// Image sources. Remote images load directly, with no proxy, so only hosted emoji images,
    /// read-receipt pixels (`mail.superhuman.com/~backend/read/…`), `cid:` attachments and
    /// non-HTTP sources change. Inline attachments keep their `cid:` URL, which the view's fetch
    /// hook resolves. Every read-receipt pixel goes, whoever it's addressed to: it's a tracker
    /// either way.
    fn fix_sources(&self, node: &NodeRef) -> Verdict {
        for name in ["src", "background", "poster"] {
            let Some(value) = dom::attr(node, name).filter(|v| !v.is_empty()) else {
                continue;
            };
            if let Some(cid) = value.strip_prefix("cid:") {
                if let Some(emoji) = GOOMOJI
                    .captures(cid)
                    .and_then(|found| u32::from_str_radix(&found[1], 16).ok())
                    .and_then(char::from_u32)
                {
                    return Verdict::Replace(emoji.into());
                }
                dom::set_attr(node, &format!("{name}-cid"), cid);
                dom::add_class(node, &["Exo-inline-image"]);
            } else if value.starts_with("http:") || value.starts_with("https:") {
                if let Some(emoji) = OUTLOOK_EMOJI
                    .captures(&value)
                    .and_then(|found| u32::from_str_radix(&found[1], 16).ok())
                    .and_then(char::from_u32)
                {
                    return Verdict::Replace(emoji.into());
                }
                if let Some(found) = HOSTED_EMOJI.captures(&value) {
                    return Verdict::Replace(
                        found[1]
                            .split('-')
                            .filter_map(|hex| {
                                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                            })
                            .collect(),
                    );
                }
                if READ_RECEIPT_PIXEL.is_match(&value) {
                    return Verdict::Remove;
                }
            } else if !value.starts_with("data:") {
                dom::set_attr(node, name, "");
            }
        }
        Verdict::Keep
    }
}

/// DOMPurify's guard against markup hiding in text: text that reads as tags in a raw-text element,
/// or beside a comment, could come back as unsanitized elements when the serialized HTML is parsed
/// again in another context.
fn hides_markup(node: &NodeRef, tag: &str) -> bool {
    let children = dom::children(node);
    let has_elements = children.iter().any(dom::is_element);
    let reads_as_markup = || MARKUP.is_match(&dom::text_content(node));
    if RAW_TEXT.contains(&tag) && dom::namespace(node) == Some(Namespace::Html) {
        return has_elements || reads_as_markup();
    }
    // Other text serializes escaped, so only a comment child puts markup in `innerHTML`.
    !has_elements && children.iter().any(dom::is_comment) && reads_as_markup()
}

/// DOMPurify's namespace check: an element may only be where parsing could have put it, so
/// its serialization parses back into the same namespaces instead of turning text into elements.
fn has_valid_namespace(node: &NodeRef, tag: &str, parent: &NodeRef) -> bool {
    use Namespace::{Html, MathMl, Svg};
    let (parent_namespace, parent_tag) = match dom::tag(parent) {
        Some(name) => (dom::namespace(parent), name.to_ascii_lowercase()),
        // The document, which DOMPurify treats as an HTML `<template>`.
        None => (Some(Html), "template".to_owned()),
    };
    let html_integration_point = parent_tag == "annotation-xml";
    let text_integration_point = MATHML_TEXT_INTEGRATION_POINTS.contains(&parent_tag.as_str());
    match (dom::namespace(node), parent_namespace) {
        (Some(Svg), Some(Html)) => tag == "svg",
        (Some(Svg), Some(MathMl)) => {
            tag == "svg" && (html_integration_point || text_integration_point)
        }
        (Some(Svg), _) => purify::SVG_TAGS.contains(tag),
        (Some(MathMl), Some(Html)) => tag == "math",
        (Some(MathMl), Some(Svg)) => tag == "math" && html_integration_point,
        (Some(MathMl), _) => purify::MATHML_TAGS.contains(tag),
        (Some(Html), Some(Svg)) if !html_integration_point => false,
        (Some(Html), Some(MathMl)) if !text_integration_point => false,
        (Some(Html), _) => {
            !purify::MATHML_TAGS.contains(tag)
                && (COMMON_SVG_AND_HTML.contains(&tag) || !purify::SVG_TAGS.contains(tag))
        }
        (None, _) => false,
    }
}

/// DOMPurify's attribute pass (values trimmed, but `value`'s), plus, for a message, no `tabindex`
/// or `autoplay`.
fn sanitize_attributes(node: &NodeRef, tag: &str, mode: Mode) {
    dom::retain_attrs(node, |name, value| {
        let name = name.to_ascii_lowercase();
        let value = if name == "value" {
            value
        } else {
            value.trim_matches(is_space)
        };
        let keep = !(mode == Mode::Message && matches!(name.as_str(), "tabindex" | "autoplay"))
            && !UNSAFE_IN_ATTRIBUTE.is_match(value)
            && !(name == "attributename" && value.contains("href"))
            && is_valid_attribute(tag, &name, value, mode.added_attributes());
        keep.then(|| value.to_owned())
    });
}

/// Whether DOMPurify keeps an attribute, with unknown URL schemes allowed and `added` on top of
/// its allowlist.
fn is_valid_attribute(tag: &str, name: &str, value: &str, added: &[&str]) -> bool {
    if DATA_ATTRIBUTE.is_match(name) || ARIA_ATTRIBUTE.is_match(name) {
        return true;
    }
    if !purify::ATTRIBUTES.contains(name) && !added.contains(&name) {
        return false;
    }
    if value.is_empty() || purify::URI_SAFE_ATTRIBUTES.contains(name) {
        return true;
    }
    let compact: String = value
        .chars()
        .filter(|&c| !is_attribute_whitespace(c))
        .collect();
    !SCRIPT_OR_DATA.is_match(&compact)
        || ALLOWED_URI.is_match(&compact)
        || (matches!(name, "src" | "href" | "xlink:href")
            && value.starts_with("data:")
            && purify::DATA_URI_TAGS.contains(tag))
}

/// DOMPurify's `ATTR_WHITESPACE`: controls and spaces, which URL parsing skips, so a script
/// scheme can hide behind them.
fn is_attribute_whitespace(c: char) -> bool {
    matches!(
        c,
        '\0'..=' '
            | '\u{a0}'
            | '\u{1680}'
            | '\u{180e}'
            | '\u{2000}'..='\u{2029}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

/// Inline CSS: content can't opt out of clicks or selection, and a hidden `<body>` shows.
fn sanitize_css(node: &NodeRef, tag: &str) {
    if !dom::has_attr(node, "style") {
        return;
    }
    let mut style = Style::of(node);
    let mut changed = false;
    if !style.get("pointer-events").is_empty() {
        style.set("pointer-events", "initial", false);
        changed = true;
    }
    if style.get("user-select") == "none" {
        style.set("user-select", "auto", false);
        changed = true;
    }
    if tag == "body" && style.get("visibility") == "hidden" {
        style.remove("visibility");
        changed = true;
    }
    if changed {
        style.apply(node);
    }
}

/// Links: bare domains and addresses become URLs, `#` links target the page itself, and the rest
/// a new window.
fn open_links_in_browser(node: &NodeRef, tag: &str) {
    let href = dom::attr(node, "href");
    if let Some(href) = &href {
        if href.trim_matches(is_space).is_empty() {
            dom::remove_attr(node, "href");
        } else if !href.starts_with('#') && !SCHEME.is_match(href) {
            let fixed = if VALID_EMAIL.is_match(href) {
                format!("mailto:{href}")
            } else {
                format!("http://{href}")
            };
            dom::set_attr(node, "href", &fixed);
        }
    }
    if tag == "form" {
        dom::set_attr(node, "target", "_blank");
    }
    if href.as_deref().is_some_and(|href| href.starts_with('#')) {
        dom::set_attr(node, "target", "_self");
        return;
    }
    if matches!(tag, "a" | "area" | "form") {
        dom::set_attr(node, "target", "_blank");
        dom::set_attr(node, "rel", "noopener noreferrer");
    }
    if !dom::has_attr(node, "target")
        && (dom::has_attr(node, "href") || dom::has_attr(node, "xlink:href"))
    {
        dom::set_attr(node, "xlink:show", "new");
    }
}

/// JavaScript's `decodeURIComponent`, giving `text` back unchanged where that would throw.
pub(crate) fn decode_uri_component(text: &str) -> String {
    let well_formed = text.match_indices('%').all(|(i, _)| {
        text.get(i + 1..i + 3)
            .is_some_and(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))
    });
    well_formed
        .then(|| String::from_utf8(net::percent_decode(text)).ok())
        .flatten()
        .unwrap_or_else(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(html: &str) -> String {
        let document = sanitize(html);
        dom::inner_html(&dom::body(&document).unwrap())
    }

    #[test]
    fn descriptions_lose_styles_scripts_and_unsafe_links() {
        assert_eq!(
            sanitize_description(
                "<style>p{color:red}</style><p>Agenda<script>x()</script></p><a href=\"javascript:x()\">y</a>"
            ),
            "<p>Agenda</p><a target=\"_blank\" rel=\"noopener noreferrer\">y</a>"
        );
    }

    #[test]
    fn removes_scripts_handlers_and_comments() {
        assert_eq!(
            clean(
                "<div onclick=x()>a<script>evil()</script><!-- note --><iframe src=x>b</iframe></div>"
            ),
            "<div>a</div>"
        );
        assert_eq!(
            clean("<a href=\"javascript:alert(1)\">x</a>"),
            "<a target=\"_blank\" rel=\"noopener noreferrer\">x</a>"
        );
    }

    #[test]
    fn strips_script_urls_hidden_behind_controls() {
        // A URL parser skips the control character, so this is a `javascript:` link.
        assert_eq!(
            clean(
                "<a href=\"&#1;javascript:alert(1)\">x</a><a href=\"java&#x2028;script:alert(1)\">y</a>"
            ),
            "<a target=\"_blank\" rel=\"noopener noreferrer\">x</a>\
             <a target=\"_blank\" rel=\"noopener noreferrer\">y</a>"
        );
    }

    #[test]
    fn trims_attribute_values() {
        assert_eq!(
            clean("<a href=\" https://x.io/a \">x</a><img src=\"\n https://x.io/a.png\">"),
            "<a href=\"https://x.io/a\" target=\"_blank\" rel=\"noopener noreferrer\">x</a>\
             <img src=\"https://x.io/a.png\">"
        );
    }

    #[test]
    fn removes_markup_hiding_in_foreign_content() {
        // Each parses into a tree whose serialization parses into an `<img onerror>`.
        for html in [
            "<math><mtext><table><mglyph><style><img src=x onerror=alert(1)></style></mglyph></table></mtext></math>",
            "<form><math><mtext></form><form><mglyph><style></math><img src onerror=alert(1)>",
            "<svg></p><style><a id=\"</style><img src=1 onerror=alert(1)>\">",
        ] {
            let html = dom::outer_html(&sanitize(html));
            let reparsed = dom::outer_html(&dom::parse(&html));
            assert!(!reparsed.contains("onerror"), "{reparsed}");
        }
    }

    #[test]
    fn caps_nesting_like_chromium() {
        let document = sanitize(&("<div>".repeat(5000) + "x"));
        let deepest = dom::descendants(&document)
            .filter(dom::is_text)
            .map(|node| std::iter::successors(dom::parent(&node), dom::parent).count())
            .max();
        assert_eq!(deepest, Some(MAX_DEPTH + 1));
    }

    #[test]
    fn editor_fragments_get_dompurify_without_the_message_hooks() {
        // No `target` (DOMPurify doesn't allow it), no link or source fixes, app classes and
        // `tabindex` kept; `src-cid` is allowed.
        assert_eq!(
            sanitize_fragment(
                "<title>t</title><div tabindex=\"1\" class=\"Theme-dark\">a <a href=\"x.com\" target=\"_blank\" onclick=\"x()\">b</a>\
                 <img src-cid=\"c\" src=\"javascript:x()\"><img src=\"data:image/png;base64,AA\"><script>x()</script><o:p>d</o:p></div>"
            ),
            "<div tabindex=\"1\" class=\"Theme-dark\">a <a href=\"x.com\">b</a>\
             <img src-cid=\"c\"><img src=\"data:image/png;base64,AA\">d</div>"
        );
        // Deeper elements become siblings, as in Chromium: none is lost, none nests past 511
        // inside `<body>`.
        let deep = sanitize_fragment(&("<div>".repeat(5000) + "x"));
        let nesting = deep
            .split_inclusive('>')
            .scan(0, |depth, tag| {
                *depth += i32::from(tag.ends_with("<div>")) - i32::from(tag.ends_with("</div>"));
                Some(*depth)
            })
            .max();
        assert_eq!((deep.matches("<div>").count(), nesting), (5000, Some(511)));
    }

    #[test]
    fn strips_the_app_classes() {
        assert_eq!(
            clean(
                "<div class=\"ShadowHTML Theme-dark Theme-carbon note\">a<span class=\"Exo-quoting-ellipsis\">b</span></div>"
            ),
            "<div class=\"note\">a<span class=\"\">b</span></div>"
        );
    }

    #[test]
    fn unwraps_unknown_elements() {
        assert_eq!(
            clean("<p>a<o:p>b</o:p><custom-x>c</custom-x></p>"),
            "<p>abc</p>"
        );
    }

    #[test]
    fn fixes_links() {
        assert_eq!(
            clean("<a href=\"example.com\">x</a><a href=\"a@b.co\">y</a><a href=\"#top\">z</a>"),
            "<a href=\"http://example.com\" target=\"_blank\" rel=\"noopener noreferrer\">x</a>\
             <a href=\"mailto:a@b.co\" target=\"_blank\" rel=\"noopener noreferrer\">y</a>\
             <a href=\"#top\" target=\"_self\">z</a>"
        );
    }

    #[test]
    fn rewrites_image_sources() {
        assert_eq!(
            clean(
                "<img src=\"cid:logo@x\"><img src=\"cid:1f600@goomoji.gmail\"><img src=\"img/a.png\">"
            ),
            "<img src=\"cid:logo@x\" src-cid=\"logo@x\" class=\"Exo-inline-image\">😀<img src=\"\">"
        );
    }

    #[test]
    fn escapes_addresses_written_as_tags() {
        assert_eq!(
            clean("Ann <ann@x.com> wrote"),
            "Ann &lt;ann@x.com&gt; wrote"
        );
    }

    #[test]
    fn decodes_uri_components_like_javascript() {
        assert_eq!(
            decode_uri_component("https://x.io/?a=1&u=https%3A%2F%2Fmailtrack.io"),
            "https://x.io/?a=1&u=https://mailtrack.io"
        );
        assert_eq!(decode_uri_component("100%"), "100%");
        assert_eq!(decode_uri_component("%E2%82"), "%E2%82");
    }
}
