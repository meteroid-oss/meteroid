use crate::compute::fees::shared::{parse_decimal, period_from_cadence, ToCents};
use crate::compute::fees::{ComputeInvoiceLineWithUsage, PeriodCalculator};
use crate::compute::{InvoiceEngine, PriceComponent, SubscriptionDetails};
use crate::models::{InvoiceLine, InvoiceLinePeriod};
use anyhow::{anyhow, Context, Error};

use meteroid_grpc::meteroid::api::components::v1::fee::{BillingType, UsageBased};
use meteroid_grpc::meteroid::api::components::v1::usage_pricing::model::Model;
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;

use rust_decimal::Decimal;

// TODO rounding etc
#[async_trait::async_trait]
impl ComputeInvoiceLineWithUsage for UsageBased {
    async fn compute(
        &self,
        subscription: &SubscriptionDetails,
        component: &PriceComponent,
        clients: &InvoiceEngine,
    ) -> Result<Option<InvoiceLine>, Error> {
        if !self.applies_this_period(subscription)? {
            return Ok(None);
        }

        let grpc_model = self
            .model
            .as_ref()
            .and_then(|p| p.model.as_ref())
            .context("No price found in usage charge")?;

        let metric = component
            .metric
            .as_ref()
            .ok_or_else(|| anyhow!("No metric found for usage"))?;

        let usage_data = clients
            .usage_client
            .fetch_usage(metric, subscription)
            .await?;

        let total_usage = usage_data.total_usage;

        let price = match grpc_model {
            Model::PerUnit(c) => {
                let unit_price: Decimal = parse_decimal(&c.unit_price)?;

                unit_price * total_usage
            }
            Model::Tiered(t) => {
                let mut subtotal = Decimal::new(0, 0);
                let mut remaining_usage = total_usage;

                let mut sorted_rows = t.rows.clone();
                sorted_rows.sort_by_key(|r| r.first_unit);
                // TODO block_size

                for tier in sorted_rows {
                    if remaining_usage.is_zero() {
                        break;
                    }

                    let tier_units = match tier.last_unit {
                        Some(last_unit) => {
                            if tier.first_unit == 0 {
                                Decimal::from(last_unit)
                            } else {
                                Decimal::from(
                                    last_unit.saturating_sub(tier.first_unit).saturating_add(1),
                                )
                            }
                        }
                        None => Decimal::MAX, // Handle infinite tier
                    };

                    let units_in_this_tier = if remaining_usage > tier_units {
                        tier_units
                    } else {
                        remaining_usage
                    };

                    let tier_price = parse_decimal(&tier.unit_price)?;

                    if units_in_this_tier > Decimal::ZERO {
                        let mut fee = units_in_this_tier * tier_price;
                        if let Some(flat_fee) = tier.flat_fee {
                            let flat_fee_dec: Decimal = flat_fee.try_into()?;
                            fee += flat_fee_dec;
                        }
                        if let Some(flat_cap) = tier.flat_cap {
                            let cap: Decimal = flat_cap.try_into()?;
                            if fee > cap {
                                fee = cap;
                            }
                        }
                        subtotal += fee;
                    }

                    remaining_usage -= units_in_this_tier;
                }

                subtotal
            }
            Model::Volume(v) => {
                let mut applicable_price_per_unit = Decimal::new(0, 0);
                let mut applicable_flat_fee = Decimal::new(0, 0);
                let mut applicable_flat_cap = Decimal::MAX;
                let mut sorted_rows = v.rows.clone();
                sorted_rows.sort_by_key(|r| r.first_unit);

                // TODO block_size
                for tier in sorted_rows {
                    if total_usage >= Decimal::from(tier.first_unit)
                        && tier
                            .last_unit
                            .map(|l| total_usage <= Decimal::from(l))
                            .unwrap_or(true)
                    {
                        applicable_price_per_unit = parse_decimal(&tier.unit_price)?;
                        if let Some(flat_fee) = tier.flat_fee {
                            applicable_flat_fee = flat_fee.try_into()?;
                        }
                        if let Some(flat_cap) = tier.flat_cap {
                            applicable_flat_cap = flat_cap.try_into()?;
                        }
                        break;
                    }
                }
                let price = total_usage * applicable_price_per_unit + applicable_flat_fee;
                if price > applicable_flat_cap {
                    applicable_flat_cap
                } else {
                    price
                }
            }
            Model::Package(p) => {
                let package_size_decimal = Decimal::from(p.block_size);
                let total_packages = (total_usage / package_size_decimal).ceil();

                total_packages * parse_decimal(&p.block_price)?
            }
        };

        let price_cents = price.to_cents()?;

        let line = InvoiceLine {
            name: component.name.to_string(),
            metadata: None,
            quantity: None,
            unit_price: None,
            total: price_cents,
            period: Some(usage_data.period),
            sub_lines: Vec::new(), // TODO we want each tier price etc as a sub line
        };

        Ok(Some(line))
    }
}

impl PeriodCalculator for UsageBased {
    fn applies_this_period(&self, subscription: &SubscriptionDetails) -> Result<bool, Error> {
        Ok(subscription.current_period_idx > 0)
    }

    fn period(&self, subscription: &SubscriptionDetails) -> Result<InvoiceLinePeriod, Error> {
        let usage_period =
            period_from_cadence(&BillingPeriod::Monthly, BillingType::Arrear, subscription)?;
        Ok(usage_period)
    }
}
