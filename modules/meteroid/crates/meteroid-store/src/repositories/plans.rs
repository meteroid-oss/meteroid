use crate::StoreResult;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::store::{PgConn, Store};

use crate::domain::credit_exhausted::CreditExhaustedRule;
use crate::domain::entitlements::Entitlement;
use crate::domain::price_components::FeeType;
use crate::domain::prices::{
    LegacyPricingData, extract_fee_structure, extract_legacy_pricing, extract_pricing,
};
use crate::domain::{
    FullPlan, FullPlanNew, PaginatedVec, PaginationRequest, Plan, PlanAndVersionPatch, PlanFilters,
    PlanOverview, PlanPatch, PlanStatusEnum, PlanVersion, PlanVersionFilter, PlanVersionNew,
    PlanVersionNewInternal, PlanWithVersion, Price, PriceComponent, PriceComponentNew, Product,
    ProductFamilyOverview, SelfServicePlan, TrialPatch,
};
use crate::errors::StoreError;
use crate::hooks::USAGE_CONTROLS_UNAVAILABLE;
use crate::repositories::entitlements::insert_entitlement_specs;
use crate::repositories::price_components::resolve_component_internal;
use common_domain::ids::{
    BaseId, PlanId, PlanVersionId, PriceComponentId, ProductFamilyId, ProductId, TenantId,
};
use common_domain::ids::{EntitlementEntityId, PriceId};
use common_eventbus::Event;
use diesel_models::entitlements::EntitlementRow;
use diesel_models::plan_component_prices::{PlanComponentPriceRow, PlanComponentPriceRowNew};
use diesel_models::plan_versions::{
    PlanVersionRow, PlanVersionRowNew, PlanVersionRowPatch, PlanVersionTrialRowPatch,
};
use diesel_models::plans::{
    FullPlanRow, PlanRow, PlanRowNew, PlanRowOverview, PlanRowPatch, SelfServicePlanRow,
};
use diesel_models::price_components::PriceComponentRow;
use diesel_models::prices::{PriceRow, PriceRowNew};
use diesel_models::product_families::ProductFamilyRow;
use diesel_models::products::{ProductRow, ProductRowNew};
use diesel_models::tenants::TenantRow;
use error_stack::Report;
use meteroid_store_macros::with_conn_delegate;
use scoped_futures::ScopedFutureExt;
use std::collections::HashMap;

#[with_conn_delegate]
#[async_trait::async_trait]
pub trait PlansInterface {
    async fn insert_plan(&self, actor: Actor, plan: FullPlanNew) -> StoreResult<FullPlan>;

    async fn get_plan(
        &self,
        id: PlanId,
        auth_tenant_id: TenantId,
        version_filter: PlanVersionFilter,
    ) -> StoreResult<PlanWithVersion>;

