//! A plan's recurring price as the portal shows it, from its price components (never MRR).
use common_domain::ids::{BillableMetricId, CreditUnitId};
use common_utils::decimals::ToSubunit;
use meteroid_grpc::meteroid::api::billablemetrics::v1::MetricUnitLabel;
use meteroid_grpc::meteroid::api::subscriptions::v1 as sub_proto;
use meteroid_grpc::meteroid::portal::subscription::v1::{
    CreditRolloverTerms, CreditTerm, IncludedCredit, PlanDiscount, PlanPrice, RecurringAmount,
    UsageRates,
};
use meteroid_grpc::meteroid::portal::wallets::v1::CreditUnitLabel;
use meteroid_store::constants::Currencies;
use meteroid_store::domain::BillableMetric;
use meteroid_store::domain::credit_units::{CreditUnit, CreditUnitsById};
use meteroid_store::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use meteroid_store::domain::plans::FullPlan;
use meteroid_store::domain::portal_credit::PortalPlanInputs;
use meteroid_store::domain::portal_credit::apply_discounts;
use meteroid_store::domain::price_components::UsagePricingModel;
use meteroid_store::domain::price_components::{CreditExpiry, CreditRollover};
use meteroid_store::domain::prices::FeeStructure;
use meteroid_store::domain::prices::Pricing;
use meteroid_store::domain::subscription_components::SubscriptionFee;
use meteroid_store::domain::subscription_coupons::AppliedCouponDetailed;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};
use tonic::Status;

pub fn cadence_to_proto(period: SubscriptionFeeBillingPeriod) -> i32 {
    match period {
        SubscriptionFeeBillingPeriod::OneTime => sub_proto::SubscriptionFeeBillingPeriod::OneTime,
        SubscriptionFeeBillingPeriod::Monthly => sub_proto::SubscriptionFeeBillingPeriod::Monthly,
        SubscriptionFeeBillingPeriod::Quarterly => {
            sub_proto::SubscriptionFeeBillingPeriod::Quarterly
        }
        SubscriptionFeeBillingPeriod::Semiannual => {
            sub_proto::SubscriptionFeeBillingPeriod::Semiannual
        }
        SubscriptionFeeBillingPeriod::Annual => sub_proto::SubscriptionFeeBillingPeriod::Yearly,
    }
    .into()
}

pub fn credit_unit_label(unit: &CreditUnit) -> CreditUnitLabel {
    CreditUnitLabel {
        code: unit.code.as_str().to_owned(),
        name: unit.name.clone(),
        plural_name: unit.plural_name.clone(),
        symbol: unit.symbol.clone(),
        precision: u32::from(unit.precision),
    }
}

fn to_minor(amount: Decimal, precision: u8) -> Result<i64, Status> {
    amount
        .to_subunit_opt(precision)
        .ok_or_else(|| Status::out_of_range("a plan amount exceeds the supported range"))
}

fn credit_term(expiry: CreditExpiry) -> CreditTerm {
    match expiry {
        CreditExpiry::Day(days) => CreditTerm {
            days: Some(days),
            ..Default::default()
        },
        CreditExpiry::Month(months) => CreditTerm {
            months: Some(months),
            ..Default::default()
        },
        CreditExpiry::BillingPeriod => CreditTerm {
            period_end: true,
            ..Default::default()
        },
    }
}

/// A credit pack's terms, independent of whether it comes from a plan price or a subscription.
struct Pack<'a> {
    price: Decimal,
    credit: Decimal,
    unit: Option<&'a CreditUnit>,
    expiry: Option<CreditExpiry>,
    rollover: Option<&'a CreditRollover>,
}

struct Builder {
    currency: String,
    precision: u8,
    cadence: SubscriptionFeeBillingPeriod,
    amount: Decimal,
    others: Vec<(SubscriptionFeeBillingPeriod, Decimal)>,
    credits: Vec<IncludedCredit>,
    usage_based: bool,
    usage: UsageCollector,
}

/// The per-unit rates of a plan's usage. Rates of different metrics' units, or in different
/// denominations, are not comparable: the range is then left out.
#[derive(Default)]
struct UsageCollector {
    rates: Vec<Decimal>,
    metrics: HashSet<BillableMetricId>,
    unit_sizes: HashSet<u64>,
    denominations: HashSet<Option<CreditUnitId>>,
    names: HashMap<BillableMetricId, String>,
    unit_labels: HashMap<BillableMetricId, MetricUnitLabel>,
}

