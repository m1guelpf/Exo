//! How a URL is shortened to show it.

use std::sync::LazyLock;

use regex::Regex;

/// `url` normalized, when it's a web link: the only kind Exo hands to the system. Mail decides
/// what its links and `List-Unsubscribe` headers say, and any other scheme would reach whichever
/// app claims it (`file:`, app URL schemes; `starts_with("http")` also lets `http-evil:` by).
pub fn web(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(String::from)
}

/// Percent-encodes every byte of `text`'s UTF-8 but letters, digits and `-_.!~*'()`.
pub fn encode_component(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// The URL a link preview shows in full: without `http://` and a trailing `/`.
pub fn shown(url: &str) -> String {
    static HTTP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^http://").expect("valid regex"));
    trailing_slash(&HTTP.replace(url, ""))
}

/// `/?`, `/#` and a final `/` lose their slash.
fn trailing_slash(url: &str) -> String {
    static SLASH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"/(\?|#|$)").expect("valid regex"));
    SLASH.replace(url, "$1").into_owned()
}

/// `url` in at most `max` characters: dropping the scheme, `www.`, the fragment and the query
/// in turn, then middle path segments (`host/a/…/file`), then cutting it short with `…`.
pub fn elide(url: &str, max: usize) -> String {
    static STEPS: LazyLock<[Regex; 5]> = LazyLock::new(|| {
        [r"^http://", r"^https://", r"^www\.", r"#.*", r"\?.*"]
            .map(|pattern| Regex::new(pattern).expect("valid regex"))
    });
    let length = |text: &str| text.encode_utf16().count();
    let mut text = trailing_slash(url);
    for step in STEPS.iter() {
        text = step.replace(&text, "").into_owned();
        if length(&text) <= max {
            return text;
        }
    }
    let host = text.split('/').next().unwrap_or_default().to_owned();
    let mut segments: Vec<String> = text
        .split_once('/')
        .map(|(_, path)| path.split('/').map(str::to_owned).collect())
        .unwrap_or_else(|| vec![String::new()]);
    let last = segments.pop().unwrap_or_default();
    while !segments.is_empty() && length(&text) > max {
        segments.pop();
        let middle = if segments.is_empty() {
            String::new()
        } else {
            format!("{}/", segments.join("/"))
        };
        text = format!("{host}/{middle}…/{last}");
    }
    if length(&text) <= max {
        return text;
    }
    let cut = |text: &str, n: usize| text.chars().take(n).collect::<String>();
    if length(&host) + 5 <= max {
        return format!("{}…", cut(&text, max.saturating_sub(1)));
    }
    text = format!("{host}/…");
    if length(&text) <= max {
        return text;
    }
    let mut labels: Vec<&str> = host.split('.').collect();
    while labels.len() > 2 && length(&text) > max {
        labels.remove(0);
        text = format!("…{}…", labels.join("."));
    }
    if length(&text) <= max {
        text
    } else {
        format!("{}…", cut(&text, max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_links_open() {
        assert_eq!(
            web("https://example.com/a b").as_deref(),
            Some("https://example.com/a%20b")
        );
        assert_eq!(
            web("HTTP://Example.com").as_deref(),
            Some("http://example.com/")
        );
        for other in [
            "http-evil://x",
            "httpx:x",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "mailto:a@b.co",
            "#top",
            "/relative",
            "zoommtg://zoom.us/join",
        ] {
            assert_eq!(web(other), None, "{other}");
        }
    }

    #[test]
    fn drops_what_it_can_before_cutting() {
        assert_eq!(
            elide("https://www.example.com/", 40),
            "https://www.example.com",
            "short enough already"
        );
        assert_eq!(elide("https://www.example.com/", 16), "www.example.com");
        assert_eq!(elide("https://example.com/a?b=c#d", 13), "example.com/a");
        assert_eq!(
            elide("https://example.com/one/two/three/file.html", 30),
            "example.com/one/…/file.html"
        );
        assert_eq!(
            elide("https://example.com/one/two/three/file.html", 24),
            "example.com/…/file.html"
        );
        assert_eq!(shown("http://example.com/path/"), "example.com/path");
        assert_eq!(shown("https://example.com/?q=1"), "https://example.com?q=1");
    }
}
