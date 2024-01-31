use crate::compute::fees::PeriodCalculator;
use crate::models::InvoiceLinePeriod;
use anyhow::{anyhow, Context, Error};
use meteroid_grpc::meteroid::api::components::v1::fee::term_fee_pricing;
use meteroid_grpc::meteroid::api::components::v1::fee::BillingType;

use common_grpc::meteroid::common::v1 as common;
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;

use crate::compute::period::{calculate_period_idx, calculate_period_range};
use crate::compute::SubscriptionDetails;
use chrono::Datelike;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::ops::Mul;

pub trait ToCents {
    fn to_cents(&self) -> Result<i64, Error>;
    fn to_cents_f64(&self) -> Result<f64, Error>;
}

impl ToCents for Decimal {
    fn to_cents(&self) -> Result<i64, Error> {
        let cents = self
            .mul(Decimal::from(100))
            .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
            .to_i64()
            .ok_or_else(|| anyhow!("Failed to convert to cents"))?;

        Ok(cents)
    }

    fn to_cents_f64(&self) -> Result<f64, Error> {
        let cents = self
            .mul(Decimal::from(100))
            .round_dp_with_strategy(6, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
            .to_f64()
            .ok_or_else(|| anyhow!("Failed to convert to cents"))?;

        Ok(cents)
    }
}

pub fn parse_decimal(opt: &Option<common::Decimal>) -> Result<Decimal, Error> {
    opt.as_ref()
        .context("Missing decimal value")?
        .clone()
        .try_into()
}

impl PeriodCalculator for term_fee_pricing::Pricing {
    fn applies_this_period(&self, subscription: &SubscriptionDetails) -> Result<bool, Error> {
        // fixme suboptimal, called 3 times
        let cadence = &self.extract_cadence(subscription)?;
        applies_from_cadence(cadence, BillingType::Advance, subscription)
    }

    fn period(&self, subscription: &SubscriptionDetails) -> Result<InvoiceLinePeriod, Error> {
        let cadence = &self.extract_cadence(subscription)?;
        period_from_cadence(cadence, BillingType::Advance, subscription)
    }
}

pub fn applies_from_cadence(
    cadence: &BillingPeriod,
    billing_type: BillingType,
    subscription: &SubscriptionDetails,
) -> Result<bool, Error> {
    let cadence_months: u32 = cadence.months_value();
    let effective_period_months: u32 = subscription.effective_billing_period.months_value();

    let is_in_period = subscription.current_period_idx * effective_period_months as i32
        % cadence_months as i32
        <= 0;

    match billing_type {
        BillingType::Advance => Ok(is_in_period),
        BillingType::Arrear => Ok(is_in_period && subscription.current_period_idx > 0),
    }
}

pub fn period_from_cadence(
    cadence: &BillingPeriod,
    billing_type: BillingType,
    subscription: &SubscriptionDetails,
) -> Result<InvoiceLinePeriod, Error> {
    // calculate the index for this cadence
    let cadence = cadence.clone();
    // calculate the idx for that cadence. This will return an index even if we are in the middle of a period, so be careful. (validate applies_this_period, or merge the two)
    let period_idx = calculate_period_idx(
        subscription.billing_start_date,
        subscription.billing_day as u32,
        subscription.invoice_date,
        cadence,
    );

    let (from, to) = match billing_type {
        BillingType::Advance => calculate_period_range(
            subscription.billing_start_date,
            subscription.billing_day as u32,
            period_idx,
            cadence.clone(),
        ),
        BillingType::Arrear => {
            // untested TODO
            calculate_period_range(
                subscription.billing_start_date,
                subscription.billing_day as u32,
                period_idx - 1,
                cadence.clone(),
            )
        }
    };

    Ok(InvoiceLinePeriod { from, to })
}

pub trait CadenceExtractor {
    fn extract_cadence(&self, subscription: &SubscriptionDetails) -> Result<BillingPeriod, Error>;
}

impl CadenceExtractor for term_fee_pricing::Pricing {
    fn extract_cadence(&self, subscription: &SubscriptionDetails) -> Result<BillingPeriod, Error> {
        let cadence: BillingPeriod = match &self {
            term_fee_pricing::Pricing::Single(p) => p.cadence(),
            term_fee_pricing::Pricing::TermBased(_) => {
                subscription.parameters.committed_billing_period()
            }
        };
        Ok(cadence)
    }
}

pub trait PriceExtractor {
    fn extract_price(&self, subscription: &SubscriptionDetails) -> Result<Decimal, Error>;
}

impl PriceExtractor for term_fee_pricing::Pricing {
    fn extract_price(&self, subscription: &SubscriptionDetails) -> Result<Decimal, Error> {
        let price: Decimal = match &self {
            term_fee_pricing::Pricing::Single(p) => parse_decimal(&p.price),
            term_fee_pricing::Pricing::TermBased(t) => {
                let term = subscription.parameters.committed_billing_period();
                parse_decimal(
                    &t.rates
                        .iter()
                        .find(|r| r.term() == term)
                        .ok_or_else(|| anyhow!("No rate found for billing period"))?
                        .price,
                )
            }
        }?;
        Ok(price)
    }
}

pub trait NaiveDateExt {
    fn days_in_month(&self) -> u32;
}

impl NaiveDateExt for chrono::NaiveDate {
    fn days_in_month(&self) -> u32 {
        let month = self.month();
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if self.leap_year() {
                    29
                } else {
                    28
                }
            }
            _ => panic!("Invalid month: {}", month),
        }
    }
}

