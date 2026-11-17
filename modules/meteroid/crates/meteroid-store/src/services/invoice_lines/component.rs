use billing_core::pricing;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use rust_decimal::Decimal;

use crate::domain::credit_units::FeeCreditUnit;
use crate::domain::enums::{BillingType, UnitConversionRoundingEnum};
use crate::domain::invoice_lines::{LineCreditUnit, UsageBreakdown, UsageBreakdownItem};
use crate::domain::price_components::MatrixRow;
use crate::domain::{
    BillableMetric, ComponentPeriods, LineItem, Period, SubLineItem, SubscriptionDetails,
    SubscriptionFee, SubscriptionFeeInterface, UsagePeriod, UsagePricingModel,
};
use crate::services::subscriptions::proration::DayFraction;

use super::fees::pricing_error;
use crate::StoreResult;
use crate::errors::StoreError;
use crate::repositories::subscriptions::SubscriptionSlotsInterface;
use crate::services::Services;
use crate::services::clients::usage::{GroupedUsageData, UsageData};
use crate::store::PgConn;
use common_domain::ids::{BillableMetricId, SubscriptionAddOnId, SubscriptionPriceComponentId};
use common_utils::decimals::{MAX_QUANTITY_SCALE, round_quantity};
use error_stack::{Report, ResultExt};
use std::collections::{HashMap, HashSet};

/// Key to match existing line items during invoice refresh
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExistingLineKey {
    pub metric_id: BillableMetricId,
    pub sub_component_id: Option<SubscriptionPriceComponentId>,
    pub sub_add_on_id: Option<SubscriptionAddOnId>,
    pub group_by_dimensions: Option<HashMap<String, String>>,
}

impl std::hash::Hash for ExistingLineKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.metric_id.hash(state);
        self.sub_component_id.hash(state);
        self.sub_add_on_id.hash(state);
        // For HashMap, we need to hash in a deterministic order
        if let Some(ref dims) = self.group_by_dimensions {
            let mut sorted_dims: Vec<_> = dims.iter().collect();
            sorted_dims.sort_by_key(|(k, _)| *k);
            for (k, v) in sorted_dims {
                k.hash(state);
                v.hash(state);
            }
        }
    }
}

impl ExistingLineKey {
    pub fn from_line_item(line: &LineItem) -> Option<Self> {
        Some(Self {
            metric_id: line.metric_id?,
            sub_component_id: line.sub_component_id,
            sub_add_on_id: line.sub_add_on_id,
            group_by_dimensions: line.group_by_dimensions.clone(),
        })
    }
}

/// The unit price an operator pinned on a draft line, in the denomination the fee now prices in:
/// a pin never crosses from money to credits or between units.
pub(crate) fn pinned_unit_price(
    line: &LineItem,
    credit_unit: Option<&FeeCreditUnit>,
) -> Option<Decimal> {
    match (&line.credit_unit, credit_unit) {
        (None, None) => line.unit_price,
        (Some(block), Some(unit)) if block.unit_id == unit.unit_id => block.unit_price,
        _ => None,
    }
}

/// Whether a line bills metered usage of a subscription component or add-on.
pub(crate) fn is_usage_based_line(line: &LineItem) -> bool {
    line.metric_id.is_some() && (line.sub_component_id.is_some() || line.sub_add_on_id.is_some())
}

/// The unit prices an operator pinned on a draft's usage lines, by the line they sit on.
#[derive(Debug, Clone, Default)]
pub struct PinnedPrices(HashMap<ExistingLineKey, Decimal>);

impl PinnedPrices {
    /// `tax_inclusive` drafts store net lines, priced in the tax-inclusive figures they were quoted in.
    /// A line priced in a credit unit pins its price in credits, which bear no tax.
    pub fn add_lines(&mut self, lines: &[LineItem], tax_inclusive: bool) {
        for line in lines.iter().filter(|line| is_usage_based_line(line)) {
            if let Some(credit) = &line.credit_unit {
                if let (Some(key), Some(price)) =
                    (ExistingLineKey::from_line_item(line), credit.unit_price)
                {
                    self.0.insert(key, price);
                }
                continue;
            }
            let line = if tax_inclusive {
                line.clone().into_tax_inclusive_quote()
            } else {
                line.clone()
            };
            if let (Some(key), Some(price)) =
                (ExistingLineKey::from_line_item(&line), line.unit_price)
            {
                self.0.insert(key, price);
            }
        }
    }

    /// Adds `other`'s prices, which win where both pin a line.
    pub(crate) fn extend(&mut self, other: &PinnedPrices) {
        self.0
            .extend(other.0.iter().map(|(key, price)| (key.clone(), *price)));
    }

    pub(crate) fn unit_price(
        &self,
        metric_id: BillableMetricId,
        sub_component_id: Option<SubscriptionPriceComponentId>,
        sub_add_on_id: Option<SubscriptionAddOnId>,
        dimensions: &HashMap<String, String>,
    ) -> Option<Decimal> {
        self.0
            .get(&ExistingLineKey {
                metric_id,
                sub_component_id,
                sub_add_on_id,
                group_by_dimensions: Some(dimensions.clone()),
            })
            .copied()
    }
}

