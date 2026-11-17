//! The store's fees as `evaluate` prices them: `compute_component`'s pricing without its reads.
//! Prices, units, conversions and pins arrive resolved in [`StoreFee`], metering in the reads.

use std::collections::{BTreeMap, HashMap};

use billing_core::charge::{ChargeKind, Denomination, Fee};
use billing_core::facts::Billing;
use billing_core::pricer::{
    CreditGrant, Pricer, Rate, Segment, Shown, UsageRead, UsageReads, UsageWindow,
};
use billing_core::pricing::{self, GroupedUsage, PricedLine, PricingError, UnitRounding};
use billing_core::proration::DayFraction;
use common_domain::ids::{
    BillableMetricId, PriceComponentId, ProductId, SubscriptionAddOnId,
    SubscriptionPriceComponentId,
};
use error_stack::Report;
use rust_decimal::Decimal;

use crate::StoreResult;
use crate::constants::Currency;
use crate::domain::enums::{BillingType, SubscriptionFeeBillingPeriod};
use crate::domain::{
    BillableMetric, LineItem, SubscriptionFee, SubscriptionFeeInterface, UsagePricingModel,
};
use crate::errors::StoreError;
use crate::services::invoice_lines::component::{is_usage_based_line, pinned_unit_price};
use crate::services::invoice_lines::fees::pricing_error;
use crate::services::subscriptions::proration::is_period_tied;

/// A metric's unit conversion, applied to each group's window total before pricing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitConversion {
    pub factor: Option<i32>,
    pub rounding: Option<UnitRounding>,
}

/// A component or add-on row, with everything its price reads resolved ahead.
#[derive(Clone, Debug)]
pub struct StoreFee {
    pub fee: SubscriptionFee,
    pub period: SubscriptionFeeBillingPeriod,
    pub name: String,
    pub product_id: Option<ProductId>,
    pub price_component_id: Option<PriceComponentId>,
    pub sub_component_id: Option<SubscriptionPriceComponentId>,
    pub sub_add_on_id: Option<SubscriptionAddOnId>,
    pub currency: String,
    /// The currency's minor-unit exponent.
    pub precision: u8,
    /// An add-on's quantity; 1 for a plan component.
    pub instances: Decimal,
    /// The conversion of the metric the fee reads, if any.
    pub conversion: UnitConversion,
    /// Unit prices an operator pinned on the draft being refreshed, by group of dimensions.
    pub pinned: BTreeMap<BTreeMap<String, String>, Decimal>,
}

impl StoreFee {
    /// The metric whose usage the fee bills: a usage fee's, a capacity fee's when it has overage.
    pub fn metric(&self) -> Option<BillableMetricId> {
        match &self.fee {
            SubscriptionFee::Usage { metric_id, .. } => Some(*metric_id),
            SubscriptionFee::Capacity {
                metric_id,
                overage_rate,
                ..
            } if *overage_rate > Decimal::ZERO => Some(*metric_id),
            _ => None,
        }
    }

    /// When the fee bills its fixed price; usage is always read at a window's end.
    pub fn billing(&self) -> Billing {
        match (&self.fee, &self.period) {
            (_, SubscriptionFeeBillingPeriod::OneTime) | (SubscriptionFee::OneTime { .. }, _) => {
                Billing::OneTime
            }
            // A product's `once` credit rides each period until a document bills it (R16).
            (SubscriptionFee::Credit { once: true, .. }, _) if self.product_id.is_none() => {
                Billing::OneTime
            }
            (SubscriptionFee::Usage { .. }, _)
            | (
                SubscriptionFee::Recurring {
                    billing_type: BillingType::Arrears,
                    ..
                },
                _,
            ) => Billing::Arrears,
            _ => Billing::Advance,
        }
    }

    /// Months of the fee's own grid (R-084).
    pub fn cadence(&self) -> u32 {
        match self.period {
            SubscriptionFeeBillingPeriod::OneTime => 1,
            period => u32::try_from(period.as_months()).unwrap_or(1),
        }
    }

    /// The credit a pack grants for a whole period, in minor units of what it grants; a
    /// period-tied pack grants the share of a period its charge bills.
    fn credit_grant(&self) -> Result<Option<CreditGrant>, PricingError> {
        let SubscriptionFee::Credit {
            quantity,
            credit_amount,
            credit_unit,
            ..
        } = &self.fee
        else {
            return Ok(None);
        };
        let precision = credit_unit
            .as_ref()
            .map_or(self.precision, |unit| unit.precision);
        Ok(Some(CreditGrant {
            amount: pricing::pack_credit_minor(
                *credit_amount,
                Decimal::from(*quantity) * self.instances,
                precision,
            )?,
            denomination: credit_unit.as_ref().map_or(Denomination::Money, |unit| {
                Denomination::CreditUnit(unit.unit_id)
            }),
            prorated: is_period_tied(&self.fee),
        }))
    }

    /// The product a `once` pack is billed once for; a legacy one without a product bills on
    /// the first invoice only, as its `Billing::OneTime`.
    fn once_per_product(&self) -> Option<ProductId> {
        match &self.fee {
            SubscriptionFee::Credit { once: true, .. } => self.product_id,
            _ => None,
        }
    }

    /// What usage is priced in: the credit unit's minor units when priced in credits.
    fn usage_denomination(&self) -> (Denomination, u8) {
        match &self.fee {
            SubscriptionFee::Usage {
                credit_unit: Some(unit),
                ..
            }
            | SubscriptionFee::Capacity {
                credit_unit: Some(unit),
                ..
            } => (Denomination::CreditUnit(unit.unit_id), unit.precision),
            _ => (Denomination::Money, self.precision),
        }
    }
}

/// What the loader resolves for a fee row besides the row itself.
pub struct Resolved<'a> {
    pub currency: &'a Currency,
    /// The subscription's metrics, among them the one the fee reads.
    pub metrics: &'a [BillableMetric],
    /// From [`pinned_prices`] on a draft refresh, else empty.
    pub pinned: BTreeMap<BTreeMap<String, String>, Decimal>,
}

