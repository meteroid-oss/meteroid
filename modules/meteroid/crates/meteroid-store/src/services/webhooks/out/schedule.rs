use chrono::{Duration, NaiveDateTime};
use rand::{Rng, RngExt};

/// Backoff after failed attempt N (1-based). The 8th failure is terminal, so only seven
/// delays exist.
pub const RETRY_DELAYS_SECS: [i64; 7] = [5, 300, 1_800, 7_200, 18_000, 36_000, 36_000];
pub const MAX_ATTEMPTS: i32 = 8;
pub const JITTER_FACTOR: f64 = 0.10;

/// Failures needed before an endpoint may be auto-disabled.
pub const AUTO_DISABLE_CONSECUTIVE_FAILURES: i32 = 20;
pub const DEFAULT_AUTO_DISABLE_AFTER_HOURS: i64 = 120;

/// Consecutive endpoint-wide failures after which the endpoint is paused, then probed
/// one delivery at a time until it answers again.
pub const PROBE_THRESHOLD: i32 = 5;
/// Pause after the Nth endpoint-wide failure past the threshold; the last step repeats.
pub const PAUSE_STEPS_SECS: [i64; 7] = [30, 60, 120, 300, 600, 1_800, 3_600];

/// No usable answer from the endpoint as a whole: nothing came back (DNS, connection,
/// TLS, timeout, egress refusal), or a status saying the server or its proxy cannot
/// take requests right now. Any other HTTP answer is about that one event.
pub fn is_endpoint_wide_failure(response_status: Option<u16>) -> bool {
    match response_status {
        None => true,
        Some(status) => matches!(status, 429 | 502 | 503 | 504),
    }
}

/// How long to pause an endpoint whose unreachable streak has reached `streak`, after
/// failing for `failing_for` (since `failing_since`). The pause is never shorter than the
/// outage so far, up to the last step: a quiet endpoint reaches the threshold only after
/// hours of its deliveries' own backoff, and must not then be probed every 30 seconds.
pub fn pause_after(streak: i32, failing_for: Duration) -> Option<Duration> {
    if streak < PROBE_THRESHOLD {
        return None;
    }
    let step = ((streak - PROBE_THRESHOLD) as usize).min(PAUSE_STEPS_SECS.len() - 1);
    let longest = Duration::seconds(PAUSE_STEPS_SECS[PAUSE_STEPS_SECS.len() - 1]);
    Some(Duration::seconds(PAUSE_STEPS_SECS[step]).max(failing_for.min(longest)))
}

/// `jitter` is a factor in `[-JITTER_FACTOR, JITTER_FACTOR]`; see [`random_jitter`].
/// Returns `None` once the attempt budget is spent.
pub fn next_delay(attempt_number: i32, jitter: f64) -> Option<Duration> {
    if !(1..MAX_ATTEMPTS).contains(&attempt_number) {
        return None;
    }

    let base = RETRY_DELAYS_SECS[(attempt_number - 1) as usize] as f64;
    let jittered = (base * (1.0 + jitter.clamp(-JITTER_FACTOR, JITTER_FACTOR))).max(0.0);

    Some(Duration::milliseconds((jittered * 1000.0).round() as i64))
}

pub fn next_attempt_at(
    now: NaiveDateTime,
    attempt_number: i32,
    jitter: f64,
) -> Option<NaiveDateTime> {
    next_delay(attempt_number, jitter).map(|delay| now + delay)
}

pub fn random_jitter<R: Rng + ?Sized>(rng: &mut R) -> f64 {
    rng.random_range(-JITTER_FACTOR..=JITTER_FACTOR)
}

/// Endpoint counters the pause and auto-disable decisions read.
#[derive(Clone, Copy, Debug)]
pub struct EndpointHealth {
    pub consecutive_failures: i32,
    /// First failure since the last success.
    pub failing_since: Option<NaiveDateTime>,
    pub unreachable_streak: i32,
}

