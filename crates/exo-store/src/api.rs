//! Every Gmail call goes through [`Api::call`], one retry policy for sync and the outbox: backoff
//! with jitter on network errors, 5xx, rate limits and `Precondition check failed`; Gmail's retry
//! hints honored; one token refresh on 401. Before it goes out, a call spends its quota units
//! from the account's [`Budget`], which paces calls under Gmail's quota and holds them while
//! Gmail is rate limiting the account. How each Gmail call ends tells the account's [`Network`]
//! what's down and what's back.

use std::path::Path;
use std::time::Duration;

use gmail::{Call, Client, Gmail};
use jiff::{SignedDuration, Timestamp};
use serde::de::DeserializeOwned;

use crate::connection::{Connection, Debounce, Network, Outage};
use crate::error::Error;
use crate::quota::{Budget, Lane, MAX_WAIT, Quota};

pub(crate) struct Api {
    client: Client,
    /// The first retry's delay; later ones double.
    base: Duration,
    budget: Budget,
    network: Network,
}

/// How a call failed, once the policy gave up on it.
#[derive(Debug)]
pub(crate) enum Failure {
    /// No response at all.
    Offline(gmail::Error),
    /// Gmail is rate limiting the account and takes calls again then, later than a call waits.
    Throttled(Timestamp),
    /// Gmail won't send mail for the account until then (its sending limit, `(Mail sending)`),
    /// later than a call waits. Only sending is held: the account's other calls go on.
    SendingLimited(Timestamp),
    /// 401 even with a fresh token.
    Unauthorized(gmail::Error),
    /// Worth retrying later (5xx and the like), but retries ran out.
    Transient(gmail::Error),
    /// Retrying won't help (4xx).
    Permanent(gmail::Error),
}

impl Failure {
    /// Gmail said the resource doesn't exist (404, or 400 `Invalid id value`).
    pub fn is_not_found(&self) -> bool {
        match self {
            Self::Permanent(error) => {
                error.is_not_found()
                    || matches!(error, gmail::Error::Api(api) if api.message.contains("Invalid id value"))
            }
            _ => false,
        }
    }

    /// Gmail refused the call in a way retrying won't change (a 4xx other than rate limits
    /// and credentials).
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent(_))
    }

    /// Gmail's error message, for logs and failure reports.
    pub fn message(&self) -> String {
        match self {
            Self::Throttled(until) => format!("rate limited until {until}"),
            Self::SendingLimited(until) => format!("Gmail's sending limit until {until}"),
            Self::Offline(error)
            | Self::Unauthorized(error)
            | Self::Transient(error)
            | Self::Permanent(error) => error.to_string(),
        }
    }
}

impl From<Failure> for Error {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Offline(error) => Self::Offline(error),
            Failure::Throttled(until) | Failure::SendingLimited(until) => Self::Throttled { until },
            Failure::Unauthorized(error) => Self::Unauthorized(error),
            Failure::Transient(error) | Failure::Permanent(error) => Self::Gmail(error),
        }
    }
}

enum Kind {
    Network,
    RateLimited(Option<Duration>),
    Unauthorized,
    Transient,
    Permanent,
}

impl Api {
    /// Calls through `client`, paced under `quota` (`None` paces nothing), for a store in `dir`
    /// whose outages end as `debounce` says.
    pub fn new(
        client: Client,
        base: Duration,
        quota: Option<Quota>,
        debounce: Debounce,
        dir: &Path,
    ) -> Self {
        Self {
            client,
            base,
            budget: Budget::new(quota),
            network: Network::new(debounce, dir),
        }
    }

    /// The connection to Gmail, as calls have found it.
    pub fn connection(&self) -> tokio::sync::watch::Receiver<Connection> {
        self.network.subscribe()
    }

    /// Asks for Gmail's profile, once (`users.getProfile`: 1 unit).
    pub async fn ping(&self) -> Result<(), Failure> {
        self.call(Lane::Act, 1, |gmail| gmail.profile())
            .await
            .map(drop)
    }

    /// Something other than a Gmail call found `outage`.
    pub fn down(&self, outage: Outage) {
        self.network.down(outage);
    }