impl Services {
    pub(super) async fn compute_component<T: SubscriptionFeeInterface>(
        &self,
        conn: &mut PgConn,
        subscription_details: &SubscriptionDetails,
        component: &T,
        periods: ComponentPeriods,
        precision: u8,
        existing_lines: &HashMap<ExistingLineKey, &LineItem>,
    ) -> StoreResult<Vec<LineItem>> {
        let is_first_period = periods.arrear.is_none();

        let mut lines: Vec<InvoiceLineInner> = vec![];

        let instances = component.instance_quantity();
        // A pack tied to the billing period covers the part of a partial first period it is
        // billed for, fee and credit alike; any other pack is bought whole.
        let pack_share =
            if crate::services::subscriptions::proration::is_period_tied(component.fee_ref()) {
                periods.proration_days.map(|(partial, full)| {
                    crate::services::subscriptions::proration::DayFraction::new(partial, full)
                })
            } else {
                None
            };
        match component.fee_ref() {
            SubscriptionFee::Rate { rate } => {
                if let Some(advance_period) = periods.advance {
                    lines.push(InvoiceLineInner::simple_prorated(
                        rate,
                        &instances,
                        advance_period,
                        periods.proration_factor,
                        precision,
                        None,
                    )?);
                }
            }
            SubscriptionFee::OneTime { rate, quantity } => {
                // only for first period
                if is_first_period && let Some(advance_period) = periods.advance {
                    lines.push(InvoiceLineInner::simple_prorated(
                        rate,
                        &(Decimal::from(*quantity) * instances),
                        advance_period,
                        periods.proration_factor,
                        precision,
                        None,
                    )?);
                }
            }
            SubscriptionFee::Credit { rate, quantity, .. } => {
                if let Some(advance_period) = periods.advance {
                    let units = Decimal::from(*quantity) * instances;
                    lines.push(InvoiceLineInner::from_priced(
                        pricing::credit_pack(rate, units, pack_share, precision)
                            .map_err(pricing_error)?,
                        advance_period,
                        None,
                    ));
                }
            }
            SubscriptionFee::Recurring {
                rate,
                quantity,
                billing_type,
            } => match billing_type {
                BillingType::Advance => {
                    if let Some(advance_period) = periods.advance {
                        lines.push(InvoiceLineInner::simple_prorated(
                            rate,
                            &(Decimal::from(*quantity) * instances),
                            advance_period,
                            periods.proration_factor,
                            precision,
                            None,
                        )?);
                    }
                }
                BillingType::Arrears => {
                    // Skip a zero-width settled period (same-day terminal): a fixed arrears fee has
                    // no elapsed days to bill, and a EUR 0 line would flip an otherwise-empty
                    // invoice into a numbered facture instead of Closing.
                    if let Some(arrears) = periods.arrear
                        && arrears.start < arrears.end
                    {
                        lines.push(InvoiceLineInner::simple_prorated(
                            rate,
                            &(Decimal::from(*quantity) * instances),
                            arrears,
                            periods.arrear_proration_factor,
                            precision,
                            None,
                        )?);
                    }
                }
            },
            SubscriptionFee::Slot {
                unit_rate,
                min_slots,
                max_slots,
                unit,
                ..
            } => {
                if let Some(advance_period) = periods.advance {
                    // The seats held when the period starts: a renewal catching up past periods
                    // bills each with its own.
                    let held = self
                        .fetch_slots(
                            conn,
                            &advance_period.start,
                            unit.clone(),
                            subscription_details,
                        )
                        .await?;
                    let slots = pricing::billed_slots(held, *min_slots, *max_slots);

                    lines.push(InvoiceLineInner::simple_prorated(
                        unit_rate,
                        &Decimal::from(slots),
                        advance_period,
                        periods.proration_factor,
                        precision,
                        None,
                    )?);
                }
            }
            SubscriptionFee::Capacity {
                rate,
                included,
                overage_rate,
                metric_id,
                credit_unit,
            } => {
                if let Some(advance_period) = periods.advance {
                    lines.push(InvoiceLineInner::simple_prorated(
                        rate,
                        &instances,
                        advance_period,
                        None, // no proration on capacity, as it provides a fixed amount
                        precision,
                        None,
                    )?);
                }

                if let Some(arrear_period) = periods.arrear
                    && overage_rate > &Decimal::ZERO
                {
                    let usage = self
                        .fetch_usage(arrear_period.clone(), *metric_id, subscription_details)
                        .await?;

                    lines.extend(capacity_overage_lines(
                        *metric_id,
                        Decimal::from(*included),
                        overage_rate,
                        &usage.data,
                        &arrear_period,
                        credit_unit
                            .as_ref()
                            .map_or(precision, |unit| unit.precision),
                    )?);
                }
            }
            SubscriptionFee::Usage {
                metric_id,
                model,
                credit_unit,
            } => {
                if let Some(arrear_period) = periods.arrear {
                    let usage = self
                        .fetch_usage(arrear_period.clone(), *metric_id, subscription_details)
                        .await?;

                    let mut metered = usage_lines(
                        model,
                        *metric_id,
                        &usage.data,
                        &arrear_period,
                        credit_unit
                            .as_ref()
                            .map_or(precision, |unit| unit.precision),
                        |dimensions| {
                            let lookup_key = ExistingLineKey {
                                metric_id: *metric_id,
                                sub_component_id: component.sub_component_id(),
                                sub_add_on_id: component.sub_add_on_id(),
                                group_by_dimensions: Some(dimensions.clone()),
                            };
                            existing_lines.get(&lookup_key).and_then(|existing_line| {
                                pinned_unit_price(existing_line, credit_unit.as_ref())
                            })
                        },
                    )?;
                    let matrix_rates = match model {
                        UsagePricingModel::Matrix { rates } => Some(rates.as_slice()),
                        _ => None,
                    };
                    self.breakdown_usage(
                        &mut metered,
                        arrear_period,
                        *metric_id,
                        matrix_rates,
                        subscription_details,
                    )
                    .await?;
                    lines.extend(metered);
                }
            }
        }
        // The fee variant in scope is the authoritative recurring/usage/one-time
        // discriminator; product_id is not (recurring fees also carry one).
        let charge_type = component.fee_ref().charge_type();
        let fee_credit_unit = match component.fee_ref() {
            SubscriptionFee::Usage { credit_unit, .. }
            | SubscriptionFee::Capacity { credit_unit, .. } => credit_unit.as_ref(),
            _ => None,
        };
        // A pack line carries the credit it sells, frozen apart from its charge so the grant is
        // exactly what was sold whatever the catalog becomes. A pack granting a credit unit is
        // counted in that unit and taxed when sold: its credit is never redeemed as money.
        let credit_grant_unit = match component.fee_ref() {
            SubscriptionFee::Credit { credit_unit, .. } => credit_unit.clone(),
            _ => None,
        };
        let credit_precision = credit_grant_unit
            .as_ref()
            .map_or(precision, |unit| unit.precision);
        let (kind, credit_grant_amount, credit_grant_expiry, credit_grant_included) =
            match component.fee_ref() {
                SubscriptionFee::Credit {
                    rate,
                    quantity,
                    credit_amount,
                    expiry,
                    ..
                } => (
                    crate::domain::LineItemKind::Credit,
                    Some({
                        let full = pack_credit_minor(
                            *credit_amount,
                            Decimal::from(*quantity) * instances,
                            credit_precision,
                        )?;
                        pack_share.map_or(full, |share| share.prorate(full))
                    }),
                    *expiry,
                    rate.is_zero(),
                ),
                _ => (crate::domain::LineItemKind::Regular, None, None, false),
            };
        let credit_grant_rollover = match component.fee_ref() {
            SubscriptionFee::Credit {
                rate,
                quantity,
                rollover: Some(rollover),
                ..
            } if rate.is_zero() => Some(pack_rollover(
                rollover,
                Decimal::from(*quantity) * instances,
                credit_precision,
            )?),
            _ => None,
        };
        let taxed_at_purchase = credit_grant_unit.is_some();
        let product_id = crate::services::subscriptions::utils::fee_tax_product(
            component.fee_ref(),
            component.product_id(),
        );

        lines
            .into_iter()
            .filter(|line| !is_zero_quantity_line(line.quantity))
            .map(|line| {
                let base_name = line
                    .custom_line_name
                    .unwrap_or_else(|| component.name_ref().clone());

                let name = if let Some(ref dimensions) = line.group_by_dimensions {
                    if dimensions.is_empty() {
                        base_name
                    } else {
                        // Sort dimensions by key for consistent ordering
                        let mut sorted_dimensions: Vec<_> = dimensions.iter().collect();
                        sorted_dimensions.sort_by_key(|(k, _)| *k);
                        let dimension_values: Vec<String> = sorted_dimensions
                            .into_iter()
                            .map(|(_, v)| v.clone())
                            .collect();
                        format!("{} ({})", base_name, dimension_values.join(", "))
                    }
                } else {
                    base_name
                };

                // Look up existing line to preserve user edits (name, description, tax_rate)
                let lookup_key = line.metric_id.map(|metric_id| ExistingLineKey {
                    metric_id,
                    sub_component_id: component.sub_component_id(),
                    sub_add_on_id: component.sub_add_on_id(),
                    group_by_dimensions: line.group_by_dimensions.clone(),
                });

                let existing_line = lookup_key.as_ref().and_then(|key| existing_lines.get(key));

                // Usage priced in a credit unit: its price lives in the credit block, and the
                // money fields carry what the unit's wallet does not cover (all of it until
                // `cover_credit_unit_lines` runs).
                let credit_unit = match fee_credit_unit {
                    Some(unit) if line.metric_id.is_some() => Some(LineCreditUnit {
                        unit_id: unit.unit_id,
                        code: unit.code.clone(),
                        precision: unit.precision,
                        rate: unit.rate,
                        price_rate: Some(unit.rate),
                        unit_price: line.unit_price,
                        sub_lines: line.sublines.clone(),
                        consumed: i64::try_from(line.total)
                            .map_err(|_| Report::new(StoreError::InvalidDecimal))?,
                        covered: 0,
                    }),
                    _ => None,
                };
                let (amount, unit_price, sub_lines) = match &credit_unit {
                    Some(block) => (
                        super::credit_units::overage_money(block, precision)?,
                        None,
                        Vec::new(),
                    ),
                    None => (line.total as i64, line.unit_price, line.sublines),
                };

                // Usage read past the end date's midnight (an immediate cancellation's last day)
                // is dated by that instant.
                let usage_ends_at = line
                    .metric_id
                    .map(|_| {
                        usage_window(subscription_details, line.period.clone())
                            .end
                            .and_utc()
                    })
                    .filter(|end| *end != line.period.end.and_time(NaiveTime::MIN).and_utc());
                Ok(LineItem {
                    local_id: existing_line
                        .map(|el| el.local_id.clone())
                        .unwrap_or_default(),
                    name: existing_line.map(|el| el.name.clone()).unwrap_or(name),
                    charge_type: Some(charge_type),
                    quantity: line.quantity,
                    unit_price,
                    start_date: line.period.start,
                    end_date: line.period.end,
                    sub_lines,
                    is_prorated: line.is_prorated,
                    price_component_id: component.price_component_id(),
                    sub_component_id: component.sub_component_id(),
                    sub_add_on_id: component.sub_add_on_id(),
                    product_id,
                    metric_id: line.metric_id,
                    description: existing_line.and_then(|el| el.description.clone()),
                    group_by_dimensions: line.group_by_dimensions,

                    amount_subtotal: amount,

                    // tax & discount are handled later
                    tax_rate: existing_line.map(|el| el.tax_rate).unwrap_or(Decimal::ZERO),
                    tax_amount: 0,
                    tax_details: vec![],
                    taxable_amount: amount,
                    amount_total: amount,
                    kind,
                    credit_grant_amount,
                    credit_grant_expiry,
                    credit_grant_included,
                    taxed_at_purchase,
                    tax_inclusive_quote: None,
                    credit_unit,
                    credit_grant_unit: credit_grant_unit.clone(),
                    credit_grant_rollover,
                    promotional_credit: None,
                    subscription_id: Some(subscription_details.subscription.id),
                    usage_ends_at,
                    usage_breakdown: line.usage_breakdown,
                })
            })
            .collect::<StoreResult<Vec<LineItem>>>()
            .map(|lines| {
                let lineages = super::charges::Lineages::of(subscription_details);
                lines
                    .into_iter()
                    .map(|line| super::charges::identified(line, &lineages))
                    .collect()
            })
    }

