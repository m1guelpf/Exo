//! The HTTP client: authentication, request execution and response decoding.

use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use reqwest::{Method, StatusCode, header};
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::Url;

use crate::endpoints::Endpoints;
use crate::error::{Error, ErrorResponse};

/// Boxed future returned by [`TokenProvider::access_token`].
pub type TokenFuture<'a> = Pin<Box<dyn Future<Output = Result<String, Error>> + Send + 'a>>;

/// Supplies OAuth access tokens. Implement it to plug in refresh and persistence; it is called
/// once per request (once per batch), so cache the token and refresh it shortly before expiry.
pub trait TokenProvider: Send + Sync {
    /// A currently valid access token.
    fn access_token(&self) -> TokenFuture<'_>;

    /// Google rejected `token`, which this provider handed out (the client calls this on every
    /// 401). Drop it if it's still the cached one, so the next
    /// [`access_token`](Self::access_token) call mints a fresh token; requests rejected together
    /// name the same token, so they cause one refresh. Callers retry once afterwards. The
    /// default does nothing, for providers that can't refresh.
    fn invalidate(&self, _token: &str) {}
}

/// One provider shared by several clients (the store's, the app's).
impl<T: TokenProvider + ?Sized> TokenProvider for Arc<T> {
    fn access_token(&self) -> TokenFuture<'_> {
        (**self).access_token()
    }

    fn invalidate(&self, token: &str) {
        (**self).invalidate(token);
    }
}

/// A fixed access token, for tests and tools.
#[derive(Clone)]
pub struct StaticToken(pub String);

/// Leaves the token out.
impl std::fmt::Debug for StaticToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("StaticToken").finish_non_exhaustive()
    }
}

impl TokenProvider for StaticToken {
    fn access_token(&self) -> TokenFuture<'_> {
        let token = self.0.clone();
        Box::pin(async move { Ok(token) })
    }
}

/// How long [`Client::new`]'s requests may take, start to finish.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Async client for Gmail, Calendar, People and userinfo. Cheap to clone.
#[derive(Clone)]
pub struct Client {
    pub(crate) http: reqwest::Client,
    pub(crate) endpoints: Arc<Endpoints>,
    tokens: Arc<dyn TokenProvider>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("endpoints", &self.endpoints)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// A client for `endpoints` that authenticates with `tokens`. A request that hasn't
    /// finished after five minutes fails.
    pub fn new(endpoints: Endpoints, tokens: impl TokenProvider + 'static) -> Self {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("the TLS backend initializes, as reqwest::Client::new assumes");
        Self::with_http(http, endpoints, tokens)
    }

    /// Like [`Client::new`], reusing an existing `reqwest` client (connection pool, timeouts).
    pub fn with_http(
        http: reqwest::Client,
        endpoints: Endpoints,
        tokens: impl TokenProvider + 'static,
    ) -> Self {
        Self {
            http,
            endpoints: Arc::new(endpoints),
            tokens: Arc::new(tokens),
        }
    }

    /// The endpoints this client talks to.
    #[must_use]
    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    /// The signed-in user's profile (`GET /oauth2/v2/userinfo`).
    pub fn userinfo(&self) -> Call<'_, crate::oauth::UserInfo> {
        Call::new(self, Method::GET, self.endpoints.oauth.userinfo.clone())
    }

    /// The access token for the next request.
    pub(crate) async fn token(&self) -> Result<String, Error> {
        self.tokens.access_token().await
    }

    /// Hands `token` back to the provider when Google answered a request made with it `status`
    /// 401.
    pub(crate) fn invalidate_on_401(&self, status: StatusCode, token: &str) {
        if status == StatusCode::UNAUTHORIZED {
            self.tokens.invalidate(token);
        }
    }
}

/// One API call, ready to run. `.await` it, or hand several to [`crate::Gmail::batch`].
#[must_use = "a Call does nothing until awaited"]
pub struct Call<'a, T> {
    client: &'a Client,
    pub(crate) request: Request,
    progress: Option<Progress>,
    response: PhantomData<fn() -> T>,
}

/// How far a call's transfer has come: the media an upload sends, or the body a download
/// receives. Called as each chunk goes or arrives; a call retried starts again from 0.
pub type Progress = Arc<dyn Fn(Transfer) + Send + Sync>;

/// Bytes moved so far, and how many there are when known (an upload always knows; a download
/// knows when Google sends a `Content-Length`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transfer {
    /// Sent or received so far.
    pub done: u64,
    /// The whole body.
    pub total: Option<u64>,
}

/// What an upload's body is streamed in, so its progress moves in steps this size.
const UPLOAD_CHUNK: usize = 64 << 10;

/// A request in transport-neutral form, so a call can run alone or inside a batch.
#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub method: Method,
    pub url: Url,
    pub body: Body,
}

#[derive(Debug, Clone)]
pub(crate) enum Body {
    Empty,
    Json(Vec<u8>),
    /// A media upload: `uploadType=multipart` with JSON metadata and an RFC 822 message.
    Upload {
        metadata: Vec<u8>,
        media: Vec<u8>,
    },
}

impl<'a, T> Call<'a, T> {
    pub(crate) fn new(client: &'a Client, method: Method, url: Url) -> Self {
        Self {
            client,
            request: Request {
                method,
                url,
                body: Body::Empty,
            },
            progress: None,
            response: PhantomData,
        }
    }

    /// Reports the transfer as it goes to `progress`: the upload's body when the call sends
    /// media, else the response's. Batches don't report.
    pub fn on_progress(mut self, progress: Progress) -> Self {
        self.progress = Some(progress);
        self
    }

