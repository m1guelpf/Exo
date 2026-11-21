//! `mailto:` links: the fields of the new message a click on one opens.

use super::{factory::Fields, recipients};

/// The new message `url` asks for: its addresses (the path and `to=`), `cc`, `bcc`, `subject`
/// and `body` (HTML-escaped, newlines as `<br>`, as compose takes it). `None` for a
/// URL that isn't `mailto:`.
pub(crate) fn fields(url: &str) -> Option<Fields> {
    let rest = url
        .get(..7)
        .filter(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
        .map(|_| &url[7..])?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut fields = Fields::default();
    let mut to: Vec<String> = Vec::new();
    let path = percent_decode(path);
    if !path.is_empty() {
        to.extend(path.split(',').map(str::to_owned));
    }
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        let list = |value: &str| value.split(',').map(recipients::parse).collect::<Vec<_>>();
        match key.to_lowercase().as_str() {
            "to" => to.extend(value.split(',').map(str::to_owned)),
            "cc" => fields.cc = list(&value),
            "bcc" => fields.bcc = list(&value),
            // The subject is a single line, so line breaks go.
            "subject" => fields.subject = value.replace(['\r', '\n'], ""),
            "body" => fields.body = escape(&value).replace("\r\n", "\n").replace('\n', "<br>"),
            _ => {}
        }
    }
    fields.to = to
        .iter()
        .map(|address| recipients::parse(address.trim()))
        .filter(|address| !address.email.is_empty() || address.name.is_some())
        .collect();
    Some(fields)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && let Some(byte) = text
                .get(at + 1..at + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            at += 3;
            continue;
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(super) fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_addresses_and_headers() {
        let fields = fields("mailto:ann@x.com,Bob%20Lee%20%3Cbob@x.com%3E?cc=c@x.com&Subject=Hello%20there&body=Line%201%0D%0ALine%20%3C2%3E").expect("mailto");
        let to: Vec<&str> = fields
            .to
            .iter()
            .map(|address| address.email.as_str())
            .collect();
        assert_eq!(to, ["ann@x.com", "bob@x.com"]);
        assert_eq!(fields.to[1].name.as_deref(), Some("Bob Lee"));
        assert_eq!(fields.cc[0].email, "c@x.com");
        assert_eq!(fields.subject, "Hello there");
        assert_eq!(fields.body, "Line 1<br>Line &lt;2&gt;");
    }

    #[test]
    fn a_subject_keeps_to_one_line() {
        let fields =
            fields("mailto:boss@x.com?subject=Invoice%0D%0ABcc:%20eve@evil.com").expect("mailto");
        assert_eq!(fields.subject, "InvoiceBcc: eve@evil.com");
    }

    #[test]
    fn only_mailto() {
        assert!(fields("https://x.com").is_none());
        assert!(fields("MAILTO:a@x.com").is_some());
    }
}