    async fn fetch_usage(
        &self,
        period: Period,
        metric_id: BillableMetricId,
        subscription_details: &SubscriptionDetails,
    ) -> StoreResult<UsageData> {
        let metric = subscription_metric(subscription_details, metric_id)?;
        let usage = self
            .read_usage(period, metric, None, subscription_details)
            .await?;
        Ok(convert_usage_units(metric, usage))
    }

    /// Breaks the metric's usage `lines` down by its breakdown key, read apart from the usage they
    /// are priced on so that no aggregation is ever recombined across the key's values.
    async fn breakdown_usage(
        &self,
        lines: &mut [InvoiceLineInner],
        period: Period,
        metric_id: BillableMetricId,
        matrix_rates: Option<&[MatrixRow]>,
        subscription_details: &SubscriptionDetails,
    ) -> StoreResult<()> {
        let metric = subscription_metric(subscription_details, metric_id)?;
        let Some(breakdown_key) = metric.usage_breakdown_key.as_deref() else {
            return Ok(());
        };
        if lines.is_empty() {
            return Ok(());
        }
        // The breakdown bills nothing: an invoice it cannot be read for goes out without it.
        match self
            .read_usage(period, metric, Some(breakdown_key), subscription_details)
            .await
        {
            Ok(usage) => breakdown_lines(lines, metric, breakdown_key, matrix_rates, &usage.data),
            Err(error) => log::warn!(
                "Billing metric {} without its usage breakdown: {error:?}",
                metric.id
            ),
        }
        Ok(())
    }