    /// Runs the Gmail call `make` builds for `lane`, trying up to `attempts` times. Sync passes
    /// a handful; the outbox passes 1 and retries at its own pace.
    pub async fn call<'a, T: DeserializeOwned + 'a>(
        &'a self,
        lane: Lane,
        attempts: u32,
        mut make: impl FnMut(Gmail<'a>) -> Call<'a, T>,
    ) -> Result<T, Failure> {
        self.send(&self.client, Some(lane), attempts, |client| {
            make(client.gmail())
        })
        .await
    }

    /// [`call`](Self::call) for Google's other APIs (Calendar, People), whose quotas are their
    /// own: neither paced nor paused by Gmail's.
    pub async fn request<'a, T: DeserializeOwned + 'a>(
        &'a self,
        attempts: u32,
        make: impl FnMut(&'a Client) -> Call<'a, T>,
    ) -> Result<T, Failure> {
        self.send(&self.client, None, attempts, make).await
    }

    /// [`request`](Self::request) through `client`, another account's (a calendar account's),
    /// on this account's connection and retry policy.
    pub async fn request_with<'a, T: DeserializeOwned + 'a>(
        &'a self,
        client: &'a Client,
        attempts: u32,
        make: impl FnMut(&'a Client) -> Call<'a, T>,
    ) -> Result<T, Failure> {
        self.send(client, None, attempts, make).await
    }

    /// The account's own client.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Runs the call through `client`, telling the connection how it ended: Gmail's (`lane`
    /// set) for every service, another API's for the network and Google's sign-in alone.
    async fn send<'a, T: DeserializeOwned + 'a>(
        &'a self,
        client: &'a Client,
        lane: Option<Lane>,
        attempts: u32,
        make: impl FnMut(&'a Client) -> Call<'a, T>,
    ) -> Result<T, Failure> {
        let gmail = lane.is_some();
        let result = self.retry(client, lane, attempts, make).await;
        match &result {
            Ok(_) => self.network.reached(gmail),
            Err(failure) => {
                if let Some(outage) = self.outage(failure, gmail) {
                    self.network.down(outage);
                }
            }
        }
        result
    }

    /// What a call that finally failed says is down. A 5xx that outlasted its retries got an
    /// answer, so it says nothing. Calendar and People that get no answer say the network is
    /// down. Their rate limits are their own, not Gmail's throttling.
    fn outage(&self, failure: &Failure, gmail: bool) -> Option<Outage> {
        match failure {
            Failure::Offline(gmail::Error::Http(http))
                if http.url() == Some(&self.client.endpoints().oauth.token) =>
            {
                Some(Outage::Auth)
            }
            Failure::Offline(_) if gmail => Some(Outage::Gmail),
            Failure::Offline(_) => Some(Outage::Offline),
            Failure::Throttled(_) if gmail => Some(Outage::Throttled),
            Failure::Permanent(gmail::Error::Api(api))
                if gmail && api.code == 400 && mentions(api, "Mail service not enabled") =>
            {
                Some(Outage::GmailDisabled)
            }
            _ => None,
        }
    }

    async fn retry<'a, T: DeserializeOwned + 'a>(
        &'a self,
        client: &'a Client,
        lane: Option<Lane>,
        attempts: u32,
        mut make: impl FnMut(&'a Client) -> Call<'a, T>,
    ) -> Result<T, Failure> {
        let mut refreshed = false;
        let mut attempt = 0;
        loop {
            let call = make(client);
            if let Some(lane) = lane {
                self.budget
                    .spend(lane, call.units())
                    .await
                    .map_err(Failure::Throttled)?;
            }
            attempt += 1;
            let error = match call.await {
                Ok(value) => {
                    if lane.is_some() {
                        self.budget.settled();
                    }
                    return Ok(value);
                }
                Err(error) => error,
            };
            let wait = match classify(&error) {
                // The client has handed the rejected token back, so the retry gets a fresh one.
                Kind::Unauthorized if !refreshed => {
                    log::info!("Gmail rejected the token; retrying once with a fresh one");
                    refreshed = true;
                    attempt -= 1;
                    continue;
                }
                Kind::Unauthorized => return Err(Failure::Unauthorized(error)),
                Kind::Permanent => return Err(Failure::Permanent(error)),
                Kind::RateLimited(after) => {
                    let wait = after.unwrap_or_else(|| self.delay(attempt));
                    // Gmail's sending limit holds sending alone, for hours at a time: it pauses
                    // nothing else.
                    let sending = sending_limit(&error);
                    if lane.is_some() && !sending {
                        // The retry, like every other Gmail call, waits for the pause in `spend`.
                        let until = self.budget.pushback(wait);
                        if attempt >= attempts {
                            return Err(Failure::Throttled(until));
                        }
                        log::debug!("Gmail rate limited a call ({error}); retrying after {until}");
                        continue;
                    }
                    if wait > MAX_WAIT || attempt >= attempts {
                        return Err(if sending {
                            Failure::SendingLimited(from_now(wait))
                        } else {
                            Failure::Throttled(from_now(wait))
                        });
                    }
                    wait
                }
                Kind::Network if attempt >= attempts => return Err(Failure::Offline(error)),
                Kind::Transient if attempt >= attempts => return Err(Failure::Transient(error)),
                Kind::Network | Kind::Transient => self.delay(attempt),
            };
            log::debug!("Gmail call failed ({error}); retrying in {wait:?}");
            tokio::time::sleep(wait).await;
        }
    }

    /// When the outbox retries after `failures` failures: in `min(120 s, base × 2^failures)`.
    pub fn postpone(&self, failures: u32) -> Timestamp {
        from_now(
            self.base
                .saturating_mul(2u32.saturating_pow(failures.min(16)))
                .min(Duration::from_secs(120)),
        )
    }

    /// `min(32 s, base × 2^(n−1) + jitter(0..base))`.
    fn delay(&self, attempt: u32) -> Duration {
        let exponential = self
            .base
            .saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1).min(16)));
        let jitter = self.base.mul_f64(random_unit());
        (exponential + jitter).min(MAX_WAIT)
    }
}

fn classify(error: &gmail::Error) -> Kind {
    match error {
        gmail::Error::Http(http) => match http.status() {
            None => Kind::Network,
            Some(status) if status.is_server_error() => Kind::Transient,
            Some(_) => Kind::Permanent,
        },
        gmail::Error::Api(api) => {
            // These count anywhere in the body; `rateLimitExceeded` also finds
            // `userRateLimitExceeded`.
            let says = |needle: &str| mentions(api, needle);
            match api.code {
                401 => Kind::Unauthorized,
                429 => Kind::RateLimited(retry_after(api)),
                403 if says("rateLimitExceeded") || says("Quota exceeded") => {
                    Kind::RateLimited(retry_after(api))
                }
                503 if says("CommandConcurrencyLimitReached") => {
                    Kind::RateLimited(retry_after(api))
                }
                500..=599 => Kind::Transient,
                400 if api.message.contains("Precondition check failed") => Kind::Transient,
                _ => Kind::Permanent,
            }
        }
        gmail::Error::Decode { status, .. } if *status >= 500 => Kind::Transient,
        gmail::Error::Token(_) | gmail::Error::OAuth(_) => Kind::Unauthorized,
        gmail::Error::Decode { .. } | gmail::Error::Batch(_) => Kind::Permanent,
    }
}

/// Gmail's daily sending limit, which it answers sends with as a rate limit "mentioning
/// `(Mail sending)`" (the Gmail API's "Resolve errors", 429) for hours, where its other calls
/// still go.
fn sending_limit(error: &gmail::Error) -> bool {
    matches!(error, gmail::Error::Api(api) if mentions(api, "(Mail sending)"))
}

/// Whether Google's error body says `needle` anywhere: message, reasons or details.
fn mentions(api: &gmail::ApiError, needle: &str) -> bool {
    api.message.contains(needle)
        || api
            .errors
            .iter()
            .any(|item| item.reason.contains(needle) || item.message.contains(needle))
        || api
            .details
            .iter()
            .any(|detail| detail.to_string().contains(needle))
}

/// How long Gmail asks to wait: the `Retry after <RFC 3339>` it writes into rate-limit
/// messages, or a `google.rpc.RetryInfo` detail's `retryDelay` (`"12s"`). Google's APIs put
/// retry hints in the error body rather than a `Retry-After` header.
fn retry_after(api: &gmail::ApiError) -> Option<Duration> {
    let in_message = || {
        let (_, rest) = api.message.split_once("Retry after ")?;
        let until: Timestamp = rest.split_whitespace().next()?.parse().ok()?;
        Duration::try_from(until.duration_since(Timestamp::now())).ok()
    };
    let in_details = || {
        api.details.iter().find_map(|detail| {
            let kind = detail.get("@type")?.as_str()?;
            if !kind.ends_with("google.rpc.RetryInfo") {
                return None;
            }
            let delay = detail.get("retryDelay")?.as_str()?.strip_suffix('s')?;
            Duration::try_from_secs_f64(delay.parse().ok()?).ok()
        })
    };
    in_message().or_else(in_details)
}

fn from_now(wait: Duration) -> Timestamp {
    Timestamp::now()
        .checked_add(SignedDuration::try_from(wait).unwrap_or(SignedDuration::MAX))
        .unwrap_or(Timestamp::MAX)
}

/// A uniform sample from `[0, 1)`.
fn random_unit() -> f64 {
    let bits = getrandom::u64().unwrap_or_default() >> 11;
    bits as f64 / (1u64 << 53) as f64
}
