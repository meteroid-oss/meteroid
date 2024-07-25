use crate::store::Store;
use crate::StoreResult;

use crate::domain::{
    FullPlan, FullPlanNew, OrderByRequest, PaginatedVec, PaginationRequest, Plan,
    PlanAndVersionPatch, PlanForList, PlanPatch, PlanVersion, PlanVersionLatest, PlanVersionNew,
    PlanWithVersion, PriceComponent, PriceComponentNew,
};
use common_eventbus::Event;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::AsyncConnection;
use diesel_models::plan_versions::{
    PlanVersionRow, PlanVersionRowLatest, PlanVersionRowNew, PlanVersionRowPatch,
};
use diesel_models::plans::{PlanRow, PlanRowForList, PlanRowNew, PlanRowPatch};
use diesel_models::price_components::PriceComponentRow;
use diesel_models::product_families::ProductFamilyRow;
use diesel_models::tenants::TenantRow;
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
    ) -> StoreResult<FullPlan>;

    async fn find_plan_by_external_id_and_status(
        &self,
        external_id: &str,
        auth_tenant_id: Uuid,
        is_draft: Option<bool>,
    ) -> StoreResult<Option<FullPlan>>;

    async fn list_plans(
        &self,
        auth_tenant_id: Uuid,
        search: Option<String>,
        product_family_external_id: Option<String>,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<PlanForList>>;
    async fn list_latest_published_plan_versions(
        &self,
        auth_tenant_id: Uuid,
    ) -> StoreResult<Vec<PlanVersionLatest>>;
    async fn get_plan_version_by_id(
        &self,
        id: Uuid,
        auth_tenant_id: Uuid,
    ) -> StoreResult<PlanVersion>;
    async fn list_plan_versions(
        &self,
        plan_id: Uuid,
        auth_tenant_id: Uuid,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<PlanVersion>>;
    async fn copy_plan_version_to_draft(
        &self,
        plan_version_id: Uuid,
        auth_tenant_id: Uuid,
        auth_actor: Uuid,
    ) -> StoreResult<PlanVersion>;

    async fn publish_plan_version(
        &self,
        plan_version_id: Uuid,
        auth_tenant_id: Uuid,
        auth_actor: Uuid,
    ) -> StoreResult<PlanVersion>;

    async fn get_last_published_plan_version(
        &self,
        plan_id: Uuid,
        auth_tenant_id: Uuid,
    ) -> StoreResult<Option<PlanVersion>>;

    async fn discard_draft_plan_version(
        &self,
        plan_version_id: Uuid,
        auth_tenant_id: Uuid,
        auth_actor: Uuid,
    ) -> StoreResult<()>;

    async fn patch_published_plan(&self, patch: PlanPatch) -> StoreResult<PlanWithVersion>;

    async fn get_plan_with_version_by_external_id(
        &self,
        external_id: &str,
        auth_tenant_id: Uuid,
    ) -> StoreResult<PlanWithVersion>;

    async fn patch_draft_plan(&self, patch: PlanAndVersionPatch) -> StoreResult<PlanWithVersion>;
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

        let product_family = ProductFamilyRow::find_by_external_id_and_tenant_id(
            &mut conn,
            plan.product_family_external_id.as_str(),
            plan.tenant_id,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        let tenant = TenantRow::find_by_id(&mut conn, plan.tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let res = conn
            .transaction(|conn| {
                async move {
                    let plan_to_insert: PlanRowNew = plan.into_raw(product_family.id);
                    let inserted: Plan = plan_to_insert
                        .insert(conn)
                        .await
                        .map(Into::into)
                        .map_err(|err| StoreError::DatabaseError(err.error))?;

                    let plan_version_to_insert: PlanVersionRowNew = PlanVersionNew {
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
                    let inserted_price_components = PriceComponentRow::insert_batch(
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
                    .map_err(StoreError::TransactionStoreError)?;

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
    ) -> StoreResult<FullPlan> {
        let mut conn = self.get_conn().await?;

        let plan: Plan =
            PlanRow::get_by_external_id_and_tenant_id(&mut conn, external_id, auth_tenant_id)
                .await
                .map(Into::into)
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let version: PlanVersion =
            PlanVersionRow::get_latest_by_plan_id_and_tenant_id(&mut conn, plan.id, auth_tenant_id)
                .await
                .map(Into::into)
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let price_components: Vec<PriceComponent> =
            PriceComponentRow::list_by_plan_version_id(&mut conn, auth_tenant_id, version.id)
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

    async fn find_plan_by_external_id_and_status(
        &self,
        external_id: &str,
        auth_tenant_id: Uuid,
        is_draft: Option<bool>,
    ) -> StoreResult<Option<FullPlan>> {
        let mut conn = self.get_conn().await?;

        let plan: Plan =
            PlanRow::get_by_external_id_and_tenant_id(&mut conn, external_id, auth_tenant_id)
                .await
                .map(Into::into)
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let version: Option<PlanVersion> = PlanVersionRow::find_latest_by_plan_id_and_tenant_id(
            &mut conn,
            plan.id,
            auth_tenant_id,
            is_draft,
        )
        .await
        .map(|opt| opt.map(Into::into))
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        match version {
            Some(version) => {
                let price_components: Vec<PriceComponent> =
                    PriceComponentRow::list_by_plan_version_id(
                        &mut conn,
                        auth_tenant_id,
                        version.id,
                    )
                    .await
                    .map_err(|err| StoreError::DatabaseError(err.error))?
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(Some(FullPlan {
                    plan,
                    version,
                    price_components,
                }))
            }
            None => Ok(None),
        }
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

        let rows = PlanRowForList::list(
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

    async fn list_latest_published_plan_versions(
        &self,
        auth_tenant_id: Uuid,
    ) -> StoreResult<Vec<PlanVersionLatest>> {
        let mut conn = self.get_conn().await?;

        PlanVersionRowLatest::list(&mut conn, auth_tenant_id)
            .await
            .map_err(Into::into)
            .map(|x| x.into_iter().map(Into::into).collect())
    }

    async fn get_plan_version_by_id(
        &self,
        id: Uuid,
        auth_tenant_id: Uuid,
    ) -> StoreResult<PlanVersion> {
        let mut conn = self.get_conn().await?;
        PlanVersionRow::find_by_id_and_tenant_id(&mut conn, id, auth_tenant_id)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    async fn list_plan_versions(
        &self,
        plan_id: Uuid,
        auth_tenant_id: Uuid,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<PlanVersion>> {
        let mut conn = self.get_conn().await?;

        let rows = PlanVersionRow::list_by_plan_id_and_tenant_id(
            &mut conn,
            plan_id,
            auth_tenant_id,
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<PlanVersion> = PaginatedVec {
            items: rows.items.into_iter().map(Into::into).collect(),
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        };

        Ok(res)
    }

    async fn copy_plan_version_to_draft(
        &self,
        plan_version_id: Uuid,
        auth_tenant_id: Uuid,
        auth_actor: Uuid,
    ) -> StoreResult<PlanVersion> {
        self.transaction(|conn| {
            async move {
                let original =
                    PlanVersionRow::find_by_id_and_tenant_id(conn, plan_version_id, auth_tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                PlanVersionRow::delete_others_draft(
                    conn,
                    original.id,
                    original.plan_id,
                    original.tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                let new = PlanVersionRowNew {
                    id: Uuid::now_v7(),
                    is_draft_version: true,
                    plan_id: original.plan_id,
                    version: original.version + 1,
                    trial_duration_days: original.trial_duration_days,
                    trial_fallback_plan_id: original.trial_fallback_plan_id,
                    tenant_id: original.tenant_id,
                    period_start_day: original.period_start_day,
                    net_terms: original.net_terms,
                    currency: original.currency,
                    billing_cycles: original.billing_cycles,
                    created_by: auth_actor,
                    billing_periods: original.billing_periods,
                }
                .insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                PriceComponentRow::clone_all(conn, original.id, new.id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                diesel_models::schedules::ScheduleRow::clone_all(conn, original.id, new.id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(new.into())
            }
            .scope_boxed()
        })
        .await
    }

    async fn publish_plan_version(
        &self,
        plan_version_id: Uuid,
        auth_tenant_id: Uuid,
        auth_actor: Uuid,
    ) -> StoreResult<PlanVersion> {
        let res = self
            .transaction(|conn| {
                async move {
                    // TODO validations
                    // - all components on committed must have values for all periods
                    let published = PlanVersionRow::publish(conn, plan_version_id, auth_tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    PlanRow::activate(conn, published.plan_id, auth_tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    Ok(published.into())
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::plan_published_version(
                auth_actor,
                plan_version_id,
                auth_tenant_id,
            ))
            .await;

        Ok(res)
    }

    async fn get_last_published_plan_version(
        &self,
        plan_id: Uuid,
        auth_tenant_id: Uuid,
    ) -> StoreResult<Option<PlanVersion>> {
        let mut conn = self.get_conn().await?;
        PlanVersionRow::find_latest_by_plan_id_and_tenant_id(
            &mut conn,
            plan_id,
            auth_tenant_id,
            Some(false),
        )
        .await
        .map(|opt| opt.map(Into::into))
        .map_err(Into::into)
    }

    async fn discard_draft_plan_version(
        &self,
        plan_version_id: Uuid,
        auth_tenant_id: Uuid,
        auth_actor: Uuid,
    ) -> StoreResult<()> {
        let res = self
            .transaction(|conn| {
                async move {
                    let original = PlanVersionRow::find_by_id_and_tenant_id(
                        conn,
                        plan_version_id,
                        auth_tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    PlanVersionRow::delete_draft(conn, plan_version_id, auth_tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    PlanRow::delete(conn, original.plan_id, auth_tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    Ok(())
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::plan_discarded_version(
                auth_actor,
                plan_version_id,
                auth_tenant_id,
            ))
            .await;

        Ok(res)
    }

    async fn patch_published_plan(&self, patch: PlanPatch) -> StoreResult<PlanWithVersion> {
        let mut conn = self.get_conn().await?;

        let patch: PlanRowPatch = patch.into();

        let plan = patch
            .update(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        PlanRow::get_with_version_by_external_id(
            &mut conn,
            plan.external_id.as_str(),
            plan.tenant_id,
        )
        .await
        .map_err(Into::into)
        .map(Into::into)
    }

    async fn get_plan_with_version_by_external_id(
        &self,
        external_id: &str,
        auth_tenant_id: Uuid,
    ) -> StoreResult<PlanWithVersion> {
        let mut conn = self.get_conn().await?;

        PlanRow::get_with_version_by_external_id(&mut conn, external_id, auth_tenant_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn patch_draft_plan(&self, patch: PlanAndVersionPatch) -> StoreResult<PlanWithVersion> {
        let mut conn = self.get_conn().await?;

        let version = self
            .transaction(|conn| {
                async move {
                    let patch_version: PlanVersionRowPatch = patch.version.into();

                    let patched_version = patch_version
                        .update_draft(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let patch_plan: PlanRowPatch = PlanPatch {
                        id: patched_version.plan_id,
                        tenant_id: patched_version.tenant_id,
                        name: patch.name,
                        description: patch.description,
                    }
                    .into();

                    patch_plan
                        .update(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    Ok(patched_version)
                }
                .scope_boxed()
            })
            .await?;

        PlanRow::get_with_version(&mut conn, version.id, version.tenant_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }
}
