//! Rules every usage limit and its alerts are validated against: thresholds, grace and the
//! errors a limit, a group or a budget can be refused with.
use crate::errors::{StoreError, StoreErrorReport};

pub const MAX_THRESHOLDS: usize = 10;
pub const MAX_GRACE_PCT: u32 = 1000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UsageMonitorValidationError {
    #[error("group_by must be a non-empty event property name without surrounding whitespace")]
    InvalidGroupBy,
    #[error("group value must be non-empty printable text without surrounding whitespace")]
    InvalidGroupValue,
    #[error("a value for {0} is required")]
    MissingGroupValue(String),
    #[error("this limit is not grouped, so no group value applies")]
    UnexpectedGroupValue,
    #[error("thresholds must be at most {MAX_THRESHOLDS} percentages between 1 and 100")]
    InvalidThresholds,
    #[error("budget amount must be positive")]
    InvalidBudgetAmount,
    #[error("currency must be a supported ISO 4217 code or a credit unit code")]
    InvalidCurrency,
    #[error("expected version must be positive")]
    InvalidVersion,
}

impl From<UsageMonitorValidationError> for StoreErrorReport {
    fn from(error: UsageMonitorValidationError) -> Self {
        StoreError::InvalidArgument(error.to_string()).into()
    }
}

type Validation<T = ()> = Result<T, UsageMonitorValidationError>;

pub fn normalize_thresholds(mut thresholds: Vec<u32>) -> Validation<Vec<u32>> {
    if thresholds.len() > MAX_THRESHOLDS {
        return Err(UsageMonitorValidationError::InvalidThresholds);
    }
    thresholds.sort_unstable();
    thresholds.dedup();
    validate_thresholds(&thresholds)?;
    Ok(thresholds)
}

pub fn validate_thresholds(thresholds: &[u32]) -> Validation {
    let ascending = thresholds.windows(2).all(|pair| pair[0] < pair[1]);
    let in_range = thresholds.iter().all(|pct| (1..=100).contains(pct));
    if thresholds.len() > MAX_THRESHOLDS || !ascending || !in_range {
        return Err(UsageMonitorValidationError::InvalidThresholds);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_are_percentages_in_ascending_order() {
        for bad in [
            vec![0],
            vec![101],
            vec![80, 80],
            vec![90, 80],
            (1..=11).collect(),
        ] {
            assert_eq!(
                validate_thresholds(&bad),
                Err(UsageMonitorValidationError::InvalidThresholds),
                "{bad:?}"
            );
        }
        assert_eq!(validate_thresholds(&[]), Ok(()));
        assert_eq!(validate_thresholds(&(1..=10).collect::<Vec<_>>()), Ok(()));
    }

    #[test]
    fn normalizing_sorts_and_dedups_but_still_rejects_out_of_range() {
        assert_eq!(
            normalize_thresholds(vec![100, 50, 80, 50]),
            Ok(vec![50, 80, 100])
        );
        assert_eq!(
            normalize_thresholds(vec![50, 0]),
            Err(UsageMonitorValidationError::InvalidThresholds)
        );
        assert_eq!(
            normalize_thresholds(vec![50; 11]),
            Err(UsageMonitorValidationError::InvalidThresholds)
        );
    }
}