    /// The same request with its response read as `U`: a type of the caller's that keeps what
    /// the API's own model would leave out (the raw source viewer shows Gmail's JSON as sent).
    pub fn returning<U>(self) -> Call<'a, U> {
        Call {
            client: self.client,
            request: self.request,
            progress: self.progress,
            response: PhantomData,
        }
    }

    /// Adds a query parameter. Repeated names are sent as repeated parameters.
    pub(crate) fn query(mut self, name: &str, value: impl ToString) -> Self {
        self.request
            .url
            .query_pairs_mut()
            .append_pair(name, &value.to_string());
        self
    }

    /// Adds `name=value` when `value` is set.
    pub(crate) fn query_opt(self, name: &str, value: Option<impl ToString>) -> Self {
        match value {
            Some(value) => self.query(name, value),
            None => self,
        }
    }

    /// Adds `name=value` for each value.
    pub(crate) fn query_all<V: ToString>(
        self,
        name: &str,
        values: impl IntoIterator<Item = V>,
    ) -> Self {
        values
            .into_iter()
            .fold(self, |call, value| call.query(name, value))
    }

    pub(crate) fn json(mut self, body: &impl Serialize) -> Self {
        self.request.body =
            Body::Json(serde_json::to_vec(body).expect("API request types always serialize"));
        self
    }

    pub(crate) fn upload(self, metadata: &impl Serialize, media: Vec<u8>) -> Self {
        let mut call = self.query("uploadType", "multipart");
        call.request.body = Body::Upload {
            metadata: serde_json::to_vec(metadata).expect("API request types always serialize"),
            media,
        };
        call
    }

    /// The Gmail quota units this call costs ([`crate::quota::units`]); 0 for other APIs.
    #[must_use]
    pub fn units(&self) -> u32 {
        let url = &self.request.url;
        crate::quota::units(self.request.method.as_str(), url.path(), url.query()).unwrap_or(0)
    }

    /// Restricts the response to a [partial response] field mask, e.g. `messages(id,labelIds)`.
    ///
    /// [partial response]: https://developers.google.com/workspace/gmail/api/guides/performance#partial
    pub fn fields(self, mask: &str) -> Self {
        self.query("fields", mask)
    }
}

impl<'a, T: DeserializeOwned + 'a> IntoFuture for Call<'a, T> {
    type Output = Result<T, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let Request { method, url, body } = self.request;
            let progress = self.progress;
            let uploads = matches!(body, Body::Upload { .. });
            log::debug!("{method} {}", url.path());
            let token = self.client.token().await?;
            let mut request = self.client.http.request(method, url).bearer_auth(&token);
            request = match body {
                Body::Empty => request,
                Body::Json(json) => request
                    .header(header::CONTENT_TYPE, "application/json; charset=UTF-8")
                    .body(json),
                Body::Upload { metadata, media } => {
                    let mut writer = crate::multipart::Writer::new(random_boundary("upload"));
                    writer.part(
                        &[("Content-Type", "application/json; charset=UTF-8")],
                        &metadata,
                    );
                    writer.part(&[("Content-Type", "message/rfc822")], &media);
                    let request =
                        request.header(header::CONTENT_TYPE, writer.content_type("related"));
                    match &progress {
                        Some(progress) => request.body(streamed(writer.finish(), progress.clone())),
                        None => request.body(writer.finish()),
                    }
                }
            };
            let mut response = request.send().await?;
            let status = response.status();
            self.client.invalidate_on_401(status, &token);
            let body = match progress.filter(|_| !uploads) {
                Some(progress) => {
                    let total = response.content_length();
                    let mut body = Vec::with_capacity(
                        total.map_or(0, |total| usize::try_from(total).unwrap_or(0)),
                    );
                    while let Some(chunk) = response.chunk().await? {
                        body.extend_from_slice(&chunk);
                        progress(Transfer {
                            done: body.len() as u64,
                            total,
                        });
                    }
                    body.into()
                }
                None => response.bytes().await?,
            };
            decode(status, &body)
        })
    }
}

/// `body` as a stream of [`UPLOAD_CHUNK`]s, each reported to `progress` as it's taken.
fn streamed(body: Vec<u8>, progress: Progress) -> reqwest::Body {
    let total = body.len() as u64;
    let chunks = (0..body.len()).step_by(UPLOAD_CHUNK).map(move |start| {
        let end = (start + UPLOAD_CHUNK).min(body.len());
        progress(Transfer {
            done: end as u64,
            total: Some(total),
        });
        Ok::<_, std::io::Error>(body[start..end].to_vec())
    });
    reqwest::Body::wrap_stream(futures::stream::iter(chunks))
}

/// Turns a response into `T` or the error Google described.
pub(crate) fn decode<T: DeserializeOwned>(status: StatusCode, body: &[u8]) -> Result<T, Error> {
    let decode_error = |reason: String| Error::Decode {
        status: status.as_u16(),
        reason: format!(
            "{reason}; body starts with {:?}",
            String::from_utf8_lossy(&body[..body.len().min(200)])
        ),
    };
    if !status.is_success() {
        return match serde_json::from_slice::<ErrorResponse>(body) {
            Ok(envelope) => Err(Error::Api(envelope.error)),
            Err(_) => Err(decode_error("not a Google error body".into())),
        };
    }
    parse(body).map_err(|error| decode_error(error.to_string()))
}

/// A successful response's body as `T`.
pub(crate) fn parse<T: DeserializeOwned>(body: &[u8]) -> serde_json::Result<T> {
    // Deletes and batch modifies answer 204 with no body; `()` deserializes from `null`.
    let body = if body.iter().all(u8::is_ascii_whitespace) {
        b"null".as_slice()
    } else {
        body
    };
    serde_json::from_slice(body)
}

/// A multipart boundary that won't collide with content.
pub(crate) fn random_boundary(prefix: &str) -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the OS random source is available");
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{prefix}_{hex}")
}
