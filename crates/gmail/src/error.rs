//! Errors, including Google's JSON error envelope.

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// Everything that can go wrong talking to Google.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Google answered with its standard JSON error envelope.
    #[error(transparent)]
    Api(#[from] ApiError),
    /// The OAuth token endpoint rejected a grant.
    #[error(transparent)]
    OAuth(#[from] crate::oauth::OAuthError),
    /// The request never got a response (connection, TLS, timeout, ...).
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// A response arrived but didn't have the shape we expected.
    #[error("unexpected response (HTTP {status}): {reason}")]
    Decode {
        /// HTTP status of the response.
        status: u16,
        /// What failed to parse, and a prefix of the body.
        reason: String,
    },
    /// The token provider couldn't produce an access token.
    #[error("couldn't get an access token: {0}")]
    Token(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// A batch couldn't be sent, or its multipart response couldn't be read.
    #[error("batch: {0}")]
    Batch(String),
}

impl Error {
    /// The HTTP status Google reported, for API and OAuth errors.
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api(error) => Some(error.code),
            Self::Decode { status, .. } => Some(*status),
            Self::Http(error) => error.status().map(|status| status.as_u16()),
            Self::OAuth(_) | Self::Token(_) | Self::Batch(_) => None,
        }
    }

    /// Whether Google said the entity doesn't exist (also how `history.list` reports an
    /// expired `startHistoryId`, which means the caller must do a full sync).
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        self.status() == Some(404)
    }
}

/// The `{"error": {...}}` envelope every Google JSON API uses for failures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// The error itself.
    pub error: ApiError,
}

/// Google's error body. Field order matches what Google sends.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{code} {}: {message}", status.as_deref().unwrap_or("ERROR"))]
pub struct ApiError {
    /// HTTP status code.
    pub code: u16,
    /// Human-readable message.
    pub message: String,
    /// Legacy (v1 error format) detail list. Google still sends it on every API we use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ErrorItem>,
    /// Canonical status, e.g. `NOT_FOUND`, `INVALID_ARGUMENT`, `UNAUTHENTICATED`.
    pub status: Option<String>,
    /// Structured `google.rpc` details (`ErrorInfo`, `BadRequest`, ...), kept as raw JSON.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<serde_json::Value>,
}

/// One entry of [`ApiError::errors`].
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorItem {
    /// Human-readable message.
    pub message: String,
    /// Usually `global`; absent on some validation errors.
    pub domain: Option<String>,
    /// Machine-readable reason, e.g. `notFound`, `invalidArgument`, `authError`.
    pub reason: String,
    /// Where the problem is, e.g. `Authorization`.
    pub location: Option<String>,
    /// What `location` refers to, e.g. `header` or `parameter`.
    pub location_type: Option<String>,
}
