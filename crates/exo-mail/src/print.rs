//! A message body for printing: the body under its `@media print` rules, as markup for a browser
//! engine's shadow root, which lays it out and paginates it the way Chromium does (Exo's print view
//! does this with WebKit).
//!
//! It is [`BodyView`](crate::BodyView)'s document without what only Blitz needs: no host
//! isolation (a real shadow root isolates) and no presentational hints (a browser maps those
//! attributes itself). It is always Snow's: the Carbon rules for bodies, like the
//! thread view's, are screen-only, so Carbon prints exactly as Snow does.
//!
//! Remote content can't load from the engine: every remote URL is rewritten to
//! [`REMOTE_SCHEME_PREFIX`]`http(s)`, which only the print view's own handler loads, through the
//! same guarded fetches as a body on screen, or, with remote content blocked, to
//! [`BLOCKED_SCHEME`], which nothing loads.

use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::{Render, Theme, ViewOptions, dom, remote, view};

/// The body's print rules, after its own stylesheets.
const PRINT_CSS: &str = include_str!("print.css");

/// What a printed body's remote `https:`/`http:` URLs start with instead: `exo-https:`,
/// `exo-http:`. A protocol-relative URL resolves against the document, which is the print
/// view's job to put on `exo-https:` too.
pub const REMOTE_SCHEME_PREFIX: &str = "exo-";
/// The scheme a blocked body's remote URLs get: no handler answers it.
pub const BLOCKED_SCHEME: &str = "exo-blocked";

