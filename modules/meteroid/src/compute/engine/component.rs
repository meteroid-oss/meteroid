use std::sync::Arc;

use chrono::NaiveDate;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use uuid::Uuid;

use meteroid_store::domain::*;

use crate::compute::clients::slots::SlotClient;
use crate::compute::clients::usage::UsageData;
use crate::compute::engine::shared::{only_positive, ToCents};
use crate::models::{InvoiceLine, InvoiceLinePeriod};

use super::super::clients::usage::UsageClient;
use super::super::errors::ComputeError;
use super::super::{ComponentPeriods, Period};
use super::fees;

pub struct ComponentEngine {
    // metric_client: HashMap<Uuid, Metric>,
    usage_client: Arc<dyn UsageClient + Send + Sync>,
    slots_client: Arc<dyn SlotClient + Send + Sync>,
    subscription_details: Arc<SubscriptionDetails>,
}

impl ComponentEngine {
    pub fn new(
        usage_client: Arc<dyn UsageClient + Send + Sync>,
        slots_client: Arc<dyn SlotClient + Send + Sync>,
        subscription_details: Arc<SubscriptionDetails>,
    ) -> Self {
        Self {
            usage_client,
            slots_client,
            subscription_details,
        }
    }

    pub async fn compute_component(
        &self,
        component: SubscriptionComponent,
        periods: ComponentPeriods,
        invoice_date: &NaiveDate,
    ) -> Result<Vec<InvoiceLine>, ComputeError> {
        let mut lines = vec![];

        let fixed_period = periods.advance;
        let is_first_period = periods.arrear.is_none();

        match &component.fee {
            SubscriptionFee::Rate { rate } => {
                lines.push(to_invoice_line_inner(
                    rate,
                    &1,
                    fixed_period,
                    periods.proration_factor,
                )?);
            }
            SubscriptionFee::OneTime { rate, quantity } => {
                // only for first period
                if is_first_period {
                    lines.push(to_invoice_line_inner(
                        rate,
                        quantity,
                        None,
                        periods.proration_factor,
                    )?);
                }
            }
            SubscriptionFee::Recurring { rate, quantity, .. } => {
                lines.push(to_invoice_line_inner(
                    rate,
                    quantity,
                    fixed_period,
                    periods.proration_factor,
                )?);
            }
            SubscriptionFee::Slot {
                unit_rate,
                min_slots,
                max_slots,
                ..
            } => {
                let invoice_date = invoice_date;

                let slots = self
                    .fetch_slots(
                        invoice_date,
                        &component
                            .price_component_id
                            .ok_or(ComputeError::InternalError)?,
                    ) // TODO we need unit instead. That would allow for subscription components not linked to a plan
                    .await?
                    .max(min_slots.unwrap_or(0) as u64)
                    .min(max_slots.unwrap_or(u32::MAX) as u64);

                let unit_price = only_positive(unit_rate.to_cents()?);
                let total = unit_price * slots;

                let slot_line = InvoiceLineInner {
                    quantity: Some(slots),
                    unit_price: Some(unit_price),
                    total: total,
                    period: fixed_period,
                    custom_line_name: None,
                };

                lines.push(slot_line);
            }
            SubscriptionFee::Capacity {
                rate,
                included: _,
                overage_rate,
                metric_id,
            } => {
                let fixed_line = InvoiceLineInner {
                    quantity: Some(1),
                    unit_price: Some(only_positive(rate.to_cents()?)),
                    total: only_positive(rate.to_cents()?),
                    period: fixed_period,
                    custom_line_name: None,
                };
                lines.push(fixed_line);

                if let Some(arrear_period) = periods.arrear {
                    if overage_rate > &Decimal::ZERO {
                        let overage_units = self
                            .fetch_usage(arrear_period.clone(), *metric_id)
                            .await?
                            .single()?;

                        if overage_units > Decimal::ZERO {
                            let overage_price = overage_rate.to_cents()?;
                            let overage_total = overage_price * overage_units.to_i64().unwrap_or(0);

                            let overage_line = InvoiceLineInner {
                                quantity: overage_units.to_u64(),
                                unit_price: Some(overage_price as u64),
                                total: overage_total as u64,
                                period: Some(arrear_period),
                                custom_line_name: Some(format!("{} - Overage", &component.name)),
                            };

                            lines.push(overage_line);
                        }
                    }
                }
            }
            SubscriptionFee::Usage { metric_id, model } => {
                if let Some(arrear_period) = periods.arrear {
                    let usage = self.fetch_usage(arrear_period.clone(), *metric_id).await?;

                    match model {
                        UsagePricingModel::Matrix { rates } => {
                            for rate in rates {
                                let period = arrear_period.clone();

                                // for each rate, we get the quantity matching that rate
                                let quantity = usage
                                    .data
                                    .iter()
                                    .find(|usage| usage.dimensions == rate.dimensions)
                                    .map(|usage| usage.value.clone())
                                    .unwrap_or(Decimal::ZERO);

                                let price_total = rate.per_unit_price * quantity;

                                let price_cents = only_positive(price_total.to_cents()?);

                                if price_cents > 0 {
                                    let usage_line = InvoiceLineInner {
                                        quantity: quantity.to_u64(),
                                        unit_price: None,
                                        total: price_cents,
                                        period: Some(period),
                                        custom_line_name: None,
                                    };

                                    lines.push(usage_line);
                                }
                            }
                        }
                        model => {
                            let usage_units = usage.single()?;

                            let price_total = match model {
                                UsagePricingModel::PerUnit { rate } => *rate * usage_units,
                                UsagePricingModel::Tiered { tiers, block_size } => {
                                    fees::compute_tier_price(usage_units, tiers, block_size)
                                }
                                UsagePricingModel::Volume { tiers, block_size } => {
                                    fees::compute_volume_price(usage_units, tiers, block_size)
                                }
                                UsagePricingModel::Package { block_size, rate } => {
                                    let package_size_decimal = Decimal::from(*block_size);
                                    let total_packages =
                                        (usage_units / package_size_decimal).ceil();

                                    total_packages * *rate
                                }
                                UsagePricingModel::Matrix { .. } => unreachable!(),
                            };

                            let price_cents = only_positive(price_total.to_cents()?);

                            if price_cents > 0 {
                                let usage_line = InvoiceLineInner {
                                    quantity: usage_units.to_u64(),
                                    unit_price: None,
                                    total: price_cents,
                                    period: Some(arrear_period),
                                    custom_line_name: None,
                                };

                                lines.push(usage_line);
                            }
                        }
                    }

                    // if usage_units > Decimal::ZERO {
                }
            }
        }
        Ok(lines
            .into_iter()
            .map(|line| InvoiceLine {
                name: line
                    .custom_line_name
                    .unwrap_or_else(|| component.name.clone()),
                metadata: None,
                quantity: line.quantity,
                unit_price: line.unit_price.map(|price| price as f64),
                total: line.total as i64,
                period: line.period.map(|period| InvoiceLinePeriod {
                    from: period.start,
                    to: period.end,
                }),
                sub_lines: Vec::new(),
            })
            .collect())
    }

