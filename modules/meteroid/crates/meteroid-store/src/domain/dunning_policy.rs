//! A tenant's automatic payment retries. Written by the EE dunning module; without one, core
//! retries after 3, 5 and 7 days and leaves the invoice in collection.
use chrono::{DateTime, Utc};
use diesel_models::dunning_policy::DunningPolicyRow;
use diesel_models::enums::DunningExhaustedActionEnum;

pub const DEFAULT_RETRY_DAYS: [u32; 3] = [3, 5, 7];
pub const MAX_RETRIES: usize = 8;
pub const MAX_RETRY_DELAY_DAYS: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExhaustedAction {
    /// The invoice stays due, for a human or the customer to collect.
    #[default]
    None,
    /// The subscription is cancelled at once; the invoice stays due.
    CancelSubscription,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DunningPolicy {
    /// Days to wait before each successive retry; its length is the retry budget.
    pub retry_days: Vec<u32>,
    pub exhausted_action: ExhaustedAction,
}

impl Default for DunningPolicy {
    fn default() -> Self {
        Self {
            retry_days: DEFAULT_RETRY_DAYS.to_vec(),
            exhausted_action: ExhaustedAction::None,
        }
    }
}

impl From<DunningPolicyRow> for DunningPolicy {
    fn from(row: DunningPolicyRow) -> Self {
        Self {
            retry_days: row
                .retry_days
                .into_iter()
                .flatten()
                .filter_map(|days| u32::try_from(days).ok())
                .collect(),
            exhausted_action: match row.exhausted_action {
                DunningExhaustedActionEnum::None => ExhaustedAction::None,
                DunningExhaustedActionEnum::CancelSubscription => {
                    ExhaustedAction::CancelSubscription
                }
            },
        }
    }
}

impl DunningPolicy {
    /// The delay before retry `attempt` (1-based), or `None` once the budget is spent.
    /// How long a hard-declined invoice waits for a new card before the cancel policy applies:
    /// as long as the retries would have taken. `None` when the policy keeps the subscription.
    pub fn hard_decline_grace_days(&self) -> Option<u32> {
        (self.exhausted_action == ExhaustedAction::CancelSubscription
            && !self.retry_days.is_empty())
        .then(|| self.retry_days.iter().sum())
    }

    pub fn delay_before(&self, attempt: u32) -> Option<u32> {
        self.retry_days
            .get(attempt.saturating_sub(1) as usize)
            .copied()
    }
}

impl DunningPolicy {
    /// The retries still to come once `failed` automatic attempts failed, the next one due at
    /// `next_at`: each further one waits its delay after the one before fails.
    pub fn retries_from(&self, failed: u32, next_at: DateTime<Utc>) -> Vec<DateTime<Utc>> {
        let mut planned = vec![next_at];
        let mut at = next_at;
        let mut attempt = failed + 1;
        while let Some(days) = self.delay_before(attempt) {
            at += chrono::Duration::days(i64::from(days));
            planned.push(at);
            attempt += 1;
        }
        planned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn day(d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, d, 9, 0, 0).unwrap()
    }

    #[test]
    fn the_retries_left_follow_the_ladder_from_the_next_one() {
        let policy = DunningPolicy::default();
        assert_eq!(
            policy.retries_from(1, day(4)),
            vec![day(4), day(9), day(16)]
        );
        assert_eq!(policy.retries_from(2, day(9)), vec![day(9), day(16)]);
        assert_eq!(policy.retries_from(3, day(16)), vec![day(16)]);
    }
}
