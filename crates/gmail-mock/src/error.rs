//! Google's error responses, as the mock sends them.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use gmail::{ApiError, ErrorItem, ErrorResponse};
use serde_json::json;

/// A request failure, rendered as Google's `{"error": {...}}` envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub error: ApiError,
    /// `WWW-Authenticate` for 401s.
    challenge: Option<&'static str>,
}

pub type Result<T, E = Failure> = std::result::Result<T, E>;

impl Failure {
    /// Any status with Google's canonical status name and the given message and reason.
    pub fn new(code: u16, message: impl Into<String>, reason: &str) -> Self {
        let message = message.into();
        Self {
            error: ApiError {
                code,
                message: message.clone(),
                errors: vec![ErrorItem {
                    message,
                    domain: Some("global".into()),
                    reason: reason.into(),
                    location: None,
                    location_type: None,
                }],
                status: Some(canonical_status(code).into()),
                details: Vec::new(),
            },
            challenge: None,
        }
    }

    /// 404, the message Gmail uses for every missing resource.
    pub fn not_found() -> Self {
        Self::new(404, "Requested entity was not found.", "notFound")
    }

    /// 400 `INVALID_ARGUMENT`.
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(400, message, "invalidArgument")
    }

    /// 400 `FAILED_PRECONDITION`.
    pub fn failed_precondition(message: impl Into<String>) -> Self {
        let mut failure = Self::new(400, message, "failedPrecondition");
        failure.error.status = Some("FAILED_PRECONDITION".into());
        failure
    }

    /// 400 for a query parameter that doesn't parse as its declared type, e.g.
    /// `Invalid value at 'max_results' (TYPE_UINT32), "abc"`. These carry no `domain`.
    pub fn bad_value(parameter: &str, kind: &str, value: &str) -> Self {
        let field = snake_case(parameter);
        Self::field_violation(
            format!("Invalid value at '{field}' ({kind}), \"{value}\""),
            Some(field),
        )
    }

    /// 400 for a request body that isn't valid JSON for the method's request message.
    pub fn invalid_json(detail: impl std::fmt::Display) -> Self {
        Self::field_violation(format!("Invalid JSON payload received. {detail}"), None)
    }

    fn field_violation(message: String, field: Option<String>) -> Self {
        let violation = match field {
            Some(field) => json!({ "field": field, "description": message }),
            None => json!({ "description": message }),
        };
        let mut failure = Self::new(400, message, "invalid");
        failure.error.errors[0].domain = None;
        failure.error.details = vec![json!({
            "@type": "type.googleapis.com/google.rpc.BadRequest",
            "fieldViolations": [violation],
        })];
        failure
    }

    /// 401. `credentials_present` picks between the "missing" and "invalid" variants.
    pub fn unauthenticated(credentials_present: bool, service: &str) -> Self {
        let (message, item, reason, challenge) = if credentials_present {
            (
                "Request had invalid authentication credentials. Expected OAuth 2 access token, \
                 login cookie or other valid authentication credential. See \
                 https://developers.google.com/identity/sign-in/web/devconsole-project.",
                ("Invalid Credentials", "authError"),
                "ACCESS_TOKEN_TYPE_UNSUPPORTED",
                r#"Bearer realm="https://accounts.google.com/", error="invalid_token""#,
            )
        } else {
            (
                "Request is missing required authentication credential. Expected OAuth 2 access \
                 token, login cookie or other valid authentication credential. See \
                 https://developers.google.com/identity/sign-in/web/devconsole-project.",
                ("Login Required.", "required"),
                "CREDENTIALS_MISSING",
                r#"Bearer realm="https://accounts.google.com/""#,
            )
        };
        let mut failure = Self::new(401, message, item.1);
        failure.error.errors[0].message = item.0.into();
        failure.error.errors[0].location = Some("Authorization".into());
        failure.error.errors[0].location_type = Some("header".into());
        failure.error.details = vec![json!({
            "@type": "type.googleapis.com/google.rpc.ErrorInfo",
            "reason": reason,
            "domain": "googleapis.com",
            "metadata": { "service": service },
        })];
        failure.challenge = Some(challenge);
        failure
    }

    /// 403 for a `userId` other than the authenticated user.
    pub fn delegation_denied(user: &str) -> Self {
        Self::new(403, format!("Delegation denied for {user}"), "forbidden")
    }

    /// Calendar v3 still uses the legacy error body: no `status` field.
    pub fn legacy(mut self) -> Self {
        self.error.status = None;
        self
    }
}

impl From<ApiError> for Failure {
    fn from(error: ApiError) -> Self {
        Self {
            error,
            challenge: None,
        }
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.error.code).unwrap_or(StatusCode::BAD_REQUEST);
        let body = ErrorResponse { error: self.error };
        let mut response = (status, axum::Json(body)).into_response();
        if let Some(challenge) = self.challenge {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static(challenge),
            );
        }
        response
    }
}

/// The `google.rpc.Code` name Google pairs with each HTTP status.
fn canonical_status(code: u16) -> &'static str {
    match code {
        400 => "INVALID_ARGUMENT",
        401 => "UNAUTHENTICATED",
        403 => "PERMISSION_DENIED",
        404 => "NOT_FOUND",
        409 => "ALREADY_EXISTS",
        410 => "NOT_FOUND",
        412 => "FAILED_PRECONDITION",
        429 => "RESOURCE_EXHAUSTED",
        499 => "CANCELLED",
        501 => "UNIMPLEMENTED",
        503 => "UNAVAILABLE",
        504 => "DEADLINE_EXCEEDED",
        _ if code >= 500 => "INTERNAL",
        _ => "UNKNOWN",
    }
}

/// Google reports parameter names in proto (snake) case: `maxResults` → `max_results`.
fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for ch in name.chars() {
        if ch.is_ascii_uppercase() {
            out.push('_');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_matches_gmail() {
        let body = serde_json::to_value(ErrorResponse {
            error: Failure::not_found().error,
        })
        .unwrap();
        assert_eq!(
            body,
            json!({"error": {
                "code": 404,
                "message": "Requested entity was not found.",
                "errors": [{"message": "Requested entity was not found.", "domain": "global", "reason": "notFound"}],
                "status": "NOT_FOUND"
            }})
        );
    }

    #[test]
    fn bad_value_names_the_proto_field() {
        let failure = Failure::bad_value("maxResults", "TYPE_UINT32", "abc");
        assert_eq!(
            failure.error.message,
            r#"Invalid value at 'max_results' (TYPE_UINT32), "abc""#
        );
        assert_eq!(failure.error.errors[0].domain, None);
        assert_eq!(failure.error.status.as_deref(), Some("INVALID_ARGUMENT"));
    }
}
