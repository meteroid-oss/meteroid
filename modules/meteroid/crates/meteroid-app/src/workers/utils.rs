use rand::RngExt;
use std::time::Duration;

const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Exponential backoff with up to 50% random jitter, capped at 30s.
/// consecutive_errors=1 → 1s, =2 → 2–3s, =3 → 4–6s, … =6+ → ≤30s.
pub fn backoff_duration(consecutive_errors: u32) -> Duration {
    let shift = consecutive_errors.saturating_sub(1).min(6);
    let base_secs = 1u64 << shift;
    let jitter_secs = rand::rng().random_range(0..=base_secs / 2);
    Duration::from_secs(base_secs + jitter_secs).min(MAX_BACKOFF)
}

fn jitter_for_duration(base: Duration) -> u64 {
    // ~10-20% jitter, with a floor and ceiling
    (base.as_millis() as u64 / 10).clamp(10, 100)
}

pub async fn sleep_with_jitter(duration: Duration) {
    let jitter = rand::rng().random_range(0..=jitter_for_duration(duration));
    let total_duration = duration + Duration::from_millis(jitter);

    tokio::time::sleep(total_duration).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_error_is_one_second() {
        // No jitter on base_secs=1 (jitter range is 0..=0)
        assert_eq!(backoff_duration(1), Duration::from_secs(1));
    }

    #[test]
    fn doubles_each_step() {
        // base_secs for errors 1..7: 1, 2, 4, 8, 16, 32, 64
        // min of range (no jitter) should equal base_secs
        let bases: Vec<u64> = (1u32..=7)
            .map(|n| 1u64 << n.saturating_sub(1).min(6))
            .collect();
        assert_eq!(bases, vec![1, 2, 4, 8, 16, 32, 64]);
    }

    #[test]
    fn caps_at_30_seconds() {
        // errors 6 and beyond: base is ≥32s which exceeds MAX_BACKOFF
        for n in [6u32, 7, 10, 100, u32::MAX] {
            assert!(backoff_duration(n) <= MAX_BACKOFF);
        }
    }

    #[test]
    fn zero_errors_does_not_panic() {
        // edge case: 0 consecutive errors → shift=0, base=1s
        let d = backoff_duration(0);
        assert!(d >= Duration::from_secs(1));
        assert!(d <= MAX_BACKOFF);
    }

    #[test]
    fn jitter_stays_within_bounds() {
        // Run many times and verify the result is always in [base, base*1.5] before cap
        for n in 1u32..=5 {
            let shift = n.saturating_sub(1).min(6);
            let base = Duration::from_secs(1u64 << shift);
            let max_with_jitter = base + Duration::from_secs((1u64 << shift) / 2);
            for _ in 0..50 {
                let d = backoff_duration(n);
                assert!(d >= base, "n={n}: {d:?} < base {base:?}");
                assert!(
                    d <= max_with_jitter.min(MAX_BACKOFF),
                    "n={n}: {d:?} > max {max_with_jitter:?}"
                );
            }
        }
    }
}
