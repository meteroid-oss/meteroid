use crate::StoreResult;
use crate::constants::{Currencies, Currency};
use crate::domain::*;
use chrono::NaiveDate;
use itertools::Itertools;

use crate::errors::StoreError;
use crate::services::Services;
use crate::store::PgConn;
use crate::utils::periods::calculate_component_period_for_invoice_date;
use error_stack::{Report, ResultExt};

impl Services {
    pub async fn compute_invoice_lines(
        &self,
        conn: &mut PgConn, // used to fetch slots
        invoice_date: &NaiveDate,
        subscription_details: &SubscriptionDetails,
    ) -> StoreResult<Vec<LineItem>> {
        let billing_start_date = subscription_details
            .subscription
            .billing_start_date
            // TODO should we return empty ?
            .ok_or(Report::new(StoreError::BillingError))
            .attach_printable("No billing_start_date is present")?;

        if subscription_details.subscription.activated_at.is_none() {
            return Err(Report::new(StoreError::BillingError))
                .attach_printable("Subscription is not activated, cannot compute invoice lines");
        }

        let currency = Currencies::resolve_currency(&subscription_details.subscription.currency)
            .ok_or(Report::new(StoreError::ValueNotFound(format!(
                "Currency {} not found",
                subscription_details.subscription.currency
            ))))?;

        let cycle_index = subscription_details.subscription.cycle_index.ok_or(
            Report::new(StoreError::BillingError)
                .attach_printable("Subscription cycle index is not set"),
        )?;

        let invoice_date = *invoice_date;

        let price_components_lines = self
            .process_fee_records(
                conn,
                &subscription_details,
                &subscription_details.price_components,
                invoice_date,
                billing_start_date,
                cycle_index,
                &currency,
            )
            .await?;

        let add_ons_lines = self
            .process_fee_records(
                conn,
                &subscription_details,
                &subscription_details.add_ons,
                invoice_date,
                billing_start_date,
                cycle_index,
                &currency,
            )
            .await?;

        let invoice_lines = price_components_lines
            .into_iter()
            .chain(add_ons_lines)
            .collect();

        Ok(invoice_lines)
    }

    async fn process_fee_records<T: SubscriptionFeeInterface>(
        &self,
        conn: &mut PgConn,
        subscription_details: &SubscriptionDetails,
        fee_records: &[T],
        invoice_date: NaiveDate,
        billing_start_or_resume_date: NaiveDate,
        cycle_index: u32,
        currency: &Currency,
    ) -> StoreResult<Vec<LineItem>> {
        let component_groups = fee_records
            .iter()
            .into_group_map_by(|c| c.period_ref().clone());

        // TODO case when invoiced early via threshold (that's for usage-based only)
        // can be quite easy => we need some last_invoice_threshold date in the subscription, to reduce the usage periods if that date is within the period

        let component_period_components: Vec<(ComponentPeriods, Vec<&T>)> = component_groups
            .into_iter()
            .filter_map(|(billing_period, components)| {
                // we calculate the periods range, for each billing_period. There can be advance, arrears, or both
                let period = calculate_component_period_for_invoice_date(
                    invoice_date,
                    &subscription_details.subscription.period,
                    &billing_period,
                    billing_start_or_resume_date,
                    cycle_index,
                    subscription_details.subscription.billing_day_anchor as u32,
                    subscription_details
                        .subscription
                        .current_period_end
                        .is_none(),
                );

                // in period is None, this means that the components are not relevant for this invoice
                period.map(|period| (period, components))
            })
            .collect();

        // we can now compute all the components for each period
        let mut invoice_lines = Vec::new();
        for (period, components) in component_period_components {
            for component in components {
                let lines = self
                    .compute_component(
                        conn,
                        &subscription_details,
                        component,
                        period.clone(),
                        &invoice_date,
                        currency.precision,
                    )
                    .await?;

                invoice_lines.extend(lines);
            }
        }

        Ok(invoice_lines)
    }
}
