//! A monotonic clock that keeps counting while the machine is asleep.
//!
//! [`std::time::Instant`] on macOS is backed by `CLOCK_UPTIME_RAW`, which
//! pauses during system sleep. A keep-awake tool that only prevents *display*
//! sleep can still see the system sleep (lid closed on battery), and when it
//! wakes the IOKit-side timeout may already have fired. Measuring deadlines
//! with a sleep-inclusive clock keeps the two views consistent.

use std::ops::{Add, Sub};
use std::time::Duration;

/// A point on the sleep-inclusive monotonic clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Monotonic(Duration);

impl Monotonic {
    /// Time elapsed since `earlier`, saturating at zero.
    pub fn saturating_duration_since(self, earlier: Self) -> Duration {
        self.0.saturating_sub(earlier.0)
    }

    /// `self - now`, saturating at zero.
    pub fn remaining_from(self, now: Self) -> Duration {
        self.0.saturating_sub(now.0)
    }
}

impl Add<Duration> for Monotonic {
    type Output = Self;
    fn add(self, rhs: Duration) -> Self {
        Self(self.0.saturating_add(rhs))
    }
}

impl Sub<Monotonic> for Monotonic {
    type Output = Duration;
    fn sub(self, rhs: Monotonic) -> Duration {
        self.0.saturating_sub(rhs.0)
    }
}

/// Current reading of the sleep-inclusive monotonic clock.
pub fn now() -> Monotonic {
    Monotonic(crate::backend::monotonic_including_sleep())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_is_monotonic_and_arithmetic_saturates() {
        let a = now();
        let b = now();
        assert!(b >= a);
        assert_eq!(a.remaining_from(b), Duration::ZERO);
        let later = a + Duration::from_secs(5);
        assert!(later.remaining_from(a) >= Duration::from_secs(5));
        assert_eq!(later - a, Duration::from_secs(5));
    }
}
