use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::domain::credit_units::FeeCreditUnit;
use crate::domain::enums::{BillingType, UnitConversionRoundingEnum};
use crate::domain::invoice_lines::LineCreditUnit;
use crate::domain::price_components::MatrixRow;
use crate::domain::{
    BillableMetric, ComponentPeriods, LineItem, Period, SubLineAttributes, SubLineItem,
    SubscriptionDetails, SubscriptionFee, SubscriptionFeeInterface, UsagePeriod, UsagePricingModel,
};
use crate::services::subscriptions::proration::DayFraction;
use crate::utils::local_id::LocalId;

use super::fees;
use crate::StoreResult;
use crate::errors::StoreError;
use crate::repositories::subscriptions::SubscriptionSlotsInterface;
use crate::services::Services;
use crate::services::clients::usage::{GroupedUsageData, UsageData};
use crate::store::PgConn;
use common_domain::ids::{BillableMetricId, SubscriptionAddOnId, SubscriptionPriceComponentId};
use common_utils::decimals::{ToSubunit, round_billable_quantity, round_quantity};
use common_utils::integers::{ToNonNegativeU64, only_positive, only_positive_decimal};
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
fn pinned_unit_price(line: &LineItem, credit_unit: Option<&FeeCreditUnit>) -> Option<Decimal> {
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
                    let full = (rate * units)
                        .to_subunit_opt(precision)
                        .ok_or(Report::new(StoreError::InvalidDecimal))?;
                    let total = pack_share.map_or(full, |share| share.prorate(full));
                    let unit_price = match pack_share {
                        Some(_) if !units.is_zero() => {
                            Decimal::new(total, u32::from(precision)) / units
                        }
                        _ => *rate,
                    };
                    lines.push(InvoiceLineInner {
                        total: u64::try_from(total.max(0))
                            .map_err(|_| Report::new(StoreError::InvalidDecimal))?,
                        quantity: Some(units),
                        unit_price: Some(unit_price),
                        period: advance_period,
                        custom_line_name: None,
                        is_prorated: pack_share.is_some_and(|share| !share.is_whole()),
                        sublines: Vec::new(),
                        metric_id: None,
                        group_by_dimensions: None,
                    });
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
                    let slots = self
                        .fetch_slots(
                            conn,
                            &advance_period.start,
                            unit.clone(),
                            subscription_details,
                        )
                        .await?
                        .max(u64::from(min_slots.unwrap_or(0)))
                        .min(u64::from(max_slots.unwrap_or(u32::MAX)));

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
                        *included,
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

                    lines.extend(usage_lines(
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
                    )?);
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
        // counted in that unit and taxed when sold: its credit is never redeemed as money (D3).
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
                        .unwrap_or_else(LocalId::no_prefix),
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
                })
            })
            .collect()
    }

    async fn fetch_usage(
        &self,
        period: Period,
        metric_id: BillableMetricId,
        subscription_details: &SubscriptionDetails,
    ) -> StoreResult<UsageData> {
        let metric = subscription_details
            .metrics
            .iter()
            .find(|metric| metric.id == metric_id)
            .ok_or(StoreError::ValueNotFound(format!(
                "metric with id {metric_id}"
            )))?;

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

        let usage = self
            .usage_client
            .fetch_usage(
                &subscription_details.subscription.tenant_id,
                &subscription_details.subscription.customer_id,
                metric,
                usage_period,
            )
            .await?;

        Ok(convert_usage_units(metric, usage))
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
    // Rounding belongs to the conversion: without a factor the usage is billed as metered.
    if factor.is_none_or(|factor| factor == 0) {
        return units;
    }
    let converted = unrounded_units(factor, units);
    match rounding {
        None | Some(UnitConversionRoundingEnum::None) => converted,
        Some(UnitConversionRoundingEnum::Up) => converted.ceil(),
        Some(UnitConversionRoundingEnum::Down) => converted.floor(),
        Some(UnitConversionRoundingEnum::Nearest) => {
            converted.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        }
        Some(UnitConversionRoundingEnum::NearestHalf) => {
            (converted * Decimal::TWO)
                .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
                / Decimal::TWO
        }
        Some(UnitConversionRoundingEnum::NearestDecile) => {
            converted.round_dp_with_strategy(1, RoundingStrategy::MidpointAwayFromZero)
        }
    }
}

