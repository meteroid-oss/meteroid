use crate::store::Store;
use crate::StoreResult;

use crate::domain::{
    FullPlan, FullPlanNew, OrderByRequest, PaginatedVec, PaginationRequest, Plan, PlanForList,
    PlanVersion, PlanVersionNew, PriceComponent, PriceComponentNew,
};
use common_eventbus::Event;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::AsyncConnection;
use error_stack::Report;
use uuid::Uuid;

use crate::errors::StoreError;

#[async_trait::async_trait]
pub trait PlansInterface {
    async fn insert_plan(&self, plan: FullPlanNew) -> StoreResult<FullPlan>;
    async fn get_plan_by_external_id(
        &self,
        external_id: &str,
        auth_tenant_id: Uuid,
        is_draft: Option<bool>,
    ) -> StoreResult<FullPlan>;
    async fn list_plans(
        &self,
        auth_tenant_id: Uuid,
        search: Option<String>,
        product_family_external_id: Option<String>,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<PlanForList>>;
}

#[async_trait::async_trait]
impl PlansInterface for Store {
    async fn insert_plan(&self, full_plan: FullPlanNew) -> StoreResult<FullPlan> {
        let mut conn = self.get_conn().await?;

        let FullPlanNew {
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
                    let inserted: Plan = plan_to_insert
                        .insert(conn)
                        .await
                        .map(Into::into)
                        .map_err(|err| StoreError::DatabaseError(err.error))?;

                    let plan_version_to_insert: diesel_models::plan_versions::PlanVersionNew =
                        PlanVersionNew {
                            tenant_id: inserted.tenant_id,
                            internal: version,
                            plan_id: inserted.id,
                            version: 1,
                            created_by: inserted.created_by,
                        }
                        .into_raw(tenant.currency);

                    let inserted_plan_version_new: PlanVersion = plan_version_to_insert
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
                                    PriceComponentNew {
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

                    Ok::<_, StoreError>(FullPlan {
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

    async fn get_plan_by_external_id(
        &self,
        external_id: &str,
        auth_tenant_id: Uuid,
        is_draft: Option<bool>,
    ) -> StoreResult<FullPlan> {
        let mut conn = self.get_conn().await?;

        let plan: Plan = diesel_models::plans::Plan::get_by_external_id_and_tenant_id(
            &mut conn,
            external_id,
            auth_tenant_id,
        )
        .await
        .map(Into::into)
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        let version: PlanVersion =
            diesel_models::plan_versions::PlanVersion::find_latest_by_plan_id_and_tenant_id(
                &mut conn,
                plan.id,
                auth_tenant_id,
                is_draft,
            )
            .await
            .map(Into::into)
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let price_components: Vec<PriceComponent> =
            diesel_models::price_components::PriceComponent::list_by_plan_version_id(
                &mut conn,
                auth_tenant_id,
                version.id,
            )
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(FullPlan {
            plan,
            version,
            price_components,
        })
    }

    async fn list_plans(
        &self,
        auth_tenant_id: Uuid,
        search: Option<String>,
        product_family_external_id: Option<String>,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<PlanForList>> {
        let mut conn = self.get_conn().await?;

        let rows = diesel_models::plans::PlanForList::list(
            &mut conn,
            auth_tenant_id,
            search,
            product_family_external_id,
            pagination.into(),
            order_by.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<PlanForList> = PaginatedVec {
            items: rows.items.into_iter().map(Into::into).collect(),
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        };

        Ok(res)
    }
}