    /// The metric's usage over the window the period bills, before unit conversion.
    async fn read_usage(
        &self,
        period: Period,
        metric: &BillableMetric,
        breakdown_by: Option<&str>,
        subscription_details: &SubscriptionDetails,
    ) -> StoreResult<UsageData> {
        let usage_period = usage_window(subscription_details, period);

        // A settled period with no usable window has nothing to query, and the metering client
        // rejects an empty window (invalid period). This covers a date-effective same-day cancel or
        // same-day pause (no instant cutoff, so the window stays midnight-to-midnight) and an
        // immediate cancel landing exactly at midnight.
        if usage_period.end <= usage_period.start {
            return Ok(UsageData {
                data: vec![],
                period: usage_period,
            });
        }

        let tenant_id = &subscription_details.subscription.tenant_id;
        let customer_id = &subscription_details.subscription.customer_id;
        match breakdown_by {
            None => {
                self.usage_client
                    .fetch_usage(tenant_id, customer_id, metric, usage_period)
                    .await
            }
            Some(key) => {
                self.usage_client
                    .fetch_usage_grouped_by(tenant_id, customer_id, metric, key, usage_period)
                    .await
            }
        }
    }

    async fn fetch_slots(
        &self,
        conn: &mut PgConn,
        invoice_date: &NaiveDate,
        unit: String,
        subscription_details: &SubscriptionDetails,
    ) -> StoreResult<u64> {
        let timestamp = invoice_date.clone().and_time(NaiveTime::MIN);

        let quantity = self
            .store
            .get_active_slots_value_with_conn(
                conn,
                subscription_details.subscription.tenant_id,
                subscription_details.subscription.id,
                unit.clone(),
                Some(timestamp),
            )
            .await?;

        Ok(u64::from(quantity))
    }
}

fn subscription_metric(
    subscription_details: &SubscriptionDetails,
    metric_id: BillableMetricId,
) -> StoreResult<&BillableMetric> {
    subscription_details
        .metrics
        .iter()
        .find(|metric| metric.id == metric_id)
        .ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(format!(
                "metric with id {metric_id}"
            )))
        })
}

/// Values of a breakdown key an invoice line lists one by one; the rest are summed as others.
const MAX_BREAKDOWN_ITEMS: usize = 100;

/// Attaches to each line the usage of `usage`, read grouped by `breakdown_key` too, that the line
/// priced. A matrix line weighs each item by its cells' prices; any other line by its quantity.
pub(crate) fn breakdown_lines(
    lines: &mut [InvoiceLineInner],
    metric: &BillableMetric,
    breakdown_key: &str,
    matrix_rates: Option<&[MatrixRow]>,
    usage: &[GroupedUsageData],
) {
    for line in lines.iter_mut() {
        if let Some(breakdown) = line_breakdown(
            line.group_by_dimensions.as_ref(),
            metric,
            breakdown_key,
            matrix_rates,
            usage,
        ) {
            line.usage_breakdown = Some(breakdown);
        }
    }
}

/// The breakdown of the one line of group `dimensions` (see [`breakdown_lines`]); none when no
/// usage of its group was read.
pub(crate) fn line_breakdown(
    dimensions: Option<&HashMap<String, String>>,
    metric: &BillableMetric,
    breakdown_key: &str,
    matrix_rates: Option<&[MatrixRow]>,
    usage: &[GroupedUsageData],
) -> Option<UsageBreakdown> {
    let matrix_keys = matrix_dimension_keys(matrix_rates.unwrap_or_default());
    let line_dimensions = sorted_dimensions(
        dimensions
            .into_iter()
            .flatten()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    let mut totals: HashMap<Option<&str>, (Decimal, Decimal)> = HashMap::new();
    for row in usage {
        let quantity = unrounded_units(metric.unit_conversion_factor, row.value);
        if quantity <= Decimal::ZERO {
            continue;
        }
        let weight = match matrix_rates {
            Some(rates) => {
                match rates
                    .iter()
                    .find(|rate| matrix_cell_matches(rate, &row.dimensions))
                {
                    Some(rate) => quantity * rate.per_unit_price,
                    None => continue,
                }
            }
            None => quantity,
        };
        let row_dimensions = sorted_dimensions(
            row.dimensions
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .filter(|(key, _)| *key != breakdown_key && !matrix_keys.contains(key)),
        );
        if row_dimensions != line_dimensions {
            continue;
        }
        // A blank value counts as no value, so both land in the same item.
        let value = row
            .dimensions
            .get(breakdown_key)
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty());
        let total = totals.entry(value).or_default();
        total.0 += quantity;
        total.1 += weight;
    }
    if totals.is_empty() {
        return None;
    }
    let mut items = totals
        .into_iter()
        .map(|(value, (quantity, weight))| UsageBreakdownItem {
            value: value.map(str::to_owned),
            quantity,
            weight,
            others: false,
        })
        .collect::<Vec<_>>();
    items.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then_with(|| b.quantity.cmp(&a.quantity))
            .then_with(|| a.value.cmp(&b.value))
    });
    if items.len() > MAX_BREAKDOWN_ITEMS {
        let rest = items.split_off(MAX_BREAKDOWN_ITEMS - 1);
        items.push(UsageBreakdownItem {
            value: None,
            quantity: rest.iter().map(|item| item.quantity).sum(),
            weight: rest.iter().map(|item| item.weight).sum(),
            others: true,
        });
    }
    for item in &mut items {
        item.quantity = round_quantity(item.quantity, MAX_QUANTITY_SCALE);
    }
    Some(UsageBreakdown {
        key: breakdown_key.to_owned(),
        items,
    })
}

fn sorted_dimensions<'a>(
    dimensions: impl Iterator<Item = (&'a str, &'a str)>,
) -> Vec<(&'a str, &'a str)> {
    let mut dimensions = dimensions.collect::<Vec<_>>();
    dimensions.sort_unstable();
    dimensions
}

/// A metric's unit conversion, applied to each group's period total after aggregation and before
/// pricing. Every path that prices usage (invoices, previews, the live credit projection, usage
/// costs) goes through it so all of them round alike.
pub(crate) fn convert_usage_units(metric: &BillableMetric, usage: UsageData) -> UsageData {
    UsageData {
        period: usage.period,
        data: usage
            .data
            .into_iter()
            .map(|usage| GroupedUsageData {
                value: converted_units(
                    metric.unit_conversion_factor,
                    metric.unit_conversion_rounding.as_ref(),
                    usage.value,
                ),
                dimensions: usage.dimensions,
            })
            .collect(),
    }
}

