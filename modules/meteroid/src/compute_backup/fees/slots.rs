use crate::compute::fees::shared::{
    prorate, should_prorate, CadenceExtractor, PriceExtractor, ToCents,
};
use crate::compute::fees::{ComputeInvoiceLine, PeriodCalculator};
use crate::compute::{PriceComponent, SubscriptionDetails};
use crate::models::InvoiceLine;
use anyhow::{anyhow, Error};

use meteroid_grpc::meteroid::api::components::v1::fee::{BillingType, SlotBased};

#[async_trait::async_trait]
impl ComputeInvoiceLine for SlotBased {
    fn compute(
        &self,
        subscription: &SubscriptionDetails,
        component: &PriceComponent,
        override_units: Option<u64>,
    ) -> Result<Option<InvoiceLine>, Error> {
        let grpc_pricing = self
            .pricing
            .as_ref()
            .and_then(|p| p.pricing.as_ref())
            .ok_or_else(|| anyhow!("No price found in slots charge"))?;

        // TODO only advance ?
        if !grpc_pricing.applies_this_period(subscription)? {
            return Ok(None);
        }

        let period = grpc_pricing.period(subscription)?;

        let cadence = grpc_pricing.extract_cadence(subscription)?;

        let mut units: u64;

        if let Some(override_units) = override_units {
            units = override_units;
        } else {
            if subscription.current_period_idx <= 0 {
                // we get usage from subscription. TODO should we instead just create the usage in db before that ?
                units = subscription
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
            } else {
                // we get usage from dedicated table TODO
                unimplemented!();
            }

            if let Some(minimum_count) = self.minimum_count {
                let minimum = minimum_count as u64;
                if units < minimum {
                    units = minimum;
                }
            }
        }

        let price_per_unit = grpc_pricing.extract_price(subscription)?;

        let mut price_per_unit_cents = price_per_unit.to_cents()?;

        if should_prorate(
            subscription.current_period_idx,
            BillingType::Advance,
            cadence,
        ) {
            price_per_unit_cents = prorate(price_per_unit_cents, &period);
        }

        let total_cents = price_per_unit_cents * units as i64;

        let line = InvoiceLine {
            name: component.name.clone(),
            metadata: None,
            quantity: Some(units),
            unit_price: Some(price_per_unit_cents as f64),
            total: total_cents,
            period: Some(period),
            sub_lines: Vec::new(),
        };

        Ok(Some(line))
    }
}