/// The conversion without the rounding, which applies to a period total only: for a share of it
/// (one event, one bucket), so the shares stay proportional to the usage they carry.
pub(crate) fn unrounded_units(factor: Option<i32>, units: Decimal) -> Decimal {
    match factor {
        Some(factor) if factor != 0 => units / Decimal::from(factor),
        _ => units,
    }
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

/// Whether usage carrying `dimensions` falls in the matrix cell `rate` prices.
pub(crate) fn matrix_cell_matches(rate: &MatrixRow, dimensions: &HashMap<String, String>) -> bool {
    dimensions.get(&rate.dimension1.key) == Some(&rate.dimension1.value)
        && rate
            .dimension2
            .as_ref()
            .is_none_or(|dimension2| dimensions.get(&dimension2.key) == Some(&dimension2.value))
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
    let mut lines: Vec<InvoiceLineInner> = vec![];
    match model {
        UsagePricingModel::Matrix { rates } => {
            // First, identify matrix dimension keys
            let mut matrix_dimension_keys = HashSet::new();
            for rate in rates {
                matrix_dimension_keys.insert(&rate.dimension1.key);
                if let Some(dimension2) = &rate.dimension2 {
                    matrix_dimension_keys.insert(&dimension2.key);
                }
            }

            // Group usage data by non-matrix dimensions (usage_group_key dimensions)
            let mut groups: HashMap<String, (HashMap<String, String>, Vec<&GroupedUsageData>)> =
                HashMap::new();

            for grouped_usage in usage {
                let group_dimensions: HashMap<String, String> = grouped_usage
                    .dimensions
                    .iter()
                    .filter(|(key, _)| !matrix_dimension_keys.contains(key))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();

                // Create a stable string key for grouping
                let mut sorted_group_keys: Vec<_> = group_dimensions.iter().collect();
                sorted_group_keys.sort_by_key(|(k, _)| *k);
                let group_key = sorted_group_keys
                    .iter()
                    .map(|(k, v)| format!("{k}:{v}"))
                    .collect::<Vec<_>>()
                    .join("|");

                groups
                    .entry(group_key)
                    .or_insert_with(|| (group_dimensions.clone(), Vec::new()))
                    .1
                    .push(grouped_usage);
            }

            // Create one line item per group with matrix sublines
            for (_, (group_dimensions, group_usage_data)) in groups {
                let mut sublines = vec![];

                for rate in rates {
                    // Find usage data that matches this matrix rate within this group
                    let matching_usage = group_usage_data
                        .iter()
                        .find(|usage_data| matrix_cell_matches(rate, &usage_data.dimensions));

                    if let Some(usage_data) = matching_usage {
                        let quantity = round_billable_quantity(
                            usage_data.value,
                            rate.per_unit_price,
                            precision,
                        );

                        if quantity > Decimal::ZERO {
                            let price_total = rate.per_unit_price * quantity;

                            let price_cents = only_positive(
                                price_total
                                    .to_subunit_opt(precision)
                                    .ok_or(Report::new(StoreError::InvalidDecimal))
                                    .attach("Failed to convert price_total to subunit")?,
                            );

                            if price_cents > 0 {
                                // we concat rate.dimension1.value and rate.dimension2.value (if defined), separated by a comma
                                let name = format!(
                                    "{}{}",
                                    rate.dimension1.value,
                                    rate.dimension2
                                        .as_ref()
                                        .map(|d| format!(",{}", d.value))
                                        .unwrap_or_default()
                                );
                                sublines.push(SubLineItem {
                                    local_id: LocalId::no_prefix(),
                                    name,
                                    total: price_cents as i64,
                                    quantity,
                                    unit_price: rate.per_unit_price,
                                    attributes: Some(SubLineAttributes::Matrix {
                                        dimension1_key: rate.dimension1.key.clone(),
                                        dimension1_value: rate.dimension1.value.clone(),
                                        dimension2_key: rate
                                            .dimension2
                                            .as_ref()
                                            .map(|d| d.key.clone()),
                                        dimension2_value: rate
                                            .dimension2
                                            .as_ref()
                                            .map(|d| d.value.clone()),
                                    }),
                                });
                            }
                        }
                    }
                }

                // Only create a line item if there are sublines for this group
                if !sublines.is_empty() {
                    let mut line = InvoiceLineInner::from_usage_sublines(
                        sublines,
                        arrear_period.clone(),
                        None,
                        metric_id,
                    )?;

                    line.group_by_dimensions = if group_dimensions.is_empty() {
                        None
                    } else {
                        Some(group_dimensions)
                    };

                    lines.push(line);
                }
            }
        }
        model => {
            // Round so aggregation float noise (e.g. a summed 5524.098500000001)
            // never reaches the invoice.
            let qty_scale = model.billable_quantity_scale(precision);

            // Handle grouped usage data - create separate line items for each group
            for grouped_usage in usage {
                let usage_units = round_quantity(grouped_usage.value, qty_scale);

                // Skip zero usage
                if usage_units <= Decimal::ZERO {
                    continue;
                }

                match model {
                    UsagePricingModel::PerUnit { rate } => {
                        let effective_rate =
                            unit_price_override(&grouped_usage.dimensions).unwrap_or(*rate);

                        let mut line = InvoiceLineInner::usage_simple(
                            &effective_rate,
                            &usage_units,
                            arrear_period.clone(),
                            precision,
                            metric_id,
                        )?;

                        // Store group_by dimensions for later use in line name generation
                        line.group_by_dimensions = Some(grouped_usage.dimensions.clone());

                        lines.push(line);
                    }
                    UsagePricingModel::Tiered { tiers, block_size } => {
                        let mut line = fees::compute_tier_price(
                            usage_units,
                            tiers,
                            arrear_period.clone(),
                            precision,
                            metric_id,
                            block_size,
                        )?;

                        line.group_by_dimensions = Some(grouped_usage.dimensions.clone());
                        lines.push(line);
                    }
                    UsagePricingModel::Volume { tiers, block_size } => {
                        let mut line = fees::compute_volume_price(
                            usage_units,
                            tiers,
                            arrear_period.clone(),
                            precision,
                            metric_id,
                            block_size,
                        )?;

                        line.group_by_dimensions = Some(grouped_usage.dimensions.clone());
                        lines.push(line);
                    }
                    UsagePricingModel::Package { block_size, rate } => {
                        // TODO we want some additional data in the frontend to display that "x$ per 20", total usage and block usage
                        let package_size_decimal = Decimal::from(*block_size);
                        let total_packages = (usage_units / package_size_decimal).ceil();

                        let price_total = total_packages * *rate;

                        let mut line = InvoiceLineInner::from_usage_sublines(
                            vec![SubLineItem {
                                local_id: LocalId::no_prefix(),
                                name: "Package".to_string(),
                                total: price_total
                                    .to_subunit_opt(precision)
                                    .ok_or(Report::new(StoreError::InvalidDecimal))
                                    .attach("Failed to convert price_total to subunit")?,
                                quantity: total_packages,
                                unit_price: *rate,
                                attributes: Some(SubLineAttributes::Package {
                                    raw_usage: usage_units,
                                }),
                            }],
                            arrear_period.clone(),
                            None,
                            metric_id,
                        )?;

                        line.group_by_dimensions = Some(grouped_usage.dimensions.clone());
                        lines.push(line);
                    }
                    UsagePricingModel::Matrix { .. } => unreachable!(),
                }
            }
        }
    }
    Ok(lines)
}

/// The overage lines of a capacity fee for the usage of one window.
pub(crate) fn capacity_overage_lines(
    metric_id: BillableMetricId,
    included: u64,
    overage_rate: &Decimal,
    usage: &[GroupedUsageData],
    arrear_period: &Period,
    precision: u8,
) -> StoreResult<Vec<InvoiceLineInner>> {
    let mut lines: Vec<InvoiceLineInner> = vec![];
    if overage_rate > &Decimal::ZERO {
        for usage_data in usage {
            let overage_units = usage_data
                .value
                .checked_sub(Decimal::from(included))
                .map(only_positive_decimal)
                .map(|units| round_billable_quantity(units, *overage_rate, precision))
                .ok_or(StoreError::InvalidDecimal)
                .attach("Capacity overage exceeds the supported range")?;

            // Multiplied before rounding, once, so a sub-minor-unit rate still bills.
            let overage_total = overage_rate
                .checked_mul(overage_units)
                .and_then(|total| {
                    total.checked_mul(Decimal::from(10_i64.pow(u32::from(precision))))
                })
                .and_then(|minor| {
                    minor
                        .round_dp_with_strategy(
                            0,
                            rust_decimal::RoundingStrategy::MidpointAwayFromZero,
                        )
                        .to_i64()
                })
                .ok_or(StoreError::InvalidDecimal)
                .attach("Capacity overage exceeds the supported range")?;

            lines.push(InvoiceLineInner {
                quantity: None,
                unit_price: None,
                total: overage_total.to_non_negative_u64(),
                period: arrear_period.clone(),
                is_prorated: false,
                custom_line_name: None,
                sublines: vec![SubLineItem {
                    local_id: LocalId::no_prefix(),
                    name: "Overage".to_string(),
                    total: overage_total,
                    quantity: overage_units,
                    unit_price: *overage_rate,
                    attributes: None,
                }],
                metric_id: Some(metric_id),
                group_by_dimensions: Some(usage_data.dimensions.clone()),
            });
        }
    }
    Ok(lines)
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
}

impl InvoiceLineInner {
    pub fn simple_prorated(
        rate: &Decimal,
        quantity: &Decimal,
        period: Period,
        proration_factor: Option<DayFraction>,
        precision: u8,
        metric_id: Option<BillableMetricId>,
    ) -> StoreResult<InvoiceLineInner> {
        let total = rate * quantity;

        let total_cents = prorate(
            total
                .to_subunit_opt(precision)
                .ok_or(Report::new(StoreError::InvalidDecimal))
                .attach("Failed to convert price_total to subunit")?,
            proration_factor,
        );

        // When prorated, derive unit_price from the rounded total so that
        // unit_price × quantity = total exactly.
        // When not prorated, use the original rate directly to avoid precision
        // artifacts from rounding total_cents then dividing back by quantity.
        let unit_price = if proration_factor.is_some() {
            if quantity.is_zero() {
                Decimal::ZERO
            } else {
                let divisor = Decimal::from(10u64.pow(precision as u32));
                Decimal::from(total_cents as i64) / divisor / quantity
            }
        } else {
            *rate
        };

        Ok(InvoiceLineInner {
            quantity: Some(*quantity),
            unit_price: Some(unit_price),
            total: total_cents,
            period,
            custom_line_name: None,
            is_prorated: proration_factor.is_some_and(|f| !f.is_whole()),
            sublines: Vec::new(),
            metric_id,
            group_by_dimensions: None,
        })
    }

    pub fn usage_simple(
        rate: &Decimal,
        quantity: &Decimal,
        period: Period,
        precision: u8,
        metric_id: BillableMetricId,
    ) -> StoreResult<InvoiceLineInner> {
        Self::simple_prorated(rate, quantity, period, None, precision, Some(metric_id))
    }

    pub fn from_usage_sublines(
        mut sublines: Vec<SubLineItem>,
        period: Period,
        proration_factor: Option<DayFraction>,
        metric_id: BillableMetricId,
    ) -> StoreResult<InvoiceLineInner> {
        let total = sublines.iter().map(|subline| subline.total).sum::<i64>();
        let total_cents = prorate(total, proration_factor);

        // When prorating, apply the same factor to each subline so the invariant
        // `line.total == Σ sublines.total` holds. Any rounding delta is absorbed
        // by the largest-magnitude subline (largest-remainder correction).
        if proration_factor.is_some_and(|f| !f.is_whole()) && !sublines.is_empty() {
            for subline in sublines.iter_mut() {
                subline.total = prorate(subline.total, proration_factor) as i64;
            }
            let current_sum: i64 = sublines.iter().map(|s| s.total).sum();
            let diff = total_cents as i64 - current_sum;
            if diff != 0
                && let Some((idx, _)) = sublines
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, s)| s.total.unsigned_abs())
            {
                sublines[idx].total += diff;
            }
        }

        Ok(InvoiceLineInner {
            quantity: None,
            unit_price: None,
            total: total_cents,
            period,
            custom_line_name: None,
            is_prorated: proration_factor.is_some_and(|f| !f.is_whole()),
            sublines,
            metric_id: Some(metric_id),
            group_by_dimensions: None,
        })
    }
}