    #[delegated]
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
        order_by: Option<String>,
    ) -> StoreResult<PaginatedVec<PlanOverview>>;

    async fn list_full_plans(
        &self,
        auth_tenant_id: TenantId,
        product_family_id: Option<ProductFamilyId>,
        filters: PlanFilters,
        pagination: PaginationRequest,
        order_by: Option<String>,
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
    ) -> StoreResult<PlanVersion>;

    async fn publish_plan_version(
        &self,
        actor: Actor,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<PlanVersion>;

    async fn discard_draft_plan_version(
        &self,
        actor: Actor,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<()>;

    async fn patch_published_plan(
        &self,
        actor: Actor,
        patch: PlanPatch,
    ) -> StoreResult<PlanOverview>;

    async fn patch_draft_plan(
        &self,
        actor: Actor,
        patch: PlanAndVersionPatch,
    ) -> StoreResult<PlanWithVersion>;

    async fn patch_trial(&self, actor: Actor, patch: TrialPatch) -> StoreResult<PlanWithVersion>;
    async fn archive_plan(
        &self,
        actor: Actor,
        id: PlanId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<()>;
    async fn unarchive_plan(&self, id: PlanId, auth_tenant_id: TenantId) -> StoreResult<()>;

    async fn list_self_service_plans(
        &self,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        currency: &str,
        exclude_plan_id: PlanId,
    ) -> StoreResult<Vec<SelfServicePlan>>;

    async fn list_published_versions_by_plan_ids(
        &self,
        plan_ids: Vec<PlanId>,
        auth_tenant_id: TenantId,
    ) -> StoreResult<Vec<PlanVersion>>;

    #[allow(clippy::too_many_arguments)]
    async fn replace_plan_version(
        &self,
        typed_actor: Actor,
        plan_id: PlanId,
        tenant_id: TenantId,
        name: String,
        description: Option<String>,
        version: PlanVersionNewInternal,
        price_components: Vec<crate::domain::price_components::PriceComponentNewInternal>,
        add_on_attachments: Vec<crate::domain::plan_version_add_ons::PlanVersionAddOnNew>,
        publish: bool,
        minimum: Option<crate::domain::plan_minimums::ReplacePlanMinimumInput>,
    ) -> StoreResult<FullPlan>;

    async fn get_plan_minimum(
        &self,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<Option<crate::domain::plan_minimums::PlanMinimum>>;

    /// Set or replace the plan-level minimum for a draft plan version. Errors if the
    /// version is published or if any listed component does not belong to the version.
    async fn set_plan_minimum(
        &self,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
        amount_subunit: i64,
        scope: crate::domain::plan_minimums::MinimumScope,
    ) -> StoreResult<crate::domain::plan_minimums::PlanMinimum>;

    async fn clear_plan_minimum(
        &self,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<()>;
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

    let entitlement_rows = EntitlementRow::list_by_entity(
        conn,
        tenant_id,
        EntitlementEntityId::PlanVersion(version.id),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    let entitlements: Vec<Entitlement> = entitlement_rows
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<_>, _>>()?;

    let mut version = version;
    version.entitlements = entitlements;
    version.minimum = load_plan_minimum(conn, version.id).await?;

    Ok(FullPlan {
        plan,
        version,
        price_components,
        product_family,
        products,
    })
}

/// Copy `plan_version_minimum` (and its product scope) from `src_id` to `dst_id`. The scope is
/// keyed by product, and products are shared across plan versions, so the product ids carry over
/// unchanged — we just keep the scoped products the destination version still has a component for.
/// If that leaves the scope empty, the minimum is upgraded to `AllComponents` rather than written
/// with an empty list (which would mean "nothing counts"), and a warning is logged for review.
pub(crate) async fn clone_plan_minimum(
    conn: &mut PgConn,
    src_id: PlanVersionId,
    dst_id: PlanVersionId,
    _src_components: &[diesel_models::price_components::PriceComponentRow],
    dst_components: &[diesel_models::price_components::PriceComponentRow],
) -> StoreResult<()> {
    use diesel_models::plan_version_adjustments::{
        PlanVersionAdjustmentComponentRow, PlanVersionAdjustmentComponentRowNew,
        PlanVersionAdjustmentRowNew,
    };

    let Some(src_min) = load_plan_minimum(conn, src_id).await? else {
        return Ok(());
    };

    let (includes_all_components, dst_product_ids) = match &src_min.scope {
        crate::domain::plan_minimums::MinimumScope::AllComponents => (true, vec![]),
        crate::domain::plan_minimums::MinimumScope::Products(ids) => {
            // Keep the scoped products the destination version still has a component for.
            let dst_products: std::collections::HashSet<_> =
                dst_components.iter().filter_map(|c| c.product_id).collect();
            let mapped: Vec<_> = ids
                .iter()
                .copied()
                .filter(|pid| dst_products.contains(pid))
                .collect();
            let dropped = ids.len() - mapped.len();

            if mapped.is_empty() {
                tracing::warn!(
                    src_plan_version_id = %src_id,
                    dst_plan_version_id = %dst_id,
                    dropped,
                    "All scoped products dropped during plan-version clone; \
                     widening minimum scope to AllComponents",
                );
                (true, vec![])
            } else {
                if dropped > 0 {
                    tracing::warn!(
                        src_plan_version_id = %src_id,
                        dst_plan_version_id = %dst_id,
                        dropped,
                        "Some scoped products dropped during plan-version clone",
                    );
                }
                (false, mapped)
            }
        }
    };

    PlanVersionAdjustmentRowNew {
        plan_version_id: dst_id,
        kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
        amount_subunit: src_min.amount_subunit,
        includes_all_components,
    }
    .upsert(conn)
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    if !includes_all_components {
        let rows: Vec<PlanVersionAdjustmentComponentRowNew> = dst_product_ids
            .into_iter()
            .map(|product_id| PlanVersionAdjustmentComponentRowNew {
                plan_version_id: dst_id,
                kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                product_id,
            })
            .collect();
        PlanVersionAdjustmentComponentRow::insert_batch(conn, &rows)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }

    Ok(())
}

impl Store {
    /// Pausing usage once the credit runs out, or invoicing it up to a cap, is enforced live by
    /// the usage controls: without them the API refuses such a rule. Stored rules still bill, and
    /// internal seeding may still write them.
    pub fn ensure_credit_rule_enforceable(
        &self,
        rule: Option<&CreditExhaustedRule>,
    ) -> StoreResult<()> {
        match rule {
            Some(CreditExhaustedRule::Pause | CreditExhaustedRule::InvoiceUpTo { .. })
                if !self.hooks.credit.available() =>
            {
                Err(Report::new(StoreError::InvalidArgument(format!(
                    "{USAGE_CONTROLS_UNAVAILABLE}: usage beyond the credit can only be invoiced"
                ))))
            }
            _ => Ok(()),
        }
    }
}

/// Credits beyond the balance are invoiced at the price's rate, unless the version pauses usage
/// once credit runs out: only then may a credit unit price leave its rate empty (zero).
pub(crate) async fn ensure_rated_beyond_credit(
    conn: &mut PgConn,
    version: &PlanVersionRow,
) -> StoreResult<()> {
    if version.credit_exhausted_rule != Some(diesel_models::enums::CreditExhaustedRuleEnum::Pause)
        && PlanVersionRow::has_unrated_credit_price(conn, version.tenant_id, version.id).await?
    {
        return Err(Report::new(StoreError::InvalidArgument(
            "a price in credits needs a price per credit beyond the balance unless usage pauses \
             once the credit runs out"
                .to_owned(),
        )));
    }
    Ok(())
}

pub(crate) async fn load_plan_minimum(
    conn: &mut PgConn,
    plan_version_id: PlanVersionId,
) -> StoreResult<Option<crate::domain::plan_minimums::PlanMinimum>> {
    use diesel_models::plan_version_adjustments::{
        PlanVersionAdjustmentComponentRow, PlanVersionAdjustmentRow,
    };

    let Some(row) = PlanVersionAdjustmentRow::find_by_plan_version_id(conn, &plan_version_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
    else {
        return Ok(None);
    };

    let components = if row.includes_all_components {
        vec![]
    } else {
        PlanVersionAdjustmentComponentRow::list_by_plan_version_id(conn, &plan_version_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
    };

    Ok(Some(crate::domain::plan_minimums::PlanMinimum::from_rows(
        row, components,
    )))
}

#[async_trait::async_trait]
impl PlansInterface for Store {
    async fn insert_plan(&self, actor: Actor, full_plan: FullPlanNew) -> StoreResult<FullPlan> {
        let mut conn = self.get_conn().await?;

        let FullPlanNew {
            plan,
            version,
            price_components,
        } = full_plan;
        let version_entitlements = version.entitlements.clone();

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
                let actor = &actor;
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
                    }
                    // TODO parameter
                    .into_raw(tenant.reporting_currency);

                    let mut inserted_plan_version_new: PlanVersion = plan_version_to_insert
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
                        self_service_rank: None,
                        issues_license: None,
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
                            product_family.id,
                            &inserted_plan_version_new.currency,
                            true,
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
                    if !inserted_plan_version_new.is_draft_version {
                        PlanVersionRow::settle_credit_exhausted_rule(
                            conn,
                            inserted.tenant_id,
                            inserted_plan_version_new.id,
                            self.hooks.credit.available(),
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                        let row = PlanVersionRow::find_by_id_and_tenant_id(
                            conn,
                            inserted_plan_version_new.id,
                            inserted.tenant_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                        ensure_rated_beyond_credit(conn, &row).await?;
                        inserted_plan_version_new = row.into();
                    }

                    if !version_entitlements.is_empty() {
                        inserted_plan_version_new.entitlements = insert_entitlement_specs(
                            conn,
                            version_entitlements,
                            common_domain::ids::EntitlementEntityId::PlanVersion(
                                inserted_plan_version_new.id,
                            ),
                            inserted.tenant_id,
                        )
                        .await?;
                    }

                    // Emit outbox event for webhooks
                    let plan_event = crate::domain::outbox_event::PlanEvent::new(
                        updated.id,
                        inserted_plan_version_new.id,
                        updated.tenant_id,
                        updated.name.clone(),
                        updated.description.clone(),
                        updated.plan_type.clone(),
                        updated.status.clone(),
                        inserted_plan_version_new.currency.clone(),
                        inserted_plan_version_new.version,
                        updated.created_at,
                    );
                    self.internal
                        .record_outbox_batch_tx(
                            conn,
                            updated.tenant_id,
                            actor,
                            vec![crate::domain::outbox_event::OutboxEvent::plan_created(
                                plan_event,
                            )],
                        )
                        .await?;

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
                actor,
                res.version.id,
                res.plan.tenant_id,
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

        let mut plan: PlanWithVersion =
            PlanRow::get_with_version_by_id(&mut conn, id, auth_tenant_id, version_filter.into())
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into();

        if let Some(version) = plan.version.as_mut() {
            version.minimum = load_plan_minimum(&mut conn, version.id).await?;
        }

        Ok(plan)
    }

    async fn get_plan_by_version_id_with_conn(
        &self,
        conn: &mut PgConn,
        id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<PlanWithVersion> {
        let mut plan: PlanWithVersion =
            PlanRow::get_with_version_by_version_id(conn, id, auth_tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into();

        if let Some(version) = plan.version.as_mut() {
            version.minimum = load_plan_minimum(conn, version.id).await?;
        }

        Ok(plan)
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
        order_by: Option<String>,
    ) -> StoreResult<PaginatedVec<PlanOverview>> {
        let mut conn = self.get_conn().await?;

        let rows = PlanRowOverview::list(
            &mut conn,
            auth_tenant_id,
            product_family_id,
            filters.into(),
            pagination.into(),
            order_by.as_deref(),
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
        filters: PlanFilters,
        pagination: PaginationRequest,
        order_by: Option<String>,
    ) -> StoreResult<PaginatedVec<FullPlan>> {
        let mut conn = self.get_conn().await?;

        let rows = FullPlanRow::list(
            &mut conn,
            auth_tenant_id,
            product_family_id,
            filters.into(),
            pagination.into(),
            order_by.as_deref(),
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
                    self_service_rank: None,
                    issues_license: None,
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
                    uses_product_pricing: true,
                    tax_inclusive: original.tax_inclusive,
                    credit_exhausted_rule: original.credit_exhausted_rule,
                    credit_exhausted_caps: original.credit_exhausted_caps,
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
                                    catalog: true,
                                    tax_category_id: None,
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
                                    catalog: true,
                                    credit_unit_id: None,
                                    credit_unit_rate: None,
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

                EntitlementRow::clone_all_for_plan_version(
                    conn,
                    original.id,
                    new.id,
                    auth_tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                // Carry invoice annex bindings forward to the new draft version.
                diesel_models::invoice_annex_templates::PlanVersionAnnexTemplateRowNew::clone_all(
                    conn,
                    original.id,
                    new.id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                // Carry the plan minimum forward, remapping component IDs by (name, product_id)
                // — same mapping strategy used above for plan_component_price.
                clone_plan_minimum(conn, original.id, new.id, &src_components, &dst_components)
                    .await?;

                PlanRowPatch {
                    id: original.plan_id,
                    tenant_id: original.tenant_id,
                    name: None,
                    description: None,
                    active_version_id: None,
                    draft_version_id: Some(Some(new.id)),
                    self_service_rank: None,
                    issues_license: None,
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
        actor: Actor,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<PlanVersion> {
        let res: PlanVersion = self
            .transaction_bumping_tenant(auth_tenant_id, |conn| {
                let actor = &actor;
                async move {
                    // TODO validations
                    // - all components on committed must have values for all periods
                    PlanVersionRow::settle_credit_exhausted_rule(
                        conn,
                        auth_tenant_id,
                        plan_version_id,
                        self.hooks.credit.available(),
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    let published = PlanVersionRow::publish(conn, plan_version_id, auth_tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    ensure_rated_beyond_credit(conn, &published).await?;

                    PlanRow::activate(conn, published.plan_id, auth_tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let plan: Plan = PlanRowPatch {
                        id: published.plan_id,
                        tenant_id: published.tenant_id,
                        name: None,
                        description: None,
                        active_version_id: Some(Some(published.id)),
                        draft_version_id: Some(None),
                        self_service_rank: None,
                        issues_license: None,
                    }
                    .update(conn)
                    .await
                    .map(Into::into)
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let version: PlanVersion = published.into();

                    // Emit plan.published webhook inside transaction
                    let plan_event = crate::domain::outbox_event::PlanEvent::new(
                        plan.id,
                        version.id,
                        auth_tenant_id,
                        plan.name.clone(),
                        plan.description.clone(),
                        plan.plan_type.clone(),
                        PlanStatusEnum::Active,
                        version.currency.clone(),
                        version.version,
                        plan.created_at,
                    );
                    self.internal
                        .record_outbox_batch_tx(
                            conn,
                            auth_tenant_id,
                            actor,
                            vec![crate::domain::outbox_event::OutboxEvent::plan_published(
                                plan_event,
                            )],
                        )
                        .await?;

                    Ok(version)
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::plan_published_version(
                actor,
                plan_version_id,
                auth_tenant_id,
            ))
            .await;

        Ok(res)
    }

    async fn discard_draft_plan_version(
        &self,
        actor: Actor,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let original =
                    PlanVersionRow::find_by_id_and_tenant_id(conn, plan_version_id, auth_tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                let plan_id = original.plan_id;

                PlanRowPatch {
                    id: plan_id,
                    tenant_id: original.tenant_id,
                    name: None,
                    description: None,
                    active_version_id: None,
                    draft_version_id: Some(None),
                    self_service_rank: None,
                    issues_license: None,
                }
                .update(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                PlanVersionRow::delete_draft(conn, plan_version_id, auth_tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // only deletes if no versions left
                PlanRow::delete(conn, plan_id, auth_tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let activity = Activity::new(
                    ActivityType::PlanDraftDiscarded,
                    EntityType::Plan,
                    plan_id.as_uuid(),
                )
                .with_metadata(serde_json::json!({
                    "plan_version_id": plan_version_id.to_string(),
                }));
                self.internal
                    .record_audit_tx(conn, auth_tenant_id, actor, AuditInput::Activity(activity))
                    .await
            }
            .scope_boxed()
        })
        .await?;

        let _ = self
            .eventbus
            .publish(Event::plan_discarded_version(
                actor,
                plan_version_id,
                auth_tenant_id,
            ))
            .await;

        Ok(())
    }

    async fn patch_published_plan(
        &self,
        actor: Actor,
        patch: PlanPatch,
    ) -> StoreResult<PlanOverview> {
        let patch: PlanRowPatch = patch.into();

        self.transaction(|conn| {
            let actor = &actor;
            let patch = &patch;
            async move {
                // Snapshot before the update so the entity.updated audit row records only
                // fields whose value actually changes — the plan form submits every field.
                let before: PlanOverview =
                    PlanRow::get_overview_by_id(conn, patch.id, patch.tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                        .map(Into::into)?;

                let plan = patch
                    .update(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let overview: PlanOverview =
                    PlanRow::get_overview_by_id(conn, plan.id, plan.tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                        .map(Into::into)?;

                let mut changed: Vec<&'static str> = Vec::new();
                if before.name != overview.name {
                    changed.push("name");
                }
                if before.description != overview.description {
                    changed.push("description");
                }
                if before.self_service_rank != overview.self_service_rank {
                    changed.push("self_service_rank");
                }
                if before.issues_license != overview.issues_license {
                    changed.push("issues_license");
                }

                if !changed.is_empty() {
                    let activity = Activity::new(
                        ActivityType::EntityUpdated,
                        EntityType::Plan,
                        plan.id.as_uuid(),
                    )
                    .with_metadata(serde_json::json!({
                        "changes": changed
                            .iter()
                            .map(|f| serde_json::json!({ "field": f }))
                            .collect::<Vec<_>>(),
                    }));
                    self.internal
                        .record_audit_tx(
                            conn,
                            plan.tenant_id,
                            actor,
                            AuditInput::Activity(activity),
                        )
                        .await?;
                }

                Ok(overview)
            }
            .scope_boxed()
        })
        .await
    }

    async fn patch_draft_plan(
        &self,
        actor: Actor,
        patch: PlanAndVersionPatch,
    ) -> StoreResult<PlanWithVersion> {
        let tenant_for_audit = patch.version.tenant_id;

        self.transaction(|conn| {
            let actor = &actor;
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
                    self_service_rank: None,
                    issues_license: None,
                }
                .into();

                patch_plan
                    .update(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let result: PlanWithVersion =
                    PlanRow::get_with_version(conn, patched_version.id, patched_version.tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                        .map(Into::into)?;

                let activity = Activity::new(
                    ActivityType::EntityUpdated,
                    EntityType::Plan,
                    patched_version.plan_id.as_uuid(),
                );
                self.internal
                    .record_audit_tx(
                        conn,
                        tenant_for_audit,
                        actor,
                        AuditInput::Activity(activity),
                    )
                    .await?;

                Ok(result)
            }
            .scope_boxed()
        })
        .await
    }

    async fn patch_trial(&self, actor: Actor, patch: TrialPatch) -> StoreResult<PlanWithVersion> {
        self.transaction_bumping_tenant(patch.tenant_id, |conn| {
            let actor = &actor;
            async move {
                let trial_patch: PlanVersionTrialRowPatch = match patch.trial {
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

                let patched_version = trial_patch
                    .update_trial(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let result: PlanWithVersion =
                    PlanRow::get_with_version(conn, patched_version.id, patched_version.tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                        .map(Into::into)?;

                let activity = Activity::new(
                    ActivityType::EntityUpdated,
                    EntityType::Plan,
                    patched_version.plan_id.as_uuid(),
                )
                .with_metadata(serde_json::json!({
                    "changes": [{ "field": "trial" }],
                }));
                self.internal
                    .record_audit_tx(
                        conn,
                        patched_version.tenant_id,
                        actor,
                        AuditInput::Activity(activity),
                    )
                    .await?;

                Ok(result)
            }
            .scope_boxed()
        })
        .await
    }

    async fn archive_plan(
        &self,
        actor: Actor,
        id: PlanId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        // Fetch plan overview BEFORE archiving to get full data
        let plan_overview: PlanOverview =
            PlanRow::get_overview_by_id(&mut conn, id, auth_tenant_id)
                .await
                .map(Into::into)
                .map_err(Into::<Report<StoreError>>::into)?;

        // Resolve the best available version (active preferred, then draft)
        let version_info: Option<PlanVersion> = if let Some(ref v) = plan_overview.active_version {
            let pv = PlanVersionRow::find_by_id_and_tenant_id(&mut conn, v.id, auth_tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            Some(pv.into())
        } else if plan_overview.has_draft_version {
            PlanRow::get_with_version_by_id(
                &mut conn,
                id,
                auth_tenant_id,
                diesel_models::plan_versions::PlanVersionFilter::Draft,
            )
            .await
            .ok()
            .and_then(|pw| {
                let pwv: PlanWithVersion = pw.into();
                pwv.version
            })
        } else {
            None
        };

        self.transaction_with(&mut conn, |conn| {
            let actor = &actor;
            async move {
                PlanRow::archive(conn, id, auth_tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Only emit webhook if we have complete version data
                if let Some(version) = version_info {
                    let plan_event = crate::domain::outbox_event::PlanEvent::new(
                        id,
                        version.id,
                        auth_tenant_id,
                        plan_overview.name.clone(),
                        plan_overview.description.clone(),
                        plan_overview.plan_type.clone(),
                        PlanStatusEnum::Archived,
                        version.currency,
                        version.version,
                        plan_overview.created_at,
                    );
                    self.internal
                        .record_outbox_batch_tx(
                            conn,
                            auth_tenant_id,
                            actor,
                            vec![crate::domain::outbox_event::OutboxEvent::plan_archived(
                                plan_event,
                            )],
                        )
                        .await?;
                } else {
                    log::warn!(
                        "Plan {} archived without webhook: no version data available",
                        id
                    );
                }

                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn unarchive_plan(&self, id: PlanId, auth_tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        PlanRow::unarchive(&mut conn, id, auth_tenant_id)
            .await
            .map_err(Into::into)
    }

    async fn list_self_service_plans(
        &self,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        currency: &str,
        exclude_plan_id: PlanId,
    ) -> StoreResult<Vec<SelfServicePlan>> {
        let mut conn = self.get_conn().await?;

        let rows = SelfServicePlanRow::list_self_service_plans(
            &mut conn,
            tenant_id,
            product_family_id,
            currency,
            exclude_plan_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows
            .into_iter()
            .map(|r| SelfServicePlan {
                plan_id: r.plan_id,
                plan_name: r.plan_name,
                description: r.description,
                plan_version_id: r.plan_version_id,
                self_service_rank: r.self_service_rank,
            })
            .collect())
    }

    async fn list_published_versions_by_plan_ids(
        &self,
        plan_ids: Vec<PlanId>,
        auth_tenant_id: TenantId,
    ) -> StoreResult<Vec<PlanVersion>> {
        let mut conn = self.get_conn().await?;
        PlanVersionRow::list_published_by_plan_ids(&mut conn, &plan_ids, auth_tenant_id)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    async fn replace_plan_version(
        &self,
        typed_actor: Actor,
        plan_id: PlanId,
        tenant_id: TenantId,
        name: String,
        description: Option<String>,
        version: crate::domain::PlanVersionNewInternal,
        price_components: Vec<crate::domain::price_components::PriceComponentNewInternal>,
        add_on_attachments: Vec<crate::domain::plan_version_add_ons::PlanVersionAddOnNew>,
        publish: bool,
        minimum: Option<crate::domain::plan_minimums::ReplacePlanMinimumInput>,
    ) -> StoreResult<FullPlan> {
        let mut conn = self.get_conn().await?;
        let version_entitlements = version.entitlements.clone();

        // Load plan metadata (version-agnostic) to get draft_version_id and product_family_id
        let plan_overview: PlanOverview =
            PlanRow::get_overview_by_id(&mut conn, plan_id, tenant_id)
                .await
                .map(Into::into)
                .map_err(Into::<Report<StoreError>>::into)?;

        let existing_draft_version_id = if plan_overview.has_draft_version {
            // We need the actual draft version ID. The overview doesn't expose it directly,
            // so fetch the plan with draft filter to get it.

            PlanRow::get_with_version_by_id(
                &mut conn,
                plan_id,
                tenant_id,
                diesel_models::plan_versions::PlanVersionFilter::Draft,
            )
            .await
            .ok()
            .and_then(|pw| {
                let pwv: PlanWithVersion = pw.into();
                pwv.version.map(|v| v.id)
            })
        } else {
            None
        };

        let product_family_id = plan_overview.product_family_id;

        let product_family: ProductFamilyOverview =
            ProductFamilyRow::find_by_id(&mut conn, product_family_id, tenant_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?
                .into();

        let tenant = TenantRow::find_by_id(&mut conn, tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        drop(conn);

        let res = self
            .transaction_bumping_tenants(|conn| {
                let typed_actor = &typed_actor;
                async move {
                    // Update plan name/description
                    PlanRowPatch {
                        id: plan_id,
                        tenant_id,
                        name: Some(name),
                        description: Some(description),
                        active_version_id: None,
                        draft_version_id: None,
                        self_service_rank: None,
                        issues_license: None,
                    }
                    .update(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    // Discard existing draft if any
                    if let Some(draft_id) = existing_draft_version_id {
                        PlanRowPatch {
                            id: plan_id,
                            tenant_id,
                            name: None,
                            description: None,
                            active_version_id: None,
                            draft_version_id: Some(None),
                            self_service_rank: None,
                            issues_license: None,
                        }
                        .update(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                        PlanVersionRow::delete_draft(conn, draft_id, tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }

                    // Determine next version number
                    let next_version =
                        PlanVersionRow::next_version_number(conn, plan_id, tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                    let new_version = PlanVersionNew {
                        plan_id,
                        version: next_version,
                        tenant_id,
                        internal: version,
                    }
                    .into_raw(tenant.reporting_currency);

                    let inserted_version: PlanVersion = new_version
                        .insert(conn)
                        .await
                        .map(Into::into)
                        .map_err(Into::<Report<StoreError>>::into)?;

                    // Insert price components, capturing name → id mapping for the minimum.
                    // name -> product_id: the minimum scope is keyed by product, and component
                    // names are unique within a plan version, so this also serves the dup check.
                    let mut inserted_components_by_name: std::collections::HashMap<
                        String,
                        common_domain::ids::ProductId,
                    > = std::collections::HashMap::new();
                    for p in &price_components {
                        let (product_id, price_ids) = resolve_component_internal(
                            conn,
                            p,
                            tenant_id,
                            product_family.id,
                            &inserted_version.currency,
                            true,
                        )
                        .await?;

                        let row_new: diesel_models::price_components::PriceComponentRowNew =
                            PriceComponentNew {
                                plan_version_id: inserted_version.id,
                                name: p.name.clone(),
                                product_id: Some(product_id),
                            }
                            .try_into()?;

                        let inserted_row = PriceComponentRow::insert(conn, row_new)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                        if inserted_components_by_name
                            .insert(p.name.clone(), product_id)
                            .is_some()
                        {
                            return Err(Report::new(StoreError::InvalidArgument(format!(
                                "Duplicate price component name '{}' — names must be unique \
                                 within a plan version",
                                p.name
                            ))));
                        }

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
                    }

                    // Persist plan-level minimum commitment if requested.
                    if let Some(min_input) = minimum.as_ref() {
                        use crate::domain::plan_minimums::{
                            PlanMinimumNew, ReplacePlanMinimumScope,
                        };

                        if min_input.amount_subunit <= 0 {
                            return Err(Report::new(StoreError::InvalidArgument(
                                "minimum_commitment.amount_subunit must be > 0".to_string(),
                            )));
                        }

                        let scope = match &min_input.scope {
                            ReplacePlanMinimumScope::AllComponents => {
                                crate::domain::plan_minimums::MinimumScope::AllComponents
                            }
                            ReplacePlanMinimumScope::Products(names) => {
                                if names.is_empty() {
                                    return Err(Report::new(StoreError::InvalidArgument(
                                        "Scope=Components requires at least one component name"
                                            .to_string(),
                                    )));
                                }
                                let mut ids = Vec::with_capacity(names.len());
                                for n in names {
                                    let id = inserted_components_by_name.get(n).ok_or_else(|| {
                                        Report::new(StoreError::InvalidArgument(format!(
                                            "minimum_commitment.scope references unknown \
                                             component '{}'",
                                            n
                                        )))
                                    })?;
                                    ids.push(*id);
                                }
                                crate::domain::plan_minimums::MinimumScope::Products(ids)
                            }
                        };

                        let new = PlanMinimumNew {
                            plan_version_id: inserted_version.id,
                            amount_subunit: min_input.amount_subunit,
                            scope,
                        };
                        new.to_row()
                            .upsert(conn)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        let comp_rows = new.component_rows();
                        if !comp_rows.is_empty() {
                            use diesel_models::plan_version_adjustments::PlanVersionAdjustmentComponentRow;
                            PlanVersionAdjustmentComponentRow::insert_batch(conn, &comp_rows)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                        }
                    }

                    // Attach add-ons
                    for add_on in &add_on_attachments {
                        let row_new: diesel_models::plan_version_add_ons::PlanVersionAddOnRowNew =
                            crate::domain::plan_version_add_ons::PlanVersionAddOnNew {
                                plan_version_id: inserted_version.id,
                                add_on_id: add_on.add_on_id,
                                price_id: add_on.price_id,
                                self_serviceable: add_on.self_serviceable,
                                max_instances_per_subscription: add_on
                                    .max_instances_per_subscription,
                                tenant_id,
                            }
                            .into();

                        row_new
                            .insert(conn)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }

                    // Entitlements belong to a version, so a replacement that mints one has to
                    // carry them over or the new version silently grants nothing.
                    if !version_entitlements.is_empty() {
                        insert_entitlement_specs(
                            conn,
                            version_entitlements.clone(),
                            common_domain::ids::EntitlementEntityId::PlanVersion(
                                inserted_version.id,
                            ),
                            tenant_id,
                        )
                        .await?;
                    }

                    // Publish or set as draft
                    if publish {
                        PlanVersionRow::settle_credit_exhausted_rule(
                            conn,
                            tenant_id,
                            inserted_version.id,
                            self.hooks.credit.available(),
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                        let published =
                            PlanVersionRow::publish(conn, inserted_version.id, tenant_id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                        ensure_rated_beyond_credit(conn, &published).await?;

                        PlanRow::activate(conn, plan_id, tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                        let updated_plan: Plan = PlanRowPatch {
                            id: plan_id,
                            tenant_id,
                            name: None,
                            description: None,
                            active_version_id: Some(Some(inserted_version.id)),
                            draft_version_id: Some(None),
                            self_service_rank: None,
                            issues_license: None,
                        }
                        .update(conn)
                        .await
                        .map(Into::into)
                        .map_err(Into::<Report<StoreError>>::into)?;

                        // Emit plan.published webhook inside transaction
                        let plan_event = crate::domain::outbox_event::PlanEvent::new(
                            plan_id,
                            inserted_version.id,
                            tenant_id,
                            updated_plan.name,
                            updated_plan.description,
                            updated_plan.plan_type,
                            PlanStatusEnum::Active,
                            inserted_version.currency.clone(),
                            inserted_version.version,
                            updated_plan.created_at,
                        );
                        self.internal
                            .record_outbox_batch_tx(
                                conn,
                                tenant_id,
                                typed_actor,
                                vec![crate::domain::outbox_event::OutboxEvent::plan_published(
                                    plan_event,
                                )],
                            )
                            .await?;
                    } else {
                        PlanRowPatch {
                            id: plan_id,
                            tenant_id,
                            name: None,
                            description: None,
                            active_version_id: None,
                            draft_version_id: Some(Some(inserted_version.id)),
                            self_service_rank: None,
                            issues_license: None,
                        }
                        .update(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    }

                    // A published version is the active one trialing subscriptions read.
                    Ok((inserted_version, publish.then_some(tenant_id).into_iter().collect()))
                }
                .scope_boxed()
            })
            .await?;

        if publish {
            let _ = self
                .eventbus
                .publish(Event::plan_published_version(
                    typed_actor.clone(),
                    res.id,
                    tenant_id,
                ))
                .await;
        } else {
            let _ = self
                .eventbus
                .publish(Event::plan_created_draft(typed_actor, res.id, tenant_id))
                .await;
        }

        // Re-fetch full plan to get all resolved data
        let full = self
            .get_full_plan(
                plan_id,
                tenant_id,
                if publish {
                    PlanVersionFilter::Active
                } else {
                    PlanVersionFilter::Draft
                },
            )
            .await?;

        Ok(full)
    }

    async fn get_plan_minimum(
        &self,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<Option<crate::domain::plan_minimums::PlanMinimum>> {
        let mut conn = self.get_conn().await?;
        ensure_plan_version_in_tenant(&mut conn, plan_version_id, auth_tenant_id).await?;
        load_plan_minimum(&mut conn, plan_version_id).await
    }

    async fn set_plan_minimum(
        &self,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
        amount_subunit: i64,
        scope: crate::domain::plan_minimums::MinimumScope,
    ) -> StoreResult<crate::domain::plan_minimums::PlanMinimum> {
        use crate::domain::plan_minimums::{MinimumScope, PlanMinimumNew};
        use diesel_models::plan_version_adjustments::PlanVersionAdjustmentComponentRow;
        use diesel_models::plan_versions::PlanVersionRow;

        // Pre-validate without DB roundtrip; deeper checks happen inside the transaction
        // under a row lock so we don't race a concurrent publish.
        if amount_subunit <= 0 {
            return Err(error_stack::Report::new(StoreError::InvalidArgument(
                "Minimum amount must be > 0".to_string(),
            )));
        }
        if let MinimumScope::Products(ids) = &scope
            && ids.is_empty()
        {
            return Err(error_stack::Report::new(StoreError::InvalidArgument(
                "Scope=Products requires at least one product id (use AllComponents \
                     to include all)."
                    .to_string(),
            )));
        }

        let new = PlanMinimumNew {
            plan_version_id,
            amount_subunit,
            scope: scope.clone(),
        };

        self.transaction(|conn| {
            let new = new.clone();
            let scope = scope.clone();
            async move {
                // Lock the plan_version row for the duration of the transaction to prevent
                // a concurrent `publish_plan_version` from flipping is_draft_version under us.
                let version = PlanVersionRow::select_for_update_by_id_and_tenant(
                    conn,
                    plan_version_id,
                    auth_tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                if !version.is_draft_version {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "Cannot edit minimum commitment on a published plan version. Create a \
                         draft version instead."
                            .to_string(),
                    )));
                }

                // Validate product scope inside the transaction, after the lock.
                if let MinimumScope::Products(ids) = &scope {
                    let rows = PriceComponentRow::list_by_plan_version_id(
                        conn,
                        auth_tenant_id,
                        plan_version_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    let valid: std::collections::HashSet<common_domain::ids::ProductId> =
                        rows.iter().filter_map(|r| r.product_id).collect();
                    for id in ids {
                        if !valid.contains(id) {
                            return Err(Report::new(StoreError::InvalidArgument(format!(
                                "Product {} is not used by any component of this plan version",
                                id
                            ))));
                        }
                    }
                }

                let row = new.to_row().upsert(conn).await?;
                PlanVersionAdjustmentComponentRow::delete_by_plan_version_id(
                    conn,
                    &plan_version_id,
                )
                .await?;
                let comp_rows = new.component_rows();
                if !comp_rows.is_empty() {
                    PlanVersionAdjustmentComponentRow::insert_batch(conn, &comp_rows).await?;
                }
                let components = if row.includes_all_components {
                    vec![]
                } else {
                    PlanVersionAdjustmentComponentRow::list_by_plan_version_id(
                        conn,
                        &plan_version_id,
                    )
                    .await?
                };
                Ok(crate::domain::plan_minimums::PlanMinimum::from_rows(
                    row, components,
                ))
            }
            .scope_boxed()
        })
        .await
    }

    async fn clear_plan_minimum(
        &self,
        plan_version_id: PlanVersionId,
        auth_tenant_id: TenantId,
    ) -> StoreResult<()> {
        use diesel_models::plan_version_adjustments::PlanVersionAdjustmentRow;
        use diesel_models::plan_versions::PlanVersionRow;

        self.transaction(|conn| {
            async move {
                let version = PlanVersionRow::select_for_update_by_id_and_tenant(
                    conn,
                    plan_version_id,
                    auth_tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                if !version.is_draft_version {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "Cannot remove minimum commitment from a published plan version"
                            .to_string(),
                    )));
                }

                PlanVersionAdjustmentRow::delete_by_plan_version_id(conn, &plan_version_id).await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }
}

async fn ensure_plan_version_in_tenant(
    conn: &mut PgConn,
    plan_version_id: PlanVersionId,
    auth_tenant_id: TenantId,
) -> StoreResult<crate::domain::PlanVersion> {
    use diesel_models::plan_versions::PlanVersionRow;
    let row = PlanVersionRow::find_by_id_and_tenant_id(conn, plan_version_id, auth_tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(row.into())
}