pub fn should_prorate(
    current_period_idx: i32,
    billing_type: BillingType,
    cadence: BillingPeriod,
) -> bool {
    return current_period_idx <= 0
        && billing_type == BillingType::Advance
        && cadence == BillingPeriod::Monthly;
}

pub fn prorate(price_cents: i64, period: &InvoiceLinePeriod) -> i64 {
    let days_in_period = period.to.signed_duration_since(period.from).num_days() as u64; // +1 ?
    let days_in_month_from = period.from.days_in_month() as u64;
    let days_in_month_to = period.to.days_in_month() as u64;

    // if from is end of month and from.day <= to.day. Ex: 2023-02-28 -> 2023-03-28+
    if period.from.day() == days_in_month_from as u32 && period.to.day() >= period.from.day() {
        return price_cents;
    }

    if days_in_period >= days_in_month_from {
        return price_cents;
    }

    // if to is end of month and from.day >= to.day. Ex: 2023-01-28+ -> 2023-02-28
    if period.to.day() == days_in_month_to as u32 && period.from.day() >= period.to.day() {
        return price_cents;
    }

    let prorated_price =
        (price_cents as f64) * (days_in_period as f64) / (days_in_month_from as f64);

    prorated_price.round() as i64
}

pub fn only_positive(price_cents: i64) -> i64 {
    if price_cents > 0 {
        price_cents
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(deprecated)]
    fn test_prorate() {
        let price_cents = 100;
        // billing day is 1, start day is 1, no proration
        let period = InvoiceLinePeriod {
            from: chrono::NaiveDate::from_ymd(2020, 1, 1),
            to: chrono::NaiveDate::from_ymd(2020, 2, 1),
        };
        let prorated_price = prorate(price_cents, &period);
        assert_eq!(
            prorated_price, 100,
            "prorated price should be equal to price when period is a full month"
        );

        let custom_price = 31;
        // billing day is 1, start day is 31, proration (1/31)
        let period = InvoiceLinePeriod {
            from: chrono::NaiveDate::from_ymd(2020, 1, 31),
            to: chrono::NaiveDate::from_ymd(2020, 2, 1),
        };

        let prorated_price = prorate(custom_price, &period);
        assert_eq!(prorated_price, 1);

        let custom_price = 28;
        // billing day is 1, start day is 28 (last day), proration (1/28)
        let period = InvoiceLinePeriod {
            from: chrono::NaiveDate::from_ymd(2023, 2, 28),
            to: chrono::NaiveDate::from_ymd(2023, 3, 1),
        };
        let prorated_price = prorate(custom_price, &period);
        assert_eq!(prorated_price, 1);

        let custom_price = 31;
        // billing day is 2, start day is 28, proration ((31 - 28 + 2)/31 == 5 / 31)
        let period = InvoiceLinePeriod {
            from: chrono::NaiveDate::from_ymd(2023, 1, 28),
            to: chrono::NaiveDate::from_ymd(2023, 2, 2),
        };
        let prorated_price = prorate(custom_price, &period);
        assert_eq!(prorated_price, 5);

        let period = InvoiceLinePeriod {
            from: chrono::NaiveDate::from_ymd(2021, 4, 1),
            to: chrono::NaiveDate::from_ymd(2021, 4, 16),
        };
        let prorated_price = prorate(price_cents, &period);
        assert_eq!(
            prorated_price, 50,
            "prorated price should be half of price when period is half a month"
        );

        let period = InvoiceLinePeriod {
            from: chrono::NaiveDate::from_ymd(2020, 1, 15),
            to: chrono::NaiveDate::from_ymd(2020, 2, 15),
        };
        let prorated_price = prorate(price_cents, &period);
        assert_eq!(prorated_price, 100, "prorated price should be equal to price when period is a full month, even if end months has less days than start month");

        let period = InvoiceLinePeriod {
            from: chrono::NaiveDate::from_ymd(2020, 2, 15),
            to: chrono::NaiveDate::from_ymd(2020, 3, 15),
        };
        let prorated_price = prorate(price_cents, &period);
        assert_eq!(prorated_price, 100, "prorated price should be equal to price when period is a full month, even if start months has less days than end month");

        // billing day is 31, with an end month < 31. No proration
        let period = InvoiceLinePeriod {
            from: chrono::NaiveDate::from_ymd(2023, 1, 31),
            to: chrono::NaiveDate::from_ymd(2023, 2, 28), // non-leap year
        };

        let prorated_price = prorate(price_cents, &period);

        assert_eq!(prorated_price, 100, "prorated price should be equal to price when period is a full month, even if start day is not present in end month");

        // billing day is 31, with a start_month.days < 31. No proration
        let period = InvoiceLinePeriod {
            from: chrono::NaiveDate::from_ymd(2023, 2, 28),
            to: chrono::NaiveDate::from_ymd(2023, 3, 31),
        };

        let prorated_price = prorate(price_cents, &period);

        assert_eq!(prorated_price, 100, "prorated price should be equal to price when period is a full month, even if end day is not present in start month");

        // billing day is 28, with a end_month.days < 28. No proration
        let period = InvoiceLinePeriod {
            from: chrono::NaiveDate::from_ymd(2023, 1, 28),
            to: chrono::NaiveDate::from_ymd(2023, 2, 28), // non-leap year
        };

        let prorated_price = prorate(price_cents, &period);

        assert_eq!(
            prorated_price, 100,
            "prorated price should be equal to price when period is a full month"
        );

        // billing day is 31, with a start month < 28. No proration
        let period = InvoiceLinePeriod {
            from: chrono::NaiveDate::from_ymd(2023, 1, 28),
            to: chrono::NaiveDate::from_ymd(2023, 2, 28), // non-leap year
        };

        let prorated_price = prorate(price_cents, &period);

        assert_eq!(
            prorated_price, 100,
            "prorated price should be equal to price when period is a full month"
        );

        let price_cents_neg = -100;

        let period = InvoiceLinePeriod {
            from: chrono::NaiveDate::from_ymd(2021, 4, 1),
            to: chrono::NaiveDate::from_ymd(2021, 4, 16),
        };

        let prorated_price = prorate(price_cents_neg, &period);

        assert_eq!(prorated_price, -50);

        // More than a month. No proration
        let period = InvoiceLinePeriod {
            from: chrono::NaiveDate::from_ymd(2020, 2, 1),
            to: chrono::NaiveDate::from_ymd(2020, 3, 31),
        };

        let prorated_price = prorate(price_cents, &period);

        assert_eq!(
            prorated_price, 100,
            "prorated price should be equal to price when period is over a month"
        );
    }
}
