use crate::compute::fees::shared::ToCents;
use crate::compute::fees::{ComputeInvoiceLine, PeriodCalculator};
use crate::compute::period::calculate_period_range;
use crate::compute::{PriceComponent, SubscriptionDetails};
use crate::models::{InvoiceLine, InvoiceLinePeriod};
use anyhow::{anyhow, Context, Error};
use meteroid_grpc::meteroid::api::components::v1::fee::{BillingType, OneTime};
use rust_decimal::Decimal;
use std::str::FromStr;

impl ComputeInvoiceLine for OneTime {
    fn compute(
        &self,
        subscription: &SubscriptionDetails,
        component: &PriceComponent,
        _override_units: Option<u64>,
    ) -> Result<Option<InvoiceLine>, Error> {
        if !self.applies_this_period(subscription)? {
            return Ok(None);
        }

        // should we define a period ?
        let period = None;

        let grpc_pricing = self
            .pricing
            .as_ref()
            .ok_or_else(|| anyhow!("No price found in one-time charge"))?;

        let unit_price_grpc = grpc_pricing
            .unit_price
            .as_ref()
            .ok_or_else(|| anyhow!("No unit price found in one-time charge"))?;

        let unit_price =
            Decimal::from_str(&unit_price_grpc.value).context("Failed to parse rate fee value")?;

        let price = unit_price * Decimal::from(grpc_pricing.quantity);

        // no proration for onetime fees
        let unit_price_cents = unit_price.to_cents_f64()?;
        let price_cents = price.to_cents()?;

        let line = InvoiceLine {
            name: component.name.clone(),
            metadata: None,
            quantity: Some(grpc_pricing.quantity as u64),
            unit_price: Some(unit_price_cents),
            total: price_cents,
            period,
            sub_lines: Vec::new(),
        };

        Ok(Some(line))
    }
}

impl PeriodCalculator for OneTime {
    fn applies_this_period(&self, subscription: &SubscriptionDetails) -> Result<bool, Error> {
        let grpc_pricing = self
            .pricing
            .as_ref()
            .ok_or_else(|| anyhow!("No price found in one-time charge"))?;
        let active_period = match grpc_pricing.billing_type() {
            BillingType::Advance => 0,
            BillingType::Arrear => 1,
        };

        Ok(subscription.current_period_idx == active_period)
    }

    fn period(&self, subscription: &SubscriptionDetails) -> Result<InvoiceLinePeriod, Error> {
        let (_, period_end) = calculate_period_range(
            subscription.billing_start_date,
            subscription.billing_day as u32,
            subscription.current_period_idx,
            subscription.effective_billing_period,
        );

        Ok(InvoiceLinePeriod {
            from: subscription.invoice_date,
            to: period_end,
        })
    }
}
