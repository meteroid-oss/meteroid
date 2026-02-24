use crate::StoreResult;
use crate::domain::{Period, SubscriptionDetails};
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::clients::usage::WindowedUsageData;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use crate::store::PgConn;
use common_domain::ids::BillableMetricId;
use error_stack::Report;

impl Services {
    /// Compute the upcoming invoice — i.e. the next invoice that will be generated
    /// at the end of the current billing period.
    ///
    /// Uses `current_period_end` as the invoice date and increments cycle_index
    /// so that arrear/usage charges cover the current period and advance charges
    /// cover the next period (full, no proration).
    pub async fn compute_upcoming_invoice(
        &self,
        conn: &mut PgConn,
        subscription_details: &SubscriptionDetails,
    ) -> StoreResult<ComputedInvoiceContent> {
        let mut details = subscription_details.clone();

        let current_cycle = details.subscription.cycle_index.unwrap_or(0);
        let next_cycle = current_cycle + 1;
        details.subscription.cycle_index = Some(next_cycle);

        let invoice_date = details
            .subscription
            .current_period_end
            .unwrap_or_else(|| chrono::Utc::now().date_naive());


        self.compute_invoice(conn, &invoice_date, &details, None, None)
            .await
    }

    pub async fn get_subscription_component_usage(
        &self,
        subscription_details: &SubscriptionDetails,
        metric_id: BillableMetricId,
    ) -> StoreResult<WindowedUsageData> {
        let metric = subscription_details
            .metrics
            .iter()
            .find(|m| m.id == metric_id)
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(format!(
                    "Metric {} not found on subscription",
                    metric_id
                )))
            })?;

        let period = Period {
            start: subscription_details.subscription.current_period_start,
            end: subscription_details
                .subscription
                .current_period_end
                .unwrap_or_else(|| chrono::Utc::now().date_naive() + chrono::Duration::days(1)),
        };

        if period.start >= period.end {
            return Ok(WindowedUsageData {
                data: vec![],
                period,
            });
        }

        self.usage_client
            .fetch_windowed_usage(
                &subscription_details.subscription.tenant_id,
                &subscription_details.subscription.customer_id,
                metric,
                period,
            )
            .await
    }
}