impl UsageCollector {
    fn add(
        &mut self,
        metric_id: BillableMetricId,
        metric: Option<&BillableMetric>,
        model: &UsagePricingModel,
        credit_unit: Option<CreditUnitId>,
    ) {
        let factor = metric
            .and_then(|metric| metric.unit_conversion_factor)
            .and_then(|factor| u64::try_from(factor).ok())
            .filter(|factor| *factor > 0)
            .unwrap_or(1);
        let (rates, block) = match model {
            UsagePricingModel::PerUnit { rate } => (vec![*rate], 1),
            UsagePricingModel::Tiered { tiers, block_size }
            | UsagePricingModel::Volume { tiers, block_size } => (
                tiers.iter().map(|tier| tier.rate).collect(),
                block_size.unwrap_or(1).max(1),
            ),
            UsagePricingModel::Package { block_size, rate } => (vec![*rate], (*block_size).max(1)),
            UsagePricingModel::Matrix { rates } => {
                (rates.iter().map(|row| row.per_unit_price).collect(), 1)
            }
        };
        self.rates.extend(rates);
        self.metrics.insert(metric_id);
        self.unit_sizes.insert(factor.saturating_mul(block));
        self.denominations.insert(credit_unit);
        if let Some(metric) = metric {
            self.names.insert(metric_id, metric.name.clone());
            if let Some(label) = crate::billablemetrics::mapping::metric::unit_label_to_server(
                metric.unit_singular.clone(),
                metric.unit_plural.clone(),
            ) {
                self.unit_labels.insert(metric_id, label);
            }
        }
    }

    fn build(self, currency: &str, units: &CreditUnitsById) -> Option<UsageRates> {
        let [unit_size] = self.unit_sizes.into_iter().collect::<Vec<_>>()[..] else {
            return None;
        };
        let [denomination] = self.denominations.into_iter().collect::<Vec<_>>()[..] else {
            return None;
        };
        let min = self.rates.iter().min()?;
        let max = self.rates.iter().max()?;
        let credit_unit = match denomination {
            Some(id) => Some(units.get(&id)?),
            None => None,
        };
        let (metric_name, unit_label) = match self.metrics.into_iter().collect::<Vec<_>>()[..] {
            [metric] => (
                self.names.get(&metric).cloned(),
                self.unit_labels.get(&metric).cloned(),
            ),
            _ => (None, None),
        };
        Some(UsageRates {
            min_rate: min.normalize().to_string(),
            max_rate: max.normalize().to_string(),
            currency: credit_unit
                .map_or_else(|| currency.to_owned(), |unit| unit.code.as_str().to_owned()),
            credit_unit: credit_unit.map(credit_unit_label),
            metric_name,
            unit_size,
            unit_label,
        })
    }
}

impl Builder {
    fn new(currency: &str, cadence: SubscriptionFeeBillingPeriod) -> Result<Self, Status> {
        let precision = Currencies::resolve_currency_precision(currency)
            .ok_or_else(|| Status::internal(format!("unknown currency {currency}")))?;
        Ok(Self {
            currency: currency.to_owned(),
            precision,
            cadence,
            amount: Decimal::ZERO,
            others: Vec::new(),
            credits: Vec::new(),
            usage_based: false,
            usage: UsageCollector::default(),
        })
    }

    fn add(&mut self, cadence: SubscriptionFeeBillingPeriod, amount: Decimal) {
        if cadence == SubscriptionFeeBillingPeriod::OneTime {
            return;
        }
        if cadence == self.cadence {
            self.amount += amount;
        } else if let Some((_, total)) = self.others.iter_mut().find(|(c, _)| *c == cadence) {
            *total += amount;
        } else {
            self.others.push((cadence, amount));
        }
    }

    fn add_pack(
        &mut self,
        cadence: SubscriptionFeeBillingPeriod,
        pack: Pack,
    ) -> Result<(), Status> {
        if cadence == SubscriptionFeeBillingPeriod::OneTime {
            return Ok(());
        }
        self.add(cadence, pack.price);
        let credit_precision = pack.unit.map_or(self.precision, |unit| unit.precision);
        let rollover = pack
            .rollover
            .map(|rollover| {
                Ok::<_, Status>(CreditRolloverTerms {
                    max_amount: rollover
                        .max_amount
                        .map(|max| to_minor(max, credit_precision))
                        .transpose()?,
                    expiry: Some(credit_term(rollover.expiry)),
                })
            })
            .transpose()?;
        self.credits.push(IncludedCredit {
            amount: to_minor(pack.credit, credit_precision)?,
            currency: pack.unit.map_or_else(
                || self.currency.clone(),
                |unit| unit.code.as_str().to_owned(),
            ),
            credit_unit: pack.unit.map(credit_unit_label),
            cadence: cadence_to_proto(cadence),
            purchased: pack.price > Decimal::ZERO,
            expiry: pack.expiry.map(credit_term),
            rollover,
        });
        Ok(())
    }