/// An endpoint is disabled once everything sent to it has failed for the whole window,
/// counted from the first failure of the streak, and only after enough
/// attempts that a few isolated failures days apart never disable it.
pub fn should_auto_disable(health: &EndpointHealth, now: NaiveDateTime, window_hours: i64) -> bool {
    health.consecutive_failures >= AUTO_DISABLE_CONSECUTIVE_FAILURES
        && health
            .failing_since
            .is_some_and(|since| since < now - Duration::hours(window_hours))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(hours_ago: i64, now: NaiveDateTime) -> NaiveDateTime {
        now - Duration::hours(hours_ago)
    }

    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, 20)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
    }

    #[test]
    fn delays_follow_the_documented_ladder_without_jitter() {
        let secs: Vec<i64> = (1..MAX_ATTEMPTS)
            .map(|n| next_delay(n, 0.0).unwrap().num_seconds())
            .collect();

        assert_eq!(secs, RETRY_DELAYS_SECS.to_vec());
    }

    #[test]
    fn the_attempt_budget_is_eight() {
        assert!(next_delay(MAX_ATTEMPTS, 0.0).is_none());
        assert!(next_delay(MAX_ATTEMPTS + 1, 0.0).is_none());
        assert!(next_delay(0, 0.0).is_none());
    }

    #[test]
    fn jitter_stays_within_ten_percent() {
        for attempt in 1..MAX_ATTEMPTS {
            let base = RETRY_DELAYS_SECS[(attempt - 1) as usize] as f64;

            let low = next_delay(attempt, -JITTER_FACTOR).unwrap().num_seconds() as f64;
            let high = next_delay(attempt, JITTER_FACTOR).unwrap().num_seconds() as f64;

            assert!((low - base * 0.9).abs() <= 1.0, "attempt {attempt}");
            assert!((high - base * 1.1).abs() <= 1.0, "attempt {attempt}");
        }
    }

    #[test]
    fn out_of_range_jitter_is_clamped() {
        let base = RETRY_DELAYS_SECS[0] as f64;
        assert_eq!(
            next_delay(1, -50.0).unwrap().num_milliseconds(),
            (base * 0.9 * 1000.0).round() as i64
        );
        assert_eq!(
            next_delay(1, 50.0).unwrap().num_milliseconds(),
            (base * 1.1 * 1000.0).round() as i64
        );
    }

    #[test]
    fn next_attempt_at_offsets_from_now() {
        let now = now();
        assert_eq!(
            next_attempt_at(now, 2, 0.0).unwrap(),
            now + Duration::seconds(RETRY_DELAYS_SECS[1])
        );
        assert!(next_attempt_at(now, MAX_ATTEMPTS, 0.0).is_none());
    }

    #[test]
    fn random_jitter_is_bounded() {
        let mut rng = rand::rng();
        for _ in 0..1000 {
            let jitter = random_jitter(&mut rng);
            assert!((-JITTER_FACTOR..=JITTER_FACTOR).contains(&jitter));
        }
    }

    fn health(consecutive_failures: i32, failing_hours: Option<i64>) -> EndpointHealth {
        EndpointHealth {
            consecutive_failures,
            failing_since: failing_hours.map(|h| at(h, now())),
            unreachable_streak: 0,
        }
    }

    #[test]
    fn auto_disable_needs_twenty_consecutive_failures() {
        assert!(!should_auto_disable(
            &health(AUTO_DISABLE_CONSECUTIVE_FAILURES - 1, Some(1000)),
            now(),
            120
        ));
        assert!(should_auto_disable(
            &health(AUTO_DISABLE_CONSECUTIVE_FAILURES, Some(1000)),
            now(),
            120
        ));
    }

    #[test]
    fn the_window_counts_from_the_first_failure_of_the_streak() {
        assert!(!should_auto_disable(&health(50, Some(119)), now(), 120));
        assert!(should_auto_disable(&health(50, Some(121)), now(), 120));
        assert!(!should_auto_disable(&health(50, None), now(), 120));
    }

    #[test]
    fn only_unanswered_attempts_are_endpoint_wide() {
        assert!(is_endpoint_wide_failure(None));
        for status in [429, 502, 503, 504] {
            assert!(is_endpoint_wide_failure(Some(status)), "{status}");
        }
        for status in [400, 401, 404, 409, 500] {
            assert!(!is_endpoint_wide_failure(Some(status)), "{status}");
        }
    }

    #[test]
    fn the_pause_grows_from_the_threshold_and_is_capped() {
        let blip = Duration::seconds(1);
        assert_eq!(pause_after(PROBE_THRESHOLD - 1, blip), None);
        assert_eq!(
            pause_after(PROBE_THRESHOLD, blip),
            Some(Duration::seconds(30))
        );
        assert_eq!(
            pause_after(PROBE_THRESHOLD + 1, blip),
            Some(Duration::seconds(60))
        );
        assert_eq!(
            pause_after(PROBE_THRESHOLD + 100, blip),
            Some(Duration::seconds(3_600))
        );
    }

    #[test]
    fn the_pause_is_never_shorter_than_the_outage_so_far() {
        assert_eq!(
            pause_after(PROBE_THRESHOLD, Duration::minutes(5)),
            Some(Duration::minutes(5))
        );
        assert_eq!(
            pause_after(PROBE_THRESHOLD, Duration::hours(3)),
            Some(Duration::seconds(3_600)),
            "a quiet endpoint that took hours to reach the threshold is not probed every 30s"
        );
        assert_eq!(
            pause_after(PROBE_THRESHOLD + 6, Duration::minutes(5)),
            Some(Duration::seconds(3_600)),
            "the step still applies when it is the longer of the two"
        );
    }
}
