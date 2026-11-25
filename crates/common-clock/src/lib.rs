//! The present as billing reads it. Production reads the wall clock; a test built with the
//! `test-clock` feature can move it, so renewals, expiries and grace periods run on dates the test
//! chooses instead of rows backdated by SQL. The test clock keeps running from where it was set.
use chrono::{DateTime, SubsecRound, Utc};

/// The current instant, in whole microseconds as the database keeps it: an instant stored by any
/// path (a bound column truncates, a JSON value cast in SQL rounds) reads back equal to it.
pub fn now() -> DateTime<Utc> {
    #[cfg(feature = "test-clock")]
    let now = Utc::now() + test::offset();
    #[cfg(not(feature = "test-clock"))]
    let now = Utc::now();
    now.trunc_subsecs(6)
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
    use std::cell::RefCell;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    /// The clock of a process running one test (cargo nextest).
    static OFFSET_MICROS: AtomicI64 = AtomicI64::new(0);

    thread_local! {
        /// The clock of the test this thread runs, when tests share a process (cargo test).
        static SCOPE: RefCell<Option<Clock>> = const { RefCell::new(None) };
    }

    /// A test's clock, shared by the threads that run it.
    #[derive(Clone, Default)]
    pub struct Clock(Arc<AtomicI64>);

    fn one_test_per_process() -> bool {
        std::env::var_os("NEXTEST").is_some()
    }

    /// The clock of the test running on this thread, created on first use. A runtime of several
    /// threads hands it to each of them with [`enter`] (`on_thread_start`).
    pub fn scope() -> Clock {
        SCOPE.with(|scope| {
            scope
                .borrow_mut()
                .get_or_insert_with(Clock::default)
                .clone()
        })
    }

    /// Makes `clock` this thread's.
    pub fn enter(clock: Clock) {
        SCOPE.with(|scope| *scope.borrow_mut() = Some(clock));
    }

    pub(super) fn offset() -> Duration {
        let micros = if one_test_per_process() {
            OFFSET_MICROS.load(Ordering::Relaxed)
        } else {
            SCOPE.with(|scope| {
                scope
                    .borrow()
                    .as_ref()
                    .map_or(0, |clock| clock.0.load(Ordering::Relaxed))
            })
        };
        Duration::microseconds(micros)
    }

    /// Moves the present to `at`; time keeps passing from there. The process's clock under
    /// nextest, else the clock of the test running on this thread.
    pub fn set(at: DateTime<Utc>) {
        let micros = (at - Utc::now())
            .num_microseconds()
            .expect("an instant within 292 000 years");
        if one_test_per_process() {
            OFFSET_MICROS.store(micros, Ordering::Relaxed);
        } else {
            scope().0.store(micros, Ordering::Relaxed);
        }
    }

    /// Moves the present forward by `by`.
    pub fn advance(by: Duration) {
        set(super::now() + by);
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "test-clock")]
    #[test]
    fn each_test_thread_has_its_own_clock() {
        if std::env::var_os("NEXTEST").is_some() {
            return;
        }
        let far = chrono::Utc::now() + chrono::Duration::days(400);
        super::test::set(far);
        assert!(super::now() >= far);
        let other = std::thread::spawn(super::now).join().unwrap();
        assert!(other < far, "another test's thread keeps its own present");
        let clock = super::test::scope();
        let shared = std::thread::spawn(move || {
            super::test::enter(clock);
            super::now()
        })
        .join()
        .unwrap();
        assert!(shared >= far, "a thread entering the test's clock reads it");
    }

    #[test]
    fn the_present_has_no_sub_microsecond_part() {
        for _ in 0..1_000 {
            assert_eq!(super::now().timestamp_subsec_nanos() % 1_000, 0);
        }
    }
}
