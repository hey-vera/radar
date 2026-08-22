// SPDX-License-Identifier: Apache-2.0
//! Per-provider health and circuit breaking.
//!
//! Of roughly 80,000 tracked x402 endpoints, about 39% carry a D or F grade, and
//! a Helius-via-Corbits proxy was observed at an F with 0% uptime. Treating any
//! single paid endpoint as reliable is therefore not a defensible default. Every
//! provider sits behind a breaker, and a dead one costs Radar one failed attempt
//! per cooldown rather than one per request.

use core::fmt;

/// How a breaker should behave.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BreakerConfig {
    /// Consecutive failures that trip the breaker open.
    pub failure_threshold: u32,
    /// How long to stay open before allowing a trial request, in the same
    /// monotonic unit the caller passes to [`Breaker::allows`].
    pub cooldown: u64,
    /// Consecutive successes in half-open before closing again.
    pub half_open_successes: u32,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            cooldown: 30_000,
            half_open_successes: 2,
        }
    }
}

/// Breaker state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Requests flow normally.
    Closed,
    /// Requests are refused until the cooldown expires.
    Open {
        /// The instant at which a trial request becomes allowed.
        until: u64,
    },
    /// Trial requests are allowed; enough successes close the breaker.
    HalfOpen,
}

/// A circuit breaker with a rolling health score.
///
/// Has no clock: the caller supplies a monotonic instant, which keeps the
/// breaker replayable alongside everything else and lets a test cross a cooldown
/// without sleeping.
#[derive(Debug)]
pub struct Breaker {
    config: BreakerConfig,
    state: State,
    consecutive_failures: u32,
    consecutive_successes: u32,
    total_successes: u64,
    total_failures: u64,
    latency_ewma_micros: u64,
    trips: u64,
}

impl Breaker {
    /// A closed breaker.
    #[must_use]
    pub const fn new(config: BreakerConfig) -> Self {
        Self {
            config,
            state: State::Closed,
            consecutive_failures: 0,
            consecutive_successes: 0,
            total_successes: 0,
            total_failures: 0,
            latency_ewma_micros: 0,
            trips: 0,
        }
    }

    /// Current state, resolving an expired cooldown into half-open.
    #[must_use]
    pub const fn state_at(&self, now: u64) -> State {
        match self.state {
            State::Open { until } if now >= until => State::HalfOpen,
            other => other,
        }
    }

    /// Whether a request may be attempted now.
    pub fn allows(&mut self, now: u64) -> bool {
        self.state = self.state_at(now);
        !matches!(self.state, State::Open { .. })
    }

    /// Records a successful call and its latency.
    pub fn record_success(&mut self, latency_micros: u64) {
        self.total_successes += 1;
        self.consecutive_failures = 0;
        // Ten-sample EWMA. Enough to notice a provider degrading without
        // reacting to a single slow response.
        self.latency_ewma_micros = if self.latency_ewma_micros == 0 {
            latency_micros
        } else {
            (self.latency_ewma_micros * 9 + latency_micros) / 10
        };

        if self.state == State::HalfOpen {
            self.consecutive_successes += 1;
            if self.consecutive_successes >= self.config.half_open_successes {
                self.state = State::Closed;
                self.consecutive_successes = 0;
            }
        }
    }

    /// Records a failed call.
    pub fn record_failure(&mut self, now: u64) {
        self.total_failures += 1;
        self.consecutive_successes = 0;
        self.consecutive_failures += 1;

        // A failure during a trial sends the breaker straight back open. Serving
        // a half-open provider a full quota because one probe succeeded earlier
        // is how a flapping endpoint keeps costing money.
        if self.state == State::HalfOpen
            || self.consecutive_failures >= self.config.failure_threshold
        {
            self.state = State::Open {
                until: now.saturating_add(self.config.cooldown),
            };
            self.consecutive_failures = 0;
            self.trips += 1;
        }
    }

