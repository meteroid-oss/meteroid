//! A batch of timeline edits applied in order under one lock and one transaction: either all of
//! them hold or none does, and the state is derived once at the end (R-156).
use common_domain::actor::Actor;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::subscriptions::SubscriptionRow;
use scoped_futures::ScopedFutureExt;

use crate::StoreResult;
use crate::domain::subscription_phases::TimelineOp;
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::subscriptions::{phases, terms, timeline_items};
use error_stack::Report;

impl Services {
    /// The invoices the subscription would bill with `ops` applied, read in a transaction rolled
    /// back: nothing changes. Refused as the batch would be.
    pub(in crate::services) async fn preview_timeline(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        ops: Vec<TimelineOp>,
        months: u32,
    ) -> StoreResult<crate::services::timeline::Projection> {
        let mut projection = None;
        let rolled_back = self
            .store
            .transaction(|conn| {
                let actor = &actor;
                let ops = &ops;
                let projection = &mut projection;
                async move {
                    self.apply_ops_tx(conn, actor, tenant_id, subscription_id, ops)
                        .await?;
                    *projection = Some(
                        crate::services::timeline::project_tx(
                            conn,
                            tenant_id,
                            subscription_id,
                            months,
                        )
                        .await?,
                    );
                    Err::<(), _>(Report::new(StoreError::FailedPrecondition(
                        "a timeline preview is rolled back".into(),
                    )))
                }
                .scope_boxed()
            })
            .await;
        match (projection, rolled_back) {
            (Some(projection), _) => Ok(projection),
            (None, Err(error)) => Err(error),
            (None, Ok(())) => Err(Report::new(StoreError::FailedPrecondition(
                "a timeline preview was not rolled back".into(),
            ))),
        }
    }

    pub(in crate::services) async fn apply_timeline(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        ops: Vec<TimelineOp>,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                let ops = &ops;
                async move {
                    self.apply_ops_tx(conn, actor, tenant_id, subscription_id, ops)
                        .await
                }
                .scope_boxed()
            })
            .await
    }

    /// The batch applied in order under the subscription's lock, its state derived once.
    async fn apply_ops_tx(
        &self,
        conn: &mut crate::store::PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        ops: &[TimelineOp],
    ) -> StoreResult<()> {
        let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id).await?;
        diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
        for op in ops {
            match op {
                TimelineOp::SetPhases(input) => {
                    phases::set_phases_tx(self, conn, actor, tenant_id, subscription_id, input)
                        .await?;
                }
                TimelineOp::AddItem(input) => {
                    timeline_items::write_item_tx(
                        self,
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        input,
                    )
                    .await?;
                }
                TimelineOp::AddDiscount(input) => {
                    timeline_items::write_discount_tx(
                        self,
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        input,
                    )
                    .await?;
                }
                TimelineOp::RemoveItem(item_id) => {
                    timeline_items::remove_item_tx(conn, tenant_id, subscription_id, *item_id)
                        .await?;
                }
                TimelineOp::SetTerm(input) => {
                    terms::set_term_tx(self, conn, actor, tenant_id, subscription_id, input)
                        .await?;
                }
                TimelineOp::SetTermWaivingNotice(input, waiver) => {
                    terms::set_term_waiving_tx(
                        self,
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        input,
                        Some(waiver),
                    )
                    .await?;
                }
            }
        }
        self.derive_state_tx(conn, actor, tenant_id, subscription_id, common_clock::now())
            .await
            .map(|_| ())
    }
}