pub(crate) fn converted_units(
    factor: Option<i32>,
    rounding: Option<&UnitConversionRoundingEnum>,
    units: Decimal,
) -> Decimal {
    pricing::converted_units(factor, rounding.map(Into::into), units)
}

pub(crate) fn unrounded_units(factor: Option<i32>, units: Decimal) -> Decimal {
    pricing::unrounded_units(factor, units)
}

/// Whether the metric's conversion rounds, which makes its price a step function of the usage.
pub(crate) fn rounds_converted_units(metric: &BillableMetric) -> bool {
    metric
        .unit_conversion_factor
        .is_some_and(|factor| factor != 0)
        && !matches!(
            metric.unit_conversion_rounding,
            None | Some(UnitConversionRoundingEnum::None)
        )
}

/// The properties a matrix prices by; a matrix line stands for the usage of all their values.
fn matrix_dimension_keys(rates: &[MatrixRow]) -> HashSet<&str> {
    rates
        .iter()
        .flat_map(|rate| {
            std::iter::once(rate.dimension1.key.as_str())
                .chain(rate.dimension2.as_ref().map(|d| d.key.as_str()))
        })
        .collect()
}

/// Whether usage carrying `dimensions` falls in the matrix cell `rate` prices.
pub(crate) fn matrix_cell_matches(rate: &MatrixRow, dimensions: &HashMap<String, String>) -> bool {
    pricing::cell_matches(
        (&rate.dimension1.key, &rate.dimension1.value),
        rate.dimension2
            .as_ref()
            .map(|dimension2| (dimension2.key.as_str(), dimension2.value.as_str())),
        dimensions,
    )
}

/// The lines a usage fee bills for the usage of one window; shared by the invoice and the live
/// credit projection so both price identically. `unit_price_override` is the per-group unit price
/// an operator pinned on a draft line.
pub(crate) fn usage_lines(
    model: &UsagePricingModel,
    metric_id: BillableMetricId,
    usage: &[GroupedUsageData],
    arrear_period: &Period,
    precision: u8,
    unit_price_override: impl Fn(&HashMap<String, String>) -> Option<Decimal>,
) -> StoreResult<Vec<InvoiceLineInner>> {
    let lines = pricing::usage_lines(&model.into(), usage, precision, unit_price_override)
        .map_err(pricing_error)
        .attach("Failed to convert price_total to subunit")?;
    Ok(lines
        .into_iter()
        .map(|line| InvoiceLineInner::from_priced(line, arrear_period.clone(), Some(metric_id)))
        .collect())
}

/// The overage lines of a capacity fee for the usage of one window.
pub(crate) fn capacity_overage_lines(
    metric_id: BillableMetricId,
    included: Decimal,
    overage_rate: &Decimal,
    usage: &[GroupedUsageData],
    arrear_period: &Period,
    precision: u8,
) -> StoreResult<Vec<InvoiceLineInner>> {
    let lines = pricing::capacity_overage_lines(included, overage_rate, usage, precision)
        .map_err(pricing_error)
        .attach("Capacity overage exceeds the supported range")?;
    Ok(lines
        .into_iter()
        .map(|line| InvoiceLineInner::from_priced(line, arrear_period.clone(), Some(metric_id)))
        .collect())
}

pub struct InvoiceLineInner {
    pub total: u64,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>,
    pub period: Period,
    pub custom_line_name: Option<String>,
    pub is_prorated: bool,
    pub sublines: Vec<SubLineItem>,
    pub metric_id: Option<BillableMetricId>,
    pub group_by_dimensions: Option<HashMap<String, String>>,
    pub usage_breakdown: Option<UsageBreakdown>,
}

impl InvoiceLineInner {
    fn from_priced(
        line: pricing::PricedLine,
        period: Period,
        metric_id: Option<BillableMetricId>,
    ) -> InvoiceLineInner {
        InvoiceLineInner {
            total: line.total,
            quantity: line.quantity,
            unit_price: line.unit_price,
            period,
            custom_line_name: None,
            is_prorated: line.is_prorated,
            sublines: line.sublines.into_iter().map(Into::into).collect(),
            metric_id,
            group_by_dimensions: line.dimensions,
            usage_breakdown: None,
        }
    }

    pub fn simple_prorated(
        rate: &Decimal,
        quantity: &Decimal,
        period: Period,
        proration_factor: Option<DayFraction>,
        precision: u8,
        metric_id: Option<BillableMetricId>,
    ) -> StoreResult<InvoiceLineInner> {
        let line = pricing::simple_prorated(rate, quantity, proration_factor, precision)
            .map_err(pricing_error)
            .attach("Failed to convert price_total to subunit")?;
        Ok(Self::from_priced(line, period, metric_id))
    }
}

/// A fixed-fee line with an explicit quantity of zero bills nothing and only clutters
/// the invoice (e.g. an add-on configured with 0 units), so drop it — mirroring the
/// zero-usage skip already applied to metered lines. Lines without an explicit quantity
/// (usage/matrix/package) carry `None` here and are kept; they guard against zero upstream.
fn is_zero_quantity_line(quantity: Option<Decimal>) -> bool {
    quantity.is_some_and(|q| q.is_zero())
}

/// The usage query window for an arrear `period`.
///
/// The date-granular `Period -> UsagePeriod` conversion truncates the exclusive upper bound to the
/// period-end midnight. For a terminated subscription's final period that drops usage consumed on
/// the termination day — there is no later period to bill it — so the bound is extended to the exact
/// termination instant. Only ever extends the window (never shrinks it), and only when the arrear
/// window ends on the termination day itself, so an ongoing subscription, an earlier period, or a
/// date-effective cancel (whose `terminated_at` is that date's midnight) is left unchanged.
fn arrear_usage_period(
    period: Period,
    is_completed: bool,
    terminated_at: Option<NaiveDateTime>,
) -> UsagePeriod {
    let period_end = period.end;
    let mut usage_period: UsagePeriod = period.into();
    if is_completed
        && let Some(terminated_at) = terminated_at
        && terminated_at.date() == period_end
        && terminated_at > usage_period.end
    {
        usage_period.end = terminated_at;
    }
    usage_period
}

