//! Where a subscription's current period stands against its minimum commitment, for the dashboard.
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{SubscriptionId, TenantId};

use crate::StoreResult;
use crate::repositories::subscriptions::SubscriptionInterfaceAuto;
use crate::services::invoice_lines::minimum::{load_period_true_up, project_minimum_shortfall};
use crate::services::invoices::upcoming::upcoming_invoice_details;
use crate::services::{Services, ServicesEdge};
use crate::utils::datetime::elapsed_fraction;

#[derive(Debug, Clone)]
pub struct MinimumProgress {
    pub currency: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    /// Prorated to the period's active days.
    pub minimum_minor: i64,
    pub counted_to_date_minor: i64,
    pub projected_minor: i64,
    pub projected_shortfall_minor: i64,
    pub elapsed: f64,
    pub scope_product_names: Vec<String>,
}

impl ServicesEdge {
    /// None when the subscription has no minimum, no open period, or none applies to its next
    /// invoice.
    pub async fn subscription_minimum_progress(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        now: NaiveDateTime,
    ) -> StoreResult<Option<MinimumProgress>> {
        self.services
            .subscription_minimum_progress(tenant_id, subscription_id, now)
            .await
    }
}

impl Services {
    async fn subscription_minimum_progress(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        now: NaiveDateTime,
    ) -> StoreResult<Option<MinimumProgress>> {
        let details = self
            .store
            .get_subscription_details(tenant_id, subscription_id)
            .await?;
        if details.minimum.is_none() {
            return Ok(None);
        }
        let period_start = details.subscription.current_period_start;
        let Some(period_end) = details.subscription.current_period_end else {
            return Ok(None);
        };
        let elapsed = elapsed_fraction(period_start, period_end, now);
        let (invoice_details, invoice_date) = upcoming_invoice_details(&details);
        let (upcoming, true_up) = {
            let mut conn = self.store.get_conn().await?;
            let upcoming = self.compute_upcoming_invoice(&mut conn, &details).await?;
            let true_up = load_period_true_up(
                &mut conn,
                &invoice_details,
                invoice_date,
                invoice_details.subscription.cycle_index.unwrap_or(0),
                None,
            )
            .await?;
            (upcoming, true_up)
        };
        let Some(projection) = true_up.as_ref().and_then(|true_up| {
            project_minimum_shortfall(&invoice_details, &upcoming.invoice_lines, elapsed, true_up)
        }) else {
            return Ok(None);
        };
        Ok(Some(MinimumProgress {
            currency: details.subscription.currency.clone(),
            period_start,
            period_end,
            minimum_minor: projection.floor_cents,
            counted_to_date_minor: projection.counted_to_date_cents,
            projected_minor: projection.projected_cents,
            projected_shortfall_minor: projection.projected_shortfall_cents,
            elapsed,
            scope_product_names: projection.scope_product_names,
        }))
    }
}