/// The fee `row` bills, refused where `compute_component` would fail on it whatever the period:
/// a price out of range, a pack granting a fraction of a minor unit, a metric not loaded.
pub fn fee_of<T: SubscriptionFeeInterface>(
    row: &T,
    resolved: Resolved<'_>,
) -> StoreResult<StoreFee> {
    let mut fee = StoreFee {
        fee: row.fee_ref().clone(),
        period: *row.period_ref(),
        name: row.name_ref().clone(),
        product_id: row.product_id(),
        price_component_id: row.price_component_id(),
        sub_component_id: row.sub_component_id(),
        sub_add_on_id: row.sub_add_on_id(),
        currency: resolved.currency.code.to_owned(),
        precision: resolved.currency.precision,
        instances: row.instance_quantity(),
        conversion: UnitConversion::default(),
        pinned: resolved.pinned,
    };
    if let Some(metric_id) = fee.metric() {
        let metric = resolved
            .metrics
            .iter()
            .find(|metric| metric.id == metric_id)
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(format!(
                    "metric with id {metric_id}"
                )))
            })?;
        fee.conversion = UnitConversion {
            factor: metric.unit_conversion_factor,
            rounding: metric.unit_conversion_rounding.as_ref().map(Into::into),
        };
    }
    fixed(&fee, &Fee::Unattached, None).map_err(pricing_error)?;
    Ok(fee)
}

/// The unit prices pinned on `draft`'s usage lines of `row`, by group of dimensions, as
/// `compute_component` reads them: a pin never crosses from money to credits or between units.
pub fn pinned_prices<T: SubscriptionFeeInterface>(
    row: &T,
    draft: &[LineItem],
) -> BTreeMap<BTreeMap<String, String>, Decimal> {
    let SubscriptionFee::Usage {
        metric_id,
        credit_unit,
        ..
    } = row.fee_ref()
    else {
        return BTreeMap::new();
    };
    draft
        .iter()
        .filter(|line| {
            is_usage_based_line(line)
                && line.metric_id == Some(*metric_id)
                && line.sub_component_id == row.sub_component_id()
                && line.sub_add_on_id == row.sub_add_on_id()
        })
        .filter_map(|line| {
            let dimensions = line.group_by_dimensions.as_ref()?;
            let price = pinned_unit_price(line, credit_unit.as_ref())?;
            Some((sorted(dimensions), price))
        })
        .collect()
}

