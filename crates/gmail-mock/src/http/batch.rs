//! `POST /batch/gmail/v1`: a `multipart/mixed` body of `application/http` requests, each run
//! through the API router as if it had arrived on its own.

use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, header};
use axum::response::{IntoResponse, Response};
use gmail::multipart;
use tower::ServiceExt;

use crate::error::Failure;

/// Google's limit on calls per batch.
const MAX_CALLS: usize = 100;

pub async fn handle(State(api): State<Router>, headers: HeaderMap, body: Bytes) -> Response {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let Some(parts) = multipart::boundary(content_type).and_then(|b| multipart::split(&body, b))
    else {
        return Failure::invalid_argument("Batch requests must be multipart/mixed").into_response();
    };
    if parts.len() > MAX_CALLS {
        return Failure::invalid_argument(format!(
            "Too many requests in batch; the limit is {MAX_CALLS}"
        ))
        .into_response();
    }

    let boundary = next_boundary();
    let mut writer = multipart::Writer::new(boundary.as_str());
    for part in parts {
        let inner = match multipart::parse_http(part.body)
            .and_then(|(line, entity)| build_request(&line, &entity.headers, entity.body, &headers))
        {
            Some(request) => api.clone().oneshot(request).await.into_response(),
            None => Failure::invalid_argument("Invalid batch part").into_response(),
        };
        let response_id = part
            .header("Content-ID")
            .map(|id| format!("<response-{}>", id.trim_matches(['<', '>'])));
        let mut part_headers = vec![("Content-Type", "application/http".to_owned())];
        if let Some(id) = response_id {
            part_headers.push(("Content-ID", id));
        }
        let headers: Vec<(&str, &str)> = part_headers
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        writer.part(&headers, &serialize_response(inner).await);
    }

    (
        [(header::CONTENT_TYPE, writer.content_type("mixed"))],
        writer.finish(),
    )
        .into_response()
}

/// An inner request from its request line and headers. Outer headers apply unless the inner
/// request overrides them, except `Content-*` ones.
fn build_request(
    line: &str,
    inner: &[(String, String)],
    body: &[u8],
    outer: &HeaderMap,
) -> Option<Request> {
    let mut words = line.split_whitespace();
    let method = Method::from_bytes(words.next()?.as_bytes()).ok()?;
    let target = words.next()?;
    // Some clients send absolute URLs; only the path and query matter.
    let target = target
        .find("://")
        .and_then(|scheme| {
            target[scheme + 3..]
                .find('/')
                .map(|path| &target[scheme + 3 + path..])
        })
        .unwrap_or(target);

    let mut request = Request::builder().method(method).uri(target);
    for (name, value) in outer {
        let is_content = name.as_str().starts_with("content-");
        let overridden = inner
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case(name.as_str()));
        if !is_content && !overridden {
            request = request.header(name, value);
        }
    }
    for (name, value) in inner {
        let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) else {
            continue;
        };
        if name != header::CONTENT_LENGTH {
            request = request.header(name, value);
        }
    }
    // Bodies end with the line break that precedes the next delimiter.
    let body = body.strip_suffix(b"\r\n").unwrap_or(body);
    request.body(Body::from(body.to_vec())).ok()
}

/// `HTTP/1.1 200 OK`, headers, blank line, body: how each inner response is framed.
async fn serialize_response(response: Response) -> Vec<u8> {
    let (parts, body) = response.into_parts();
    let body = to_bytes(body, usize::MAX).await.unwrap_or_default();
    let mut out = format!(
        "HTTP/1.1 {} {}\r\n",
        parts.status.as_u16(),
        parts.status.canonical_reason().unwrap_or_default()
    )
    .into_bytes();
    for (name, value) in &parts.headers {
        if name == header::CONTENT_LENGTH {
            continue;
        }
        out.extend_from_slice(title_case(name.as_str()).as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(value.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&body);
    out
}

/// `content-type` → `Content-Type`, as Google writes header names.
fn title_case(name: &str) -> String {
    name.split('-')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_ascii_uppercase().to_string() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// Google's boundaries look like `batch_pK7JBAk73-E=_AA5eFwv4m2Q=`; ours only need to be
/// unique and absent from the content.
fn next_boundary() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("batch_{:016x}", n.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}
