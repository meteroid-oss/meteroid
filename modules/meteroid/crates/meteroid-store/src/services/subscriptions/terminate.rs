use crate::StoreResult;
use crate::services::{InvoiceBillingMode, Services};
use crate::store::PgConn;
use chrono::NaiveDate;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::subscriptions::SubscriptionCycleRowPatch;

impl Services {
    pub(in crate::services) async fn terminate_subscription(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        date: NaiveDate,
        terminate_with_state: SubscriptionStatusEnum,
    ) -> StoreResult<()> {
        let patch = SubscriptionCycleRowPatch {
            id: subscription_id,
            tenant_id,
            status: Some(terminate_with_state),
            next_cycle_action: Some(None),
            current_period_start: Some(date),
            current_period_end: Some(None),
            cycle_index: None, // we don't increase cycle index on termination
        };

        patch.patch(conn).await?;

        self.bill_subscription_tx(
            conn,
            tenant_id,
            subscription_id,
            InvoiceBillingMode::AwaitGracePeriodIfApplicable,
        )
        .await?;

        Ok(())
    }
}