    async fn fetch_usage(
        &self,
        period: Period,
        metric_id: Uuid,
    ) -> Result<UsageData, ComputeError> {
        let metric = self
            .subscription_details
            .metrics
            .iter()
            .find(|metric| metric.id == metric_id)
            .ok_or(ComputeError::MetricNotFound)?;

        let usage = self
            .usage_client
            .fetch_usage(
                &self.subscription_details.tenant_id,
                &self.subscription_details.customer_id,
                &self.subscription_details.customer_external_id,
                metric,
                period,
            )
            .await?;

        Ok(usage)
    }

    async fn fetch_slots(
        &self,
        invoice_date: &NaiveDate,
        component_id: &Uuid,
    ) -> Result<u64, ComputeError> {
        let quantity = self
            .slots_client
            .fetch_slots(
                &self.subscription_details.tenant_id,
                &self.subscription_details.id,
                component_id,
                invoice_date,
            )
            .await?;

        Ok(quantity as u64)
    }
}

struct InvoiceLineInner {
    pub total: u64,
    pub quantity: Option<u64>,
    pub unit_price: Option<u64>,
    pub period: Option<Period>,
    pub custom_line_name: Option<String>,
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

fn to_invoice_line_inner(
    rate: &Decimal,
    quantity: &u32,
    period: Option<Period>,
    proration_factor: Option<f64>,
) -> Result<InvoiceLineInner, ComputeError> {
    let unit_price_cents = prorate(rate.to_cents()?, proration_factor);

    let total = rate * Decimal::from(*quantity);

    let total_cents = prorate(total.to_cents()?, proration_factor);

    Ok(InvoiceLineInner {
        quantity: Some(*quantity as u64),
        unit_price: Some(unit_price_cents),
        total: total_cents,
        period,
        custom_line_name: None,
    })
}
