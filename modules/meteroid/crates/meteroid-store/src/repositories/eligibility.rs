use crate::StoreResult;
use crate::domain::eligibility::Eligibility;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::errors::StoreError;
use crate::store::Store;
use common_domain::ids::{BaseId, PlanId, TenantId};
use diesel_models::plans::PlanRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

#[async_trait::async_trait]
pub trait EligibilityInterface {
    /// Who may get the plan, live: no version is made, subscriptions are left as they
    /// are. A rule letting anyone in clears it; the rule as stored is returned.
    async fn set_plan_eligibility(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        plan_id: PlanId,
        eligibility: Eligibility,
    ) -> StoreResult<Option<Eligibility>>;
}

#[async_trait::async_trait]
impl EligibilityInterface for Store {
    async fn set_plan_eligibility(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        plan_id: PlanId,
        eligibility: Eligibility,
    ) -> StoreResult<Option<Eligibility>> {
        let rule = eligibility.normalized();
        rule.validate()
            .map_err(|message| Report::new(StoreError::InvalidArgument(message)))?;
        let stored = Eligibility::to_row(Some(&rule));

        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let before = PlanRow::list_by_ids(conn, &[plan_id], tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        Report::new(StoreError::ValueNotFound(format!("plan {plan_id}")))
                    })?;
                let before = Eligibility::from_row(before.eligibility);

                let row = PlanRow::set_eligibility(conn, plan_id, tenant_id, stored)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let after = Eligibility::from_row(row.eligibility);

                if before != after {
                    let activity = Activity::new(
                        ActivityType::EntityUpdated,
                        EntityType::Plan,
                        plan_id.as_uuid(),
                    )
                    .with_metadata(serde_json::json!({
                        "changes": [{ "field": "eligibility" }],
                    }));
                    self.internal
                        .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                        .await?;
                }

                Ok(after)
            }
            .scope_boxed()
        })
        .await
    }
}
