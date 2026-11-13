//! The present as billing reads it. Production reads the wall clock; a test built with the
//! `test-clock` feature can move it, so renewals, expiries and grace periods run on dates the test
//! chooses instead of rows backdated by SQL. The test clock keeps running from where it was set.
use chrono::{DateTime, Utc};

/// The current instant.
pub fn now() -> DateTime<Utc> {
    #[cfg(feature = "test-clock")]
    {
        Utc::now() + test::offset()
    }
    #[cfg(not(feature = "test-clock"))]
    {
        Utc::now()
    }
}

/// The real instant: expiries that libraries check against the system clock (tokens, signatures),
/// the process's own schedules, and infrastructure rows the database stamps (webhook delivery,
/// batch jobs). Billing reads `now`.
pub fn wall() -> DateTime<Utc> {
    Utc::now()
}

#[cfg(feature = "test-clock")]
pub mod test {
    use super::*;
    use chrono::Duration;
    use std::sync::atomic::{AtomicI64, Ordering};

    static OFFSET_MICROS: AtomicI64 = AtomicI64::new(0);

    pub(super) fn offset() -> Duration {
        Duration::microseconds(OFFSET_MICROS.load(Ordering::Relaxed))
    }

    /// Moves the present to `at`; time keeps passing from there. The clock is per process, so
    /// this needs a runner with one process per test (cargo nextest).
    pub fn set(at: DateTime<Utc>) {
        assert!(
            std::env::var_os("NEXTEST").is_some(),
            "moving the clock needs cargo nextest: tests of one process would share it"
        );
        let micros = (at - Utc::now())
            .num_microseconds()
            .expect("an instant within 292 000 years");
        OFFSET_MICROS.store(micros, Ordering::Relaxed);
    }

    /// Moves the present forward by `by`.
    pub fn advance(by: Duration) {
        set(super::now() + by);
    }
}