pub(crate) fn usage_window(
    subscription_details: &SubscriptionDetails,
    period: Period,
) -> UsagePeriod {
    let sub = &subscription_details.subscription;
    let is_completed = sub.current_period_end.is_none() && !sub.pending_checkout;
    arrear_usage_period(period, is_completed, sub.terminated_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }
    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn arrear_window_extends_to_termination_instant_same_day() {
        // Start and immediate-cancel on the same day: the date window [D, D) is empty; the terminal
        // cutoff extends the upper bound to the actual instant so the day's usage is billed.
        let d = day(2025, 3, 10);
        let terminated_at = at(2025, 3, 10, 14, 37);
        let up = arrear_usage_period(Period { start: d, end: d }, true, Some(terminated_at));
        assert_eq!(up.start, d.and_time(NaiveTime::MIN));
        assert_eq!(up.end, terminated_at);
        assert!(
            up.end > up.start,
            "window must be non-empty so usage is queried"
        );
    }

    #[test]
    fn arrear_window_extends_only_the_termination_day_for_multi_day_period() {
        let up = arrear_usage_period(
            Period {
                start: day(2025, 3, 1),
                end: day(2025, 3, 10),
            },
            true,
            Some(at(2025, 3, 10, 9, 0)),
        );
        assert_eq!(up.start, day(2025, 3, 1).and_time(NaiveTime::MIN));
        assert_eq!(up.end, at(2025, 3, 10, 9, 0));
    }

    #[test]
    fn arrear_window_unchanged_for_active_subscription() {
        // Not completed: midnight-exclusive bound preserved even if a cutoff were somehow present.
        let up = arrear_usage_period(
            Period {
                start: day(2025, 3, 1),
                end: day(2025, 3, 31),
            },
            false,
            Some(at(2025, 3, 31, 12, 0)),
        );
        assert_eq!(up.end, day(2025, 3, 31).and_time(NaiveTime::MIN));
    }

    #[test]
    fn arrear_window_unchanged_for_date_effective_cancel_at_midnight() {
        // Date-effective cancel records terminated_at at the date's midnight: no extension, so the
        // exclusive boundary is preserved (regression guard for scheduled / end-of-period cancels).
        let end = day(2025, 3, 15);
        let up = arrear_usage_period(
            Period {
                start: day(2025, 3, 1),
                end,
            },
            true,
            Some(end.and_time(NaiveTime::MIN)),
        );
        assert_eq!(up.end, end.and_time(NaiveTime::MIN));
    }

    #[test]
    fn arrear_window_unchanged_when_cutoff_is_on_a_different_day() {
        // A cutoff that does not fall on the arrear window's end day must not move the window.
        let up = arrear_usage_period(
            Period {
                start: day(2025, 3, 1),
                end: day(2025, 3, 31),
            },
            true,
            Some(at(2025, 3, 20, 10, 0)),
        );
        assert_eq!(up.end, day(2025, 3, 31).and_time(NaiveTime::MIN));
    }

    #[test]
    fn zero_quantity_lines_are_dropped() {
        assert!(is_zero_quantity_line(Some(Decimal::ZERO)));
        assert!(is_zero_quantity_line(Some(dec!(0.00))));
        assert!(!is_zero_quantity_line(Some(dec!(1))));
        // Usage/matrix/package lines carry no explicit quantity and must be kept.
        assert!(!is_zero_quantity_line(None));
    }
}

/// Credit a pack sells, in whole minor units of the invoice currency. A fractional minor unit is
/// refused rather than rounded: the grant must be exactly what was sold.
pub(crate) fn pack_credit_minor(
    credit_amount: Decimal,
    units: Decimal,
    precision: u8,
) -> StoreResult<i64> {
    pricing::pack_credit_minor(credit_amount, units, precision).map_err(pricing_error)
}

/// The rollover an included pack line freezes: the cap is per pack, like the credit it grants.
pub(crate) fn pack_rollover(
    rollover: &crate::domain::price_components::CreditRollover,
    units: Decimal,
    precision: u8,
) -> StoreResult<crate::domain::credit::GrantRollover> {
    Ok(crate::domain::credit::GrantRollover {
        max_minor: rollover
            .max_amount
            .map(|max| pack_credit_minor(max, units, precision))
            .transpose()?,
        expiry: rollover.expiry,
    })
}

#[cfg(test)]
mod pack_credit_tests {
    use super::{pack_credit_minor, pack_rollover};
    use crate::domain::price_components::{CreditExpiry, CreditRollover};
    use rust_decimal_macros::dec;

    #[test]
    fn an_included_pack_freezes_its_rollover_cap_per_pack_in_minor_units() {
        let rollover = CreditRollover {
            max_amount: Some(dec!(200)),
            expiry: CreditExpiry::Month(1),
        };
        let frozen = pack_rollover(&rollover, dec!(3), 2).unwrap();
        assert_eq!(frozen.max_minor, Some(60_000));
        assert_eq!(frozen.expiry, CreditExpiry::Month(1));
        let uncapped = CreditRollover {
            max_amount: None,
            ..rollover
        };
        assert_eq!(
            pack_rollover(&uncapped, dec!(3), 2).unwrap().max_minor,
            None
        );
    }

    #[test]
    fn a_fractional_grant_is_refused() {
        assert!(pack_credit_minor(dec!(0.005), dec!(1), 2).is_err());
    }
}

#[cfg(test)]
mod credit_unit_pin_tests {
    use super::*;
    use crate::domain::LineItemKind;
    use common_domain::ids::{BaseId, CreditUnitId};
    use rust_decimal_macros::dec;

    fn usage_line(
        metric_id: BillableMetricId,
        sub_component_id: SubscriptionPriceComponentId,
        unit_price: Option<Decimal>,
        credit_unit: Option<LineCreditUnit>,
    ) -> LineItem {
        let day = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        LineItem {
            local_id: "l".to_owned(),
            name: "API calls".to_owned(),
            charge_type: None,
            amount_subtotal: 0,
            tax_rate: dec!(0.2),
            taxable_amount: 0,
            tax_amount: 0,
            amount_total: 0,
            tax_details: Vec::new(),
            quantity: Some(dec!(10)),
            unit_price,
            start_date: day,
            end_date: day,
            sub_lines: Vec::new(),
            is_prorated: false,
            price_component_id: None,
            sub_component_id: Some(sub_component_id),
            sub_add_on_id: None,
            product_id: None,
            metric_id: Some(metric_id),
            description: None,
            group_by_dimensions: Some(HashMap::new()),
            kind: LineItemKind::Regular,
            credit_grant_amount: None,
            credit_grant_expiry: None,
            credit_grant_included: false,
            taxed_at_purchase: false,
            tax_inclusive_quote: None,
            credit_unit,
            credit_grant_unit: None,
            credit_grant_rollover: None,
            promotional_credit: None,
            subscription_id: None,
            usage_ends_at: None,
            usage_breakdown: None,
        }
    }

