use crate::compute::fees::shared::{
    applies_from_cadence, parse_decimal, period_from_cadence, prorate, should_prorate, ToCents,
};
use crate::compute::{InvoiceEngine, PriceComponent, SubscriptionDetails};
use crate::models::InvoiceLine;

use anyhow::{anyhow, Error};
use async_trait::async_trait;

use meteroid_grpc::meteroid::api::components::v1::fee::capacity::capacity_pricing::{
    Pricing, Threshold,
};
use meteroid_grpc::meteroid::api::components::v1::fee::{BillingType, Capacity};
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

#[async_trait]
pub trait ComputeCapacity {
    async fn compute(
        &self,
        subscription: &SubscriptionDetails,
        component_id: &PriceComponent,
        clients: &InvoiceEngine,
    ) -> Result<Vec<InvoiceLine>, Error>;
}

// TODO proration if first month & billing_day != period_start_day
#[async_trait]
impl ComputeCapacity for Capacity {
    async fn compute(
        &self,
        subscription: &SubscriptionDetails,
        component: &PriceComponent,
        clients: &InvoiceEngine,
    ) -> Result<Vec<InvoiceLine>, Error> {
        let mut lines: Vec<InvoiceLine> = Vec::new();

        let grpc_pricing = self
            .pricing
            .as_ref()
            .and_then(|p| p.pricing.as_ref())
            .ok_or_else(|| anyhow!("No price found in capacity charge"))?;

        let threshold = self.extract_threshold(subscription, grpc_pricing, component)?;

        // 1 - Committed fee
        if let Some(fixed_fee_line) = self
            .compute_committed(subscription, grpc_pricing, &threshold, component)
            .await?
        {
            lines.push(fixed_fee_line);
        }

        // 2 - Overage fee
        if subscription.current_period_idx > 0 {
            let overage_line = self
                .compute_overage(subscription, clients, &threshold, component)
                .await?;

            if overage_line.total > 0 {
                lines.push(overage_line);
            }
        }

        Ok(lines)
    }
}

#[async_trait]
trait ComputeCapacityInner {
    fn extract_threshold(
        &self,
        subscription: &SubscriptionDetails,
        pricing: &Pricing,
        component: &PriceComponent,
    ) -> Result<Threshold, Error>;
    async fn compute_committed(
        &self,
        subscription: &SubscriptionDetails,
        pricing: &Pricing,
        threshold: &Threshold,
        component: &PriceComponent,
    ) -> Result<Option<InvoiceLine>, Error>;
    async fn compute_overage(
        &self,
        subscription: &SubscriptionDetails,
        clients: &InvoiceEngine,
        threshold: &Threshold,
        component: &PriceComponent,
    ) -> Result<InvoiceLine, Error>;
}

#[async_trait]
impl ComputeCapacityInner for Capacity {
    fn extract_threshold(
        &self,
        subscription: &SubscriptionDetails,
        pricing: &Pricing,
        component: &PriceComponent,
    ) -> Result<Threshold, Error> {
        let committed_billing_period = subscription.parameters.committed_billing_period();

        let threshold_value = subscription
            .parameters
            .parameters
            .iter()
            .find_map(|p| {
                if p.component_id == component.id {
                    Some(p.value)
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow!("No capacity parameter found"))?;

        let threshold_opt = match pricing {
            Pricing::Single(p) => p
                .thresholds
                .iter()
                .find(|t| t.included_amount == threshold_value),
            Pricing::TermBased(t) => t
                .rates
                .iter()
                .find(|r| r.term() == committed_billing_period)
                .ok_or_else(|| anyhow!("No rate found for billing period"))?
                .thresholds
                .iter()
                .find(|t| t.included_amount == threshold_value),
        };

        let threshold = threshold_opt.ok_or_else(|| anyhow!("No threshold found for capacity"))?;
        Ok(threshold.clone())
    }

    async fn compute_committed(
        &self,
        subscription: &SubscriptionDetails,
        grpc_pricing: &Pricing,
        threshold: &Threshold,
        component: &PriceComponent,
    ) -> Result<Option<InvoiceLine>, Error> {
        let fixed_fee_cadence = extract_cadence(grpc_pricing, subscription)?;

        let billing_type = BillingType::Advance; // TODO advance only ?

        if applies_from_cadence(&fixed_fee_cadence, billing_type, subscription)? {
            let fixed_fee_period =
                period_from_cadence(&fixed_fee_cadence, billing_type, subscription)?;

            let mut fixed_fee_price = parse_decimal(&threshold.price)?.to_cents()?;

            if should_prorate(
                subscription.current_period_idx,
                billing_type,
                fixed_fee_cadence,
            ) {
                fixed_fee_price = prorate(fixed_fee_price, &fixed_fee_period);
            }

            let fixed_fee_line = InvoiceLine {
                name: format!("{} - Committed fee", component.name),
                metadata: None,
                quantity: Some(1),
                unit_price: Some(fixed_fee_price as f64),
                total: fixed_fee_price,
                period: Some(fixed_fee_period),
                sub_lines: Vec::new(),
            };

            Ok(Some(fixed_fee_line))
        } else {
            Ok(None)
        }
    }

    async fn compute_overage(
        &self,
        subscription: &SubscriptionDetails,
        clients: &InvoiceEngine,
        threshold: &Threshold,
        component: &PriceComponent,
    ) -> Result<InvoiceLine, Error> {
        let metric = component
            .metric
            .as_ref()
            .ok_or_else(|| anyhow!("No metric found for capacity"))?;

        let usage_data = clients
            .usage_client
            .fetch_usage(metric, subscription)
            .await?;

        let per_unit_overage = parse_decimal(&threshold.per_unit_overage)?;

        let included_amount = threshold.included_amount;

        let overage_units = usage_data.total_usage - Decimal::from(included_amount);

        let overage_total = if overage_units > Decimal::ZERO {
            overage_units * per_unit_overage
        } else {
            Decimal::ZERO
        };

        let total_price = overage_total;

        let total_price_cents = total_price.to_cents()?;

        let per_unit_overage_cents = per_unit_overage.to_cents_f64()?;

        let overage_units_u64 = overage_units
            .to_u64()
            .ok_or_else(|| anyhow!("Overage units out of u64 bounds"))?;

        let overage_line = InvoiceLine {
            name: format!("{} - Overage", component.name),
            metadata: None,
            quantity: Some(overage_units_u64),
            unit_price: Some(per_unit_overage_cents),
            total: total_price_cents,
            period: Some(usage_data.period),
            sub_lines: Vec::new(),
        };

        Ok(overage_line)
    }
}

fn extract_cadence(
    pricing: &Pricing,
    subscription: &SubscriptionDetails,
) -> Result<BillingPeriod, Error> {
    let cadence: BillingPeriod = match pricing {
        Pricing::Single(_p) => BillingPeriod::Monthly,
        Pricing::TermBased(_) => subscription.parameters.committed_billing_period(),
    };
    Ok(cadence)
}
