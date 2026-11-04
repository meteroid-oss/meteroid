//! Pure rules shared by the entitlement check path and the folds that feed it, so both sides
//! resolve the same window, the same group key and the same crossed thresholds.
use crate::domain::BillableMetric;
use crate::domain::entitlements::{PeriodUnit, ResetPeriod};
use crate::domain::usage_alert_events::LIMIT_PCT;
use crate::domain::usage_limits::UsageMonitorValidationError as Invalid;
use crate::services::usage_grouping::{self, foldable, metric_dimensions};
use chrono::{DateTime, Datelike, Days, Duration, Months, NaiveDate, NaiveDateTime, NaiveTime};
use rust_decimal::Decimal;
use std::collections::HashMap;

pub const MAX_GROUP_PROPERTY_BYTES: usize = 128;
pub const MAX_GROUP_VALUE_BYTES: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsagePeriodBounds {
    pub start: NaiveDateTime,
    /// `None` for `Never` reset — no upper bound. When set, this is exclusive (= reset_at).
    pub end: Option<NaiveDateTime>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct BillingCyclePeriod {
    pub period_start: NaiveDate,
    /// Exclusive: the next cycle starts on `current_period_end`.
    pub period_end: Option<NaiveDate>,
}

/// Compute the time window over which to count usage for a metered entitlement.
///
/// - `Never` — open-ended window starting at the customer's subscription activation date
///   (or `now` if no activation is known). `end = None`: usage is cumulative for life.
/// - `BillingCycle` — anchored on the subscription's current billing period. With no
///   active subscription, falls back to `[now, ∞)`, which surfaces as zero usage.
/// - `Calendar { unit, interval }` — fixed buckets anchored on a global epoch (the Unix
///   epoch for Hour/Day, ISO Monday 1969-12-29 for Week, calendar year zero for Month/Year).
///   Same `now` always falls into the same bucket regardless of when the subscription
///   started — useful for "calendar month" semantics shared across customers. The anchor
///   is arbitrary but deterministic: e.g. `Year { interval: 3 }` produces buckets
///   `[…, 2022-2024, 2025-2027, …]`, not buckets starting on the customer's signup year.
/// - `SlidingWindow { unit, interval }` — continuous rolling window `[now − interval, now]`.
///   Usage drops out the moment it ages past the window edge — no boundary moment, no reset.
/// - `FixedWindow { unit, interval }` — discrete, non-overlapping bucket of length
///   `interval × unit` anchored on the customer's subscription activation date. Bucket N
///   covers `[activation + N × interval, activation + (N+1) × interval)`. Usage resets to
///   zero at every bucket boundary. With no known activation, falls back to `[now, ∞)`
///   (same as `Never`) so the caller gets an empty window rather than an arbitrary anchor.
pub fn compute_usage_period(
    reset_period: &ResetPeriod,
    billing_cycle: Option<BillingCyclePeriod>,
    activation_date: Option<NaiveDateTime>,
    now: NaiveDateTime,
) -> UsagePeriodBounds {
    match reset_period {
        ResetPeriod::Never => UsagePeriodBounds {
            start: activation_date.unwrap_or(now),
            end: None,
        },
        ResetPeriod::BillingCycle => match billing_cycle {
            Some(bc) => UsagePeriodBounds {
                start: bc.period_start.and_time(NaiveTime::MIN),
                end: bc.period_end.map(|d| d.and_time(NaiveTime::MIN)),
            },
            None => UsagePeriodBounds {
                start: now,
                end: None,
            },
        },
        ResetPeriod::Calendar { unit, interval } => calendar_period(now, unit, *interval),
        ResetPeriod::FixedWindow { unit, interval } => match activation_date {
            Some(act) => fixed_window(act, now, unit, *interval),
            None => UsagePeriodBounds {
                start: now,
                end: None,
            },
        },
        ResetPeriod::SlidingWindow { unit, interval } => UsagePeriodBounds {
            start: subtract_duration(now, unit, *interval),
            end: Some(now),
        },
    }
}

/// Find the fixed-window bucket containing `now`, with buckets of length `interval × unit`
/// anchored at `activation`. Returns `[bucket_start, bucket_end)`.
fn fixed_window(
    activation: NaiveDateTime,
    now: NaiveDateTime,
    unit: &PeriodUnit,
    interval: u32,
) -> UsagePeriodBounds {
    match unit {
        // Constant-length units → closed-form via Duration arithmetic.
        PeriodUnit::Hour | PeriodUnit::Day | PeriodUnit::Week => {
            let step_hours: i64 = match unit {
                PeriodUnit::Hour => interval as i64,
                PeriodUnit::Day => 24 * interval as i64,
                PeriodUnit::Week => 24 * 7 * interval as i64,
                _ => unreachable!(),
            };
            // If now < activation (clock skew or pre-dated activation), clamp to bucket 0.
            let elapsed = (now - activation).num_hours().max(0);
            let bucket = elapsed / step_hours;
            let start = activation + Duration::hours(bucket * step_hours);
            let end = start + Duration::hours(step_hours);
            UsagePeriodBounds {
                start,
                end: Some(end),
            }
        }
        // Variable-length units (Month/Year): walk forward in `interval`-sized steps.
        // Bounded by activation-to-now span in months; even 10 years on a 1-month reset
        // is at most ~120 iterations.
        PeriodUnit::Month | PeriodUnit::Year => {
            let step_months: u32 = match unit {
                PeriodUnit::Month => interval,
                PeriodUnit::Year => interval * 12,
                _ => unreachable!(),
            };
            let mut start = activation;
            loop {
                let next =
                    NaiveDateTime::new(start.date() + Months::new(step_months), start.time());
                if next > now {
                    return UsagePeriodBounds {
                        start,
                        end: Some(next),
                    };
                }
                start = next;
            }
        }
    }
}

fn calendar_period(now: NaiveDateTime, unit: &PeriodUnit, interval: u32) -> UsagePeriodBounds {
    match unit {
        PeriodUnit::Hour => {
            let epoch = DateTime::UNIX_EPOCH.naive_utc();
            let hours_since_epoch = (now - epoch).num_hours();
            let period_idx = hours_since_epoch / interval as i64;
            let start = epoch + Duration::hours(period_idx * interval as i64);
            let end = start + Duration::hours(interval as i64);
            UsagePeriodBounds {
                start,
                end: Some(end),
            }
        }
        PeriodUnit::Day => {
            let today = now.date();
            let epoch = DateTime::UNIX_EPOCH.date_naive();
            let days = (today - epoch).num_days();
            let period_idx = days / interval as i64;
            let start_date = epoch + Days::new((period_idx * interval as i64) as u64);
            let end_date = start_date + Days::new(interval as u64);
            UsagePeriodBounds {
                start: start_date.and_time(NaiveTime::MIN),
                end: Some(end_date.and_time(NaiveTime::MIN)),
            }
        }
        PeriodUnit::Week => {
            let today = now.date();
            let iso_epoch = NaiveDate::from_ymd_opt(1969, 12, 29).unwrap();
            let days_since_iso_epoch = (today - iso_epoch).num_days();
            let weeks_since_iso_epoch = days_since_iso_epoch / 7;
            let period_idx = weeks_since_iso_epoch / interval as i64;
            let start_date = iso_epoch + Days::new((period_idx * 7 * interval as i64) as u64);
            let end_date = start_date + Days::new(7 * interval as u64);
            UsagePeriodBounds {
                start: start_date.and_time(NaiveTime::MIN),
                end: Some(end_date.and_time(NaiveTime::MIN)),
            }
        }
        PeriodUnit::Month => {
            let today = now.date();
            let interval = interval as i64;
            let months_since_zero = today.year() as i64 * 12 + (today.month() as i64 - 1);
            let period_idx = months_since_zero.div_euclid(interval);
            let start_months = period_idx * interval;
            let start_year = start_months.div_euclid(12) as i32;
            let start_month = (start_months.rem_euclid(12) + 1) as u32;
            let start_date = NaiveDate::from_ymd_opt(start_year, start_month, 1).unwrap_or(today);
            let end_date = start_date + Months::new(interval as u32);
            UsagePeriodBounds {
                start: start_date.and_time(NaiveTime::MIN),
                end: Some(end_date.and_time(NaiveTime::MIN)),
            }
        }
        PeriodUnit::Year => {
            let today = now.date();
            let year = today.year() as i64;
            let interval = interval as i64;
            let period_idx = year.div_euclid(interval);
            let start_year = (period_idx * interval) as i32;
            let start_date = NaiveDate::from_ymd_opt(start_year, 1, 1).unwrap_or(today);
            let end_date =
                NaiveDate::from_ymd_opt(start_year.saturating_add(interval as i32), 1, 1)
                    .unwrap_or(today);
            UsagePeriodBounds {
                start: start_date.and_time(NaiveTime::MIN),
                end: Some(end_date.and_time(NaiveTime::MIN)),
            }
        }
    }
}

fn subtract_duration(now: NaiveDateTime, unit: &PeriodUnit, interval: u32) -> NaiveDateTime {
    match unit {
        PeriodUnit::Hour => now - Duration::hours(interval as i64),
        PeriodUnit::Day => NaiveDateTime::new(now.date() - Days::new(interval as u64), now.time()),
        PeriodUnit::Week => {
            NaiveDateTime::new(now.date() - Days::new(7 * interval as u64), now.time())
        }
        PeriodUnit::Month => NaiveDateTime::new(now.date() - Months::new(interval), now.time()),
        PeriodUnit::Year => NaiveDateTime::new(now.date() - Months::new(interval * 12), now.time()),
    }
}

/// Fits `usage_alert_state.window_key`; a bounded window keys on both edges, an open one on its start.
pub fn window_key(start: NaiveDateTime, end: Option<NaiveDateTime>) -> String {
    let start = start.and_utc().timestamp_micros();
    match end {
        Some(end) => format!("{start}:{}", end.and_utc().timestamp_micros()),
        None => format!("{start}:"),
    }
}

fn saturating_scale(value: Decimal, factor: u32) -> Decimal {
    value
        .checked_mul(Decimal::from(factor))
        .unwrap_or(if value.is_sign_negative() {
            Decimal::MIN
        } else {
            Decimal::MAX
        })
}

/// The thresholds (percentages of `limit`, ascending) that `used` has reached. A zero limit is
/// reached from the start, as the limit alone: it has no fraction to warn at.
pub fn crossed_pcts(used: Decimal, limit: Decimal, thresholds: &[u32]) -> Vec<u32> {
    if limit.is_zero() {
        return if used < Decimal::ZERO {
            Vec::new()
        } else {
            vec![LIMIT_PCT]
        };
    }
    let used = saturating_scale(used, 100);
    thresholds
        .iter()
        .copied()
        .filter(|pct| used >= saturating_scale(limit, *pct))
        .collect()
}

const REARM_MARGIN_PCT: u32 = 5;

/// The levels `used` holds: those reached, and those it stands within a margin under, where a
/// reading that wavers around a level is not a recovery from it.
pub fn held_pcts(used: Decimal, limit: Decimal, thresholds: &[u32]) -> Vec<u32> {
    if limit.is_zero() {
        return crossed_pcts(used, limit, thresholds);
    }
    let used = saturating_scale(used, 100);
    thresholds
        .iter()
        .copied()
        .filter(|pct| used >= saturating_scale(limit, pct - rearm_margin(*pct)))
        .collect()
}

/// Percentage points of the limit under a level that its usage must fall to re-arm it.
fn rearm_margin(pct: u32) -> u32 {
    (pct / 2).min(REARM_MARGIN_PCT)
}

/// Usage from which a hard limit refuses more, the limit plus its grace percentage.
pub fn hard_ceiling(limit: Decimal, grace_pct: Option<u32>) -> Decimal {
    saturating_scale(limit, 100u32.saturating_add(grace_pct.unwrap_or(0)))
        .checked_div(Decimal::ONE_HUNDRED)
        .unwrap_or(Decimal::MAX)
}

pub fn hard_limit_reached(used: Decimal, limit: Decimal, grace_pct: Option<u32>) -> bool {
    saturating_scale(used, 100)
        >= saturating_scale(limit, 100u32.saturating_add(grace_pct.unwrap_or(0)))
}

/// An entitlement's usage can be told per group only when the metric's own
/// dimensions add nothing, or when summing the metric's rows is exact.
fn groupable(metric: &BillableMetric, group_by: Option<&str>) -> bool {
    foldable(&metric.aggregation_type)
        || metric_dimensions(metric)
            .iter()
            .all(|dimension| Some(dimension.as_str()) == group_by)
}

/// Why a metric's usage cannot be monitored per `group_by`, when it cannot.
pub fn ungroupable_reason(metric: &BillableMetric, group_by: Option<&str>) -> Option<&'static str> {
    if group_by.is_some_and(|property| property.contains(',')) {
        Some("its group_by contains a comma")
    } else if !groupable(metric, group_by) {
        Some(
            "the metric's aggregation cannot be added up across its own dimensions, and the limit is not grouped by them",
        )
    } else {
        None
    }
}

