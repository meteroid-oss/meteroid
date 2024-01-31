use crate::compute::fees::shared::{
    only_positive, parse_decimal, prorate, should_prorate, CadenceExtractor, ToCents,
};
use crate::compute::fees::{ComputeInvoiceLine, PeriodCalculator};
use crate::models::InvoiceLine;
use anyhow::{anyhow, Error};
use meteroid_grpc::meteroid::api::components::v1::fee::term_fee_pricing::Pricing;
use meteroid_grpc::meteroid::api::components::v1::fee::{BillingType, SubscriptionRate};

use crate::compute::{PriceComponent, SubscriptionDetails};

// TODO proration if first month & billing_day != period_start_day
impl ComputeInvoiceLine for SubscriptionRate {
    fn compute(
        &self,
        subscription: &SubscriptionDetails,
        component: &PriceComponent,
        _override_units: Option<u64>,
    ) -> Result<Option<InvoiceLine>, Error> {
        let grpc_pricing = self
            .pricing
            .as_ref()
            .and_then(|p| p.pricing.as_ref())
            .ok_or_else(|| anyhow!("No price found in rate charge"))?;

        if !grpc_pricing.applies_this_period(subscription)? {
            return Ok(None);
        }

        let committed_billing_period = subscription.parameters.committed_billing_period();

        let price_opt_grpc = match grpc_pricing {
            Pricing::Single(p) => &p.price,
            Pricing::TermBased(t) => {
                &t.rates
                    .iter()
                    .find(|r| r.term() == committed_billing_period)
                    .ok_or_else(|| anyhow!("No rate found for billing period"))?
                    .price
            }
        };

        let period = grpc_pricing.period(subscription)?;
        let cadence = grpc_pricing.extract_cadence(subscription)?;

        let price = parse_decimal(price_opt_grpc)?;

        let mut price_cents = only_positive(price.to_cents()?);

        if should_prorate(
            subscription.current_period_idx,
            BillingType::Advance,
            cadence,
        ) {
            price_cents = prorate(price_cents, &period);
        }

        let line = InvoiceLine {
            name: component.name.clone(),
            metadata: None,
            quantity: Some(1),
            unit_price: Some(price_cents as f64),
            total: price_cents,
            period: Some(period),
            sub_lines: Vec::new(),
        };

        Ok(Some(line))
    }
}
