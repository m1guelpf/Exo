//! MIME multipart framing for HTTP bodies: `multipart/mixed` batches and `multipart/related`
//! media uploads. Lenient on input (CRLF or bare LF, folded headers), CRLF on output.

/// A MIME entity: headers followed by a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity<'a> {
    /// Headers in order, names as sent, folded lines joined.
    pub headers: Vec<(String, String)>,
    /// Everything after the blank line that ends the headers.
    pub body: &'a [u8],
}

impl Entity<'_> {
    /// The first header named `name` (case-insensitive).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// The `boundary` parameter of a `multipart/*` content type.
#[must_use]
pub fn boundary(content_type: &str) -> Option<&str> {
    let (kind, params) = content_type.split_once(';')?;
    if !kind.trim().to_ascii_lowercase().starts_with("multipart/") {
        return None;
    }
    params.split(';').find_map(|param| {
        let (name, value) = param.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("boundary")
            .then(|| value.trim().trim_matches('"'))
    })
}

/// Splits a multipart body into its parts. `None` if no delimiter line is found.
#[must_use]
pub fn split<'a>(body: &'a [u8], boundary: &str) -> Option<Vec<Entity<'a>>> {
    let delimiter = format!("--{boundary}");
    let delimiter = delimiter.as_bytes();
    let mut cursor = find_delimiter(body, delimiter, 0)?;
    let mut parts = Vec::new();
    loop {
        let after = cursor + delimiter.len();
        if body[after..].starts_with(b"--") {
            return Some(parts);
        }
        // Skip transport padding and the line break after the delimiter.
        let start = body[after..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(body.len(), |offset| after + offset + 1);
        let Some(next) = find_delimiter(body, delimiter, start) else {
            // Unterminated: treat the rest as the last part, as most parsers do.
            parts.push(parse_entity(&body[start..]));
            return Some(parts);
        };
        // The line break before a delimiter belongs to the delimiter.
        let mut end = next;
        if body[..end].ends_with(b"\n") {
            end -= 1;
            if body[..end].ends_with(b"\r") {
                end -= 1;
            }
        }
        parts.push(parse_entity(&body[start..end.max(start)]));
        cursor = next;
    }
}

/// Position of the next `delimiter` that starts a line, at or after `from`.
fn find_delimiter(body: &[u8], delimiter: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    while at + delimiter.len() <= body.len() {
        let offset = body[at..]
            .windows(delimiter.len())
            .position(|window| window == delimiter)?;
        let found = at + offset;
        let starts_line = found == 0 || body[found - 1] == b'\n';
        // RFC 2046: only `--`, whitespace or a line break may follow the boundary.
        let ends_delimiter = body
            .get(found + delimiter.len())
            .is_none_or(|byte| b"-\r\n \t".contains(byte));
        if starts_line && ends_delimiter {
            return Some(found);
        }
        at = found + 1;
    }
    None
}

/// Parses headers up to the first blank line; the rest is the body.
#[must_use]
pub fn parse_entity(bytes: &[u8]) -> Entity<'_> {
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let line_end = bytes[at..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(bytes.len(), |offset| at + offset);
        let line = String::from_utf8_lossy(&bytes[at..line_end]);
        let line = line.trim_end_matches('\r');
        at = (line_end + 1).min(bytes.len());
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = headers.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
        } else if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    Entity {
        headers,
        body: &bytes[at..],
    }
}

/// Parses an `application/http` payload: a request or status line, then an entity.
#[must_use]
pub fn parse_http(bytes: &[u8]) -> Option<(String, Entity<'_>)> {
    // Tolerate blank lines before the start line.
    let start = bytes.iter().position(|byte| !byte.is_ascii_whitespace())?;
    let bytes = &bytes[start..];
    let line_end = bytes.iter().position(|&byte| byte == b'\n')?;
    let start_line = String::from_utf8_lossy(&bytes[..line_end])
        .trim_end_matches('\r')
        .to_owned();
    Some((start_line, parse_entity(&bytes[line_end + 1..])))
}

/// Writes a multipart body.
#[derive(Debug)]
pub struct Writer {
    boundary: String,
    out: Vec<u8>,
}

impl Writer {
    /// Starts a body delimited by `boundary`, which must not occur in any part.
    #[must_use]
    pub fn new(boundary: impl Into<String>) -> Self {
        Self {
            boundary: boundary.into(),
            out: Vec::new(),
        }
    }

    /// The `Content-Type` header value for this body, e.g. `multipart/mixed; boundary=...`.
    #[must_use]
    pub fn content_type(&self, subtype: &str) -> String {
        format!("multipart/{subtype}; boundary={}", self.boundary)
    }

    /// Appends a part.
    pub fn part(&mut self, headers: &[(&str, &str)], body: &[u8]) {
        self.out.extend_from_slice(b"--");
        self.out.extend_from_slice(self.boundary.as_bytes());
        self.out.extend_from_slice(b"\r\n");
        for (name, value) in headers {
            self.out
                .extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        self.out.extend_from_slice(b"\r\n");
        self.out.extend_from_slice(body);
        self.out.extend_from_slice(b"\r\n");
    }

    /// Closes the body.
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        self.out.extend_from_slice(b"--");
        self.out.extend_from_slice(self.boundary.as_bytes());
        self.out.extend_from_slice(b"--\r\n");
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The batch response from Google's "Batching requests" guide, verbatim (LF line endings).
    const GOOGLE_BATCH_RESPONSE: &str = r#"--batch_foobarbaz
Content-Type: application/http
Content-ID: <response-item1:12930812@barnyard.example.com>

HTTP/1.1 200 OK
Content-Type application/json
Content-Length: response_part_1_content_length
ETag: "etag/pony"

{
  "kind": "farm#animal",
  "animalName": "pony"
}

--batch_foobarbaz
Content-Type: application/http
Content-ID: <response-item3:12930812@barnyard.example.com>

HTTP/1.1 304 Not Modified
ETag: "etag/animals"

--batch_foobarbaz--
"#;

    #[test]
    fn reads_googles_documented_batch_response() {
        let parts = split(GOOGLE_BATCH_RESPONSE.as_bytes(), "batch_foobarbaz").unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(
            parts[0].header("content-id"),
            Some("<response-item1:12930812@barnyard.example.com>")
        );
        let (status, inner) = parse_http(parts[0].body).unwrap();
        assert_eq!(status, "HTTP/1.1 200 OK");
        assert_eq!(inner.header("ETag"), Some("\"etag/pony\""));
        let json: serde_json::Value = serde_json::from_slice(inner.body).unwrap();
        assert_eq!(json["animalName"], "pony");
        let (status, inner) = parse_http(parts[1].body).unwrap();
        assert_eq!(status, "HTTP/1.1 304 Not Modified");
        assert!(inner.body.is_empty());
    }

    #[test]
    fn round_trips_and_unfolds() {
        let mut writer = Writer::new("b0undary");
        writer.part(&[("Content-Type", "text/plain")], b"one\r\n--b0undaryish");
        writer.part(&[("X-Long", "a\r\n\tb")], b"");
        let body = writer.finish();
        let parts = split(&body, "b0undary").unwrap();
        assert_eq!(parts[0].body, b"one\r\n--b0undaryish");
        assert_eq!(parts[1].header("x-long"), Some("a b"));
    }

    #[test]
    fn reads_boundary_parameter() {
        assert_eq!(boundary("multipart/mixed; boundary=\"a b\""), Some("a b"));
        assert_eq!(boundary("multipart/related;boundary=xyz"), Some("xyz"));
        assert_eq!(boundary("text/plain; boundary=xyz"), None);
    }
}