pub fn validate_group_property(name: &str) -> Result<(), Invalid> {
    if is_plain_text(name, MAX_GROUP_PROPERTY_BYTES) {
        Ok(())
    } else {
        Err(Invalid::InvalidGroupBy)
    }
}

pub fn validate_group_value(value: &str) -> Result<(), Invalid> {
    if is_plain_text(value, MAX_GROUP_VALUE_BYTES) {
        Ok(())
    } else {
        Err(Invalid::InvalidGroupValue)
    }
}

fn is_plain_text(text: &str, max_bytes: usize) -> bool {
    !text.trim().is_empty()
        && text.trim() == text
        && text.len() <= max_bytes
        && !text.chars().any(char::is_control)
}

/// Key of the fold row a limit reads: empty when not grouped, else the canonical form the fold
/// writes. A grouped limit needs the value of its property, an ungrouped one takes none.
pub fn group_key(group_by: Option<&str>, value: Option<&str>) -> Result<String, Invalid> {
    match (group_by, value) {
        (None, None) => Ok(String::new()),
        (None, Some(_)) => Err(Invalid::UnexpectedGroupValue),
        (Some(property), None) => Err(Invalid::MissingGroupValue(property.to_owned())),
        (Some(property), Some(value)) => {
            validate_group_property(property)?;
            validate_group_value(value)?;
            Ok(usage_grouping::group_key(&HashMap::from([(
                property.to_owned(),
                value.to_owned(),
            )])))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::usage_limits::UsageMonitorValidationError as Invalid;
    use rust_decimal_macros::dec;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, s)
            .unwrap()
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDateTime {
        at(y, m, d, 0, 0, 0)
    }

    fn period(reset: &ResetPeriod, now: NaiveDateTime) -> (NaiveDateTime, Option<NaiveDateTime>) {
        let bounds = compute_usage_period(reset, None, None, now);
        (bounds.start, bounds.end)
    }

    fn calendar(unit: PeriodUnit, interval: u32) -> ResetPeriod {
        ResetPeriod::Calendar { unit, interval }
    }

    const LEVELS: [u32; 3] = [50, 80, 100];

    #[test]
    fn thresholds_are_crossed_exactly_at_the_boundaries() {
        let crossed = |used| crossed_pcts(used, dec!(100), &LEVELS);
        assert!(crossed(dec!(-1)).is_empty());
        assert!(crossed(dec!(49.999)).is_empty());
        assert_eq!(crossed(dec!(50)), vec![50]);
        assert_eq!(crossed(dec!(79.999999999)), vec![50]);
        assert_eq!(crossed(dec!(80)), vec![50, 80]);
        assert_eq!(crossed(dec!(99.999)), vec![50, 80]);
        assert_eq!(crossed(dec!(100)), vec![50, 80, 100]);
        assert_eq!(crossed(dec!(100.001)), vec![50, 80, 100]);
    }

    #[test]
    fn thresholds_are_exact_where_floats_would_drift() {
        assert_eq!(crossed_pcts(dec!(0.099), dec!(0.3), &[33]), vec![33]);
        assert!(crossed_pcts(dec!(0.098999999), dec!(0.3), &[33]).is_empty());
        assert_eq!(crossed_pcts(dec!(1), dec!(3), &[33, 34]), vec![33]);
        assert_eq!(crossed_pcts(dec!(1.02), dec!(3), &[33, 34]), vec![33, 34]);
    }

    #[test]
    fn a_zero_limit_is_reached_from_the_start_as_the_limit_alone() {
        assert_eq!(crossed_pcts(dec!(0), dec!(0), &LEVELS), vec![100]);
        assert_eq!(crossed_pcts(dec!(5), dec!(0), &LEVELS), vec![100]);
        assert!(crossed_pcts(dec!(-1), dec!(0), &LEVELS).is_empty());
        assert!(crossed_pcts(dec!(5), dec!(100), &[]).is_empty());
    }

    #[test]
    fn a_level_is_held_within_five_points_under_it_and_released_below() {
        let held = |used| held_pcts(used, dec!(100), &LEVELS);
        assert!(held(dec!(44.99)).is_empty());
        assert_eq!(held(dec!(45)), vec![50]);
        assert_eq!(held(dec!(74.99)), vec![50]);
        assert_eq!(held(dec!(75)), vec![50, 80]);
        assert_eq!(held(dec!(94.99)), vec![50, 80]);
        assert_eq!(held(dec!(95)), vec![50, 80, 100]);
        assert_eq!(held(dec!(200)), vec![50, 80, 100]);
    }

    #[test]
    fn a_held_level_always_includes_the_reached_ones() {
        for used in [
            dec!(0),
            dec!(49.9),
            dec!(50),
            dec!(79.9),
            dec!(80),
            dec!(100),
            dec!(250),
        ] {
            let held = held_pcts(used, dec!(100), &LEVELS);
            assert!(
                crossed_pcts(used, dec!(100), &LEVELS)
                    .iter()
                    .all(|pct| held.contains(pct))
            );
        }
    }

    #[test]
    fn a_small_level_keeps_a_margin_of_half_of_it() {
        assert_eq!(rearm_margin(2), 1);
        assert_eq!(rearm_margin(10), 5);
        assert_eq!(rearm_margin(80), 5);
        assert_eq!(held_pcts(dec!(1), dec!(100), &[2]), vec![2]);
        assert!(held_pcts(dec!(0.99), dec!(100), &[2]).is_empty());
    }

    #[test]
    fn a_zero_limit_holds_its_single_level() {
        assert_eq!(held_pcts(dec!(0), dec!(0), &LEVELS), vec![100]);
        assert!(held_pcts(dec!(-1), dec!(0), &LEVELS).is_empty());
    }

    #[test]
    fn extreme_values_saturate_instead_of_panicking() {
        assert_eq!(
            crossed_pcts(Decimal::MAX, dec!(100), &LEVELS),
            vec![50, 80, 100]
        );
        assert!(crossed_pcts(Decimal::MIN, dec!(100), &LEVELS).is_empty());
        assert!(crossed_pcts(dec!(1), Decimal::MAX, &LEVELS).is_empty());
        assert!(hard_limit_reached(Decimal::MAX, dec!(100), Some(1000)));
    }

    #[test]
    fn hard_limits_refuse_at_the_limit_plus_grace() {
        assert_eq!(hard_ceiling(dec!(100), None), dec!(100));
        assert_eq!(hard_ceiling(dec!(100), Some(10)), dec!(110));
        assert_eq!(hard_ceiling(dec!(0.3), Some(10)), dec!(0.33));
        assert_eq!(hard_ceiling(dec!(0), Some(50)), dec!(0));
        assert!(!hard_limit_reached(dec!(99.999), dec!(100), None));
        assert!(hard_limit_reached(dec!(100), dec!(100), None));
        assert!(!hard_limit_reached(dec!(109.999), dec!(100), Some(10)));
        assert!(hard_limit_reached(dec!(110), dec!(100), Some(10)));
        assert!(hard_limit_reached(dec!(0), dec!(0), Some(10)));
    }

    #[test]
    fn group_keys_match_what_the_fold_writes() {
        let dims = HashMap::from([("seat".to_owned(), "u-1".to_owned())]);
        assert_eq!(
            group_key(Some("seat"), Some("u-1")).unwrap(),
            usage_grouping::group_key(&dims)
        );
        assert_eq!(
            group_key(Some("seat"), Some("u-1")).unwrap(),
            r#"{"seat":"u-1"}"#
        );
        assert_eq!(
            group_key(None, None).unwrap(),
            usage_grouping::group_key(&HashMap::new())
        );
        assert_eq!(group_key(None, None).unwrap(), "");
        let tricky = "é\"\\ /x";
        let dims = HashMap::from([("k \"q\"".to_owned(), tricky.to_owned())]);
        assert_eq!(
            group_key(Some("k \"q\""), Some(tricky)).unwrap(),
            usage_grouping::group_key(&dims)
        );
    }

    #[test]
    fn group_input_is_validated() {
        assert_eq!(
            group_key(None, Some("a")),
            Err(Invalid::UnexpectedGroupValue)
        );
        assert_eq!(
            group_key(Some("seat"), None),
            Err(Invalid::MissingGroupValue("seat".to_owned()))
        );
        for bad in ["", " ", " a", "a ", "a\nb", &"v".repeat(256)] {
            assert_eq!(
                group_key(Some("seat"), Some(bad)),
                Err(Invalid::InvalidGroupValue)
            );
        }
        for bad in ["", " ", "a ", "a\u{0}", &"p".repeat(129)] {
            assert_eq!(
                group_key(Some(bad), Some("x")),
                Err(Invalid::InvalidGroupBy)
            );
        }
        assert!(group_key(Some(&"p".repeat(128)), Some(&"v".repeat(255))).is_ok());
    }

    #[test]
    fn the_longest_group_key_fits_the_fold_column() {
        let key = group_key(Some(&"\"".repeat(128)), Some(&"\\".repeat(255))).unwrap();
        assert!(
            key.len() <= usage_grouping::MAX_GROUP_KEY_BYTES,
            "{}",
            key.len()
        );
    }

    #[test]
    fn calendar_months_follow_month_ends_and_leap_years() {
        let month = calendar(PeriodUnit::Month, 1);
        assert_eq!(
            period(&month, at(2024, 2, 29, 23, 59, 59)),
            (day(2024, 2, 1), Some(day(2024, 3, 1)))
        );
        assert_eq!(
            period(&month, day(2024, 3, 1)),
            (day(2024, 3, 1), Some(day(2024, 4, 1)))
        );
        assert_eq!(
            period(&month, at(2023, 2, 28, 23, 59, 59)),
            (day(2023, 2, 1), Some(day(2023, 3, 1)))
        );
        assert_eq!(
            period(&month, at(2024, 12, 31, 23, 59, 59)),
            (day(2024, 12, 1), Some(day(2025, 1, 1)))
        );
        assert_eq!(
            period(&calendar(PeriodUnit::Month, 2), day(2025, 1, 31)),
            (day(2025, 1, 1), Some(day(2025, 3, 1)))
        );
    }

    #[test]
    fn calendar_days_weeks_and_years_are_plain_utc_buckets() {
        // Europe's clock change on 2025-03-30 makes no difference to UTC buckets.
        assert_eq!(
            period(&calendar(PeriodUnit::Day, 1), at(2025, 3, 30, 1, 30, 0)),
            (day(2025, 3, 30), Some(day(2025, 3, 31)))
        );
        assert_eq!(
            period(&calendar(PeriodUnit::Hour, 6), at(2025, 10, 26, 7, 0, 0)),
            (at(2025, 10, 26, 6, 0, 0), Some(at(2025, 10, 26, 12, 0, 0)))
        );
        assert_eq!(
            period(&calendar(PeriodUnit::Week, 1), at(2025, 6, 15, 23, 0, 0)),
            (day(2025, 6, 9), Some(day(2025, 6, 16)))
        );
        assert_eq!(
            period(&calendar(PeriodUnit::Year, 1), at(2024, 12, 31, 12, 0, 0)),
            (day(2024, 1, 1), Some(day(2025, 1, 1)))
        );
        assert_eq!(
            period(&calendar(PeriodUnit::Year, 4), day(2027, 6, 1)),
            (day(2024, 1, 1), Some(day(2028, 1, 1)))
        );
    }

    #[test]
    fn a_window_ends_where_the_next_begins() {
        let month = calendar(PeriodUnit::Month, 1);
        let (_, end) = period(&month, at(2024, 2, 29, 23, 59, 59));
        assert_eq!(period(&month, end.unwrap()).0, end.unwrap());
    }

    #[test]
    fn fixed_windows_count_in_exact_hours_from_activation() {
        let activation = at(2024, 2, 28, 12, 0, 0);
        let bounds = compute_usage_period(
            &ResetPeriod::FixedWindow {
                unit: PeriodUnit::Day,
                interval: 1,
            },
            None,
            Some(activation),
            at(2024, 3, 1, 11, 59, 0),
        );
        assert_eq!(bounds.start, at(2024, 2, 29, 12, 0, 0));
        assert_eq!(bounds.end, Some(at(2024, 3, 1, 12, 0, 0)));
    }

    #[test]
    fn billing_cycle_and_lifetime_windows() {
        let cycle = BillingCyclePeriod {
            period_start: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2024, 2, 29),
        };
        let bounds = compute_usage_period(
            &ResetPeriod::BillingCycle,
            Some(cycle),
            None,
            day(2024, 2, 10),
        );
        assert_eq!(
            (bounds.start, bounds.end),
            (day(2024, 1, 31), Some(day(2024, 2, 29)))
        );
        let activation = day(2023, 5, 5);
        let lifetime = compute_usage_period(
            &ResetPeriod::Never,
            None,
            Some(activation),
            day(2024, 2, 10),
        );
        assert_eq!((lifetime.start, lifetime.end), (activation, None));
    }

    #[test]
    fn window_keys_identify_a_window_and_fit_the_column() {
        let a = window_key(day(2024, 2, 1), Some(day(2024, 3, 1)));
        assert_eq!(a, window_key(day(2024, 2, 1), Some(day(2024, 3, 1))));
        assert_ne!(a, window_key(day(2024, 3, 1), Some(day(2024, 4, 1))));
        assert_ne!(a, window_key(day(2024, 2, 1), None));
        assert_ne!(
            window_key(day(2024, 2, 1), None),
            window_key(day(2024, 2, 2), None)
        );
        let widest = window_key(NaiveDateTime::MIN, Some(NaiveDateTime::MAX));
        assert!(!widest.trim().is_empty() && widest.len() <= 128, "{widest}");
    }

    mod compute_usage_period {
        use super::*;
        use chrono::NaiveTime;

        #[test]
        fn usage_period_sliding_window_day() {
            let now = dt(2025, 6, 15);
            let b = compute_usage_period(
                &ResetPeriod::SlidingWindow {
                    unit: PeriodUnit::Day,
                    interval: 10,
                },
                None,
                None,
                now,
            );
            assert_eq!(b.start, dt(2025, 6, 5));
            assert_eq!(b.end, Some(now));
        }

        // ── compute_usage_period ──────────────────────────────────────────────────

        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn dt(y: i32, m: u32, day: u32) -> NaiveDateTime {
            d(y, m, day).and_time(NaiveTime::MIN)
        }

        fn dt_hms(y: i32, m: u32, day: u32, h: u32, min: u32, s: u32) -> NaiveDateTime {
            NaiveDateTime::new(d(y, m, day), NaiveTime::from_hms_opt(h, min, s).unwrap())
        }

        #[test]
        fn usage_period_never_uses_activation_date() {
            let now = dt(2025, 6, 15);
            let activation = dt(2024, 1, 1);
            let b = compute_usage_period(&ResetPeriod::Never, None, Some(activation), now);
            assert_eq!(b.start, dt(2024, 1, 1));
            assert_eq!(b.end, None);
        }

        #[test]
        fn usage_period_never_falls_back_to_now_when_no_activation() {
            let now = dt(2025, 6, 15);
            let b = compute_usage_period(&ResetPeriod::Never, None, None, now);
            assert_eq!(b.start, now);
            assert_eq!(b.end, None);
        }

        #[test]
        fn billing_cycle_usage_period_end_is_exclusive() {
            let now = dt(2025, 6, 15);
            let bc = BillingCyclePeriod {
                period_start: d(2025, 6, 1),
                period_end: Some(d(2025, 7, 1)),
            };
            let b = compute_usage_period(&ResetPeriod::BillingCycle, Some(bc), None, now);
            assert_eq!(b.start, dt(2025, 6, 1));
            assert_eq!(b.end, Some(dt(2025, 7, 1)));
        }

        #[test]
        fn usage_period_billing_cycle_no_subscription_unbounded() {
            let now = dt(2025, 6, 15);
            let b = compute_usage_period(&ResetPeriod::BillingCycle, None, None, now);
            assert_eq!(b.start, now);
            assert_eq!(b.end, None);
        }

        #[test]
        fn usage_period_sliding_window_month() {
            // SlidingWindow always returns [now − interval, now].
            let now = dt(2025, 6, 15);
            let b = compute_usage_period(
                &ResetPeriod::SlidingWindow {
                    unit: PeriodUnit::Month,
                    interval: 1,
                },
                None,
                None,
                now,
            );
            assert_eq!(b.start, dt(2025, 5, 15));
            assert_eq!(b.end, Some(now));
        }

        #[test]
        fn usage_period_sliding_window_week() {
            let now = dt(2025, 6, 15);
            let b = compute_usage_period(
                &ResetPeriod::SlidingWindow {
                    unit: PeriodUnit::Week,
                    interval: 2,
                },
                None,
                None,
                now,
            );
            assert_eq!(b.start, dt(2025, 6, 1));
            assert_eq!(b.end, Some(now));
        }

        #[test]
        fn usage_period_sliding_window_hour() {
            let now = dt_hms(2025, 6, 15, 14, 30, 0);
            let b = compute_usage_period(
                &ResetPeriod::SlidingWindow {
                    unit: PeriodUnit::Hour,
                    interval: 3,
                },
                None,
                None,
                now,
            );
            assert_eq!(b.start, dt_hms(2025, 6, 15, 11, 30, 0));
            assert_eq!(b.end, Some(now));
        }

        #[test]
        fn usage_period_fixed_window_no_activation_unbounded() {
            // No anchor → can't form a fixed bucket; degrade to the same "[now, ∞)" empty
            // window that BillingCycle/Never use when their anchor is missing.
            let now = dt(2025, 6, 15);
            let b = compute_usage_period(
                &ResetPeriod::FixedWindow {
                    unit: PeriodUnit::Day,
                    interval: 7,
                },
                None,
                None,
                now,
            );
            assert_eq!(b.start, now);
            assert_eq!(b.end, None);
        }

        #[test]
        fn usage_period_fixed_window_day_first_bucket_starts_at_activation() {
            // Activation 2025-06-10, 7-day buckets. `now` 2025-06-13 falls in bucket 0.
            let activation = dt(2025, 6, 10);
            let now = dt(2025, 6, 13);
            let b = compute_usage_period(
                &ResetPeriod::FixedWindow {
                    unit: PeriodUnit::Day,
                    interval: 7,
                },
                None,
                Some(activation),
                now,
            );
            assert_eq!(b.start, dt(2025, 6, 10));
            assert_eq!(b.end, Some(dt(2025, 6, 17)));
        }

        #[test]
        fn usage_period_fixed_window_day_advances_to_current_bucket() {
            // Activation 2025-06-10, 7-day buckets. `now` 2025-07-02 → bucket 3
            // (start 2025-07-01, end 2025-07-08).
            let activation = dt(2025, 6, 10);
            let now = dt(2025, 7, 2);
            let b = compute_usage_period(
                &ResetPeriod::FixedWindow {
                    unit: PeriodUnit::Day,
                    interval: 7,
                },
                None,
                Some(activation),
                now,
            );
            assert_eq!(b.start, dt(2025, 7, 1));
            assert_eq!(b.end, Some(dt(2025, 7, 8)));
        }

        #[test]
        fn usage_period_fixed_window_hour_anchored_on_activation() {
            // Activation midnight 2025-06-10, 6-hour buckets. `now` 2025-06-10 14:30
            // → bucket 2 (start 12:00, end 18:00).
            let activation = dt(2025, 6, 10);
            let now = dt_hms(2025, 6, 10, 14, 30, 0);
            let b = compute_usage_period(
                &ResetPeriod::FixedWindow {
                    unit: PeriodUnit::Hour,
                    interval: 6,
                },
                None,
                Some(activation),
                now,
            );
            assert_eq!(b.start, dt_hms(2025, 6, 10, 12, 0, 0));
            assert_eq!(b.end, Some(dt_hms(2025, 6, 10, 18, 0, 0)));
        }

        #[test]
        fn usage_period_fixed_window_hour_anchored_on_activation_with_time() {
            let activation = dt_hms(2025, 5, 27, 14, 10, 0);
            let now = dt_hms(2025, 5, 27, 14, 20, 0);
            let b = compute_usage_period(
                &ResetPeriod::FixedWindow {
                    unit: PeriodUnit::Hour,
                    interval: 2,
                },
                None,
                Some(activation),
                now,
            );
            assert_eq!(b.start, dt_hms(2025, 5, 27, 14, 10, 0));
            assert_eq!(b.end, Some(dt_hms(2025, 5, 27, 16, 10, 0)));
        }

        #[test]
        fn usage_period_fixed_window_month_advances_to_current_bucket() {
            // Activation 2025-01-15, 1-month buckets. `now` 2025-06-20 → bucket starts 2025-06-15.
            let activation = dt(2025, 1, 15);
            let now = dt(2025, 6, 20);
            let b = compute_usage_period(
                &ResetPeriod::FixedWindow {
                    unit: PeriodUnit::Month,
                    interval: 1,
                },
                None,
                Some(activation),
                now,
            );
            assert_eq!(b.start, dt(2025, 6, 15));
            assert_eq!(b.end, Some(dt(2025, 7, 15)));
        }

        #[test]
        fn usage_period_fixed_window_year_advances_to_current_bucket() {
            // Activation 2020-03-10, 1-year buckets. `now` 2025-06-15 → bucket starts 2025-03-10.
            let activation = dt(2020, 3, 10);
            let now = dt(2025, 6, 15);
            let b = compute_usage_period(
                &ResetPeriod::FixedWindow {
                    unit: PeriodUnit::Year,
                    interval: 1,
                },
                None,
                Some(activation),
                now,
            );
            assert_eq!(b.start, dt(2025, 3, 10));
            assert_eq!(b.end, Some(dt(2026, 3, 10)));
        }

        #[test]
        fn calendar_period_month_interval_1_mid_month() {
            let b = calendar_period(dt(2025, 6, 15), &PeriodUnit::Month, 1);
            assert_eq!(b.start, dt(2025, 6, 1));
            assert_eq!(b.end, Some(dt(2025, 7, 1)));
        }

        #[test]
        fn calendar_period_month_interval_1_end_of_month() {
            let b = calendar_period(dt(2025, 1, 31), &PeriodUnit::Month, 1);
            assert_eq!(b.start, dt(2025, 1, 1));
            assert_eq!(b.end, Some(dt(2025, 2, 1)));
        }

        #[test]
        fn calendar_period_month_interval_3_q2() {
            let b = calendar_period(dt(2025, 6, 15), &PeriodUnit::Month, 3);
            assert_eq!(b.start, dt(2025, 4, 1));
            assert_eq!(b.end, Some(dt(2025, 7, 1)));
        }

        #[test]
        fn calendar_period_week_interval_1_wednesday() {
            let b = calendar_period(dt(2025, 6, 11), &PeriodUnit::Week, 1);
            assert_eq!(b.start, dt(2025, 6, 9));
            assert_eq!(b.end, Some(dt(2025, 6, 16)));
        }

        #[test]
        fn calendar_period_week_interval_2() {
            let b = calendar_period(dt(2025, 6, 11), &PeriodUnit::Week, 2);
            assert_eq!(b.start, dt(2025, 6, 2));
            assert_eq!(b.end, Some(dt(2025, 6, 16)));
        }

        #[test]
        fn calendar_period_year_interval_1() {
            let b = calendar_period(dt(2025, 8, 20), &PeriodUnit::Year, 1);
            assert_eq!(b.start, dt(2025, 1, 1));
            assert_eq!(b.end, Some(dt(2026, 1, 1)));
        }

        #[test]
        fn calendar_period_day_interval_1() {
            let now = dt(2025, 6, 15);
            let b = calendar_period(now, &PeriodUnit::Day, 1);
            assert_eq!(b.start, dt(2025, 6, 15));
            assert_eq!(b.end, Some(dt(2025, 6, 16)));
        }

        #[test]
        fn calendar_period_day_interval_7() {
            let now = dt(2025, 6, 15);
            let b = calendar_period(now, &PeriodUnit::Day, 7);
            assert!(b.start <= now);
            assert!(b.end.unwrap() > now);
            assert_eq!((b.end.unwrap() - b.start).num_days(), 7);
        }

        #[test]
        fn calendar_period_hour_interval_1() {
            let now = dt_hms(2025, 6, 15, 14, 30, 0);
            let b = calendar_period(now, &PeriodUnit::Hour, 1);
            assert_eq!(b.start, dt_hms(2025, 6, 15, 14, 0, 0));
            assert_eq!(b.end, Some(dt_hms(2025, 6, 15, 15, 0, 0)));
        }

        #[test]
        fn calendar_period_hour_interval_4_across_midnight() {
            let now = dt_hms(2025, 6, 15, 1, 15, 0);
            let b = calendar_period(now, &PeriodUnit::Hour, 4);
            assert_eq!(b.start, dt_hms(2025, 6, 15, 0, 0, 0));
            assert_eq!(b.end, Some(dt_hms(2025, 6, 15, 4, 0, 0)));
        }

        #[test]
        fn subtract_month() {
            assert_eq!(
                subtract_duration(dt(2025, 3, 31), &PeriodUnit::Month, 1),
                dt(2025, 2, 28)
            );
        }

        #[test]
        fn subtract_year() {
            assert_eq!(
                subtract_duration(dt(2025, 6, 15), &PeriodUnit::Year, 2),
                dt(2023, 6, 15)
            );
        }

        #[test]
        fn subtract_week() {
            assert_eq!(
                subtract_duration(dt(2025, 6, 15), &PeriodUnit::Week, 3),
                dt(2025, 5, 25)
            );
        }

        #[test]
        fn subtract_day() {
            assert_eq!(
                subtract_duration(dt(2025, 6, 15), &PeriodUnit::Day, 10),
                dt(2025, 6, 5)
            );
        }

        #[test]
        fn subtract_hour() {
            assert_eq!(
                subtract_duration(dt_hms(2025, 6, 15, 10, 30, 0), &PeriodUnit::Hour, 3),
                dt_hms(2025, 6, 15, 7, 30, 0)
            );
        }

        #[test]
        fn subtract_hour_crosses_midnight() {
            assert_eq!(
                subtract_duration(dt_hms(2025, 6, 15, 1, 0, 0), &PeriodUnit::Hour, 3),
                dt_hms(2025, 6, 14, 22, 0, 0)
            );
        }
    }
}