    fn block(unit_id: CreditUnitId) -> LineCreditUnit {
        LineCreditUnit {
            unit_id,
            code: "CREDITS".to_owned(),
            precision: 2,
            rate: dec!(0.01),
            price_rate: None,
            unit_price: Some(dec!(3)),
            sub_lines: Vec::new(),
            consumed: 3_000,
            covered: 1_000,
        }
    }

    fn fee_unit(unit_id: CreditUnitId) -> FeeCreditUnit {
        FeeCreditUnit {
            unit_id,
            code: "CREDITS".to_owned(),
            precision: 2,
            rate: dec!(0.01),
        }
    }

    #[test]
    fn a_credit_priced_line_pins_its_price_in_credits_untaxed() {
        let (metric, component, unit) = (
            BillableMetricId::new(),
            SubscriptionPriceComponentId::new(),
            CreditUnitId::new(),
        );
        let line = usage_line(metric, component, None, Some(block(unit)));
        let mut pinned = PinnedPrices::default();
        pinned.add_lines(std::slice::from_ref(&line), true);
        assert_eq!(
            pinned.unit_price(metric, Some(component), None, &HashMap::new()),
            Some(dec!(3)),
            "the price in credits, never grossed up by the line's tax rate"
        );
    }

    #[test]
    fn a_pin_never_crosses_from_money_to_credits_or_between_units() {
        let (metric, component, unit) = (
            BillableMetricId::new(),
            SubscriptionPriceComponentId::new(),
            CreditUnitId::new(),
        );
        let money = usage_line(metric, component, Some(dec!(0.5)), None);
        let credits = usage_line(metric, component, None, Some(block(unit)));
        assert_eq!(pinned_unit_price(&money, None), Some(dec!(0.5)));
        assert_eq!(pinned_unit_price(&money, Some(&fee_unit(unit))), None);
        assert_eq!(pinned_unit_price(&credits, None), None);
        assert_eq!(
            pinned_unit_price(&credits, Some(&fee_unit(unit))),
            Some(dec!(3))
        );
        assert_eq!(
            pinned_unit_price(&credits, Some(&fee_unit(CreditUnitId::new()))),
            None
        );
    }
}

#[cfg(test)]
mod unit_conversion_tests {
    use super::*;
    use crate::domain::enums::BillingMetricAggregateEnum;
    use crate::services::credit::billing_windows::{PlannedWindow, price_window_groups};
    use crate::services::invoice_lines::invoice_lines::MeteredFee;
    use common_domain::ids::{BaseId, ProductFamilyId, SubscriptionId, TenantId};
    use rust_decimal_macros::dec;

