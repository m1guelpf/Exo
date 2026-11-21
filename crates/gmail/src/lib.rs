#![deny(
    clippy::correctness,
    clippy::suspicious,
    clippy::complexity,
    clippy::perf
)]
//! Typed models and an async client for the Google APIs Exo uses: Gmail v1, Calendar v3,
//! People v1 and OAuth 2.0.
//!
//! The models are transcribed by hand from Google's discovery documents (kept in `discovery/`)
//! and are shared with `gmail-mock`, which serves the same wire format. Service origins are
//! configuration ([`Endpoints`]), so pointing Exo at the mock is a matter of swapping them.
//!
//! ```no_run
//! # async fn demo() -> Result<(), gmail::Error> {
//! use gmail::{Client, Endpoints, ListQuery, StaticToken};
//!
//! let client = Client::new(Endpoints::google(), StaticToken("ya29...".into()));
//! let inbox = client
//!     .gmail()
//!     .threads(&ListQuery { label_ids: vec!["INBOX".into()], ..ListQuery::default() })
//!     .await?;
//! # Ok(()) }
//! ```

mod api;
pub mod calendar;
mod client;
mod endpoints;
mod error;
pub mod mime;
mod model;
pub mod multipart;
pub mod oauth;
pub mod people;
pub mod quota;
pub mod wire;

pub use api::{DraftQuery, Gmail, HistoryQuery, InsertOptions, ListQuery, MAX_BATCH};
pub use client::{Call, Client, Progress, StaticToken, TokenFuture, TokenProvider, Transfer};
pub use endpoints::{Endpoints, OAuthEndpoints};
pub use error::{ApiError, Error, ErrorItem, ErrorResponse};
pub use model::*;
