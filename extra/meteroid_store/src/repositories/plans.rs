use crate::errors;
use crate::store::Store;
use crate::{domain, StoreResult};
use diesel::sql_types::{Array, Nullable};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::AsyncConnection;
use diesel_models::enums::BillingPeriodEnum;
use diesel_models::errors::{DatabaseError, DatabaseErrorContainer};
use o2o::traits::IntoExisting;

#[async_trait::async_trait]
pub trait PlansInterface {
    async fn insert_plan(&self, plan: domain::FullPlanNew) -> StoreResult<domain::FullPlan>;
}

#[async_trait::async_trait]
impl PlansInterface for Store {
    async fn insert_plan(&self, full_plan: domain::FullPlanNew) -> StoreResult<domain::FullPlan> {
        let mut conn = self.get_conn().await?;

        let domain::FullPlanNew {
            plan,
            version,
            price_components,
        } = full_plan;

        conn.transaction(|conn| {
            async move {
                let plan_to_insert: diesel_models::plans::PlanNew = plan.into();
                let inserted: domain::Plan = plan_to_insert.insert(conn).await.map(Into::into)?;
                let plan_version_to_insert: diesel_models::plan_versions::PlanVersionNew =
                    domain::PlanVersionNew {
                        internal: version,
                        plan_id: inserted.id,
                        version: 0,
                    }
                    .into();

                let inserted_plan_version_new: domain::PlanVersion =
                    plan_version_to_insert.insert(conn).await.map(Into::into)?;

                // insert price component as batch, etc
                let inserted_price_components =
                    diesel_models::price_components::PriceComponent::insert_batch(
                        conn,
                        price_components
                            .into_iter()
                            .map(|p| {
                                domain::PriceComponentNew {
                                    internal: p,
                                    plan_version_id: inserted_plan_version_new.id,
                                }
                                .into()
                            })
                            .collect(),
                    )
                    .await?
                    .into_iter()
                    .map(Into::into)
                    .collect();

                Ok::<_, DatabaseErrorContainer>(domain::FullPlan {
                    price_components: inserted_price_components,
                    plan: inserted,
                    version: inserted_plan_version_new,
                })
            }
            .scope_boxed()
        })
        .await
        .map_err(Into::into)
    }
}
