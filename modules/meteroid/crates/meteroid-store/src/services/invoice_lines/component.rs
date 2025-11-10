use chrono::{NaiveDate, NaiveTime};
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal_macros::dec;

use crate::domain::enums::BillingType;
use crate::domain::{
    ComponentPeriods, LineItem, Period, SubLineAttributes, SubLineItem, SubscriptionDetails,
    SubscriptionFee, SubscriptionFeeInterface, UsagePricingModel,
};
use crate::utils::local_id::LocalId;

use super::fees;
use crate::StoreResult;
use crate::errors::StoreError;
use crate::repositories::subscriptions::SubscriptionSlotsInterface;
use crate::services::Services;
use crate::services::clients::usage::{GroupedUsageData, UsageData};
use crate::store::PgConn;
use common_domain::ids::{BillableMetricId, SubscriptionAddOnId, SubscriptionPriceComponentId};
use common_utils::decimals::ToSubunit;
use common_utils::integers::{ToNonNegativeU64, only_positive, only_positive_decimal};
use error_stack::{Report, ResultExt};
use std::collections::{HashMap, HashSet};

/// Key to match existing line items during invoice refresh
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExistingLineKey {
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

impl Services {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn compute_component<T: SubscriptionFeeInterface>(
        &self,
        conn: &mut PgConn,
        subscription_details: &SubscriptionDetails,
        component: &T,
        periods: ComponentPeriods,
        invoice_date: &NaiveDate,
        precision: u8,
        existing_lines: &HashMap<ExistingLineKey, &LineItem>,
    ) -> StoreResult<Vec<LineItem>> {
        let fixed_period = match periods.advance {
            Some(period) => period,
            None => return Ok(Vec::new()),
        };

        let is_first_period = periods.arrear.is_none();

        let mut lines: Vec<InvoiceLineInner> = vec![];

        match component.fee_ref() {
            SubscriptionFee::Rate { rate } => {
                lines.push(InvoiceLineInner::simple_prorated(
                    rate,
                    &dec!(1),
                    fixed_period,
                    periods.proration_factor,
                    precision,
                    None,
                )?);
            }
            SubscriptionFee::OneTime { rate, quantity } => {
                // only for first period
                if is_first_period {
                    lines.push(InvoiceLineInner::simple_prorated(
                        rate,
                        &Decimal::from(*quantity),
                        fixed_period,
                        periods.proration_factor,
                        precision,
                        None,
                    )?);
                }
            }
            SubscriptionFee::Recurring {
                rate,
                quantity,
                billing_type,
            } => match billing_type {
                BillingType::Advance => {
                    lines.push(InvoiceLineInner::simple_prorated(
                        rate,
                        &Decimal::from(*quantity),
                        fixed_period,
                        periods.proration_factor,
                        precision,
                        None,
                    )?);
                }
                BillingType::Arrears => {
                    if let Some(arrears) = periods.arrear {
                        lines.push(InvoiceLineInner::simple_prorated(
                            rate,
                            &Decimal::from(*quantity),
                            arrears,
                            periods.proration_factor,
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
                let slots = self
                    .fetch_slots(conn, invoice_date, unit.clone(), subscription_details) // TODO we need unit instead. That would allow for subscription components not linked to a plan. It'd also match Sequence model
                    .await?
                    .max(u64::from(min_slots.unwrap_or(0)))
                    .min(u64::from(max_slots.unwrap_or(u32::MAX)));

                lines.push(InvoiceLineInner::simple_prorated(
                    unit_rate,
                    &Decimal::from(slots),
                    fixed_period,
                    periods.proration_factor,
                    precision,
                    None,
                )?);
            }
            SubscriptionFee::Capacity {
                rate,
                included,
                overage_rate,
                metric_id,
            } => {
                lines.push(InvoiceLineInner::simple_prorated(
                    rate,
                    &dec!(1),
                    fixed_period,
                    None, // no proration on capacity, as it provides a fixed amount
                    precision,
                    None,
                )?);

                if let Some(arrear_period) = periods.arrear
                    && overage_rate > &Decimal::ZERO
                {
                    let usage = self
                        .fetch_usage(arrear_period.clone(), *metric_id, subscription_details)
                        .await?;

                    let overage_price = overage_rate
                        .to_subunit_opt(precision)
                        .ok_or(StoreError::InvalidDecimal)
                        .attach("Failed to convert overage_rate to subunit")?;

                    usage.data.iter().for_each(|usage_data| {
                        let overage_units = usage_data.value - Decimal::from(*included);

                        let overage_units = only_positive_decimal(overage_units);

                        let overage_total = overage_price * overage_units.to_i64().unwrap_or(0);

                        let overage_line = InvoiceLineInner {
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
                            metric_id: Some(*metric_id),
                            group_by_dimensions: Some(usage_data.dimensions.clone()), // meh, should we group in subline instead ? + improve name
                        };

                        lines.push(overage_line);
                    });
                }
            }
            SubscriptionFee::Usage { metric_id, model } => {
                if let Some(arrear_period) = periods.arrear {
                    let usage = self
                        .fetch_usage(arrear_period.clone(), *metric_id, subscription_details)
                        .await?;

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
                            let mut groups: HashMap<
                                String,
                                (HashMap<String, String>, Vec<&GroupedUsageData>),
                            > = HashMap::new();

                            for grouped_usage in &usage.data {
                                let group_dimensions: HashMap<String, String> = grouped_usage
                                    .dimensions
                                    .iter()
                                    .filter(|(key, _)| !matrix_dimension_keys.contains(key))
                                    .map(|(k, v)| (k.clone(), v.clone()))
                                    .collect();

                                // Create a stable string key for grouping
                                let mut sorted_group_keys: Vec<_> =
                                    group_dimensions.iter().collect();
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
                                    let matching_usage =
                                        group_usage_data.iter().find(|usage_data| {
                                            let d1_match =
                                                usage_data.dimensions.get(&rate.dimension1.key)
                                                    == Some(&rate.dimension1.value);

                                            let d2_match =
                                                if let Some(dimension2) = &rate.dimension2 {
                                                    usage_data.dimensions.get(&dimension2.key)
                                                        == Some(&dimension2.value)
                                                } else {
                                                    true
                                                };

                                            d1_match && d2_match
                                        });

                                    if let Some(usage_data) = matching_usage {
                                        let quantity = usage_data.value;

                                        if quantity > Decimal::ZERO {
                                            let price_total = rate.per_unit_price * quantity;

                                            let price_cents = only_positive(
                                                price_total
                                                    .to_subunit_opt(precision)
                                                    .ok_or(Report::new(StoreError::InvalidDecimal))
                                                    .attach(
                                                        "Failed to convert price_total to subunit",
                                                    )?,
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
                                                        dimension1_value: rate
                                                            .dimension1
                                                            .value
                                                            .clone(),
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
                                        *metric_id,
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
                            // Handle grouped usage data - create separate line items for each group
                            for grouped_usage in &usage.data {
                                let usage_units = grouped_usage.value;

                                // Skip zero usage
                                if usage_units <= Decimal::ZERO {
                                    continue;
                                }

                                match model {
                                    UsagePricingModel::PerUnit { rate } => {
                                        // Check if there's an existing line with a custom unit_price
                                        let lookup_key = ExistingLineKey {
                                            metric_id: *metric_id,
                                            sub_component_id: component.sub_component_id(),
                                            sub_add_on_id: component.sub_add_on_id(),
                                            group_by_dimensions: Some(
                                                grouped_usage.dimensions.clone(),
                                            ),
                                        };

                                        let effective_rate = existing_lines
                                            .get(&lookup_key)
                                            .and_then(|existing_line| existing_line.unit_price)
                                            .unwrap_or(*rate);

                                        let mut line = InvoiceLineInner::usage_simple(
                                            &effective_rate,
                                            &usage_units,
                                            arrear_period.clone(),
                                            precision,
                                            *metric_id,
                                        )?;

                                        // Store group_by dimensions for later use in line name generation
                                        line.group_by_dimensions =
                                            Some(grouped_usage.dimensions.clone());

                                        lines.push(line);
                                    }
                                    UsagePricingModel::Tiered { tiers, block_size } => {
                                        let mut line = fees::compute_tier_price(
                                            usage_units,
                                            tiers,
                                            arrear_period.clone(),
                                            precision,
                                            *metric_id,
                                            block_size,
                                        )?;

                                        line.group_by_dimensions =
                                            Some(grouped_usage.dimensions.clone());
                                        lines.push(line);
                                    }
                                    UsagePricingModel::Volume { tiers, block_size } => {
                                        let mut line = fees::compute_volume_price(
                                            usage_units,
                                            tiers,
                                            arrear_period.clone(),
                                            precision,
                                            *metric_id,
                                            block_size,
                                        )?;

                                        line.group_by_dimensions =
                                            Some(grouped_usage.dimensions.clone());
                                        lines.push(line);
                                    }
                                    UsagePricingModel::Package { block_size, rate } => {
                                        // TODO we want some additional data in the frontend to display that "x$ per 20", total usage and block usage
                                        let package_size_decimal = Decimal::from(*block_size);
                                        let total_packages =
                                            (usage_units / package_size_decimal).ceil();

                                        let price_total = total_packages * *rate;

                                        let mut line = InvoiceLineInner::from_usage_sublines(
                                            vec![SubLineItem {
                                                local_id: LocalId::no_prefix(),
                                                name: "Package".to_string(),
                                                total: price_total
                                                    .to_subunit_opt(precision)
                                                    .ok_or(Report::new(StoreError::InvalidDecimal))
                                                    .attach(
                                                        "Failed to convert price_total to subunit",
                                                    )?,
                                                quantity: total_packages,
                                                unit_price: *rate,
                                                attributes: Some(SubLineAttributes::Package {
                                                    raw_usage: usage_units,
                                                }),
                                            }],
                                            arrear_period.clone(),
                                            None,
                                            *metric_id,
                                        )?;

                                        line.group_by_dimensions =
                                            Some(grouped_usage.dimensions.clone());
                                        lines.push(line);
                                    }
                                    UsagePricingModel::Matrix { .. } => unreachable!(),
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(lines
            .into_iter()
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

                LineItem {
                    local_id: existing_line
                        .map(|el| el.local_id.clone())
                        .unwrap_or_else(LocalId::no_prefix),
                    name: existing_line.map(|el| el.name.clone()).unwrap_or(name),
                    quantity: line.quantity,
                    unit_price: line.unit_price,
                    start_date: line.period.start,
                    end_date: line.period.end,
                    sub_lines: line.sublines,
                    is_prorated: line.is_prorated,
                    price_component_id: component.price_component_id(),
                    sub_component_id: component.sub_component_id(),
                    sub_add_on_id: component.sub_add_on_id(),
                    product_id: component.product_id(),
                    metric_id: line.metric_id,
                    description: existing_line.and_then(|el| el.description.clone()),
                    group_by_dimensions: line.group_by_dimensions,

                    amount_subtotal: line.total as i64,

                    // tax & discount are handled later
                    tax_rate: existing_line.map(|el| el.tax_rate).unwrap_or(Decimal::ZERO),
                    tax_amount: 0,
                    tax_details: vec![],
                    taxable_amount: line.total as i64,
                    amount_total: line.total as i64,
                }
            })
            .collect())
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

        let usage = self
            .usage_client
            .fetch_usage(
                &subscription_details.subscription.tenant_id,
                &subscription_details.subscription.customer_id,
                metric,
                period,
            )
            .await?;

        if let Some(factor) = metric.unit_conversion_factor {
            if factor == 0 {
                return Ok(usage);
            }
            let usage = UsageData {
                period: usage.period,
                data: usage
                    .data
                    .iter()
                    .map(|usage| GroupedUsageData {
                        value: usage.value / Decimal::from_i32(factor).unwrap_or(dec!(1)),
                        dimensions: usage.dimensions.clone(),
                    })
                    .collect(),
            };
            return Ok(usage);
        }
        Ok(usage)
    }

    async fn fetch_slots(
        &self,
        conn: &mut PgConn,
        invoice_date: &NaiveDate,
        unit: String,
        subscription_details: &SubscriptionDetails,
    ) -> StoreResult<u64> {
        let quantity = self
            .store
            .get_current_slots_value_with_conn(
                conn,
                subscription_details.subscription.tenant_id,
                subscription_details.subscription.id,
                unit,
                Some(invoice_date.clone().and_time(NaiveTime::MIN)),
            )
            .await?;

        Ok(u64::from(quantity))
    }
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
        proration_factor: Option<f64>,
        precision: u8,
        metric_id: Option<BillableMetricId>,
    ) -> StoreResult<InvoiceLineInner> {
        let unit_price_cents = prorate_dec(*rate, proration_factor);

        let total = rate * quantity;

        let total_cents = prorate(
            total
                .to_subunit_opt(precision)
                .ok_or(Report::new(StoreError::InvalidDecimal))
                .attach("Failed to convert price_total to subunit")?,
            proration_factor,
        );

        Ok(InvoiceLineInner {
            quantity: Some(*quantity),
            unit_price: Some(unit_price_cents),
            total: total_cents,
            period,
            custom_line_name: None,
            is_prorated: proration_factor.is_some_and(|f| f < 1.0),
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
        sublines: Vec<SubLineItem>,
        period: Period,
        proration_factor: Option<f64>,
        metric_id: BillableMetricId,
    ) -> StoreResult<InvoiceLineInner> {
        let total = sublines.iter().map(|subline| subline.total).sum::<i64>();
        let total_cents = prorate(total, proration_factor);

        Ok(InvoiceLineInner {
            quantity: None,
            unit_price: None,
            total: total_cents,
            period,
            custom_line_name: None,
            is_prorated: proration_factor.is_some_and(|f| f < 1.0),
            sublines,
            metric_id: Some(metric_id),
            group_by_dimensions: None,
        })
    }
}

fn prorate(price_cents: i64, proration_factor: Option<f64>) -> u64 {
    match proration_factor {
        Some(proration_factor) => {
            let prorated_price = (price_cents as f64 * proration_factor).round() as i64;
            only_positive(prorated_price)
        }
        None => only_positive(price_cents),
    }
}

fn prorate_dec(price_cents: Decimal, proration_factor: Option<f64>) -> Decimal {
    match proration_factor {
        Some(proration_factor) => {
            let prorated_price =
                price_cents * Decimal::from_f64(proration_factor).unwrap_or(dec!(1.0));
            only_positive_decimal(prorated_price)
        }
        None => only_positive_decimal(price_cents),
    }
}
