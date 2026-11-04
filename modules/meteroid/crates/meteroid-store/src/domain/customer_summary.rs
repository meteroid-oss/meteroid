//! What an account is worth, whether it pays and when its term ends; one subscription's usage
//! against its limits this cycle. Billing facts, readable without the Signals add-on.

use std::collections::BTreeMap;

use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{FeatureId, InvoiceId, ScheduledEventId, SubscriptionId};
use diesel_models::query::account_signals::SignalSubscriptionRow;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use serde::{Deserialize, Serialize};

/// The parts of a summary the caller may see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SummaryParts {
    /// Receivables: invoices:read.
    pub invoices: bool,
    /// Term: subscriptions:read.
    pub subscriptions: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CustomerSummary {
    /// Billing MRR of the active subscriptions, in `reporting_currency`.
    pub mrr_cents: i64,
    pub reporting_currency: String,
    pub receivables: Option<CustomerReceivables>,
    pub term: Option<CustomerTerm>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MoneyTotal {
    pub currency: String,
    pub amount_cents: i64,
    pub invoice_count: u32,
}

/// Finalized invoices with an amount due, per currency.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CustomerReceivables {
    pub open: Vec<MoneyTotal>,
    pub overdue: Vec<MoneyTotal>,
    pub last_failure_at: Option<NaiveDateTime>,
    pub last_failure_reason: Option<String>,
    pub last_failure_invoice_id: Option<InvoiceId>,
    pub next_retry_at: Option<NaiveDateTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenewalMode {
    Ending,
    Renewing,
}

/// The earliest renewal or scheduled end among the billing-active subscriptions.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomerTerm {
    pub subscription_id: SubscriptionId,
    pub plan_name: String,
    pub date: NaiveDate,
    pub mode: RenewalMode,
    /// The pending cancellation behind an ending, when there is one.
    pub cancellation_event_id: Option<ScheduledEventId>,
}

/// A billing cycle, half-open: `period_end` is the first day of the next cycle.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageCycle {
    pub period_start: NaiveDate,
    pub period_end: Option<NaiveDate>,
    pub elapsed_pct: f64,
    pub days_left: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureUsage {
    pub feature_id: FeatureId,
    pub feature_name: String,
    pub consumed: Decimal,
    pub limit: Option<Decimal>,
    pub pct: Option<f64>,
    pub warning_threshold_pct: u32,
    /// The entitlement's reset period, e.g. "billing_cycle".
    pub reset: String,
    pub period_start: Option<NaiveDate>,
    pub resets_at: Option<NaiveDateTime>,
    /// "allow" | "block".
    pub overage_behavior: String,
    /// At the current pace, to the end of the usage window.
    pub projected: Option<Decimal>,
    /// The limit blocks and its ceiling (limit plus grace) is reached.
    pub refused: bool,
    pub grace_pct: Option<u32>,
    /// The limit applies per value of this property; `consumed` is the largest value's.
    pub group_by: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubscriptionUsage {
    pub cycle: Option<UsageCycle>,
    pub usage: Vec<FeatureUsage>,
}

/// The usage thresholds of the `limit_burndown` built-in signal, which usage bars mark.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LimitBurndownSettings {
    /// Used when the entitlement carries no alert thresholds.
    pub default_warning_pct: u32,
    pub min_projection_elapsed_pct: u32,
}

impl Default for LimitBurndownSettings {
    fn default() -> Self {
        Self {
            default_warning_pct: 80,
            min_projection_elapsed_pct: 10,
        }
    }
}

impl LimitBurndownSettings {
    pub fn threshold_pct(&self, entitlement_threshold_pct: Option<u32>) -> u32 {
        entitlement_threshold_pct.unwrap_or(self.default_warning_pct)
    }

    /// `None` / JSON null (the column default) yields the defaults; the result is validated.
    pub fn parse(value: Option<&serde_json::Value>) -> Result<Self, String> {
        let settings = match value {
            None | Some(serde_json::Value::Null) => return Ok(Self::default()),
            Some(v) if v.is_object() => serde_json::from_value::<Self>(v.clone())
                .map_err(|e| format!("invalid limit_burndown settings: {e}"))?,
            Some(_) => return Err("limit_burndown settings must be a JSON object".to_string()),
        };
        settings.validate()?;
        Ok(settings)
    }

    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("default_warning_pct", self.default_warning_pct),
            (
                "min_projection_elapsed_pct",
                self.min_projection_elapsed_pct,
            ),
        ] {
            if !(1..=100).contains(&value) {
                return Err(format!("{field} must be between 1 and 100, got {value}"));
            }
        }
        Ok(())
    }
}

/// Amount due and invoice count of one currency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurrencyTotal {
    pub currency: String,
    pub amount_due_cents: i64,
    pub count: u32,
}