    /// Success rate over all recorded calls, in the range 0.0 to 1.0. Returns
    /// `None` before any call, which is different from a score of zero.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "a ratio for ranking and display; exactness beyond f64 is meaningless here"
    )]
    pub fn success_rate(&self) -> Option<f64> {
        let total = self.total_successes + self.total_failures;
        (total > 0).then(|| self.total_successes as f64 / total as f64)
    }

    /// Smoothed latency in microseconds, or `None` before any success.
    #[must_use]
    pub const fn latency_micros(&self) -> Option<u64> {
        if self.latency_ewma_micros == 0 {
            None
        } else {
            Some(self.latency_ewma_micros)
        }
    }

    /// How many times this breaker has tripped. A climbing count is the signal
    /// to drop a provider from the catalogue rather than keep retrying it.
    #[must_use]
    pub const fn trips(&self) -> u64 {
        self.trips
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("closed"),
            Self::Open { until } => write!(f, "open until {until}"),
            Self::HalfOpen => f.write_str("half-open"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BreakerConfig {
        BreakerConfig {
            failure_threshold: 3,
            cooldown: 1_000,
            half_open_successes: 2,
        }
    }

    #[test]
    fn a_healthy_provider_stays_closed() {
        let mut b = Breaker::new(config());
        for _ in 0..100 {
            assert!(b.allows(0));
            b.record_success(1_000);
        }
        assert_eq!(b.state_at(0), State::Closed);
        assert_eq!(b.trips(), 0);
    }

    #[test]
    fn repeated_failures_trip_the_breaker_and_stop_the_bleeding() {
        // A dead endpoint should cost one attempt per cooldown, not one per
        // request. This is the difference between a bad provider being an
        // annoyance and being a bill.
        let mut b = Breaker::new(config());
        for _ in 0..3 {
            assert!(b.allows(0));
            b.record_failure(0);
        }
        assert_eq!(b.state_at(0), State::Open { until: 1_000 });
        assert!(!b.allows(500), "still cooling down");
        assert_eq!(b.trips(), 1);
    }

    #[test]
    fn the_cooldown_expiring_allows_a_trial() {
        let mut b = Breaker::new(config());
        for _ in 0..3 {
            b.record_failure(0);
        }
        assert!(b.allows(1_000));
        assert_eq!(b.state_at(1_000), State::HalfOpen);
    }

    #[test]
    fn a_recovered_provider_closes_after_enough_successes() {
        let mut b = Breaker::new(config());
        for _ in 0..3 {
            b.record_failure(0);
        }
        assert!(b.allows(1_000));
        b.record_success(500);
        assert_eq!(
            b.state_at(1_000),
            State::HalfOpen,
            "one success is not enough"
        );
        b.record_success(500);
        assert_eq!(b.state_at(1_000), State::Closed);
    }

    #[test]
    fn a_flapping_provider_reopens_on_the_first_trial_failure() {
        // Without this, an endpoint that succeeds once per cooldown gets a full
        // quota of requests forever.
        let mut b = Breaker::new(config());
        for _ in 0..3 {
            b.record_failure(0);
        }
        assert!(b.allows(1_000));
        b.record_failure(1_000);
        assert_eq!(b.state_at(1_000), State::Open { until: 2_000 });
        assert_eq!(b.trips(), 2);
    }

    #[test]
    fn success_rate_is_none_before_any_call() {
        // Unknown health and zero health rank very differently when choosing a
        // provider; conflating them would retire every new endpoint on sight.
        let b = Breaker::new(config());
        assert_eq!(b.success_rate(), None);
        assert_eq!(b.latency_micros(), None);
    }

    #[test]
    fn success_rate_tracks_the_mix() {
        let mut b = Breaker::new(config());
        b.record_success(100);
        b.record_success(100);
        b.record_failure(0);
        b.record_success(100);
        assert_eq!(b.success_rate(), Some(0.75));
    }
}
