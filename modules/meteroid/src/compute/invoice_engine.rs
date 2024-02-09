use crate::compute::fees::capacity::ComputeCapacity;
use std::sync::Arc;

use crate::compute::fees::{ComputeInvoiceLine, ComputeInvoiceLineWithUsage};
use crate::models::*;
use anyhow::anyhow;
use anyhow::Result;
use meteroid_grpc::meteroid::api::components::v1::fee::r#type::Fee;

use chrono::NaiveDate;
use cornucopia_async::GenericClient;

use meteroid_grpc::meteroid::api::schedules::v1::plan_ramps::PlanRamp;
use meteroid_grpc::meteroid::api::schedules::v1::PlanRamps;
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;

use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;

use crate::compute::clients::subscription::SubscriptionClient;
use crate::compute::clients::usage::{MeteringUsageClient, UsageClient};
use common_grpc::middleware::client::LayeredClientService;
use uuid::Uuid;

#[derive(Clone)]
pub struct InvoiceEngine {
    pub(super) usage_client: Arc<dyn UsageClient + Send + Sync>,
}

impl InvoiceEngine {
    pub fn new(metering_client: UsageQueryServiceClient<LayeredClientService>) -> Self {
        Self {
            usage_client: Arc::new(MeteringUsageClient::new(metering_client)),
        }
    }
}

impl InvoiceEngine {
    // TODO dates
    // Computes the invoice lines for a subscription using the provided usage data and price point
    pub async fn calculate_invoice_lines<C: GenericClient>(
        &self,
        // this allows to fetch the subscription as part of a transaction
        db_client: &C,
        subscription_id: &Uuid,
        invoice_date: &NaiveDate,
    ) -> Result<InvoiceLines> {
        // Fetch subscription price point details
        let sub = SubscriptionClient::fetch_subscription_details(
            db_client,
            subscription_id,
            invoice_date,
        )
        .await?;

        let period_idx = sub.current_period_idx as u32;

        // TODO new line for minimum
        //  and ??? for invoice-level discount => probably only fixed fees/specific fees
        // => for some competitor, a ramp has a full component-level adjustment capability
        // similarly, minimums could apply to a subset of charges (cf Composite Charge by metrononome)

        // retrieve the ramp that match this period idx
        let _current_ramp = sub
            .schedule
            .as_ref()
            .and_then(|schedule| schedule.ramps.as_ref())
            .and_then(|ramps| find_matching_ramp(ramps, period_idx, sub.effective_billing_period));

        let mut invoice_lines = Vec::new();

        // TODO parallelize
        for component in &sub.price_components {
            let fee = component.fee.fee.clone().ok_or(anyhow!("Missing fee"))?;

            match &fee {
                Fee::Rate(pricing) => {
                    let line = pricing.compute(&sub, &component, None)?;
                    invoice_lines.extend(line);
                }
                Fee::SlotBased(pricing) => {
                    let line = pricing.compute(&sub, &component, None)?;
                    invoice_lines.extend(line);
                }
                Fee::Capacity(pricing) => {
                    let lines = pricing.compute(&sub, &component, self).await?;
                    invoice_lines.extend(lines);
                }
                Fee::UsageBased(pricing) => {
                    let line = pricing.compute(&sub, &component, self).await?;
                    invoice_lines.extend(line);
                }
                Fee::Recurring(pricing) => {
                    let line = pricing.compute(&sub, &component, None)?;
                    invoice_lines.extend(line);
                }
                Fee::OneTime(pricing) => {
                    let line = pricing.compute(&sub, &component, None)?;
                    invoice_lines.extend(line);
                }
            };
        }

        // TODO merge lines by product & period
        // if sub.group_by_product {
        //     unimplemented!()
        // }

        Ok(InvoiceLines {
            total: invoice_lines.iter().map(|line| line.total).sum(),
            lines: invoice_lines,
        })
    }
}

fn find_matching_ramp(
    ramps: &PlanRamps,
    current_period_idx: u32,
    effective_billing_period: BillingPeriod,
) -> Option<&PlanRamp> {
    let mut cumulative_duration = 0;

    let current_month_idx = effective_billing_period.months_value() * current_period_idx;

    for ramp in &ramps.ramps {
        match ramp.duration_in_months {
            Some(duration) => cumulative_duration += duration,
            None => return Some(ramp), // If duration is not set, treat it as infinite
        }

        if current_month_idx < cumulative_duration {
            return Some(ramp);
        }
    }

    // If current_period_idx is outside of all ramps' ranges, return the last ramp
    ramps.ramps.last()
}