/// Amount due and invoice count per currency, ordered by code.
pub fn currency_totals<'a>(
    amounts: impl IntoIterator<Item = (&'a str, i64)>,
) -> Vec<CurrencyTotal> {
    let mut totals: BTreeMap<&str, CurrencyTotal> = BTreeMap::new();
    for (currency, amount) in amounts {
        let total = totals.entry(currency).or_insert_with(|| CurrencyTotal {
            currency: currency.to_string(),
            amount_due_cents: 0,
            count: 0,
        });
        total.amount_due_cents += amount;
        total.count += 1;
    }
    totals.into_values().collect()
}

pub fn usage_pct(consumed: Decimal, limit: Decimal) -> Option<f64> {
    if limit <= Decimal::ZERO {
        return None;
    }
    (consumed / limit * Decimal::ONE_HUNDRED).to_f64()
}

/// `end_exclusive − today`: the day the next cycle starts is not a day left.
pub fn days_left(end_exclusive: NaiveDate, today: NaiveDate) -> u32 {
    (end_exclusive - today).num_days().max(0) as u32
}

/// Linear end-of-period projection, `None` before `min_elapsed` (fraction) of the period.
pub fn project_linear(consumed: Decimal, elapsed: f64, min_elapsed: f64) -> Option<Decimal> {
    if elapsed <= 0.0 || elapsed < min_elapsed {
        return None;
    }
    if elapsed >= 1.0 {
        return Some(consumed);
    }
    Decimal::from_f64(elapsed).map(|e| (consumed / e).round_dp(4))
}

/// The scheduled end: `end_date` or a pending cancellation, whichever comes first.
pub fn ends_on(sub: &SignalSubscriptionRow) -> Option<NaiveDate> {
    [sub.end_date, sub.pending_cancel_date]
        .into_iter()
        .flatten()
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::datetime::elapsed_fraction;
    use rust_decimal_macros::dec;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn limit_projection_is_linear_and_guarded() {
        let consumed = dec!(500);
        assert_eq!(project_linear(consumed, 0.25, 0.10), Some(dec!(2000)));
        assert_eq!(project_linear(consumed, 0.05, 0.10), None);
        assert_eq!(project_linear(consumed, 1.0, 0.10), Some(consumed));
        assert_eq!(project_linear(consumed, 0.0, 0.0), None);
    }

    #[test]
    fn elapsed_and_days_left_use_the_exclusive_end() {
        let at = |date: NaiveDate, h: u32| date.and_hms_opt(h, 0, 0).unwrap();
        let (start, end) = (d(2026, 9, 1), d(2026, 10, 1));
        let noon = at(d(2026, 9, 30), 12);
        assert_eq!(days_left(end, noon.date()), 1);
        assert!((elapsed_fraction(start, end, noon) - 0.9833).abs() < 1e-3);
        assert_eq!(days_left(end, end), 0);
        assert_eq!(elapsed_fraction(start, end, at(end, 0)), 1.0);
        assert_eq!(elapsed_fraction(start, end, at(d(2026, 8, 20), 0)), 0.0);
        assert_eq!(elapsed_fraction(start, start, at(start, 5)), 1.0);
        assert_eq!(days_left(end, d(2026, 10, 5)), 0);
    }

    #[test]
    fn currency_totals_sum_per_code() {
        assert_eq!(
            currency_totals([("USD", 200), ("EUR", 100), ("USD", 50)]),
            vec![
                CurrencyTotal {
                    currency: "EUR".into(),
                    amount_due_cents: 100,
                    count: 1
                },
                CurrencyTotal {
                    currency: "USD".into(),
                    amount_due_cents: 250,
                    count: 2
                },
            ]
        );
    }

    #[test]
    fn limit_settings_default_when_null_and_reject_out_of_bounds() {
        assert_eq!(
            LimitBurndownSettings::parse(None).unwrap(),
            LimitBurndownSettings::default()
        );
        assert_eq!(
            LimitBurndownSettings::parse(Some(&serde_json::Value::Null)).unwrap(),
            LimitBurndownSettings::default()
        );
        let custom = serde_json::json!({"default_warning_pct": 90});
        assert_eq!(
            LimitBurndownSettings::parse(Some(&custom))
                .unwrap()
                .threshold_pct(None),
            90
        );
        let zero = serde_json::json!({"default_warning_pct": 0});
        assert!(LimitBurndownSettings::parse(Some(&zero)).is_err());
        let unknown = serde_json::json!({"warning": 50});
        assert!(LimitBurndownSettings::parse(Some(&unknown)).is_err());
        assert!(LimitBurndownSettings::parse(Some(&serde_json::json!([]))).is_err());
    }
}
