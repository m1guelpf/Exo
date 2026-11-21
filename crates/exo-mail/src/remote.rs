//! What a body loads from the network, for the Images setting: finding it, so ⌘K Show Images is
//! offered, and loading the read receipts `Show all images` loads.

use std::sync::LazyLock;

use crate::{
    Render,
    dom::{self, NodeRef},
    net::{Done, Fetch},
    view::BASE_URL,
};

/// A CSS `url()` or `@import` that loads from the network.
static REMOTE_CSS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?i)(?:url\(|@import)\s*['"]?\s*(?:https?:|//)"#).expect("valid regex")
});

/// Whether `url`, as a body names it, loads from the network: an absolute or protocol-relative
/// http(s) URL. (Relative ones resolve against a domain that never resolves.)
pub(crate) fn is_remote(url: &str) -> bool {
    let url = url.trim_start().as_bytes();
    let starts = |prefix: &str| {
        url.len() >= prefix.len() && url[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
    };
    starts("http:") || starts("https:") || starts("//")
}

/// Whether a `srcset` names a remote image.
pub(crate) fn srcset_is_remote(srcset: &str) -> bool {
    srcset.split(',').any(is_remote)
}

/// Whether anything under `root` loads from the network: an image, video poster or table
/// background, or a CSS image, font or stylesheet: the CSS ones are blocked too, so Show Images
/// is offered for them as well.
pub(crate) fn has_remote_content(root: &NodeRef) -> bool {
    !dom::select(root, |node| {
        let attribute = |name: &str| dom::attr(node, name).unwrap_or_default();
        ["src", "background", "poster"]
            .iter()
            .any(|name| is_remote(&attribute(name)))
            || srcset_is_remote(&attribute("srcset"))
            || REMOTE_CSS.is_match(&attribute("style"))
            || (dom::is(node, "style") && REMOTE_CSS.is_match(&dom::text_content(node)))
    })
    .is_empty()
}

/// Loads `render`'s read receipts, as `Show all images` does each time a message is shown
/// expanded; what they answer is dropped.
pub fn load_read_receipts(render: &Render, fetch: &dyn Fetch) {
    let base = url::Url::parse(BASE_URL).expect("the base URL parses");
    for url in &render.read_status_urls {
        let Some(url) = base
            .join(url)
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https"))
        else {
            continue;
        };
        let off_limits = url
            .host_str()
            .is_some_and(|host| host.ends_with("superhuman.com"));
        if !off_limits && !url.as_str().starts_with(BASE_URL) {
            fetch.fetch(url.as_str(), Done::ignored(url.as_str()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(html: &str) -> bool {
        has_remote_content(&dom::parse(html))
    }

    #[test]
    fn finds_what_loads_from_the_network() {
        assert!(
            is_remote("https://x.io/a.png")
                && is_remote(" HTTP://x.io")
                && is_remote("//x.io/a.png")
        );
        assert!(
            !is_remote("cid:a@b")
                && !is_remote("data:image/gif;base64,R0lG")
                && !is_remote("a.png")
        );
        assert!(remote("<img src=\"https://x.io/a.png\">"));
        assert!(remote(
            "<img src=\"cid:a\" srcset=\"cid:b 1x, https://x.io/b.png 2x\">"
        ));
        assert!(remote(
            "<table background=\"//x.io/bg.png\"><tr><td>x</td></tr></table>"
        ));
        assert!(remote(
            "<div style=\"background:url( 'https://x.io/bg.png' )\">x</div>"
        ));
        assert!(remote("<style>@import \"https://x.io/a.css\";</style>"));
        assert!(remote(
            "<style>@font-face { src: url(https://x.io/f.woff) }</style>"
        ));
        assert!(!remote(
            "<img src=\"cid:a\"><div style=\"background:url(data:image/png;base64,iVBO)\">x</div>"
        ));
        assert!(
            !remote("<p>See https://x.io/a.png</p>"),
            "text isn't loaded"
        );
    }
}
