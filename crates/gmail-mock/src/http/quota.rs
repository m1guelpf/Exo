//! Gmail's per-user quota, enforced when the mock is given one ([`Config::quota`]): every Gmail
//! call costs its method's units ([`gmail::quota::units`]), and one that would take the last
//! minute's spending past the quota is refused with a 429 saying when to retry, in the message
//! as Gmail writes it and in a `Retry-After` header. Refused calls cost nothing.
//!
//! [`Config::quota`]: crate::Config::quota

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use crate::error::Failure;
use crate::world::App;

const WINDOW: Duration = Duration::from_secs(60);

/// What the account has spent, and what was refused.
#[derive(Debug)]
pub struct Meter {
    /// Units allowed in any minute; `None` allows anything (and still counts).
    per_minute: Option<u32>,
    /// What was charged in the last minute: when, and how many units.
    recent: VecDeque<(Instant, u32)>,
    units: u64,
    calls: u64,
    refused: u64,
    /// Units and calls charged, by API method (`history.list`).
    methods: BTreeMap<&'static str, (u64, u64)>,
}

impl Meter {
    pub fn new(per_minute: Option<u32>) -> Self {
        Self {
            per_minute,
            recent: VecDeque::new(),
            units: 0,
            calls: 0,
            refused: 0,
            methods: BTreeMap::new(),
        }
    }

    /// Charges `units` for a call to `method` at `now`, or says how long until they'd fit.
    fn charge(&mut self, method: &'static str, units: u32, now: Instant) -> Result<(), Duration> {
        while self
            .recent
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) >= WINDOW)
        {
            self.recent.pop_front();
        }
        let used: u32 = self.recent.iter().map(|(_, units)| units).sum();
        if let Some(limit) = self.per_minute
            && used + units > limit
        {
            self.refused += 1;
            // Until enough of the minute's spending has aged out.
            let mut freed = 0;
            let wait = self.recent.iter().find_map(|(at, spent)| {
                freed += spent;
                (used + units - freed <= limit).then(|| WINDOW - now.duration_since(*at))
            });
            return Err(wait.unwrap_or(WINDOW));
        }
        self.recent.push_back((now, units));
        self.units += u64::from(units);
        self.calls += 1;
        let charged = self.methods.entry(method).or_default();
        charged.0 += u64::from(units);
        charged.1 += 1;
        Ok(())
    }

    /// `/__mock/quota`'s answer: `methods` has each API method's `units` and `calls`.
    pub fn summary(&self, now: Instant) -> Value {
        let last_minute: u32 = self
            .recent
            .iter()
            .filter(|(at, _)| now.duration_since(*at) < WINDOW)
            .map(|(_, units)| units)
            .sum();
        json!({
            "perMinute": self.per_minute,
            "lastMinute": last_minute,
            "units": self.units,
            "calls": self.calls,
            "refused": self.refused,
            "methods": self.methods.iter().map(|(method, (units, calls))| ((*method).to_owned(), json!({ "units": units, "calls": calls }))).collect::<serde_json::Map<_, _>>(),
        })
    }
}

/// Charges every Gmail call, refusing those over the quota.
pub async fn enforce(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let uri = request.uri();
    let Some((method, units)) =
        gmail::quota::cost(request.method().as_str(), uri.path(), uri.query())
    else {
        return next.run(request).await;
    };
    let charged = app.lock().quota.charge(method, units, Instant::now());
    match charged {
        Ok(()) => next.run(request).await,
        Err(wait) => refuse(wait),
    }
}

/// Gmail's 429. `Retry after` is in real time, which the client's clock compares it with, not
/// the mock's.
fn refuse(wait: Duration) -> Response {
    let until = Timestamp::now()
        .checked_add(SignedDuration::try_from(wait).unwrap_or(SignedDuration::MAX))
        .unwrap_or(Timestamp::MAX);
    let mut response = Failure::new(
        429,
        format!("User-rate limit exceeded.  Retry after {until}"),
        "rateLimitExceeded",
    )
    .into_response();
    let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from(seconds));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minute_holds_the_quota_and_frees_as_it_ages() {
        let start = Instant::now();
        let mut meter = Meter::new(Some(300));
        assert_eq!(meter.charge("threads.get", 120, start), Ok(()));
        assert_eq!(
            meter.charge("threads.get", 120, start + Duration::from_secs(10)),
            Ok(())
        );
        // 240 spent: a third full read would make 360.
        assert_eq!(
            meter.charge("threads.get", 120, start + Duration::from_secs(20)),
            Err(Duration::from_secs(40))
        );
        assert_eq!(
            meter.charge("messages.get", 60, start + Duration::from_secs(20)),
            Ok(())
        );
        assert_eq!(
            meter.charge("threads.get", 120, start + Duration::from_secs(61)),
            Ok(()),
            "the first read aged out"
        );
        assert_eq!((meter.calls, meter.refused, meter.units), (4, 1, 420));
        assert_eq!(
            meter.charge("threads.get", 400, start),
            Err(WINDOW),
            "more than a minute's quota never fits"
        );
        let methods = &meter.summary(start)["methods"];
        assert_eq!(
            (
                methods["threads.get"]["units"].as_u64(),
                methods["threads.get"]["calls"].as_u64()
            ),
            (Some(360), Some(3)),
            "refused calls cost nothing"
        );
        assert_eq!(methods["messages.get"]["units"], 60);
    }

    #[test]
    fn without_a_quota_everything_goes() {
        let mut meter = Meter::new(None);
        let now = Instant::now();
        assert!((0..1000).all(|_| meter.charge("threads.get", 120, now).is_ok()));
        assert_eq!(meter.summary(now)["lastMinute"], 120_000);
    }
}