fn sorted(dimensions: &HashMap<String, String>) -> BTreeMap<String, String> {
    dimensions
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// What `fee` bills that is not usage for a whole period with `seats` held: rounded once from
/// `rate × quantity` (R-001, R-079); none for a usage fee or a quantity of zero.
fn fixed(fee: &StoreFee, lineage: &Fee, seats: Option<u64>) -> Result<Option<Rate>, PricingError> {
    let whole = |rate: &Decimal, quantity: Decimal| {
        pricing::simple_prorated(rate, &quantity, None, fee.precision)
            .map(|line| (quantity, line.total, line.unit_price))
    };
    let instances = |quantity: u32| Decimal::from(quantity) * fee.instances;
    let (kind, (quantity, total, unit_price), prorated) = match &fee.fee {
        SubscriptionFee::Rate { rate } => (ChargeKind::Fee, whole(rate, fee.instances)?, true),
        // Billed whole once, never prorated (`process_fee_records`).
        SubscriptionFee::OneTime { rate, quantity } => {
            (ChargeKind::Fee, whole(rate, instances(*quantity))?, false)
        }
        SubscriptionFee::Recurring { rate, quantity, .. } => {
            (ChargeKind::Fee, whole(rate, instances(*quantity))?, true)
        }
        SubscriptionFee::Slot {
            unit_rate,
            min_slots,
            max_slots,
            ..
        } => {
            let slots = pricing::billed_slots(seats.unwrap_or(0), *min_slots, *max_slots);
            (
                ChargeKind::Fee,
                whole(unit_rate, Decimal::from(slots))?,
                true,
            )
        }
        // A fixed amount of capacity, billed whole where its window is a whole period (R-002).
        SubscriptionFee::Capacity { rate, .. } => {
            (ChargeKind::Fee, whole(rate, fee.instances)?, false)
        }
        SubscriptionFee::Credit { rate, quantity, .. } => {
            let units = instances(*quantity);
            let line = pricing::credit_pack(rate, units, None, fee.precision)?;
            (
                ChargeKind::Credit,
                (units, line.total, line.unit_price),
                is_period_tied(&fee.fee),
            )
        }
        SubscriptionFee::Usage { .. } => return Ok(None),
    };
    if quantity.is_zero() {
        return Ok(None);
    }
    Ok(Some(Rate {
        fee: lineage.clone(),
        kind,
        quantity: Some(quantity),
        amount: i64::try_from(total).map_err(|_| PricingError::OutOfRange)?,
        denomination: Denomination::Money,
        prorated,
        credit_grant: fee.credit_grant()?,
        once_per_product: fee.once_per_product(),
        shown: Shown {
            unit_price,
            ..Shown::default()
        },
    }))
}

/// The usage of `metric` over `window`, per group, converted to the metric's billed unit.
fn read(
    fee: &StoreFee,
    metric: BillableMetricId,
    window: &UsageWindow,
    reads: &UsageReads,
) -> Option<Vec<GroupedUsage>> {
    let usage = reads.get(&UsageRead {
        metric,
        window: *window,
    })?;
    Some(
        usage
            .iter()
            .map(|(dimensions, value)| GroupedUsage {
                value: pricing::converted_units(
                    fee.conversion.factor,
                    fee.conversion.rounding,
                    *value,
                ),
                dimensions: dimensions
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            })
            .collect(),
    )
}

/// What `fee` bills for its usage over `window`, one rate per line, in what it is priced in
/// (R-002).
fn usage_rates(
    fee: &StoreFee,
    lineage: &Fee,
    allowance: DayFraction,
    window: &UsageWindow,
    reads: &UsageReads,
) -> Result<Vec<Rate>, PricingError> {
    let Some(metric) = fee.metric() else {
        return Ok(Vec::new());
    };
    let Some(usage) = read(fee, metric, window, reads) else {
        return Ok(Vec::new());
    };
    let (denomination, precision) = fee.usage_denomination();
    let lines = match &fee.fee {
        SubscriptionFee::Usage { model, .. } => {
            pricing::usage_lines(&model.into(), &usage, precision, |dimensions| {
                fee.pinned.get(&sorted(dimensions)).copied()
            })?
        }
        SubscriptionFee::Capacity {
            included,
            overage_rate,
            ..
        } => pricing::capacity_overage_lines(
            allowance.prorate_decimal(Decimal::from(*included)),
            overage_rate,
            &usage,
            precision,
        )?,
        _ => Vec::new(),
    };
    lines
        .into_iter()
        .filter(|line| !line.quantity.is_some_and(|quantity| quantity.is_zero()))
        .map(|line| usage_rate(lineage, metric, denomination, line))
        .collect()
}

fn usage_rate(
    lineage: &Fee,
    metric: BillableMetricId,
    denomination: Denomination,
    line: PricedLine,
) -> Result<Rate, PricingError> {
    Ok(Rate {
        fee: lineage.clone(),
        kind: ChargeKind::Usage {
            metric,
            dimensions: line.dimensions.as_ref().map(sorted).unwrap_or_default(),
        },
        quantity: line.quantity,
        amount: i64::try_from(line.total).map_err(|_| PricingError::OutOfRange)?,
        denomination,
        prorated: false,
        credit_grant: None,
        once_per_product: None,
        shown: Shown {
            unit_price: line.unit_price,
            sub_lines: line.sublines,
            dimensions: line.dimensions,
        },
    })
}

/// Prices [`StoreFee`]s; a price out of range fails the whole segment.
#[derive(Clone, Copy, Debug, Default)]
pub struct StorePricer;

impl Pricer<StoreFee> for StorePricer {
    fn metrics(&self, fee: &StoreFee) -> Vec<BillableMetricId> {
        fee.metric().into_iter().collect()
    }

    /// Only usage priced by the unit, a matrix's cells included, prices each part on its own.
    fn per_part(&self, fee: &StoreFee) -> bool {
        match &fee.fee {
            SubscriptionFee::Capacity { .. } => false,
            SubscriptionFee::Usage { model, .. } => !matches!(
                model,
                UsagePricingModel::Tiered { .. }
                    | UsagePricingModel::Volume { .. }
                    | UsagePricingModel::Package { .. }
            ),
            _ => true,
        }
    }

    fn rates(&self, segment: &Segment<'_, StoreFee>) -> Result<Vec<Rate>, PricingError> {
        let mut rates = Vec::new();
        for in_force in &segment.fees {
            rates.extend(fixed(in_force.fee, in_force.lineage, in_force.seats)?);
        }
        Ok(rates)
    }

    fn usage(
        &self,
        segment: &Segment<'_, StoreFee>,
        window: &UsageWindow,
        reads: &UsageReads,
    ) -> Result<Vec<Rate>, PricingError> {
        let mut rates = Vec::new();
        for in_force in &segment.fees {
            rates.extend(usage_rates(
                in_force.fee,
                in_force.lineage,
                in_force.allowance,
                window,
                reads,
            )?);
        }
        Ok(rates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::credit_units::FeeCreditUnit;
    use crate::domain::enums::BillingMetricAggregateEnum;
    use crate::domain::price_components::{CreditExpiry, MatrixDimension, MatrixRow};
    use crate::domain::{LineItemKind, TierRow, UsagePricingModel};
    use crate::domain::{Period, UsagePeriod};
    use crate::services::clients::usage::{GroupedUsageData, UsageData};
    use crate::services::invoice_lines::component::{
        capacity_overage_lines, convert_usage_units, usage_lines as store_usage_lines,
    };
    use billing_core::DayFraction;
    use billing_core::pricer::InForce;
    use billing_core::reconcile::Term;
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    use common_domain::ids::{BaseId, CreditUnitId, ProductFamilyId, TenantId};
    use rust_decimal_macros::dec;
    use uuid::Uuid;

    const USD: Currency = Currency {
        code: "USD",
        name: "US Dollar",
        symbol: "$",
        precision: 2,
    };

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn store_fee(fee: SubscriptionFee) -> StoreFee {
        StoreFee {
            fee,
            period: SubscriptionFeeBillingPeriod::Monthly,
            name: "Fee".to_owned(),
            product_id: None,
            price_component_id: None,
            sub_component_id: Some(SubscriptionPriceComponentId::new()),
            sub_add_on_id: None,
            currency: "USD".to_owned(),
            precision: 2,
            instances: Decimal::ONE,
            conversion: UnitConversion::default(),
            pinned: BTreeMap::new(),
        }
    }

    fn lineage() -> Fee {
        Fee::Component(Uuid::from_u128(7).into())
    }

    fn rates_of(fee: &StoreFee, seats: Option<u64>) -> Vec<Rate> {
        let lineage = lineage();
        let segment = Segment {
            from: d(2026, 1, 1),
            until: Some(d(2026, 2, 1)),
            fees: vec![InForce {
                lineage: &lineage,
                row: Uuid::from_u128(8),
                billing: fee.billing(),
                cadence: fee.cadence(),
                seats,
                fee,
                allowance: DayFraction::WHOLE,
            }],
        };
        StorePricer.rates(&segment).unwrap()
    }

    fn window() -> UsageWindow {
        UsageWindow {
            from: d(2026, 1, 1).and_time(NaiveTime::MIN).and_utc(),
            until: d(2026, 2, 1).and_time(NaiveTime::MIN).and_utc(),
        }
    }

    fn reads_of(metric: BillableMetricId, usage: &[(&[(&str, &str)], Decimal)]) -> UsageReads {
        let usage = usage
            .iter()
            .map(|(dimensions, value)| {
                (
                    dimensions
                        .iter()
                        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                        .collect(),
                    *value,
                )
            })
            .collect();
        [(
            UsageRead {
                metric,
                window: window(),
            },
            usage,
        )]
        .into_iter()
        .collect()
    }

    fn usage_of(fee: &StoreFee, reads: &UsageReads) -> Vec<Rate> {
        let lineage = lineage();
        let segment = Segment {
            from: d(2026, 1, 1),
            until: Some(d(2026, 2, 1)),
            fees: vec![InForce {
                lineage: &lineage,
                row: Uuid::from_u128(8),
                billing: fee.billing(),
                cadence: fee.cadence(),
                seats: None,
                fee,
                allowance: DayFraction::WHOLE,
            }],
        };
        StorePricer.usage(&segment, &window(), reads).unwrap()
    }

    /// The grouped usage `compute_component` prices, from the same reads.
    fn grouped(usage: &[(&[(&str, &str)], Decimal)]) -> Vec<GroupedUsageData> {
        usage
            .iter()
            .map(|(dimensions, value)| GroupedUsageData {
                value: *value,
                dimensions: dimensions
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
            })
            .collect()
    }

    fn january() -> Period {
        Period {
            start: d(2026, 1, 1),
            end: d(2026, 2, 1),
        }
    }

    /// The share of January from the 11th, as `evaluate` takes it of a prorated rate.
    fn from_the_11th(full: i64) -> i64 {
        Term {
            fee: lineage(),
            full,
            period: (d(2026, 1, 1), d(2026, 2, 1)),
            from: d(2026, 1, 11),
            until: d(2026, 2, 1),
        }
        .amount()
    }

    fn the_11th_share() -> DayFraction {
        DayFraction::new(21, 31)
    }

    fn only(rates: Vec<Rate>) -> Rate {
        assert_eq!(rates.len(), 1, "{rates:?}");
        rates.into_iter().next().unwrap()
    }

    #[test]
    fn a_rate_bills_its_instances_prorated_as_simple_prorated_does() {
        let mut fee = store_fee(SubscriptionFee::Rate { rate: dec!(10) });
        fee.instances = dec!(3);
        let rate = only(rates_of(&fee, None));
        let whole = pricing::simple_prorated(&dec!(10), &dec!(3), None, 2).unwrap();
        assert_eq!(rate.amount, whole.total as i64);
        assert_eq!(rate.quantity, Some(dec!(3)));
        assert_eq!(rate.kind, ChargeKind::Fee);
        assert!(rate.prorated);
        let partial =
            pricing::simple_prorated(&dec!(10), &dec!(3), Some(the_11th_share()), 2).unwrap();
        assert_eq!(from_the_11th(rate.amount), partial.total as i64);
    }

    #[test]
    fn a_recurring_fee_bills_its_quantity_prorated_in_advance_and_arrears() {
        for billing_type in [BillingType::Advance, BillingType::Arrears] {
            let fee = store_fee(SubscriptionFee::Recurring {
                rate: dec!(9.99),
                quantity: 2,
                billing_type: billing_type.clone(),
            });
            let rate = only(rates_of(&fee, None));
            let whole = pricing::simple_prorated(&dec!(9.99), &dec!(2), None, 2).unwrap();
            assert_eq!(rate.amount, 1_998);
            assert_eq!(rate.amount, whole.total as i64);
            assert!(rate.prorated);
            let partial =
                pricing::simple_prorated(&dec!(9.99), &dec!(2), Some(the_11th_share()), 2).unwrap();
            assert_eq!(from_the_11th(rate.amount), partial.total as i64);
        }
        let arrears = store_fee(SubscriptionFee::Recurring {
            rate: dec!(1),
            quantity: 1,
            billing_type: BillingType::Arrears,
        });
        assert_eq!(arrears.billing(), Billing::Arrears);
    }

    #[test]
    fn a_one_time_fee_bills_whole() {
        let mut fee = store_fee(SubscriptionFee::OneTime {
            rate: dec!(49.5),
            quantity: 2,
        });
        fee.instances = dec!(2);
        let rate = only(rates_of(&fee, None));
        let whole = pricing::simple_prorated(&dec!(49.5), &dec!(4), None, 2).unwrap();
        assert_eq!(rate.amount, whole.total as i64);
        assert_eq!(rate.quantity, Some(dec!(4)));
        assert!(!rate.prorated);
        assert_eq!(fee.billing(), Billing::OneTime);
    }

    #[test]
    fn slots_bill_the_segments_seats_within_their_bounds() {
        let fee = store_fee(SubscriptionFee::Slot {
            unit: "seat".to_owned(),
            unit_rate: dec!(25),
            min_slots: Some(5),
            max_slots: Some(20),
            initial_slots: 1,
        });
        let at_minimum = only(rates_of(&fee, Some(3)));
        assert_eq!(at_minimum.quantity, Some(dec!(5)));
        assert_eq!(
            at_minimum.amount,
            pricing::simple_prorated(&dec!(25), &dec!(5), None, 2)
                .unwrap()
                .total as i64
        );
        assert!(at_minimum.prorated);
        let held = only(rates_of(&fee, Some(8)));
        assert_eq!((held.quantity, held.amount), (Some(dec!(8)), 20_000));
        assert_eq!(only(rates_of(&fee, Some(50))).quantity, Some(dec!(20)));
        let partial =
            pricing::simple_prorated(&dec!(25), &dec!(8), Some(the_11th_share()), 2).unwrap();
        assert_eq!(from_the_11th(held.amount), partial.total as i64);
    }

    #[test]
    fn no_seats_bill_nothing_like_a_zero_quantity_line() {
        let fee = store_fee(SubscriptionFee::Slot {
            unit: "seat".to_owned(),
            unit_rate: dec!(25),
            min_slots: None,
            max_slots: None,
            initial_slots: 4,
        });
        assert!(rates_of(&fee, None).is_empty());
        let mut add_on = store_fee(SubscriptionFee::Rate { rate: dec!(10) });
        add_on.instances = Decimal::ZERO;
        assert!(rates_of(&add_on, None).is_empty());
    }

    #[test]
    fn capacity_bills_its_rate_unprorated_and_its_overage_as_usage() {
        let metric = BillableMetricId::new();
        let fee = store_fee(SubscriptionFee::Capacity {
            rate: dec!(100),
            included: 1_000,
            overage_rate: dec!(0.5),
            metric_id: metric,
            credit_unit: None,
        });
        let rate = only(rates_of(&fee, None));
        assert_eq!(rate.amount, 10_000);
        assert_eq!(rate.kind, ChargeKind::Fee);
        assert!(!rate.prorated);
        assert_eq!(fee.billing(), Billing::Advance);
        assert_eq!(StorePricer.metrics(&fee), vec![metric]);

        let usage: &[(&[(&str, &str)], Decimal)] = &[(&[("region", "eu")], dec!(1_500))];
        let overage = only(usage_of(&fee, &reads_of(metric, usage)));
        let today = capacity_overage_lines(
            metric,
            dec!(1_000),
            &dec!(0.5),
            &grouped(usage),
            &january(),
            2,
        )
        .unwrap();
        assert_eq!(overage.amount, today[0].total as i64);
        assert_eq!(overage.amount, 25_000);
        assert_eq!(
            overage.kind,
            ChargeKind::Usage {
                metric,
                dimensions: BTreeMap::from([("region".to_owned(), "eu".to_owned())]),
            }
        );
        assert!(!overage.prorated);
    }

    /// 31.00 for 310 units at 0.10 over, cut to 16 of January's 31 days: 160 units included, so
    /// 200 used owe 40 × 10 = 400; whole, they owe nothing.
    /// Rules: R-002
    #[test]
    fn a_capacity_counts_the_share_of_its_allowance_it_is_given() {
        let metric = BillableMetricId::new();
        let fee = store_fee(SubscriptionFee::Capacity {
            rate: dec!(31),
            included: 310,
            overage_rate: dec!(0.10),
            metric_id: metric,
            credit_unit: None,
        });
        let reads = reads_of(metric, &[(&[], dec!(200))]);
        let lineage = lineage();
        let priced = |allowance| {
            let segment = Segment {
                from: d(2026, 1, 1),
                until: Some(d(2026, 2, 1)),
                fees: vec![InForce {
                    lineage: &lineage,
                    row: Uuid::from_u128(8),
                    billing: fee.billing(),
                    cadence: fee.cadence(),
                    seats: None,
                    fee: &fee,
                    allowance,
                }],
            };
            StorePricer
                .usage(&segment, &window(), &reads)
                .unwrap()
                .iter()
                .map(|rate| rate.amount)
                .sum::<i64>()
        };
        assert_eq!(priced(DayFraction::new(16, 31)), 400);
        assert_eq!(priced(DayFraction::WHOLE), 0);
    }

    /// Usage priced by the unit or by a matrix's cells prices each part of a period on its own;
    /// a capacity, tiers and packages price the period once.
    /// Rules: R-097
    #[test]
    fn only_linear_usage_prices_each_part_on_its_own() {
        let metric = BillableMetricId::new();
        let per_part = |model| StorePricer.per_part(&usage_fee(metric, model));
        assert!(per_part(UsagePricingModel::PerUnit { rate: dec!(1) }));
        assert!(per_part(UsagePricingModel::Matrix { rates: vec![] }));
        for model in [
            UsagePricingModel::Tiered {
                tiers: vec![tier(0, dec!(1))],
                block_size: None,
            },
            UsagePricingModel::Volume {
                tiers: vec![tier(0, dec!(1))],
                block_size: None,
            },
            UsagePricingModel::Package {
                block_size: 10,
                rate: dec!(1),
            },
        ] {
            assert!(!per_part(model));
        }
        let capacity = store_fee(SubscriptionFee::Capacity {
            rate: dec!(31),
            included: 310,
            overage_rate: dec!(0.10),
            metric_id: metric,
            credit_unit: None,
        });
        assert!(!StorePricer.per_part(&capacity));
        assert!(StorePricer.per_part(&store_fee(SubscriptionFee::Rate { rate: dec!(1) })));
    }

    #[test]
    fn capacity_without_overage_reads_nothing() {
        let metric = BillableMetricId::new();
        let fee = store_fee(SubscriptionFee::Capacity {
            rate: dec!(100),
            included: 1_000,
            overage_rate: Decimal::ZERO,
            metric_id: metric,
            credit_unit: None,
        });
        assert!(StorePricer.metrics(&fee).is_empty());
        let usage: &[(&[(&str, &str)], Decimal)] = &[(&[], dec!(5_000))];
        assert!(usage_of(&fee, &reads_of(metric, usage)).is_empty());
    }

    fn pack(expiry: Option<CreditExpiry>, once: bool) -> StoreFee {
        store_fee(SubscriptionFee::Credit {
            rate: dec!(100),
            quantity: 2,
            credit_amount: dec!(50),
            expiry,
            credit_unit: None,
            rollover: None,
            once,
        })
    }

    #[test]
    fn a_period_tied_pack_is_charged_and_grants_its_share() {
        let fee = pack(Some(CreditExpiry::BillingPeriod), false);
        let rate = only(rates_of(&fee, None));
        assert_eq!(rate.kind, ChargeKind::Credit);
        assert!(rate.prorated);
        let whole = pricing::credit_pack(&dec!(100), dec!(2), None, 2).unwrap();
        assert_eq!(rate.amount, whole.total as i64);
        let share = the_11th_share();
        let partial = pricing::credit_pack(&dec!(100), dec!(2), Some(share), 2).unwrap();
        assert_eq!(from_the_11th(rate.amount), partial.total as i64);
        let granted = pricing::pack_credit_minor(dec!(50), dec!(2), 2).unwrap();
        let grant = rate.credit_grant.expect("a pack grants credit");
        assert_eq!(grant.amount, granted);
        assert_eq!(grant.denomination, Denomination::Money);
        assert!(grant.prorated);
        assert_eq!(from_the_11th(grant.amount), share.prorate(granted));
        assert_eq!(rate.once_per_product, None);
    }

    #[test]
    fn any_other_pack_is_bought_whole() {
        for fee in [
            pack(Some(CreditExpiry::Month(3)), false),
            pack(None, false),
            pack(Some(CreditExpiry::BillingPeriod), true),
        ] {
            let rate = only(rates_of(&fee, None));
            assert!(!rate.prorated);
            assert_eq!(rate.amount, 20_000);
            let grant = rate.credit_grant.expect("a pack grants credit");
            assert_eq!((grant.amount, grant.prorated), (10_000, false));
        }
        assert_eq!(
            pack(Some(CreditExpiry::BillingPeriod), true).billing(),
            Billing::OneTime
        );
    }

    fn usage_fee(metric: BillableMetricId, model: UsagePricingModel) -> StoreFee {
        store_fee(SubscriptionFee::Usage {
            metric_id: metric,
            model,
            credit_unit: None,
        })
    }

    /// The Pricer's usage rates against `usage_lines` on the same usage, line by line.
    fn assert_priced_as_today(fee: &StoreFee, usage: &[(&[(&str, &str)], Decimal)]) -> Vec<Rate> {
        let SubscriptionFee::Usage {
            metric_id, model, ..
        } = &fee.fee
        else {
            panic!("a usage fee");
        };
        let rates = usage_of(fee, &reads_of(*metric_id, usage));
        let today = store_usage_lines(
            model,
            *metric_id,
            &grouped(usage),
            &january(),
            fee.usage_denomination().1,
            |dimensions| fee.pinned.get(&sorted(dimensions)).copied(),
        )
        .unwrap();
        let mut expected: Vec<(BTreeMap<String, String>, i64, Option<Decimal>)> = today
            .iter()
            .map(|line| {
                (
                    line.group_by_dimensions
                        .as_ref()
                        .map(sorted)
                        .unwrap_or_default(),
                    line.total as i64,
                    line.quantity,
                )
            })
            .collect();
        let mut actual: Vec<(BTreeMap<String, String>, i64, Option<Decimal>)> = rates
            .iter()
            .map(|rate| {
                let ChargeKind::Usage { dimensions, .. } = &rate.kind else {
                    panic!("a usage charge");
                };
                assert!(!rate.prorated);
                (dimensions.clone(), rate.amount, rate.quantity)
            })
            .collect();
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected);
        rates
    }

    #[test]
    fn per_unit_usage_bills_each_group() {
        let metric = BillableMetricId::new();
        let fee = usage_fee(metric, UsagePricingModel::PerUnit { rate: dec!(0.02) });
        assert_eq!(StorePricer.metrics(&fee), vec![metric]);
        assert_eq!(fee.billing(), Billing::Arrears);
        assert!(rates_of(&fee, None).is_empty());
        let rates = assert_priced_as_today(
            &fee,
            &[
                (&[("model", "a")], dec!(1_234)),
                (&[("model", "b")], dec!(10)),
                (&[("model", "c")], Decimal::ZERO),
            ],
        );
        assert_eq!(rates.len(), 2);
    }

    #[test]
    fn a_pinned_unit_price_overrides_the_rate_of_its_group() {
        let metric = BillableMetricId::new();
        let mut fee = usage_fee(metric, UsagePricingModel::PerUnit { rate: dec!(1) });
        fee.pinned = BTreeMap::from([(
            BTreeMap::from([("model".to_owned(), "a".to_owned())]),
            dec!(0.5),
        )]);
        let rates = assert_priced_as_today(
            &fee,
            &[(&[("model", "a")], dec!(10)), (&[("model", "b")], dec!(10))],
        );
        let amounts: BTreeMap<String, i64> = rates
            .iter()
            .map(|rate| match &rate.kind {
                ChargeKind::Usage { dimensions, .. } => (dimensions["model"].clone(), rate.amount),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(amounts["a"], 500);
        assert_eq!(amounts["b"], 1_000);
    }

    fn tier(first_unit: u64, rate: Decimal) -> TierRow {
        TierRow {
            first_unit,
            rate,
            flat_fee: None,
            flat_cap: None,
        }
    }

    #[test]
    fn tiered_volume_and_package_usage_price_as_today() {
        let metric = BillableMetricId::new();
        let tiers = vec![tier(0, dec!(1)), tier(100, dec!(0.5))];
        let usage: &[(&[(&str, &str)], Decimal)] = &[(&[], dec!(150))];
        let tiered = assert_priced_as_today(
            &usage_fee(
                metric,
                UsagePricingModel::Tiered {
                    tiers: tiers.clone(),
                    block_size: None,
                },
            ),
            usage,
        );
        assert_eq!(tiered[0].amount, 12_500);
        let volume = assert_priced_as_today(
            &usage_fee(
                metric,
                UsagePricingModel::Volume {
                    tiers,
                    block_size: None,
                },
            ),
            usage,
        );
        assert_eq!(volume[0].amount, 7_500);
        let package = assert_priced_as_today(
            &usage_fee(
                metric,
                UsagePricingModel::Package {
                    block_size: 100,
                    rate: dec!(3),
                },
            ),
            usage,
        );
        assert_eq!(package[0].amount, 600);
    }

    fn cell(value: &str, price: Decimal) -> MatrixRow {
        MatrixRow {
            dimension1: MatrixDimension {
                key: "region".to_owned(),
                value: value.to_owned(),
            },
            dimension2: None,
            per_unit_price: price,
        }
    }

    #[test]
    fn matrix_usage_bills_one_line_per_group_of_other_dimensions() {
        let metric = BillableMetricId::new();
        let fee = usage_fee(
            metric,
            UsagePricingModel::Matrix {
                rates: vec![cell("eu", dec!(2)), cell("us", dec!(3))],
            },
        );
        let rates = assert_priced_as_today(
            &fee,
            &[
                (&[("region", "eu"), ("team", "x")], dec!(10)),
                (&[("region", "us"), ("team", "x")], dec!(5)),
                (&[("region", "eu"), ("team", "y")], dec!(1)),
            ],
        );
        assert_eq!(rates.len(), 2);
        let x = rates
            .iter()
            .find(|rate| {
                matches!(&rate.kind, ChargeKind::Usage { dimensions, .. }
                    if dimensions == &BTreeMap::from([("team".to_owned(), "x".to_owned())]))
            })
            .unwrap();
        assert_eq!(x.amount, 3_500);
    }

    #[test]
    fn usage_priced_in_credits_is_rated_in_the_units_minor_units() {
        let metric = BillableMetricId::new();
        let fee = store_fee(SubscriptionFee::Usage {
            metric_id: metric,
            model: UsagePricingModel::PerUnit { rate: dec!(1.5) },
            credit_unit: Some(FeeCreditUnit {
                unit_id: CreditUnitId::new(),
                code: "CREDITS".to_owned(),
                precision: 0,
                rate: dec!(0.01),
            }),
        });
        let rates = assert_priced_as_today(&fee, &[(&[], dec!(3))]);
        assert_eq!(rates[0].amount, 5);
    }

    fn tokens(rounding: crate::domain::enums::UnitConversionRoundingEnum) -> BillableMetric {
        BillableMetric {
            id: BillableMetricId::new(),
            name: "Tokens".to_owned(),
            description: None,
            code: "tokens".to_owned(),
            aggregation_type: BillingMetricAggregateEnum::Sum,
            aggregation_key: Some("tokens".to_owned()),
            unit_conversion_factor: Some(1_000_000),
            unit_conversion_rounding: Some(rounding),
            unit_singular: Some("token".to_owned()),
            unit_plural: Some("tokens".to_owned()),
            segmentation_matrix: None,
            filters: None,
            usage_group_key: None,
            usage_breakdown_key: None,
            created_at: NaiveDateTime::default(),
            updated_at: None,
            archived_at: None,
            tenant_id: TenantId::new(),
            product_family_id: ProductFamilyId::new(),
            product_id: None,
            usage_scope: Default::default(),
        }
    }

    /// The numbers of `invoice_and_live_projection_price_rounded_usage_alike`.
    #[test]
    fn usage_is_converted_to_the_metrics_unit_before_pricing() {
        use crate::domain::enums::UnitConversionRoundingEnum as Rounding;
        let model = UsagePricingModel::PerUnit { rate: dec!(12) };
        let usage: &[(&[(&str, &str)], Decimal)] = &[
            (&[("model", "standard")], dec!(9_004_601)),
            (&[("model", "lite")], dec!(250_000)),
        ];
        for (rounding, expected) in [
            (Rounding::Up, 13_200),
            (Rounding::Down, 10_800),
            (Rounding::Nearest, 10_800),
        ] {
            let metric = tokens(rounding);
            let component = crate::domain::SubscriptionComponent {
                id: SubscriptionPriceComponentId::new(),
                price_component_id: None,
                product_id: None,
                subscription_id: common_domain::ids::SubscriptionId::new(),
                name: "Tokens".to_owned(),
                period: SubscriptionFeeBillingPeriod::Monthly,
                fee: SubscriptionFee::Usage {
                    metric_id: metric.id,
                    model: model.clone(),
                    credit_unit: None,
                },
                price_id: None,
                effective_from: d(2026, 1, 1),
                effective_to: None,
                lineage_id: None,
                added_by_amendment: false,
            };
            let fee = fee_of(
                &component,
                Resolved {
                    currency: &USD,
                    metrics: std::slice::from_ref(&metric),
                    pinned: BTreeMap::new(),
                },
            )
            .unwrap();
            let priced: i64 = usage_of(&fee, &reads_of(metric.id, usage))
                .iter()
                .map(|rate| rate.amount)
                .sum();
            let converted = convert_usage_units(
                &metric,
                UsageData {
                    data: grouped(usage),
                    period: UsagePeriod {
                        start: d(2026, 1, 1).and_time(NaiveTime::MIN),
                        end: d(2026, 2, 1).and_time(NaiveTime::MIN),
                    },
                },
            );
            let today: i64 =
                store_usage_lines(&model, metric.id, &converted.data, &january(), 2, |_| None)
                    .unwrap()
                    .iter()
                    .map(|line| line.total as i64)
                    .sum();
            assert_eq!(priced, expected);
            assert_eq!(priced, today);
        }
    }

    #[test]
    fn a_missing_read_bills_nothing() {
        let metric = BillableMetricId::new();
        let fee = usage_fee(metric, UsagePricingModel::PerUnit { rate: dec!(1) });
        let other: &[(&[(&str, &str)], Decimal)] = &[(&[], dec!(10))];
        assert!(usage_of(&fee, &reads_of(BillableMetricId::new(), other)).is_empty());
    }

    #[test]
    fn fee_of_refuses_what_compute_component_fails_on() {
        let add_on = |fee: SubscriptionFee| crate::domain::SubscriptionAddOn {
            id: SubscriptionAddOnId::new(),
            subscription_id: common_domain::ids::SubscriptionId::new(),
            add_on_id: common_domain::ids::AddOnId::new(),
            name: "Add-on".to_owned(),
            period: SubscriptionFeeBillingPeriod::Monthly,
            fee,
            created_at: NaiveDateTime::default(),
            product_id: None,
            price_id: None,
            quantity: 3,
            effective_from: d(2026, 1, 1),
            effective_to: None,
            lineage_id: None,
            added_by_amendment: false,
        };
        let resolved = || Resolved {
            currency: &USD,
            metrics: &[],
            pinned: BTreeMap::new(),
        };
        let fee = fee_of(
            &add_on(SubscriptionFee::Rate { rate: dec!(10) }),
            resolved(),
        )
        .unwrap();
        assert_eq!(fee.instances, dec!(3));
        assert_eq!(only(rates_of(&fee, None)).amount, 3_000);
        assert!(
            fee_of(
                &add_on(SubscriptionFee::Usage {
                    metric_id: BillableMetricId::new(),
                    model: UsagePricingModel::PerUnit { rate: dec!(1) },
                    credit_unit: None,
                }),
                resolved(),
            )
            .is_err(),
            "a metric not loaded"
        );
        assert!(
            fee_of(
                &add_on(SubscriptionFee::Credit {
                    rate: dec!(1),
                    quantity: 1,
                    credit_amount: dec!(0.005),
                    expiry: None,
                    credit_unit: None,
                    rollover: None,
                    once: false,
                }),
                resolved(),
            )
            .is_err(),
            "a pack granting a fraction of a cent"
        );
    }

    fn draft_line(
        metric: BillableMetricId,
        component: SubscriptionPriceComponentId,
        dimensions: &[(&str, &str)],
        unit_price: Decimal,
    ) -> LineItem {
        LineItem {
            local_id: "l".to_owned(),
            name: "API calls".to_owned(),
            charge_type: None,
            amount_subtotal: 0,
            tax_rate: Decimal::ZERO,
            taxable_amount: 0,
            tax_amount: 0,
            amount_total: 0,
            tax_details: Vec::new(),
            quantity: Some(dec!(10)),
            unit_price: Some(unit_price),
            start_date: d(2026, 1, 1),
            end_date: d(2026, 2, 1),
            sub_lines: Vec::new(),
            is_prorated: false,
            price_component_id: None,
            sub_component_id: Some(component),
            sub_add_on_id: None,
            product_id: None,
            metric_id: Some(metric),
            description: None,
            group_by_dimensions: Some(
                dimensions
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
            ),
            kind: LineItemKind::Regular,
            credit_grant_amount: None,
            credit_grant_expiry: None,
            credit_grant_included: false,
            taxed_at_purchase: false,
            tax_inclusive_quote: None,
            credit_unit: None,
            credit_grant_unit: None,
            credit_grant_rollover: None,
            promotional_credit: None,
            subscription_id: None,
            usage_ends_at: None,
            usage_breakdown: None,
        }
    }

    #[test]
    fn pins_are_read_from_the_drafts_lines_of_the_fee() {
        let metric = BillableMetricId::new();
        let component = crate::domain::SubscriptionComponent {
            id: SubscriptionPriceComponentId::new(),
            price_component_id: None,
            product_id: None,
            subscription_id: common_domain::ids::SubscriptionId::new(),
            name: "API calls".to_owned(),
            period: SubscriptionFeeBillingPeriod::Monthly,
            fee: SubscriptionFee::Usage {
                metric_id: metric,
                model: UsagePricingModel::PerUnit { rate: dec!(1) },
                credit_unit: None,
            },
            price_id: None,
            effective_from: d(2026, 1, 1),
            effective_to: None,
            lineage_id: None,
            added_by_amendment: false,
        };
        let draft = [
            draft_line(metric, component.id, &[("model", "a")], dec!(0.5)),
            draft_line(
                metric,
                SubscriptionPriceComponentId::new(),
                &[("model", "b")],
                dec!(0.7),
            ),
        ];
        assert_eq!(
            pinned_prices(&component, &draft),
            BTreeMap::from([(
                BTreeMap::from([("model".to_owned(), "a".to_owned())]),
                dec!(0.5)
            )])
        );
    }

    #[test]
    fn a_once_pack_names_its_product_for_reconcile() {
        let mut fee = pack(Some(CreditExpiry::Month(1)), true);
        let product = ProductId::new();
        fee.product_id = Some(product);
        let rate = only(rates_of(&fee, None));
        assert_eq!(rate.once_per_product, Some(product));
        fee.product_id = None;
        assert_eq!(only(rates_of(&fee, None)).once_per_product, None);
        let mut recurring = pack(None, false);
        recurring.product_id = Some(product);
        assert_eq!(only(rates_of(&recurring, None)).once_per_product, None);
    }

    #[test]
    fn a_pack_of_a_credit_unit_grants_in_that_unit() {
        let unit = CreditUnitId::new();
        let fee = store_fee(SubscriptionFee::Credit {
            rate: dec!(10),
            quantity: 1,
            credit_amount: dec!(500),
            expiry: None,
            credit_unit: Some(crate::domain::credit_units::CreditUnitRef {
                unit_id: unit,
                code: "CREDITS".to_owned(),
                precision: 0,
                product_id: None,
            }),
            rollover: None,
            once: false,
        });
        let rate = only(rates_of(&fee, None));
        assert_eq!(rate.denomination, Denomination::Money);
        assert_eq!(
            rate.credit_grant,
            Some(CreditGrant {
                amount: 500,
                denomination: Denomination::CreditUnit(unit),
                prorated: false,
            })
        );
    }

    #[test]
    fn usage_priced_in_credits_is_denominated_in_the_unit() {
        let (metric, unit) = (BillableMetricId::new(), CreditUnitId::new());
        let credit_unit = FeeCreditUnit {
            unit_id: unit,
            code: "CREDITS".to_owned(),
            precision: 0,
            rate: dec!(0.01),
        };
        let usage: &[(&[(&str, &str)], Decimal)] = &[(&[], dec!(1_500))];
        let fee = store_fee(SubscriptionFee::Usage {
            metric_id: metric,
            model: UsagePricingModel::PerUnit { rate: dec!(2) },
            credit_unit: Some(credit_unit.clone()),
        });
        let rate = only(usage_of(&fee, &reads_of(metric, usage)));
        assert_eq!(rate.denomination, Denomination::CreditUnit(unit));
        assert_eq!(rate.amount, 3_000);
        let capacity = store_fee(SubscriptionFee::Capacity {
            rate: dec!(100),
            included: 1_000,
            overage_rate: dec!(1),
            metric_id: metric,
            credit_unit: Some(credit_unit),
        });
        assert_eq!(
            only(rates_of(&capacity, None)).denomination,
            Denomination::Money
        );
        let overage = only(usage_of(&capacity, &reads_of(metric, usage)));
        assert_eq!(
            (overage.denomination, overage.amount),
            (Denomination::CreditUnit(unit), 500)
        );
    }

    #[test]
    fn a_price_out_of_range_is_an_error_never_nothing() {
        let lineage = lineage();
        let fee = store_fee(SubscriptionFee::Slot {
            unit: "seat".to_owned(),
            unit_rate: Decimal::MAX,
            min_slots: None,
            max_slots: None,
            initial_slots: 0,
        });
        let segment = Segment {
            from: d(2026, 1, 1),
            until: Some(d(2026, 2, 1)),
            fees: vec![InForce {
                lineage: &lineage,
                row: Uuid::from_u128(8),
                billing: fee.billing(),
                cadence: fee.cadence(),
                seats: Some(1),
                fee: &fee,
                allowance: DayFraction::WHOLE,
            }],
        };
        assert_eq!(StorePricer.rates(&segment), Err(PricingError::OutOfRange));
    }
}
