use std::sync::Arc;

use super::period::calculate_component_period;
use crate::compute::engine::component::ComponentEngine;
use crate::compute::errors::ComputeError;
use crate::constants::Currency;
use crate::domain::*;
use crate::repositories::TenantInterface;
use crate::Store;
use chrono::NaiveDate;
use itertools::Itertools;

use error_stack::{Report, Result, ResultExt};

#[async_trait::async_trait]
pub trait InvoiceLineInterface {
    async fn compute_dated_invoice_lines(
        &self,
        invoice_date: &NaiveDate,
        subscription_details: &SubscriptionDetails,
    ) -> Result<Vec<LineItem>, ComputeError>;
}

#[async_trait::async_trait]
impl InvoiceLineInterface for Store {
    // Here we consider that we HAVE the invoice_date, and we want to compute the invoice that need to be produced for a billing on that date.
    // However, this may not be the right/the only approach, and we may prefer an alternative approach where we compute the next invoices following the current date. (so the next invoice for each component)
    // => I guess that this ends up being similar ? we just compute these "invoice_dates" right ?
    async fn compute_dated_invoice_lines(
        &self,
        invoice_date: &NaiveDate,
        subscription_details: &SubscriptionDetails,
    ) -> Result<Vec<LineItem>, ComputeError> {
        if *invoice_date < subscription_details.billing_start_date {
            return Err(Report::new(ComputeError::InvalidInvoiceDate));
        }

        let currency = self
            .get_reporting_currency_by_tenant_id(subscription_details.tenant_id)
            .await
            .change_context(ComputeError::InternalError)?;

        let billing_start_date = subscription_details.billing_start_date;
        let billing_day = subscription_details.billing_day;
        let invoice_date = *invoice_date;

        let component_engine = ComponentEngine::new(
            self.usage_client.clone(),
            Arc::new(self.clone()), // TODO just use store
            Arc::new(subscription_details.clone()),
        );

        let price_components_lines = compute_invoice_lines(
            &component_engine,
            &subscription_details.price_components,
            billing_start_date,
            billing_day,
            invoice_date,
            &currency,
        )
        .await?;

        let add_ons_lines = compute_invoice_lines(
            &component_engine,
            &subscription_details.add_ons,
            billing_start_date,
            billing_day,
            invoice_date,
            &currency,
        )
        .await?;

        let invoice_lines = price_components_lines
            .into_iter()
            .chain(add_ons_lines)
            .collect();

        Ok(invoice_lines)
    }
}

async fn compute_invoice_lines<T: SubscriptionFeeInterface>(
    component_engine: &ComponentEngine,
    fee_records: &[T],
    billing_start_date: NaiveDate,
    billing_day: i16,
    invoice_date: NaiveDate,
    currency: &Currency,
) -> Result<Vec<LineItem>, ComputeError> {
    let component_groups = fee_records
        .iter()
        .into_group_map_by(|c| c.period_ref().clone());

    // TODO case when invoiced early via threshold (that's for usage-based only)
    // can be quite easy => we need some last_invoice_threshold date in the subscription, to reduce the usage periods if that date is within the period

    let component_period_components: Vec<(ComponentPeriods, Vec<&T>)> = component_groups
        .into_iter()
        .filter_map(|(billing_period, components)| {
            // we calculate the periods range, for each billing_period. Then there are 3 possibilities :
            // - if invoice date is the billing start date. This means that this is the first invoice. We only consider advance fees.
            //    - if billing_day is null or is the invoice date's day. No proration needed
            //    - else, proration needed
            // - else : invoice date is not the billing start date. We consider advance and arrear fees. No proration to apply
            let period = calculate_component_period(
                billing_start_date,
                billing_day as u32,
                invoice_date,
                &billing_period,
            );
            // in period is None, this means that the components are not relevant for this invoice
            period.map(|period| (period, components))
        })
        .collect();

    // we can now compute all the components for each period
    let mut invoice_lines = Vec::new();
    for (period, components) in component_period_components {
        for component in components {
            let lines = component_engine
                .compute_component(component, period.clone(), &invoice_date, currency.precision)
                .await?;
            invoice_lines.extend(lines);
        }
    }

    Ok(invoice_lines)
}
