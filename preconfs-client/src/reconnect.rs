//! Resubscribing after a stream drops.
//!
//! Points of presence restart on every deploy and the anycast address can
//! move a connection between them, so a long lived stream will drop. With
//! reconnect on (the default) the stream resubscribes with the same request
//! after a backoff and yields [`Event::Reconnected`](crate::Event::Reconnected)
//! so the program knows it missed the data produced in between. Preconfs
//! from the gap cannot be replayed: they were only valuable before the slot
//! landed.
//!
//! Errors that retrying cannot fix end the stream instead: a bad token,
//! a refused filter, a region the server does not serve, a revoked feed.

use {
    crate::error::StreamError,
    std::time::Duration,
    tonic::{Code, Status},
};

/// Retry schedule for resubscribing after the stream drops.
///
/// The interval starts at `initial_interval` and grows by `multiplier`
/// after every failed attempt, capped at `max_interval`. Attempts reset once
/// a stream delivers an event again. With `max_retries` set, the stream
/// ends with the last error after that many consecutive failures; without
/// it, retries continue until a terminal error.
///
/// The default retries forever with 100ms, 200ms, 400ms ... up to 10s
/// between attempts. Pass it to [`Connector::reconnect`](crate::Connector::reconnect),
/// or call [`Connector::no_reconnect`](crate::Connector::no_reconnect) to
/// have the stream end on the first drop.
///
/// ```
/// use std::time::Duration;
/// use triton_preconfs_client::Reconnect;
///
/// let reconnect = Reconnect::default()
///     .max_interval(Duration::from_secs(30))
///     .max_retries(Some(20));
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Reconnect {
    /// Delay before the first retry.
    pub initial_interval: Duration,
    /// Growth factor applied after each failed attempt.
    pub multiplier: f64,
    /// Upper bound for the delay between attempts.
    pub max_interval: Duration,
    /// Consecutive failures after which the stream gives up; `None` never
    /// gives up.
    pub max_retries: Option<u32>,
}

impl Default for Reconnect {
    fn default() -> Self {
        Self {
            initial_interval: Duration::from_millis(100),
            multiplier: 2.0,
            max_interval: Duration::from_secs(10),
            max_retries: None,
        }
    }
}

impl Reconnect {
    /// Delay before the first retry.
    pub const fn initial_interval(mut self, interval: Duration) -> Self {
        self.initial_interval = interval;
        self
    }

    /// Growth factor applied after each failed attempt.
    pub const fn multiplier(mut self, multiplier: f64) -> Self {
        self.multiplier = multiplier;
        self
    }

    /// Upper bound for the delay between attempts.
    pub const fn max_interval(mut self, interval: Duration) -> Self {
        self.max_interval = interval;
        self
    }

    /// Consecutive failures after which the stream gives up; `None` never
    /// gives up.
    pub const fn max_retries(mut self, retries: Option<u32>) -> Self {
        self.max_retries = retries;
        self
    }

    /// Delay before attempt number `attempt` (1 based). The growth is
    /// computed in seconds and converted at the end: `Duration::mul_f64`
    /// panics once the product no longer fits, which the defaults reach at
    /// attempt 69, and a stream that keeps failing gets there in about ten
    /// minutes. Anything that does not convert is the cap.
    pub fn interval(&self, attempt: u32) -> Duration {
        let exponent = i32::try_from(attempt.saturating_sub(1)).unwrap_or(i32::MAX);
        let factor = self.multiplier.powi(exponent).max(1.0);
        Duration::try_from_secs_f64(self.initial_interval.as_secs_f64() * factor)
            .map_or(self.max_interval, |delay| delay.min(self.max_interval))
    }

    /// Whether `attempts` consecutive failures exhaust the budget.
    pub fn exhausted(&self, attempts: u32) -> bool {
        self.max_retries.is_some_and(|max| attempts > max)
    }
}

/// Whether resubscribing can fix this error. A closed stream and the
/// statuses a restarting or overloaded server sends are retried; statuses
/// about the request itself or the token are not.
pub(crate) fn retryable(error: &StreamError) -> bool {
    match error {
        StreamError::Closed => true,
        StreamError::Status(status) => retryable_status(status),
    }
}

fn retryable_status(status: &Status) -> bool {
    match status.code() {
        // Unavailable: the point of presence is restarting or the anycast
        // route moved. DataLoss: the server dropped this subscriber for
        // lagging. ResourceExhausted: too many streams or the coverage
        // cooloff, both clear with time.
        Code::Unavailable
        | Code::DataLoss
        | Code::ResourceExhausted
        | Code::Internal
        | Code::Unknown
        | Code::Aborted
        | Code::Cancelled
        | Code::DeadlineExceeded => true,
        Code::Ok
        | Code::Unauthenticated
        | Code::PermissionDenied
        | Code::InvalidArgument
        | Code::FailedPrecondition
        | Code::NotFound
        | Code::AlreadyExists
        | Code::OutOfRange
        | Code::Unimplemented => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let reconnect = Reconnect::default();
        assert_eq!(reconnect.interval(1), Duration::from_millis(100));
        assert_eq!(reconnect.interval(2), Duration::from_millis(200));
        assert_eq!(reconnect.interval(4), Duration::from_millis(800));
        assert_eq!(reconnect.interval(20), Duration::from_secs(10));
        // Past what a Duration can hold, and past what fits the exponent.
        assert_eq!(reconnect.interval(69), Duration::from_secs(10));
        assert_eq!(reconnect.interval(u32::MAX), Duration::from_secs(10));
        assert!(!reconnect.exhausted(1_000));
        let bounded = Reconnect::default().max_retries(Some(3));
        assert!(!bounded.exhausted(3));
        assert!(bounded.exhausted(4));
    }

    #[test]
    fn request_errors_are_terminal_and_outages_are_not() {
        assert!(retryable(&StreamError::Closed));
        assert!(retryable(&StreamError::Status(Status::unavailable(
            "restarting"
        ))));
        assert!(retryable(&StreamError::Status(Status::resource_exhausted(
            "cooling off"
        ))));
        assert!(!retryable(&StreamError::Status(Status::unauthenticated(
            "bad token"
        ))));
        assert!(!retryable(&StreamError::Status(Status::invalid_argument(
            "bad filter"
        ))));
    }
}
