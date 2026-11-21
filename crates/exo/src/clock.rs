//! What time it is, for everything dated relative to now: rows and headers, the calendar's
//! today, Get Me To Zero's cutoff, the defaults pickers start from.

use std::sync::Arc;

use jiff::Zoned;

/// Where Exo reads the time: the system's clock in the app. Tests and screenshots set it, since
/// gmail-mock's mail is dated relative to the mock's clock (`Config::now`).
#[derive(Clone)]
pub struct Clock(Arc<dyn Fn() -> Zoned + Send + Sync>);

impl Clock {
    /// The system's clock, in the system's time zone.
    pub fn system() -> Self {
        Self(Arc::new(Zoned::now))
    }

    /// Always `at`.
    pub fn fixed(at: Zoned) -> Self {
        Self(Arc::new(move || at.clone()))
    }

    /// Whatever `now` says, each time it's asked.
    pub fn new(now: impl Fn() -> Zoned + Send + Sync + 'static) -> Self {
        Self(Arc::new(now))
    }

    /// The time now.
    pub fn now(&self) -> Zoned {
        (self.0)()
    }
}
