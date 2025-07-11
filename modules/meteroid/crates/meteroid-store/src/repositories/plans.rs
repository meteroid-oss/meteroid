use crate::StoreResult;
use crate::store::Store;

use crate::domain::{
    FullPlan, FullPlanNew, OrderByRequest, PaginatedVec, PaginationRequest, Plan,
    PlanAndVersionPatch, PlanFilters, PlanOverview, PlanPatch, PlanVersion, PlanVersionFilter,
    PlanVersionNew, PlanWithVersion, PriceComponent, PriceComponentNew, TrialPatch,
};
use crate::errors::StoreError;
use common_domain::ids::{BaseId, PlanId, PlanVersionId, ProductFamilyId, TenantId};
use common_eventbus::Event;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::plan_versions::{
    PlanVersionRow, PlanVersionRowNew, PlanVersionRowPatch, PlanVersionTrialRowPatch,
};
use diesel_models::plans::{PlanRow, PlanRowNew, PlanRowOverview, PlanRowPatch};
use diesel_models::price_components::PriceComponentRow;
use diesel_models::product_families::ProductFamilyRow;
use diesel_models::tenants::TenantRow;
use error_stack::Report;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait PlansInterface {
    async fn insert_plan(&self, plan: FullPlanNew) -> StoreResult<FullPlan>;

    async fn get_plan(
        &self,
        id: PlanId,
        auth_tenant_id: TenantId,
        version_filter: PlanVersionFilter,
    ) -> StoreResult<PlanWithVersion>;

    async fn get_plan_by_version_id(
        &self,
        id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<PlanWithVersion>;
    /**
     * Details of a plan irrespective of version
     */
    async fn get_plan_overview(
        &self,
        id: PlanId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<PlanOverview>;

    /**
     * Find a plan by local id and version, including pricing components
     */
    async fn get_detailed_plan(
        &self,
        id: PlanId,
        auth_tenant_id: TenantId,
        version_filter: PlanVersionFilter,
    ) -> StoreResult<FullPlan>;

    async fn list_plans(
        &self,
        auth_tenant_id: TenantId,
        product_family_id: Option<ProductFamilyId>,
        filters: PlanFilters,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<PlanOverview>>;

    async fn get_plan_version_by_id(
        &self,
        id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<PlanVersion>;

    async fn list_plan_versions(
        &self,
        plan_id: PlanId,
        auth_tenant_id: TenantId,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<PlanVersion>>;

    async fn copy_plan_version_to_draft(
        &self,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
        auth_actor: Uuid,
    ) -> StoreResult<PlanVersion>;

    async fn publish_plan_version(
        &self,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
        auth_actor: Uuid,
    ) -> StoreResult<PlanVersion>;

    async fn discard_draft_plan_version(
        &self,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
        auth_actor: Uuid,
    ) -> StoreResult<()>;

    async fn patch_published_plan(&self, patch: PlanPatch) -> StoreResult<PlanOverview>;

    async fn patch_draft_plan(&self, patch: PlanAndVersionPatch) -> StoreResult<PlanWithVersion>;

    async fn patch_trial(&self, patch: TrialPatch) -> StoreResult<PlanWithVersion>;
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
            ProductFamilyRow::find_by_id(&mut conn, plan.product_family_id, plan.tenant_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let tenant = TenantRow::find_by_id(&mut conn, plan.tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let res = self
            .transaction_with(&mut conn, |conn| {
                async move {
                    let plan_to_insert: PlanRowNew = plan.into_raw(product_family.id);
                    let inserted: Plan = plan_to_insert
                        .insert(conn)
                        .await
                        .map(Into::into)
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let plan_version_to_insert: PlanVersionRowNew = PlanVersionNew {
                        tenant_id: inserted.tenant_id,
                        internal: version,
                        plan_id: inserted.id,
                        version: 1,
                        created_by: inserted.created_by,
                    }
                    // TODO parameter
                    .into_raw(tenant.reporting_currency);

                    let inserted_plan_version_new: PlanVersion = plan_version_to_insert
                        .insert(conn)
                        .await
                        .map(Into::into)
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let (active_version_id, draft_version_id) =
                        match inserted_plan_version_new.is_draft_version {
                            true => (None, Some(Some(inserted_plan_version_new.id))),
                            false => (Some(Some(inserted_plan_version_new.id)), None),
                        };

                    let updated: Plan = PlanRowPatch {
                        id: inserted.id,
                        tenant_id: inserted.tenant_id,
                        name: None,
                        description: None,
                        active_version_id,
                        draft_version_id,
                    }
                    .update(conn)
                    .await
                    .map(Into::into)
                    .map_err(Into::<Report<StoreError>>::into)?;

                    // insert price component as batch, etc
                    let inserted_price_components = PriceComponentRow::insert_batch(
                        conn,
                        price_components
                            .into_iter()
                            .map(|p| {
                                PriceComponentNew {
                                    plan_version_id: inserted_plan_version_new.id,
                                    name: p.name,
                                    product_id: p.product_id,
                                    fee: p.fee,
                                }
                                .try_into()
                            })
                            .collect::<error_stack::Result<Vec<_>, _>>()?,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<error_stack::Result<Vec<_>, _>>()?;

                    Ok(FullPlan {
                        price_components: inserted_price_components,
                        plan: updated,
                        version: inserted_plan_version_new,
                    })
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::plan_created_draft(
                res.plan.created_by,
                res.version.id.as_uuid(),
                res.plan.tenant_id.as_uuid(),
            ))
            .await;

        Ok(res)
    }

    async fn get_plan(
        &self,
        id: PlanId,
        auth_tenant_id: TenantId,
        version_filter: PlanVersionFilter,
    ) -> StoreResult<PlanWithVersion> {
        let mut conn = self.get_conn().await?;

        PlanRow::get_with_version_by_id(&mut conn, id, auth_tenant_id, version_filter.into())
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn get_plan_by_version_id(
        &self,
        id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<PlanWithVersion> {
        let mut conn = self.get_conn().await?;

        PlanRow::get_with_version_by_version_id(&mut conn, id, auth_tenant_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn get_plan_overview(
        &self,
        id: PlanId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<PlanOverview> {
        let mut conn = self.get_conn().await?;

        PlanRow::get_overview_by_id(&mut conn, id, auth_tenant_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn get_detailed_plan(
        &self,
        id: PlanId,
        auth_tenant_id: TenantId,
        version_filter: PlanVersionFilter,
    ) -> StoreResult<FullPlan> {
        let mut conn = self.get_conn().await?;

        let plan_with_version: PlanWithVersion =
            PlanRow::get_with_version_by_id(&mut conn, id, auth_tenant_id, version_filter.into())
                .await
                .map(Into::into)
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        match plan_with_version.version {
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

                Ok(FullPlan {
                    plan: plan_with_version.plan,
                    version,
                    price_components,
                })
            }
            None => {
                Err(StoreError::ValueNotFound("Plan version was not resolved".to_string()).into())
            }
        }
    }

    async fn list_plans(
        &self,
        auth_tenant_id: TenantId,
        product_family_id: Option<ProductFamilyId>,
        filters: PlanFilters,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<PlanOverview>> {
        let mut conn = self.get_conn().await?;

        let rows = PlanRowOverview::list(
            &mut conn,
            auth_tenant_id,
            product_family_id,
            filters.into(),
            pagination.into(),
            order_by.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<PlanOverview> = PaginatedVec {
            items: rows.items.into_iter().map(Into::into).collect(),
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        };

        Ok(res)
    }

    async fn get_plan_version_by_id(
        &self,
        id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<PlanVersion> {
        let mut conn = self.get_conn().await?;
        PlanVersionRow::find_by_id_and_tenant_id(&mut conn, id, auth_tenant_id)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    async fn list_plan_versions(
        &self,
        plan_id: PlanId,
        auth_tenant_id: TenantId,
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
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
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
                    id: PlanVersionId::new(),
                    is_draft_version: true,
                    plan_id: original.plan_id,
                    version: original.version + 1,
                    trial_duration_days: original.trial_duration_days,
                    downgrade_plan_id: original.downgrade_plan_id,
                    trialing_plan_id: original.trialing_plan_id,
                    action_after_trial: original.action_after_trial,
                    trial_is_free: original.trial_is_free,
                    tenant_id: original.tenant_id,
                    period_start_day: original.period_start_day,
                    net_terms: original.net_terms,
                    currency: original.currency,
                    billing_cycles: original.billing_cycles,
                    created_by: auth_actor,
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

                PlanRowPatch {
                    id: original.plan_id,
                    tenant_id: original.tenant_id,
                    name: None,
                    description: None,
                    active_version_id: None,
                    draft_version_id: Some(Some(new.id)),
                }
                .update(conn)
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
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
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

                    PlanRowPatch {
                        id: published.plan_id,
                        tenant_id: published.tenant_id,
                        name: None,
                        description: None,
                        active_version_id: Some(Some(published.id)),
                        draft_version_id: Some(None),
                    }
                    .update(conn)
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
                plan_version_id.as_uuid(),
                auth_tenant_id.as_uuid(),
            ))
            .await;

        Ok(res)
    }

    async fn discard_draft_plan_version(
        &self,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
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

                    PlanRowPatch {
                        id: original.plan_id,
                        tenant_id: original.tenant_id,
                        name: None,
                        description: None,
                        active_version_id: None,
                        draft_version_id: Some(None),
                    }
                    .update(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    PlanVersionRow::delete_draft(conn, plan_version_id, auth_tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    // only deletes if no versions left
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
                plan_version_id.as_uuid(),
                auth_tenant_id.as_uuid(),
            ))
            .await;

        Ok(res)
    }

    async fn patch_published_plan(&self, patch: PlanPatch) -> StoreResult<PlanOverview> {
        let mut conn = self.get_conn().await?;

        let patch: PlanRowPatch = patch.into();

        let plan = patch
            .update(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        PlanRow::get_overview_by_id(&mut conn, plan.id, plan.tenant_id)
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
                        active_version_id: None,
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

    async fn patch_trial(&self, patch: TrialPatch) -> StoreResult<PlanWithVersion> {
        let mut conn = self.get_conn().await?;

        let version = self
            .transaction(|conn| {
                async move {
                    let patch: PlanVersionTrialRowPatch = match patch.trial {
                        None => PlanVersionTrialRowPatch {
                            id: patch.plan_version_id,
                            tenant_id: patch.tenant_id,
                            trialing_plan_id: Some(None),
                            action_after_trial: Some(None),
                            trial_is_free: Some(false),
                            trial_duration_days: Some(None),
                            downgrade_plan_id: Some(None),
                        },
                        Some(trial) => PlanVersionTrialRowPatch {
                            id: patch.plan_version_id,
                            tenant_id: patch.tenant_id,
                            trialing_plan_id: Some(trial.trialing_plan_id),
                            action_after_trial: Some(trial.action_after_trial.map(Into::into)),
                            trial_is_free: Some(trial.require_pre_authorization),
                            trial_duration_days: Some(Some(trial.duration_days as i32)),
                            downgrade_plan_id: Some(trial.downgrade_plan_id),
                        },
                    };

                    let patched_version = patch
                        .update_trial(conn)
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
