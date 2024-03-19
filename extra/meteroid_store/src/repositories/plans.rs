use crate::errors;
use crate::errors::db_error_to_store;
use crate::store::Store;
use crate::{domain, StoreResult};
use diesel::sql_types::{Array, Nullable};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::AsyncConnection;
use diesel_models::enums::BillingPeriodEnum;
use error_stack::ResultExt;

#[async_trait::async_trait]
pub trait PlansInterface {
    async fn insert_plan(&self, plan: domain::PlanNew) -> StoreResult<domain::PlanNew>;
}

#[async_trait::async_trait]
impl PlansInterface for Store {
    async fn insert_plan(&self, plan: domain::PlanNew) -> StoreResult<domain::PlanNew> {
        let mut conn = self.get_conn().await?;

        let res = conn
            .transaction(|conn| {
                async move {
                    // TODO fix error handling
                    let plan_to_insert = diesel_models::plans::PlanNew {
                        id: plan.id,
                        name: plan.name,
                        description: plan.description,
                        created_by: plan.created_by,
                        plan_type: plan.plan_type,
                        external_id: plan.external_id,
                        status: plan.status,
                        tenant_id: plan.tenant_id,
                        product_family_id: plan.product_family_id,
                    };

                    let inserted: domain::Plan = plan_to_insert
                        .insert(conn)
                        .await
                        .map_err(db_error_to_store)
                        .map(Into::into)?;

                    diesel_models::plan_versions::PlanVersionNew {
                        id: uuid::Uuid::now_v7(),
                        is_draft_version: plan.version_details.is_draft_version,
                        plan_id: inserted.id,
                        version: 0,
                        trial_duration_days: plan.version_details.trial_duration_days,
                        created_by: inserted.created_by,
                        tenant_id: inserted.tenant_id,
                        period_start_day: plan.version_details.period_start_day,
                        net_terms: plan.version_details.net_terms,
                        currency: plan.version_details.currency,
                        trial_fallback_plan_id: plan.version_details.trial_fallback_plan_id,
                        billing_cycles: plan.version_details.billing_cycles,
                        billing_periods: plan
                            .version_details
                            .billing_periods
                            .into_iter()
                            .map(|v| Some(v))
                            .collect(),
                    }
                    .insert(conn)
                    .await
                    .map_err(db_error_to_store)?;

                    // insert price component as batch, etc

                    Ok(inserted)
                }
                .scope_boxed()
            })
            .await
            .map_err(db_error_to_store)?;

        Ok(todo!())
    }
}
