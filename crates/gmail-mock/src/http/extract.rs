//! Request extraction with Google's parsing rules and error bodies: query parameters,
//! strict JSON bodies, and Gmail's media uploads.

use axum::body::Bytes;
use axum::extract::{FromRequest, FromRequestParts, OriginalUri, Request};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

use crate::error::{Failure, Result};

/// Query parameters. Names match in camelCase or snake_case, as Google accepts both. Unknown
/// parameters are ignored (a cache buster like `_nocache`, say).
#[derive(Debug, Clone, Default)]
pub struct Params(Vec<(String, String)>);

impl<S: Send + Sync> FromRequestParts<S> for Params {
    type Rejection = Failure;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self> {
        Ok(Self::parse(parts.uri.query().unwrap_or_default()))
    }
}

impl Params {
    pub fn parse(query: &str) -> Self {
        Self(
            url::form_urlencoded::parse(query.as_bytes())
                .map(|(name, value)| (camel_case(&name), value.into_owned()))
                .collect(),
        )
    }

    /// The last value of `name`, if any.
    pub fn get(&self, name: &str) -> Option<&str> {
        let name = camel_case(name);
        self.0
            .iter()
            .rev()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Every value of a repeated parameter.
    pub fn all(&self, name: &str) -> Vec<String> {
        let name = camel_case(name);
        self.0
            .iter()
            .filter(|(key, _)| *key == name)
            .map(|(_, value)| value.clone())
            .collect()
    }

    /// A non-empty string parameter.
    pub fn text(&self, name: &str) -> Option<&str> {
        self.get(name).filter(|value| !value.is_empty())
    }

    pub fn u32(&self, name: &str) -> Result<Option<u32>> {
        self.parse_with(name, "TYPE_UINT32", |value| value.parse().ok())
    }

    pub fn u64(&self, name: &str) -> Result<Option<u64>> {
        self.parse_with(name, "TYPE_UINT64", |value| value.parse().ok())
    }

    pub fn bool(&self, name: &str) -> Result<bool> {
        Ok(self
            .parse_with(name, "TYPE_BOOL", |value| {
                match value.to_ascii_lowercase().as_str() {
                    "true" | "1" => Some(true),
                    "false" | "0" => Some(false),
                    _ => None,
                }
            })?
            .unwrap_or(false))
    }

    /// An enum parameter, accepting the wire spelling (`messageAdded`) or the proto one
    /// (`MESSAGE_ADDED`), as Google does.
    pub fn enumeration<T: DeserializeOwned>(&self, name: &str, proto: &str) -> Result<Option<T>> {
        self.get(name)
            .map(|value| parse_enum(name, proto, value))
            .transpose()
    }

    /// Every value of a repeated enum parameter.
    pub fn enumerations<T: DeserializeOwned>(&self, name: &str, proto: &str) -> Result<Vec<T>> {
        self.all(name)
            .iter()
            .map(|value| parse_enum(name, proto, value))
            .collect()
    }

    /// `maxResults`, defaulting and capping like Gmail (which caps silently).
    pub fn max_results(&self, default: u32, cap: u32) -> Result<usize> {
        let requested = self
            .u32("maxResults")?
            .filter(|&n| n > 0)
            .unwrap_or(default);
        Ok(requested.min(cap) as usize)
    }

    fn parse_with<T>(
        &self,
        name: &str,
        kind: &str,
        parse: impl Fn(&str) -> Option<T>,
    ) -> Result<Option<T>> {
        self.get(name)
            .map(|value| parse(value).ok_or_else(|| Failure::bad_value(name, kind, value)))
            .transpose()
    }
}

fn parse_enum<T: DeserializeOwned>(name: &str, proto: &str, value: &str) -> Result<T> {
    let attempt = |candidate: &str| serde_json::from_value(candidate.into()).ok();
    attempt(value)
        .or_else(|| attempt(&camel_case(&value.to_ascii_lowercase())))
        .ok_or_else(|| Failure::bad_value(name, proto, value))
}

fn camel_case(name: &str) -> String {
    if !name[1.min(name.len())..].contains('_') {
        return name.to_owned();
    }
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for (index, ch) in name.chars().enumerate() {
        if ch == '_' && index > 0 {
            upper = true;
        } else if upper {
            out.push(ch.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// A JSON request body, rejecting unknown fields the way Google does. An empty body is the
/// empty message.
pub struct Json<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Default> FromRequest<S> for Json<T> {
    type Rejection = Failure;

    async fn from_request(request: Request, state: &S) -> Result<Self> {
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|error| Failure::invalid_argument(error.to_string()))?;
        parse_json(&bytes).map(Json)
    }
}

/// Parses `bytes` as `T`, failing on fields `T` doesn't know.
pub fn parse_json<T: DeserializeOwned + Default>(bytes: &[u8]) -> Result<T> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    let mut unknown = None;
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = serde_ignored::deserialize(&mut deserializer, |path| {
        unknown.get_or_insert_with(|| path.to_string());
    })
    .map_err(Failure::invalid_json)?;
    if let Some(path) = unknown {
        let detail = match path.rsplit_once('.') {
            Some((parent, name)) => {
                format!("Unknown name \"{name}\" at '{parent}': Cannot find field.")
            }
            None => format!("Unknown name \"{path}\": Cannot find field."),
        };
        return Err(Failure::invalid_json(detail));
    }
    Ok(value)
}

/// A request resource that carries an RFC 822 message (`Message.raw`, `Draft.message.raw`).
pub trait CarriesRaw: DeserializeOwned + Default {
    fn set_raw(&mut self, raw: Vec<u8>);
}

impl CarriesRaw for gmail::Message {
    fn set_raw(&mut self, raw: Vec<u8>) {
        self.raw = Some(raw);
    }
}

impl CarriesRaw for gmail::Draft {
    fn set_raw(&mut self, raw: Vec<u8>) {
        self.message.get_or_insert_default().raw = Some(raw);
    }
}

/// The body of send/insert/import/drafts calls: JSON with `raw` on the metadata path, or a
/// media upload (`uploadType=media` or `multipart`) on the `/upload/` path.
pub struct Upload<T>(pub T);

impl<S: Send + Sync, T: CarriesRaw> FromRequest<S> for Upload<T> {
    type Rejection = Failure;

    async fn from_request(request: Request, state: &S) -> Result<Self> {
        // Nested routers see a stripped path; the original says whether this is an upload.
        let is_upload = request
            .extensions()
            .get::<OriginalUri>()
            .map_or(request.uri(), |original| &original.0)
            .path()
            .starts_with("/upload/");
        let params = Params::parse(request.uri().query().unwrap_or_default());
        let content_type = request
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|error| Failure::invalid_argument(error.to_string()))?;
        if !is_upload {
            return parse_json(&bytes).map(Upload);
        }
        match params.get("uploadType") {
            Some("media") => {
                let mut resource = T::default();
                resource.set_raw(bytes.to_vec());
                Ok(Upload(resource))
            }
            Some("multipart") => {
                let parts = gmail::multipart::boundary(&content_type)
                    .and_then(|boundary| gmail::multipart::split(&bytes, boundary))
                    .filter(|parts| parts.len() == 2)
                    .ok_or_else(|| {
                        Failure::invalid_argument(
                            "Multipart uploads need a JSON metadata part and a media part",
                        )
                    })?;
                let mut resource: T = parse_json(parts[0].body)?;
                resource.set_raw(parts[1].body.to_vec());
                Ok(Upload(resource))
            }
            // Resumable uploads need session state the mock doesn't keep.
            Some(other) => Err(Failure::invalid_argument(format!(
                "Unsupported uploadType: {other}"
            ))),
            None => Err(Failure::invalid_argument(
                "Upload requests must include an uploadType URL parameter and a URL path \
                 beginning with /upload/",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_snake_case_and_ignores_unknown() {
        let params = Params::parse("max_results=5&_nocache=0.3&labelIds=A&labelIds=B");
        assert_eq!(params.u32("maxResults").unwrap(), Some(5));
        assert_eq!(params.all("labelIds"), ["A", "B"]);
        assert!(params.u32("labelIds").is_err());
    }

    #[test]
    fn enums_accept_proto_spelling() {
        let params = Params::parse("format=METADATA&historyTypes=LABEL_ADDED");
        let format: Option<gmail::Format> = params.enumeration("format", "x").unwrap();
        assert_eq!(format, Some(gmail::Format::Metadata));
        let types: Vec<gmail::HistoryType> = params.enumerations("historyTypes", "x").unwrap();
        assert_eq!(types, [gmail::HistoryType::LabelAdded]);
    }

    #[test]
    fn rejects_unknown_json_fields() {
        let error =
            parse_json::<gmail::ModifyMessageRequest>(br#"{"addLabels": ["A"]}"#).unwrap_err();
        assert_eq!(
            error.error.message,
            "Invalid JSON payload received. Unknown name \"addLabels\": Cannot find field."
        );
        let draft: gmail::Draft = parse_json(b"").unwrap();
        assert_eq!(draft, gmail::Draft::default());
    }
}