    fn build(
        self,
        minimum: Option<i64>,
        coupons: &[AppliedCouponDetailed],
        units: &CreditUnitsById,
    ) -> Result<PlanPrice, Status> {
        let list_amount = to_minor(self.amount, self.precision)?;
        let (amount, discounts) =
            apply_discounts(list_amount, &self.currency, self.precision, coupons);
        Ok(PlanPrice {
            amount,
            list_amount: (!discounts.is_empty()).then_some(list_amount),
            discounts: discounts
                .into_iter()
                .map(|discount| PlanDiscount {
                    code: discount.code,
                    percentage: discount.percentage.map(|pct| pct.normalize().to_string()),
                    amount: discount.amount_minor,
                })
                .collect(),
            other_cadences: self
                .others
                .into_iter()
                .map(|(cadence, amount)| {
                    Ok::<_, Status>(RecurringAmount {
                        amount: to_minor(amount, self.precision)?,
                        cadence: cadence_to_proto(cadence),
                    })
                })
                .collect::<Result<_, _>>()?,
            usage_rates: self.usage.build(&self.currency, units),
            currency: self.currency,
            cadence: cadence_to_proto(self.cadence),
            included_credits: self.credits,
            usage_based: self.usage_based,
            minimum,
        })
    }
}

fn resolve_unit(
    units: &CreditUnitsById,
    id: Option<common_domain::ids::CreditUnitId>,
) -> Result<Option<&CreditUnit>, Status> {
    id.map(|id| {
        units
            .get(&id)
            .ok_or_else(|| Status::internal(format!("credit unit {id} is not loaded")))
    })
    .transpose()
}

/// The subscription's own price: its components as they stand, seats in use and its minimum.
pub fn subscription_plan_price(
    currency: &str,
    period: BillingPeriodEnum,
    inputs: &PortalPlanInputs,
    units: &CreditUnitsById,
) -> Result<PlanPrice, Status> {
    let mut builder = Builder::new(currency, period.as_subscription_billing_period())?;
    for component in &inputs.components {
        let cadence = component.period;
        match &component.fee {
            SubscriptionFee::Rate { rate } => builder.add(cadence, *rate),
            SubscriptionFee::Recurring { rate, quantity, .. } => {
                builder.add(cadence, rate * Decimal::from(*quantity))
            }
            SubscriptionFee::Slot {
                unit,
                unit_rate,
                initial_slots,
                ..
            } => {
                let seats = inputs.slots.get(unit).copied().unwrap_or(*initial_slots);
                builder.add(cadence, unit_rate * Decimal::from(seats));
            }
            SubscriptionFee::Capacity { rate, .. } => {
                builder.usage_based = true;
                builder.add(cadence, *rate);
            }
            // A pack granted once at start is not part of the recurring price, like a one-time fee.
            SubscriptionFee::Credit { once: true, .. } => {}
            SubscriptionFee::Credit {
                rate,
                quantity,
                credit_amount,
                expiry,
                credit_unit,
                rollover,
                once: false,
            } => builder.add_pack(
                cadence,
                Pack {
                    price: rate * Decimal::from(*quantity),
                    credit: credit_amount * Decimal::from(*quantity),
                    unit: resolve_unit(units, credit_unit.as_ref().map(|unit| unit.unit_id))?,
                    expiry: *expiry,
                    rollover: rollover.as_ref(),
                },
            )?,
            SubscriptionFee::Usage {
                metric_id,
                model,
                credit_unit,
            } => {
                builder.usage_based = true;
                builder.usage.add(
                    *metric_id,
                    inputs.metrics.get(metric_id),
                    model,
                    credit_unit.as_ref().map(|unit| unit.unit_id),
                );
            }
            SubscriptionFee::OneTime { .. } => {}
        }
    }
    builder.build(inputs.minimum_minor, &inputs.coupons, units)
}

/// The metric a plan price's usage is measured by, from its product.
struct PlanUsage<'a> {
    metric: Option<(BillableMetricId, Option<&'a BillableMetric>)>,
    credit_unit: Option<CreditUnitId>,
}

