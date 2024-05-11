use crate::store::Store;
use crate::{domain, StoreResult};

use common_eventbus::Event;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::AsyncConnection;
use error_stack::Report;

use crate::errors::StoreError;

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

        let product_family =
            diesel_models::product_families::ProductFamily::find_by_external_id_and_tenant_id(
                &mut conn,
                plan.product_family_external_id.as_str(),
                plan.tenant_id,
            )
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let tenant = diesel_models::tenants::Tenant::find_by_id(&mut conn, plan.tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let res = conn
            .transaction(|conn| {
                async move {
                    let plan_to_insert: diesel_models::plans::PlanNew =
                        plan.into_raw(product_family.id);
                    let inserted: domain::Plan = plan_to_insert
                        .insert(conn)
                        .await
                        .map(Into::into)
                        .map_err(|err| StoreError::DatabaseError(err.error))?;

                    let plan_version_to_insert: diesel_models::plan_versions::PlanVersionNew =
                        domain::PlanVersionNew {
                            tenant_id: inserted.tenant_id,
                            internal: version,
                            plan_id: inserted.id,
                            version: 1,
                            created_by: inserted.created_by,
                        }
                        .into_raw(tenant.currency);

                    let inserted_plan_version_new: domain::PlanVersion = plan_version_to_insert
                        .insert(conn)
                        .await
                        .map(Into::into)
                        .map_err(|err| StoreError::DatabaseError(err.error))?;

                    // insert price component as batch, etc
                    let inserted_price_components =
                        diesel_models::price_components::PriceComponent::insert_batch(
                            conn,
                            price_components
                                .into_iter()
                                .map(|p| {
                                    domain::PriceComponentNew {
                                        plan_version_id: inserted_plan_version_new.id,
                                        name: p.name,
                                        product_item_id: p.product_item_id,
                                        fee: p.fee,
                                    }
                                    .try_into()
                                })
                                .collect::<Result<Vec<_>, _>>()?,
                        )
                        .await
                        .map_err(|err| StoreError::DatabaseError(err.error))?
                        .into_iter()
                        .map(TryInto::try_into)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|err| StoreError::TransactionStoreError(err))?;

                    Ok::<_, StoreError>(domain::FullPlan {
                        price_components: inserted_price_components,
                        plan: inserted,
                        version: inserted_plan_version_new,
                    })
                }
                .scope_boxed()
            })
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let _ = self
            .eventbus
            .publish(Event::plan_created_draft(
                res.plan.created_by,
                res.version.id,
                res.plan.tenant_id,
            ))
            .await;

        Ok(res)
    }
}
