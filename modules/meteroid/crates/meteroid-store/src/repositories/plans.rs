use crate::StoreResult;
use crate::store::{PgConn, Store};

use crate::domain::price_components::FeeType;
use crate::domain::prices::{
    LegacyPricingData, extract_fee_structure, extract_legacy_pricing, extract_pricing,
};
use crate::domain::{
    FullPlan, FullPlanNew, OrderByRequest, PaginatedVec, PaginationRequest, Plan,
    PlanAndVersionPatch, PlanFilters, PlanOverview, PlanPatch, PlanStatusEnum, PlanTypeEnum,
    PlanVersion, PlanVersionFilter, PlanVersionNew, PlanWithVersion, Price, PriceComponent,
    PriceComponentNew, Product, ProductFamilyOverview, TrialPatch,
};
use crate::errors::StoreError;
use crate::repositories::price_components::resolve_component_internal;
use common_domain::ids::PriceId;
use common_domain::ids::{
    BaseId, PlanId, PlanVersionId, PriceComponentId, ProductFamilyId, ProductId, TenantId,
};
use common_eventbus::Event;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::plan_component_prices::{PlanComponentPriceRow, PlanComponentPriceRowNew};
use diesel_models::plan_versions::{
    PlanVersionRow, PlanVersionRowNew, PlanVersionRowPatch, PlanVersionTrialRowPatch,
};
use diesel_models::plans::{FullPlanRow, PlanRow, PlanRowNew, PlanRowOverview, PlanRowPatch};
use diesel_models::price_components::PriceComponentRow;
use diesel_models::prices::{PriceRow, PriceRowNew};
use diesel_models::product_families::ProductFamilyRow;
use diesel_models::products::{ProductRow, ProductRowNew};
use diesel_models::tenants::TenantRow;
use error_stack::Report;
use std::collections::HashMap;
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

    async fn get_full_plan(
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

    async fn list_full_plans(
        &self,
        auth_tenant_id: TenantId,
        product_family_id: Option<ProductFamilyId>,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<FullPlan>>;

    async fn get_plan_version_by_id(
        &self,
        id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<PlanVersion>;

    async fn resolve_published_version_id(
        &self,
        plan_id: PlanId,
        plan_version: Option<i32>,
        auth_tenant_id: TenantId,
    ) -> StoreResult<PlanVersionId>;

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
    async fn archive_plan(&self, id: PlanId, auth_tenant_id: TenantId) -> StoreResult<()>;
    async fn unarchive_plan(&self, id: PlanId, auth_tenant_id: TenantId) -> StoreResult<()>;
}

/// Convert a FullPlanRow into a FullPlan with prices and products loaded.
async fn convert_full_plan_row(
    conn: &mut PgConn,
    row: FullPlanRow,
    tenant_id: TenantId,
) -> StoreResult<FullPlan> {
    let plan: Plan = row.plan.into();
    let version: PlanVersion = row.version.into();
    let product_family: ProductFamilyOverview = row.product_family.into();

    let mut price_components: Vec<PriceComponent> = row
        .price_components
        .into_iter()
        .map(|v| v.try_into())
        .collect::<StoreResult<Vec<_>>>()?;

    // Load v2 prices
    let component_ids: Vec<PriceComponentId> = price_components.iter().map(|c| c.id).collect();
    let mut prices_by_component: HashMap<PriceComponentId, Vec<Price>> = HashMap::new();

    if !component_ids.is_empty() {
        let pcp_rows = PlanComponentPriceRow::list_by_component_ids(conn, &component_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if !pcp_rows.is_empty() {
            let price_ids: Vec<PriceId> = pcp_rows.iter().map(|pcp| pcp.price_id).collect();
            let price_rows = PriceRow::list_by_ids(conn, &price_ids, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            let prices_by_id: HashMap<PriceId, Price> = price_rows
                .into_iter()
                .map(|r| {
                    let id = r.id;
                    Price::try_from(r).map(|p| (id, p))
                })
                .collect::<Result<HashMap<_, _>, _>>()?;

            for pcp in &pcp_rows {
                if let Some(price) = prices_by_id.get(&pcp.price_id) {
                    prices_by_component
                        .entry(pcp.plan_component_id)
                        .or_default()
                        .push(price.clone());
                }
            }
        }
    }

    // Extract legacy pricing for v1 components
    let has_v1 = price_components
        .iter()
        .any(|c| !prices_by_component.contains_key(&c.id) && c.product_id.is_none());

    // For legacy components, we need to read legacy_fee from the raw rows
    // Since FullPlanRow gave us PriceComponentRows that we already converted,
    // we re-read legacy data from the DB for v1 components
    let mut legacy_by_component: HashMap<PriceComponentId, LegacyPricingData> = HashMap::new();
    if has_v1 {
        let v1_ids: Vec<PriceComponentId> = price_components
            .iter()
            .filter(|c| !prices_by_component.contains_key(&c.id) && c.product_id.is_none())
            .map(|c| c.id)
            .collect();

        if !v1_ids.is_empty() {
            let raw_rows = PriceComponentRow::list_by_plan_version_id(conn, tenant_id, version.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            for raw_row in &raw_rows {
                if v1_ids.contains(&raw_row.id)
                    && let Some(legacy_json) = &raw_row.legacy_fee
                {
                    let legacy = extract_legacy_pricing(legacy_json, version.currency.clone())?;
                    legacy_by_component.insert(raw_row.id, legacy);
                }
            }
        }
    }

    // Attach prices and legacy data
    for comp in &mut price_components {
        if let Some(prices) = prices_by_component.remove(&comp.id) {
            comp.prices = prices;
        }
        if let Some(legacy) = legacy_by_component.remove(&comp.id) {
            comp.legacy_pricing = Some(legacy);
        }
    }

    // Load products referenced by components
    let product_ids: Vec<ProductId> = price_components
        .iter()
        .filter_map(|c| c.product_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let products: HashMap<ProductId, Product> = if product_ids.is_empty() {
        HashMap::new()
    } else {
        ProductRow::list_by_ids(conn, &product_ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|r| Product::try_from(r).map(|p| (p.id, p)))
            .collect::<Result<HashMap<_, _>, _>>()?
    };

    Ok(FullPlan {
        plan,
        version,
        price_components,
        product_family,
        products,
    })
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

        let product_family: ProductFamilyOverview =
            ProductFamilyRow::find_by_id(&mut conn, plan.product_family_id, plan.tenant_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?
                .into();

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
                        if inserted_plan_version_new.is_draft_version {
                            (None, Some(Some(inserted_plan_version_new.id)))
                        } else {
                            (Some(Some(inserted_plan_version_new.id)), None)
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

                    // Insert price components via resolve_component_internal
                    let mut all_inserted_components = Vec::new();

                    for p in &price_components {
                        let (product_id, price_ids) = resolve_component_internal(
                            conn,
                            p,
                            inserted.tenant_id,
                            inserted.created_by,
                            product_family.id,
                            &inserted_plan_version_new.currency,
                        )
                        .await?;

                        let row_new: diesel_models::price_components::PriceComponentRowNew =
                            PriceComponentNew {
                                plan_version_id: inserted_plan_version_new.id,
                                name: p.name.clone(),
                                product_id: Some(product_id),
                            }
                            .try_into()?;

                        let inserted_row = PriceComponentRow::insert(conn, row_new)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                        // Create plan_component_price join rows
                        if !price_ids.is_empty() {
                            let pcp_rows: Vec<PlanComponentPriceRowNew> = price_ids
                                .iter()
                                .map(|pid| PlanComponentPriceRowNew {
                                    plan_component_id: inserted_row.id,
                                    price_id: *pid,
                                })
                                .collect();
                            PlanComponentPriceRowNew::insert_batch(conn, &pcp_rows)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                        }

                        let comp: crate::domain::PriceComponent = inserted_row.try_into()?;
                        all_inserted_components.push(comp);
                    }

                    let inserted_price_components = all_inserted_components;

                    Ok(FullPlan {
                        price_components: inserted_price_components,
                        plan: updated,
                        version: inserted_plan_version_new,
                        product_family,
                        products: HashMap::new(),
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

    async fn get_full_plan(
        &self,
        id: PlanId,
        auth_tenant_id: TenantId,
        version_filter: PlanVersionFilter,
    ) -> StoreResult<FullPlan> {
        let mut conn = self.get_conn().await?;

        let row = FullPlanRow::get_by_id(&mut conn, id, auth_tenant_id, version_filter.into())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        convert_full_plan_row(&mut conn, row, auth_tenant_id).await
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

    async fn list_full_plans(
        &self,
        auth_tenant_id: TenantId,
        product_family_id: Option<ProductFamilyId>,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<FullPlan>> {
        let mut conn = self.get_conn().await?;

        let rows = FullPlanRow::list(
            &mut conn,
            auth_tenant_id,
            product_family_id,
            PlanFilters {
                filter_status: vec![PlanStatusEnum::Active],
                filter_type: vec![PlanTypeEnum::Free, PlanTypeEnum::Standard],
                search: None,
                filter_currency: None,
            }
            .into(),
            pagination.into(),
            order_by.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let mut items = Vec::new();
        for row in rows.items {
            items.push(convert_full_plan_row(&mut conn, row, auth_tenant_id).await?);
        }

        Ok(PaginatedVec {
            items,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
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

    async fn resolve_published_version_id(
        &self,
        plan_id: PlanId,
        plan_version: Option<i32>,
        auth_tenant_id: TenantId,
    ) -> StoreResult<PlanVersionId> {
        let mut conn = self.get_conn().await?;
        PlanVersionRow::resolve_published_version_id(
            &mut conn,
            plan_id,
            plan_version,
            auth_tenant_id,
        )
        .await
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

                // Clear draft_version_id before deleting old drafts to avoid FK violation
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

                PlanVersionRow::delete_others_draft(
                    conn,
                    original.id,
                    original.plan_id,
                    original.tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                let original_currency = original.currency.clone();
                let original_uses_product_pricing = original.uses_product_pricing;

                let new = PlanVersionRowNew {
                    id: PlanVersionId::new(),
                    is_draft_version: true,
                    plan_id: original.plan_id,
                    version: original.version + 1,
                    trial_duration_days: original.trial_duration_days,
                    trialing_plan_id: original.trialing_plan_id,
                    trial_is_free: original.trial_is_free,
                    tenant_id: original.tenant_id,
                    period_start_day: original.period_start_day,
                    net_terms: original.net_terms,
                    currency: original.currency,
                    billing_cycles: original.billing_cycles,
                    created_by: auth_actor,
                    uses_product_pricing: true,
                }
                .insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                // Fetch source components before cloning (need the ID mapping)
                let src_components =
                    PriceComponentRow::list_by_plan_version_id(conn, auth_tenant_id, original.id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                PriceComponentRow::clone_all(conn, original.id, new.id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Fetch destination components once for reuse below
                let dst_components =
                    PriceComponentRow::list_by_plan_version_id(conn, auth_tenant_id, new.id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                // Clone plan_component_price entries: map old→new component IDs by matching on (name, product_id)
                let src_component_ids: Vec<_> = src_components.iter().map(|c| c.id).collect();
                let src_pcp_rows =
                    PlanComponentPriceRow::list_by_component_ids(conn, &src_component_ids)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                if !src_pcp_rows.is_empty() {
                    // Build mapping: (name, product_id) → new component id
                    let dst_map: std::collections::HashMap<_, _> = dst_components
                        .iter()
                        .map(|c| ((c.name.clone(), c.product_id), c.id))
                        .collect();

                    let new_pcp_rows: Vec<PlanComponentPriceRowNew> = src_pcp_rows
                        .iter()
                        .filter_map(|pcp| {
                            let src_comp = src_components
                                .iter()
                                .find(|c| c.id == pcp.plan_component_id)?;
                            let dst_id =
                                dst_map.get(&(src_comp.name.clone(), src_comp.product_id))?;
                            Some(PlanComponentPriceRowNew {
                                plan_component_id: *dst_id,
                                price_id: pcp.price_id,
                            })
                        })
                        .collect();

                    if !new_pcp_rows.is_empty() {
                        PlanComponentPriceRowNew::insert_batch(conn, &new_pcp_rows)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }
                }

                // Auto-upgrade v1 components to v2 when source plan didn't use product pricing
                if !original_uses_product_pricing {
                    let plan_with_version =
                        PlanRow::get_with_version(conn, original.id, auth_tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    let product_family_id = plan_with_version.plan.product_family_id;

                    let dst_component_ids: Vec<_> = dst_components.iter().map(|c| c.id).collect();
                    let existing_pcp =
                        PlanComponentPriceRow::list_by_component_ids(conn, &dst_component_ids)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    let components_with_pcp: std::collections::HashSet<_> =
                        existing_pcp.iter().map(|p| p.plan_component_id).collect();

                    let mut upgrade_pcp_rows = Vec::new();
                    for comp in &dst_components {
                        if components_with_pcp.contains(&comp.id) {
                            continue;
                        }

                        if let Some(ref legacy_fee_value) = comp.legacy_fee {
                            let fee: FeeType = serde_json::from_value(legacy_fee_value.clone())
                                .map_err(|e| {
                                    Report::new(StoreError::SerdeError(
                                        "Failed to parse legacy_fee".to_string(),
                                        e,
                                    ))
                                })?;

                            let product_id = if let Some(pid) = comp.product_id {
                                pid
                            } else {
                                let (fee_type_enum, fee_structure) = extract_fee_structure(&fee);
                                let product_row = ProductRowNew {
                                    id: ProductId::new(),
                                    name: comp.name.clone(),
                                    description: None,
                                    created_by: auth_actor,
                                    tenant_id: auth_tenant_id,
                                    product_family_id,
                                    fee_type: fee_type_enum.into(),
                                    fee_structure: serde_json::to_value(&fee_structure).map_err(
                                        |e| {
                                            Report::new(StoreError::SerdeError(
                                                "Failed to serialize fee_structure".to_string(),
                                                e,
                                            ))
                                        },
                                    )?,
                                }
                                .insert(conn)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;

                                // Update the component's product_id
                                let mut updated_comp = comp.clone();
                                updated_comp.product_id = Some(product_row.id);
                                updated_comp
                                    .update(conn, auth_tenant_id)
                                    .await
                                    .map_err(Into::<Report<StoreError>>::into)?;

                                product_row.id
                            };

                            let pricing_list = extract_pricing(&fee);
                            for (cadence, pricing) in &pricing_list {
                                let price_row = PriceRowNew {
                                    id: PriceId::new(),
                                    product_id,
                                    cadence: (*cadence).into(),
                                    currency: original_currency.clone(),
                                    pricing: serde_json::to_value(pricing).map_err(|e| {
                                        Report::new(StoreError::SerdeError(
                                            "Failed to serialize pricing".to_string(),
                                            e,
                                        ))
                                    })?,
                                    tenant_id: auth_tenant_id,
                                    created_by: auth_actor,
                                }
                                .insert(conn)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;

                                upgrade_pcp_rows.push(PlanComponentPriceRowNew {
                                    plan_component_id: comp.id,
                                    price_id: price_row.id,
                                });
                            }
                        }
                    }

                    if !upgrade_pcp_rows.is_empty() {
                        PlanComponentPriceRowNew::insert_batch(conn, &upgrade_pcp_rows)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }
                }

                diesel_models::schedules::ScheduleRow::clone_all(conn, original.id, new.id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                diesel_models::plan_version_add_ons::PlanVersionAddOnRow::clone_all(
                    conn,
                    original.id,
                    new.id,
                )
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
                            trial_is_free: Some(false),
                            trial_duration_days: Some(None),
                        },
                        Some(trial) => PlanVersionTrialRowPatch {
                            id: patch.plan_version_id,
                            tenant_id: patch.tenant_id,
                            trialing_plan_id: Some(trial.trialing_plan_id),
                            trial_is_free: Some(trial.trial_is_free),
                            trial_duration_days: Some(Some(trial.duration_days as i32)),
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

    async fn archive_plan(&self, id: PlanId, auth_tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        PlanRow::archive(&mut conn, id, auth_tenant_id)
            .await
            .map_err(Into::into)
    }

    async fn unarchive_plan(&self, id: PlanId, auth_tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        PlanRow::unarchive(&mut conn, id, auth_tenant_id)
            .await
            .map_err(Into::into)
    }
}