/// A fixed-fee line with an explicit quantity of zero bills nothing and only clutters
/// the invoice (e.g. an add-on configured with 0 units), so drop it — mirroring the
/// zero-usage skip already applied to metered lines. Lines without an explicit quantity
/// (usage/matrix/package) carry `None` here and are kept; they guard against zero upstream.
fn is_zero_quantity_line(quantity: Option<Decimal>) -> bool {
    quantity.is_some_and(|q| q.is_zero())
}

/// Whole days over whole days, half-up once (OQ14): a true half-cent is never flipped.
fn prorate(price_cents: i64, proration_factor: Option<DayFraction>) -> u64 {
    only_positive(proration_factor.map_or(price_cents, |factor| factor.prorate(price_cents)))
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
    use common_domain::ids::BaseId;
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
    fn prorate_rounds_half_away_from_zero_without_float_drift() {
        assert_eq!(prorate(3100, Some(DayFraction::new(15, 31))), 1500);
        assert_eq!(prorate(3100, Some(DayFraction::new(10, 31))), 1000);
        assert_eq!(prorate(5, Some(DayFraction::new(1, 2))), 3); // 2.5 rounds away from zero
        // One day of a 28-day February on 0.70 is 2.5 cents: the f64 factor billed 2 (OQ14).
        assert_eq!(prorate(70, Some(DayFraction::new(1, 28))), 3);
        assert_eq!(prorate(2500, None), 2500);
        assert_eq!(prorate(-100, Some(DayFraction::new(1, 2))), 0); // negatives clamp to zero
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

    fn subline(name: &str, total: i64) -> SubLineItem {
        SubLineItem {
            local_id: name.to_string(),
            name: name.to_string(),
            total,
            quantity: dec!(1),
            unit_price: Decimal::from(total) / Decimal::from(100),
            attributes: None,
        }
    }

    fn period() -> Period {
        Period {
            start: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
        }
    }

    #[test]
    fn zero_quantity_lines_are_dropped() {
        assert!(is_zero_quantity_line(Some(Decimal::ZERO)));
        assert!(is_zero_quantity_line(Some(dec!(0.00))));
        assert!(!is_zero_quantity_line(Some(dec!(1))));
        // Usage/matrix/package lines carry no explicit quantity and must be kept.
        assert!(!is_zero_quantity_line(None));
    }

    #[test]
    fn usage_sublines_invariant_no_proration() {
        let sublines = vec![subline("a", 1_000), subline("b", 2_345), subline("c", 777)];
        let line = InvoiceLineInner::from_usage_sublines(
            sublines,
            period(),
            None,
            BillableMetricId::new(),
        )
        .unwrap();
        let sum: i64 = line.sublines.iter().map(|s| s.total).sum();
        assert_eq!(line.total as i64, sum);
    }

    #[test]
    fn usage_sublines_invariant_with_proration() {
        // 0.37 proration introduces per-subline rounding drift
        let sublines = vec![
            subline("a", 1_000),
            subline("b", 2_345),
            subline("c", 777),
            subline("d", 133),
        ];
        let line = InvoiceLineInner::from_usage_sublines(
            sublines,
            period(),
            Some(DayFraction::new(37, 100)),
            BillableMetricId::new(),
        )
        .unwrap();
        let sum: i64 = line.sublines.iter().map(|s| s.total).sum();
        assert_eq!(
            line.total as i64, sum,
            "line.total ({}) != Σ sublines.total ({})",
            line.total, sum
        );
    }

    #[test]
    fn usage_sublines_invariant_factor_one() {
        let sublines = vec![subline("a", 1_000), subline("b", 2_345)];
        let line = InvoiceLineInner::from_usage_sublines(
            sublines,
            period(),
            Some(DayFraction::WHOLE),
            BillableMetricId::new(),
        )
        .unwrap();
        let sum: i64 = line.sublines.iter().map(|s| s.total).sum();
        assert_eq!(line.total as i64, sum);
    }
}

/// Credit a pack sells, in whole minor units of the invoice currency. A fractional minor unit is
/// refused rather than rounded: the grant must be exactly what was sold.
pub(crate) fn pack_credit_minor(
    credit_amount: Decimal,
    units: Decimal,
    precision: u8,
) -> StoreResult<i64> {
    let minor = credit_amount
        .checked_mul(units)
        .and_then(|amount| amount.checked_mul(Decimal::from(10_i64.pow(u32::from(precision)))))
        .filter(|amount| amount.fract().is_zero() && !amount.is_sign_negative())
        .and_then(|amount| amount.to_i64())
        .ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "a credit pack must grant a non-negative amount of credit, with no more decimal places than its currency or credit unit allows".to_owned(),
            ))
        })?;
    Ok(minor)
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
    fn a_pack_grants_its_amount_per_unit_in_minor_units() {
        assert_eq!(pack_credit_minor(dec!(50), dec!(3), 2).unwrap(), 15_000);
        assert_eq!(pack_credit_minor(dec!(5000), dec!(1), 0).unwrap(), 5_000);
        assert_eq!(pack_credit_minor(dec!(1.234), dec!(2), 3).unwrap(), 2_468);
    }

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
    fn a_fractional_or_negative_grant_is_refused() {
        assert!(pack_credit_minor(dec!(0.005), dec!(1), 2).is_err());
        assert!(pack_credit_minor(dec!(-1), dec!(1), 2).is_err());
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
mod capacity_overage_tests {
    use super::*;
    use common_domain::ids::BaseId;
    use rust_decimal_macros::dec;

    fn overage_total(rate: Decimal, included: u64, used: Decimal, precision: u8) -> (i64, u64) {
        let period = Period {
            start: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
        };
        let usage = [GroupedUsageData {
            value: used,
            dimensions: HashMap::new(),
        }];
        let lines = capacity_overage_lines(
            BillableMetricId::new(),
            included,
            &rate,
            &usage,
            &period,
            precision,
        )
        .unwrap();
        assert_eq!(lines.len(), 1);
        (lines[0].sublines[0].total, lines[0].total)
    }

    #[test]
    fn capacity_overage_rounds_once_after_multiplying() {
        // 0.4 credits x 100 at precision 0: 40, where rounding the rate first billed 0.
        assert_eq!(overage_total(dec!(0.4), 0, dec!(100), 0), (40, 40));
        // 0.5 x 3 = 1.5 rounds half away to 2, where the rate rounded to 1 first billed 3.
        assert_eq!(overage_total(dec!(0.5), 10, dec!(13), 0), (2, 2));
        // A sub-cent money rate: $0.004 x 1000 is $4.00, not $0.
        assert_eq!(overage_total(dec!(0.004), 0, dec!(1000), 2), (400, 400));
        // Fractional overage is billed, not truncated: 10.5 x $0.02 = $0.21.
        assert_eq!(overage_total(dec!(0.02), 5, dec!(15.5), 2), (21, 21));
        assert_eq!(overage_total(dec!(0.02), 20, dec!(15.5), 2), (0, 0));
    }

    #[test]
    fn a_capacity_overage_beyond_the_supported_range_is_refused() {
        let period = Period {
            start: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
        };
        let usage = [GroupedUsageData {
            value: Decimal::MAX,
            dimensions: HashMap::new(),
        }];
        assert!(
            capacity_overage_lines(BillableMetricId::new(), 0, &dec!(10), &usage, &period, 2)
                .is_err()
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

    fn convert(rounding: UnitConversionRoundingEnum, units: Decimal) -> Decimal {
        converted_units(Some(1_000_000), Some(&rounding), units)
    }

    #[test]
    fn rounding_none_keeps_the_fraction() {
        assert_eq!(
            convert(UnitConversionRoundingEnum::None, dec!(9_004_601)),
            dec!(9.004601)
        );
        assert_eq!(
            converted_units(Some(1_000_000), None, dec!(9_004_601)),
            dec!(9.004601)
        );
    }

    #[test]
    fn rounding_up_takes_the_next_whole_unit() {
        assert_eq!(
            convert(UnitConversionRoundingEnum::Up, dec!(9_004_601)),
            dec!(10)
        );
        assert_eq!(
            convert(UnitConversionRoundingEnum::Up, dec!(9_000_000)),
            dec!(9)
        );
        assert_eq!(convert(UnitConversionRoundingEnum::Up, dec!(1)), dec!(1));
    }

    #[test]
    fn rounding_down_drops_the_fraction() {
        assert_eq!(
            convert(UnitConversionRoundingEnum::Down, dec!(9_999_999)),
            dec!(9)
        );
        assert_eq!(
            convert(UnitConversionRoundingEnum::Down, dec!(999_999)),
            dec!(0)
        );
    }

    #[test]
    fn rounding_nearest_rounds_halves_away_from_zero() {
        assert_eq!(
            convert(UnitConversionRoundingEnum::Nearest, dec!(9_499_999)),
            dec!(9)
        );
        assert_eq!(
            convert(UnitConversionRoundingEnum::Nearest, dec!(9_500_000)),
            dec!(10)
        );
        assert_eq!(
            convert(UnitConversionRoundingEnum::Nearest, dec!(8_500_000)),
            dec!(9)
        );
        assert_eq!(
            convert(UnitConversionRoundingEnum::Nearest, dec!(-2_500_000)),
            dec!(-3)
        );
    }

    #[test]
    fn rounding_to_halves_and_tenths() {
        assert_eq!(
            convert(UnitConversionRoundingEnum::NearestHalf, dec!(9_240_000)),
            dec!(9)
        );
        assert_eq!(
            convert(UnitConversionRoundingEnum::NearestHalf, dec!(9_250_000)),
            dec!(9.5)
        );
        assert_eq!(
            convert(UnitConversionRoundingEnum::NearestDecile, dec!(9_050_000)),
            dec!(9.1)
        );
    }

    #[test]
    fn no_or_zero_factor_leaves_usage_as_is() {
        let up = Some(&UnitConversionRoundingEnum::Up);
        assert_eq!(converted_units(None, up, dec!(1.5)), dec!(1.5));
        assert_eq!(converted_units(Some(0), up, dec!(1.5)), dec!(1.5));
    }

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
}