fn add_plan_pricing(
    builder: &mut Builder,
    cadence: SubscriptionFeeBillingPeriod,
    pricing: &Pricing,
    usage: PlanUsage,
    units: &CreditUnitsById,
) -> Result<(), Status> {
    match pricing {
        Pricing::Rate { rate } => builder.add(cadence, *rate),
        Pricing::Slot {
            unit_rate,
            min_slots,
            ..
        } => builder.add(
            cadence,
            unit_rate * Decimal::from(min_slots.unwrap_or(1).max(1)),
        ),
        Pricing::Capacity { rate, .. } => {
            builder.usage_based = true;
            builder.add(cadence, *rate);
        }
        Pricing::ExtraRecurring {
            unit_price,
            quantity,
        } => builder.add(cadence, unit_price * Decimal::from(*quantity)),
        Pricing::Credit { once: true, .. } => {}
        Pricing::Credit {
            unit_price,
            quantity,
            credit_amount,
            expiry,
            credit_unit_id,
            rollover,
            once: false,
        } => builder.add_pack(
            cadence,
            Pack {
                price: unit_price * Decimal::from(*quantity),
                credit: credit_amount * Decimal::from(*quantity),
                unit: resolve_unit(units, *credit_unit_id)?,
                expiry: *expiry,
                rollover: rollover.as_ref(),
            },
        )?,
        Pricing::Usage(model) => {
            builder.usage_based = true;
            if let Some((metric_id, metric)) = usage.metric {
                builder
                    .usage
                    .add(metric_id, metric, model, usage.credit_unit);
            } else {
                // A usage price without a metric cannot be put on a common unit.
                builder.usage.unit_sizes.insert(0);
            }
        }
        Pricing::OneTime { .. } => {}
    }
    Ok(())
}

/// A plan's price in `currency` on `cadence`, for a customer considering it. A component without a
/// price on `cadence` counts at the cadence it has. Seats count at their minimum.
pub fn plan_price(
    plan: &FullPlan,
    currency: &str,
    cadence: BillingPeriodEnum,
    units: &CreditUnitsById,
    metrics: &HashMap<BillableMetricId, BillableMetric>,
) -> Result<PlanPrice, Status> {
    let mut builder = Builder::new(currency, cadence.as_subscription_billing_period())?;
    for component in &plan.price_components {
        let metric_id =
            component
                .product_id
                .and_then(|id| plan.products.get(&id))
                .and_then(|product| match &product.fee_structure {
                    FeeStructure::Usage { metric_id, .. }
                    | FeeStructure::Capacity { metric_id } => Some(*metric_id),
                    _ => None,
                })
                .or_else(|| {
                    component.legacy_pricing.as_ref().and_then(|legacy| {
                        match &legacy.fee_structure {
                            FeeStructure::Usage { metric_id, .. }
                            | FeeStructure::Capacity { metric_id } => Some(*metric_id),
                            _ => None,
                        }
                    })
                });
        let metric = metric_id.map(|id| (id, metrics.get(&id)));
        let prices = component
            .prices
            .iter()
            .filter(|price| price.currency.eq_ignore_ascii_case(currency))
            .collect::<Vec<_>>();
        if let Some(price) = prices
            .iter()
            .find(|price| price.cadence == cadence)
            .or_else(|| prices.first())
        {
            add_plan_pricing(
                &mut builder,
                price.cadence.as_subscription_billing_period(),
                &price.pricing,
                PlanUsage {
                    metric,
                    credit_unit: price.credit_unit.as_ref().map(|unit| unit.unit_id),
                },
                units,
            )?;
            continue;
        }
        if let Some(legacy) = &component.legacy_pricing
            && legacy.currency.eq_ignore_ascii_case(currency)
            && let Some((entry_cadence, pricing)) = legacy
                .pricing_entries
                .iter()
                .find(|(entry_cadence, _)| *entry_cadence == cadence)
                .or_else(|| legacy.pricing_entries.first())
        {
            add_plan_pricing(
                &mut builder,
                entry_cadence.as_subscription_billing_period(),
                pricing,
                PlanUsage {
                    metric,
                    credit_unit: None,
                },
                units,
            )?;
        }
    }
    builder.build(None, &[], units)
}

/// The metrics a plan's usage prices are measured by.
pub fn plan_metric_ids(plan: &FullPlan) -> Vec<BillableMetricId> {
    plan.products
        .values()
        .filter_map(|product| match &product.fee_structure {
            FeeStructure::Usage { metric_id, .. } | FeeStructure::Capacity { metric_id } => {
                Some(*metric_id)
            }
            _ => None,
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

/// The credit units a subscription's price refers to: what its packs fund, what usage is priced in.
pub fn plan_credit_unit_ids(inputs: &PortalPlanInputs) -> Vec<CreditUnitId> {
    inputs
        .components
        .iter()
        .filter_map(|component| match &component.fee {
            SubscriptionFee::Credit {
                credit_unit: Some(unit),
                ..
            } => Some(unit.unit_id),
            SubscriptionFee::Usage {
                credit_unit: Some(unit),
                ..
            }
            | SubscriptionFee::Capacity {
                credit_unit: Some(unit),
                ..
            } => Some(unit.unit_id),
            _ => None,
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}