    fn tokens(rounding: UnitConversionRoundingEnum) -> BillableMetric {
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

    #[test]
    fn invoice_and_live_projection_price_rounded_usage_alike() {
        let start = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        let period = Period { start, end };
        let model = UsagePricingModel::PerUnit { rate: dec!(12) };
        let raw = || {
            vec![
                GroupedUsageData {
                    value: dec!(9_004_601),
                    dimensions: HashMap::from([("model".to_owned(), "standard".to_owned())]),
                },
                GroupedUsageData {
                    value: dec!(250_000),
                    dimensions: HashMap::from([("model".to_owned(), "lite".to_owned())]),
                },
            ]
        };

        for (rounding, expected_minor) in [
            (UnitConversionRoundingEnum::Up, Some(13_200)),
            (UnitConversionRoundingEnum::Down, Some(10_800)),
            (UnitConversionRoundingEnum::Nearest, Some(10_800)),
            (UnitConversionRoundingEnum::None, None),
        ] {
            let metric = tokens(rounding);
            let invoiced = convert_usage_units(
                &metric,
                UsageData {
                    data: raw(),
                    period: UsagePeriod {
                        start: start.and_time(NaiveTime::MIN),
                        end: end.and_time(NaiveTime::MIN),
                    },
                },
            );
            let invoice_minor: i64 =
                usage_lines(&model, metric.id, &invoiced.data, &period, 2, |_| None)
                    .unwrap()
                    .iter()
                    .map(|line| line.total as i64)
                    .sum();

            let window = PlannedWindow {
                subscription_id: SubscriptionId::new(),
                metric: metric.clone(),
                fee: MeteredFee::Usage(model.clone()),
                sub_component_id: None,
                sub_add_on_id: None,
                product_id: None,
                tax_inclusive: false,
                taxes: Vec::new(),
                period: period.clone(),
                start: start.and_time(NaiveTime::MIN),
                end: end.and_time(NaiveTime::MIN),
                currency: "USD".to_owned(),
                precision: 2,
                overage: None,
            };
            let projected_minor = price_window_groups(
                &window,
                end.and_time(NaiveTime::MIN),
                raw(),
                &PinnedPrices::default(),
            )
            .unwrap();

            if let Some(expected) = expected_minor {
                assert_eq!(invoice_minor, expected);
            }
            assert_eq!(projected_minor, invoice_minor);
        }
    }

    fn breakdown_metric() -> BillableMetric {
        BillableMetric {
            unit_conversion_factor: None,
            unit_conversion_rounding: None,
            usage_breakdown_key: Some("project".to_owned()),
            ..tokens(UnitConversionRoundingEnum::None)
        }
    }

    fn row(value: Decimal, dimensions: &[(&str, &str)]) -> GroupedUsageData {
        GroupedUsageData {
            value,
            dimensions: dimensions
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    fn june() -> Period {
        Period {
            start: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        }
    }

    fn items(line: &InvoiceLineInner) -> Vec<(Option<&str>, Decimal, i64)> {
        let breakdown = line.usage_breakdown.as_ref().expect("a breakdown line");
        breakdown
            .items
            .iter()
            .zip(breakdown.allocate(line.total as i64))
            .map(|(item, amount)| (item.value.as_deref(), item.quantity, amount))
            .collect()
    }

    #[test]
    fn a_breakdown_shares_the_package_price_of_the_whole_usage() {
        let metric = breakdown_metric();
        let model = UsagePricingModel::Package {
            block_size: 100,
            rate: dec!(10),
        };
        let mut lines = usage_lines(
            &model,
            metric.id,
            &[row(dec!(150), &[])],
            &june(),
            2,
            |_| None,
        )
        .unwrap();
        breakdown_lines(
            &mut lines,
            &metric,
            "project",
            None,
            &[
                row(dec!(120), &[("project", "alpha")]),
                row(dec!(30), &[("project", "beta")]),
            ],
        );

        // Billed apart, 120 and 30 would take three packages; together they take two.
        assert_eq!(lines[0].total, 2_000);
        assert_eq!(
            items(&lines[0]),
            vec![
                (Some("alpha"), dec!(120), 1_600),
                (Some("beta"), dec!(30), 400)
            ]
        );
    }

    #[test]
    fn a_blank_value_counts_as_no_value() {
        let metric = breakdown_metric();
        let model = UsagePricingModel::PerUnit { rate: dec!(1) };
        let mut lines = usage_lines(&model, metric.id, &[row(dec!(10), &[])], &june(), 2, |_| {
            None
        })
        .unwrap();
        breakdown_lines(
            &mut lines,
            &metric,
            "project",
            None,
            &[
                row(dec!(6), &[("project", "alpha")]),
                row(dec!(2), &[]),
                row(dec!(1), &[("project", "")]),
                row(dec!(1), &[("project", "  ")]),
            ],
        );

        assert_eq!(
            items(&lines[0]),
            vec![(Some("alpha"), dec!(6), 600), (None, dec!(4), 400)]
        );
    }

    #[test]
    fn a_breakdown_breaks_down_each_group_line_on_its_own() {
        let metric = BillableMetric {
            usage_group_key: Some("region".to_owned()),
            ..breakdown_metric()
        };
        let model = UsagePricingModel::PerUnit { rate: dec!(1) };
        let mut lines = usage_lines(
            &model,
            metric.id,
            &[
                row(dec!(10), &[("region", "eu")]),
                row(dec!(5), &[("region", "us")]),
            ],
            &june(),
            2,
            |_| None,
        )
        .unwrap();
        breakdown_lines(
            &mut lines,
            &metric,
            "project",
            None,
            &[
                row(dec!(7), &[("region", "eu"), ("project", "alpha")]),
                row(dec!(3), &[("region", "eu")]),
                row(dec!(5), &[("region", "us"), ("project", "alpha")]),
            ],
        );

        let line = |region: &str| {
            lines
                .iter()
                .find(|line| line.group_by_dimensions.as_ref().unwrap()["region"] == region)
                .unwrap()
        };
        assert_eq!(
            items(line("eu")),
            vec![(Some("alpha"), dec!(7), 700), (None, dec!(3), 300)]
        );
        assert_eq!(items(line("us")), vec![(Some("alpha"), dec!(5), 500)]);
    }

    #[test]
    fn a_matrix_breakdown_weighs_each_value_by_its_cells_prices() {
        let metric = breakdown_metric();
        let cell = |model: &str, price: Decimal| MatrixRow {
            dimension1: crate::domain::price_components::MatrixDimension {
                key: "model".to_owned(),
                value: model.to_owned(),
            },
            dimension2: None,
            per_unit_price: price,
        };
        let rates = vec![cell("large", dec!(10)), cell("small", dec!(1))];
        let model = UsagePricingModel::Matrix {
            rates: rates.clone(),
        };
        let mut lines = usage_lines(
            &model,
            metric.id,
            &[
                row(dec!(1), &[("model", "large")]),
                row(dec!(10), &[("model", "small")]),
            ],
            &june(),
            2,
            |_| None,
        )
        .unwrap();
        breakdown_lines(
            &mut lines,
            &metric,
            "project",
            Some(&rates),
            &[
                row(dec!(1), &[("model", "large"), ("project", "alpha")]),
                row(dec!(10), &[("model", "small"), ("project", "beta")]),
            ],
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].total, 2_000);
        let mut by_value = items(&lines[0]);
        by_value.sort();
        assert_eq!(
            by_value,
            vec![
                (Some("alpha"), dec!(1), 1_000),
                (Some("beta"), dec!(10), 1_000)
            ]
        );
    }

    #[test]
    fn a_breakdown_past_its_listed_values_sums_the_rest_as_others() {
        let metric = breakdown_metric();
        let model = UsagePricingModel::PerUnit { rate: dec!(1) };
        let mut lines = usage_lines(
            &model,
            metric.id,
            &[row(dec!(150), &[])],
            &june(),
            2,
            |_| None,
        )
        .unwrap();
        let values = (0..150).map(|i| format!("p{i:03}")).collect::<Vec<_>>();
        let usage = values
            .iter()
            .map(|value| row(Decimal::ONE, &[("project", value.as_str())]))
            .collect::<Vec<_>>();
        breakdown_lines(&mut lines, &metric, "project", None, &usage);

        let breakdown = lines[0].usage_breakdown.as_ref().unwrap();
        assert_eq!(breakdown.items.len(), MAX_BREAKDOWN_ITEMS);
        let others = breakdown.items.last().unwrap();
        assert!(others.others && others.value.is_none());
        assert_eq!(others.quantity, dec!(51));
        assert_eq!(breakdown.allocate(15_000).iter().sum::<i64>(), 15_000);
    }

    #[test]
    fn breakdown_quantities_are_converted_without_rounding() {
        let metric = BillableMetric {
            usage_breakdown_key: Some("project".to_owned()),
            ..tokens(UnitConversionRoundingEnum::Up)
        };
        let model = UsagePricingModel::PerUnit { rate: dec!(12) };
        let usage = convert_usage_units(
            &metric,
            UsageData {
                data: vec![row(dec!(1_500_000), &[])],
                period: UsagePeriod {
                    start: june().start.and_time(NaiveTime::MIN),
                    end: june().end.and_time(NaiveTime::MIN),
                },
            },
        );
        let mut lines = usage_lines(&model, metric.id, &usage.data, &june(), 2, |_| None).unwrap();
        breakdown_lines(
            &mut lines,
            &metric,
            "project",
            None,
            &[
                row(dec!(1_000_000), &[("project", "alpha")]),
                row(dec!(500_000), &[("project", "beta")]),
            ],
        );

        // Two million tokens billed, rounded up from 1.5M: the items keep the usage they carry.
        assert_eq!(lines[0].total, 2_400);
        assert_eq!(
            items(&lines[0]),
            vec![
                (Some("alpha"), dec!(1), 1_600),
                (Some("beta"), dec!(0.5), 800)
            ]
        );
    }
}