/// A remote URL's scheme (or a protocol-relative URL's `//`) in CSS: `url(…)` or `@import "…"`.
static CSS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(url\(\s*['"]?\s*|@import\s*['"]\s*)(https?:|//)"#).expect("valid regex")
});
/// A remote candidate's scheme in a `srcset`.
static SRCSET_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(^|,)(\s*)(https?:|//)").expect("valid regex"));

/// `render`'s body as it prints, with its quoted part when `show_quoted` (the `…`
/// itself doesn't print), loading nothing remote when `block_remote_content`: the markup that
/// goes inside the message's shadow root.
pub fn printable_body(render: &Render, show_quoted: bool, block_remote_content: bool) -> String {
    let options = ViewOptions {
        theme: Theme::Light,
        show_quoted,
        ..ViewOptions::default()
    };
    let root_class = view::root_class(render, options);
    let mut printed = Render {
        unquoted_html: rewrite_html(&render.unquoted_html, block_remote_content),
        quoted_html: if show_quoted {
            rewrite_html(&render.quoted_html, block_remote_content)
        } else {
            String::new()
        },
        has_quoted_content: render.has_quoted_content,
        body_attributes: render.body_attributes.clone(),
        unquoted_is_marketing: render.unquoted_is_marketing,
        quoted_is_marketing: render.quoted_is_marketing,
        ..Render::default()
    };
    if let Some(style) = &mut printed.body_attributes.style {
        *style = rewrite_css(style, block_remote_content);
    }
    view::shadow_root(&printed, show_quoted, &root_class, "", PRINT_CSS)
}

/// `html` (a fragment) with its remote URLs rewritten, and with `block`, its remote images
/// marked for `body.css`'s blocked-image box.
fn rewrite_html(html: &str, block: bool) -> String {
    if html.is_empty() {
        return String::new();
    }
    // Inside a `<div>` in the body, even a leading `<style>` stays where it is.
    let document = dom::parse(&format!(
        "<!DOCTYPE html><html><body><div>{html}</div></body></html>"
    ));
    let Some(root) =
        dom::body(&document).and_then(|body| dom::element_children(&body).into_iter().next())
    else {
        return html.to_owned();
    };
    for element in dom::select(&root, dom::is_element) {
        if block
            && dom::is(&element, "img")
            && (remote::is_remote(&dom::attr(&element, "src").unwrap_or_default())
                || remote::srcset_is_remote(&dom::attr(&element, "srcset").unwrap_or_default()))
        {
            // `body.css`'s box; the print view sizes it as Chromium sizes a broken image.
            dom::set_attr(&element, "data-exo-blocked", "");
        }
        // A lazy image would never load: a page that prints isn't scrolled.
        if dom::is(&element, "img") {
            dom::remove_attr(&element, "loading");
        }
        for name in ["src", "background", "poster"] {
            if let Some(url) = dom::attr(&element, name)
                && let Some(printed) = rewrite_url(&url, block)
            {
                dom::set_attr(&element, name, &printed);
            }
        }
        if let Some(srcset) = dom::attr(&element, "srcset") {
            let printed = SRCSET_URL.replace_all(&srcset, |found: &Captures| {
                format!("{}{}{}", &found[1], &found[2], scheme(&found[3], block))
            });
            dom::set_attr(&element, "srcset", &printed);
        }
        if let Some(style) = dom::attr(&element, "style") {
            dom::set_attr(&element, "style", &rewrite_css(&style, block));
        }
        if dom::is(&element, "style") {
            let css = dom::text_content(&element);
            let printed = rewrite_css(&css, block);
            if printed != css {
                dom::set_children(&element, vec![dom::text_node(&printed)]);
            }
        }
    }
    dom::inner_html(&root)
}

/// `css` with the URLs it loads from the network rewritten.
fn rewrite_css(css: &str, block: bool) -> String {
    CSS_URL
        .replace_all(css, |found: &Captures| {
            format!("{}{}", &found[1], scheme(&found[2], block))
        })
        .into_owned()
}

/// A remote `url` as the printed body names it; `None` for one that isn't remote.
fn rewrite_url(url: &str, block: bool) -> Option<String> {
    let trimmed = url.trim_start();
    if !remote::is_remote(trimmed) {
        return None;
    }
    let (start, rest) = match trimmed.strip_prefix("//") {
        Some(rest) => ("//", rest),
        None => {
            let colon = trimmed.find(':')?;
            (&trimmed[..=colon], &trimmed[colon + 1..])
        }
    };
    Some(format!("{}{rest}", scheme(start, block)))
}

/// What replaces a remote URL's `https:`, `http:` or `//`.
fn scheme(start: &str, block: bool) -> String {
    let start = start.to_ascii_lowercase();
    match (start.as_str(), block) {
        ("//", false) => format!("{REMOTE_SCHEME_PREFIX}https://"),
        ("//", true) => format!("{BLOCKED_SCHEME}://"),
        (_, false) => format!("{REMOTE_SCHEME_PREFIX}{start}"),
        (_, true) => format!("{BLOCKED_SCHEME}:"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BodyAttributes;

    #[test]
    fn remote_urls_load_through_the_print_view() {
        let html = rewrite_html(
            r#"<img src="https://x.io/a.png" srcset="cid:b 1x, HTTP://x.io/b.png 2x"><table background="//x.io/bg.png"><tr><td style="background:url( 'https://x.io/c.png' )">x</td></tr></table><style>@import "http://x.io/a.css"; p { background: url(//x.io/d.png) }</style><p>See https://x.io/e.png</p><img src="cid:f"><img src="data:image/gif;base64,R0lG">"#,
            false,
        );
        assert!(html.contains(r#"src="exo-https://x.io/a.png""#), "{html}");
        assert!(
            html.contains(r#"srcset="cid:b 1x, exo-http://x.io/b.png 2x""#),
            "{html}"
        );
        assert!(
            html.contains(r#"background="exo-https://x.io/bg.png""#),
            "{html}"
        );
        assert!(html.contains("url( 'exo-https://x.io/c.png' )"), "{html}");
        assert!(
            html.contains(r#"@import "exo-http://x.io/a.css""#)
                && html.contains("url(exo-https://x.io/d.png)"),
            "{html}"
        );
        assert!(
            html.contains("See https://x.io/e.png"),
            "text stays: {html}"
        );
        assert!(
            html.contains(r#"src="cid:f""#) && html.contains(r#"src="data:image/gif;base64,R0lG""#),
            "{html}"
        );
        assert!(!html.contains("data-exo-blocked"), "{html}");
    }

    #[test]
    fn blocked_bodies_load_nothing_remote() {
        let html = rewrite_html(
            r#"<img src="https://x.io/a.png" alt="A"><img src="cid:b"><div style="background:url(//x.io/c.png)">x</div>"#,
            true,
        );
        assert!(
            html.contains(r#"src="exo-blocked://x.io/a.png""#) && html.contains("data-exo-blocked"),
            "{html}"
        );
        assert!(
            html.contains(r#"<img src="cid:b">"#),
            "an inline attachment still shows: {html}"
        );
        assert!(html.contains("url(exo-blocked://x.io/c.png)"), "{html}");
        assert!(!html.contains("https:"), "{html}");
    }

    #[test]
    fn a_body_prints_in_snow_without_its_quote_unless_shown() {
        let render = Render {
            unquoted_html: "<p>New</p>".into(),
            quoted_html: "<blockquote>Old</blockquote>".into(),
            has_quoted_content: true,
            body_attributes: BodyAttributes {
                style: Some("background:url(https://x.io/bg.png)".into()),
                ..BodyAttributes::default()
            },
            ..Render::default()
        };
        let folded = printable_body(&render, false, false);
        assert!(
            folded.contains("ShadowHTML Theme-light Theme-snow"),
            "{folded}"
        );
        assert!(
            folded.contains("New") && !folded.contains("Old"),
            "{folded}"
        );
        assert!(
            folded.contains("exo-https://x.io/bg.png"),
            "the body's own background: {folded}"
        );
        assert!(
            folded.ends_with(&format!("<style>{PRINT_CSS}</style></div></div>")),
            "print rules come last"
        );
        let open = printable_body(&render, true, false);
        assert!(
            open.contains("Old") && open.contains("Exo-quoting-ellipsis isOpen"),
            "{open}"
        );
    }
}
