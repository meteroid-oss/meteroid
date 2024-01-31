use crate::compute::fees::shared::{
    applies_from_cadence, only_positive, parse_decimal, period_from_cadence, prorate,
    should_prorate, ToCents,
};
use crate::compute::fees::ComputeInvoiceLine;
use crate::compute::{PriceComponent, SubscriptionDetails};
use crate::models::InvoiceLine;
use anyhow::{anyhow, Error};
use meteroid_grpc::meteroid::api::components::v1::fee::RecurringFixedFee;

impl ComputeInvoiceLine for RecurringFixedFee {
    fn compute(
        &self,
        subscription: &SubscriptionDetails,
        component: &PriceComponent,
        _override_units: Option<u64>,
    ) -> Result<Option<InvoiceLine>, Error> {
        let grpc_pricing = self
            .fee
            .as_ref()
            .ok_or_else(|| anyhow!("No price found in recurring charge"))?;

        if !applies_from_cadence(&self.cadence(), grpc_pricing.billing_type(), subscription)? {
            return Ok(None);
        }

        let cadence = self.cadence();

        let period = period_from_cadence(&cadence, grpc_pricing.billing_type(), subscription)?;

        let unit_price = parse_decimal(&grpc_pricing.unit_price)?;
        let mut unit_price_cents = only_positive(unit_price.to_cents()?);

        if should_prorate(
            subscription.current_period_idx,
            grpc_pricing.billing_type(),
            cadence,
        ) {
            unit_price_cents = prorate(unit_price_cents, &period);
        }

        let price_cents = unit_price_cents * grpc_pricing.quantity as i64;

        let line = InvoiceLine {
            name: component.name.clone(),
            metadata: None,
            quantity: Some(grpc_pricing.quantity as u64),
            unit_price: Some(unit_price_cents as f64),
            total: price_cents,
            period: Some(period),
            sub_lines: Vec::new(),
        };
        Ok(Some(line))
    }
}
