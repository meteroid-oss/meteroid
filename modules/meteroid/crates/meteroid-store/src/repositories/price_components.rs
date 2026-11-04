use crate::StoreResult;
use crate::domain::Price;
use crate::domain::price_components::{
    PriceComponent, PriceComponentNew, PriceComponentNewInternal, PriceEntry, ProductRef,
};
use crate::domain::prices::{LegacyPricingData, extract_legacy_pricing};
use crate::errors::StoreError;
use crate::store::{PgConn, Store};
use common_domain::ids::{
    BaseId, CreditUnitId, PlanVersionId, PriceComponentId, PriceId, ProductFamilyId, ProductId,
    TenantId,
};
use diesel_models::credit_units::CreditUnitRow;
use diesel_models::plan_component_prices::{PlanComponentPriceRow, PlanComponentPriceRowNew};
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::price_components::{PriceComponentRow, PriceComponentRowNew};
use diesel_models::prices::{PriceRow, PriceRowNew};
use diesel_models::products::ProductRowNew;
use error_stack::Report;
use meteroid_store_macros::with_conn_delegate;
use scoped_futures::ScopedFutureExt;
use std::collections::HashMap;

pub use crate::domain::price_components::PriceInput;

/// Refuses a credit unit the tenant does not have, or archived: no new price may use it.
pub(crate) async fn ensure_credit_units_usable(
    conn: &mut PgConn,
    tenant_id: TenantId,
    unit_ids: impl Iterator<Item = CreditUnitId>,
) -> StoreResult<()> {
    let mut ids: Vec<CreditUnitId> = unit_ids.collect();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let units = CreditUnitRow::list_by_ids(conn, tenant_id, &ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    for id in ids {
        match units.iter().find(|unit| unit.id == id) {
            None => {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "credit unit {id} not found"
                ))));
            }
            Some(unit) if unit.archived_at.is_some() => {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "credit unit {} is archived",
                    unit.code
                ))));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// Resolve a PriceComponentNewInternal into (ProductId, Vec<PriceId>).
/// 1. Resolve ProductRef → ProductId (insert if New)
/// 2. For each PriceEntry: Existing → validate + use, New → validate currency + insert PriceRow
/// 3. Validate: ProductRef::New + PriceEntry::Existing is an error
pub async fn resolve_component_internal(
    conn: &mut PgConn,
    internal: &PriceComponentNewInternal,
    tenant_id: TenantId,
    product_family_id: ProductFamilyId,
    expected_currency: &str,
    catalog: bool,
) -> StoreResult<(ProductId, Vec<PriceId>)> {
    let product_id = match &internal.product_ref {
        ProductRef::Existing(pid) => *pid,
        ProductRef::New {
            name,
            fee_type,
            fee_structure,
        } => {
            let product_row = ProductRowNew {
                id: ProductId::new(),
                name: name.clone(),
                description: None,
                tenant_id,
                product_family_id,
                fee_type: (*fee_type).into(),
                fee_structure: serde_json::to_value(fee_structure).map_err(|e| {
                    Report::new(StoreError::SerdeError(
                        "Failed to serialize fee_structure".to_string(),
                        e,
                    ))
                })?,
                catalog,
                tax_category_id: None,
            }
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
            product_row.id
        }
    };

    let mut price_ids = Vec::new();
    for entry in &internal.prices {
        match entry {
            PriceEntry::Existing(pid) => {
                if matches!(internal.product_ref, ProductRef::New { .. }) {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "Cannot use existing price with a new product".to_string(),
                    )));
                }
                // Validate existing price belongs to this product, tenant, currency, and is active
                let price_row = PriceRow::find_by_id_and_tenant_id(conn, *pid, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if price_row.product_id != product_id {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "Price {} belongs to product {}, not {}",
                        pid, price_row.product_id, product_id
                    ))));
                }
                if price_row.currency != expected_currency {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "Price {} currency '{}' does not match expected currency '{}'",
                        pid, price_row.currency, expected_currency
                    ))));
                }
                if price_row.archived_at.is_some() {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "Price {} is archived",
                        pid
                    ))));
                }
                price_ids.push(*pid);
            }
            PriceEntry::New(input) => {
                if input.currency != expected_currency {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "Price currency '{}' does not match expected currency '{}'",
                        input.currency, expected_currency
                    ))));
                }
                crate::domain::prices::validate_pricing(&input.pricing, &input.currency)?;
                crate::domain::prices::validate_price_credit_unit(
                    &input.pricing,
                    input.credit_unit.as_ref(),
                )?;
                ensure_credit_units_usable(
                    conn,
                    tenant_id,
                    input.credit_unit.iter().map(|unit| unit.unit_id),
                )
                .await?;
                crate::services::credit::unit_wallets::ensure_credit_packs_usable(
                    conn,
                    tenant_id,
                    &input.currency,
                    [&input.pricing],
                )
                .await?;
                let pricing_json = serde_json::to_value(&input.pricing).map_err(|e| {
                    Report::new(StoreError::SerdeError(
                        "Failed to serialize pricing".to_string(),
                        e,
                    ))
                })?;
                let price_row = PriceRowNew {
                    id: PriceId::new(),
                    product_id,
                    cadence: input.cadence.into(),
                    currency: input.currency.clone(),
                    pricing: pricing_json,
                    tenant_id,
                    catalog,
                    credit_unit_id: input.credit_unit.as_ref().map(|unit| unit.unit_id),
                    credit_unit_rate: input.credit_unit.as_ref().map(|unit| unit.rate),
                }
                .insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                price_ids.push(price_row.id);
            }
        }
    }

    Ok((product_id, price_ids))
}

/// Locks a plan version for a change of its components, refused unless it is a draft: a published
/// version's components price live subscriptions. A concurrent publish waits for the change.
async fn lock_draft_version_tx(
    conn: &mut PgConn,
    plan_version_id: PlanVersionId,
    tenant_id: TenantId,
) -> StoreResult<PlanVersionRow> {
    let version =
        PlanVersionRow::select_for_update_by_id_and_tenant(conn, plan_version_id, tenant_id)
            .await?;
    if !version.is_draft_version {
        return Err(Report::new(StoreError::FailedPrecondition(
            "The price components of a published plan version cannot change. Create a draft \
             version instead."
                .to_string(),
        )));
    }
    Ok(version)
}

#[with_conn_delegate]
#[async_trait::async_trait]
pub trait PriceComponentInterface {
    #[delegated]
    async fn list_price_components(
        &self,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<PriceComponent>>;

    async fn get_price_component_by_id(
        &self,
        tenant_id: TenantId,
        id: PriceComponentId,
    ) -> StoreResult<PriceComponent>;

    async fn create_price_component(
        &self,
        price_component: PriceComponentNew,
        tenant_id: TenantId,
    ) -> StoreResult<PriceComponent>;

    /// Create a price component with associated Price entities.
    /// Each PriceInput creates a Price row + plan_component_price join row.
    async fn create_price_component_with_prices(
        &self,
        price_component: PriceComponentNew,
        prices: Vec<PriceInput>,
        tenant_id: TenantId,
    ) -> StoreResult<PriceComponent>;

    async fn create_price_component_batch(
        &self,
        price_component: Vec<PriceComponentNew>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<PriceComponent>>;

    async fn delete_price_component(
        &self,
        component_id: PriceComponentId,
        tenant_id: TenantId,
    ) -> StoreResult<()>;

    /// Update a price component and replace its associated prices.
    /// Deletes old join rows + old price entities, creates new ones.
    async fn update_price_component_with_prices(
        &self,
        price_component: PriceComponent,
        prices: Vec<PriceInput>,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
    ) -> StoreResult<PriceComponent>;

    /// Create a price component from high-level ProductRef + PriceEntry.
    /// Resolves the product (creates if New) and prices (creates if New),
    /// then creates the component row and join rows.
    async fn create_price_component_from_ref(
        &self,
        name: String,
        product_ref: ProductRef,
        price_entries: Vec<PriceEntry>,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
    ) -> StoreResult<PriceComponent>;
}

#[async_trait::async_trait]
impl PriceComponentInterface for Store {
    async fn list_price_components_with_conn(
        &self,
        conn: &mut PgConn,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<PriceComponent>> {
        let component_rows =
            PriceComponentRow::list_by_plan_version_id(conn, tenant_id, plan_version_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        // Eagerly load v2 prices via plan_component_price join
        let component_ids: Vec<PriceComponentId> = component_rows.iter().map(|c| c.id).collect();
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
                    .map(|row| {
                        let id = row.id;
                        Price::try_from(row).map(|p| (id, p))
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

        // Extract legacy pricing data for v1 components (no fake IDs)
        let mut legacy_by_component: HashMap<PriceComponentId, LegacyPricingData> = HashMap::new();
        let has_v1 = component_rows
            .iter()
            .any(|r| !prices_by_component.contains_key(&r.id) && r.legacy_fee.is_some());
        if has_v1 {
            let pv = PlanVersionRow::find_by_id_and_tenant_id(conn, plan_version_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            for row in &component_rows {
                if !prices_by_component.contains_key(&row.id)
                    && let Some(legacy_json) = &row.legacy_fee
                {
                    let legacy = extract_legacy_pricing(legacy_json, pv.currency.clone())?;
                    legacy_by_component.insert(row.id, legacy);
                }
            }
        }

        // Convert rows → domain, attach prices and legacy data
        let components = component_rows
            .into_iter()
            .map(|row| {
                let id = row.id;
                let mut comp: PriceComponent = row.try_into()?;
                if let Some(prices) = prices_by_component.remove(&id) {
                    comp.prices = prices;
                }
                if let Some(legacy) = legacy_by_component.remove(&id) {
                    comp.legacy_pricing = Some(legacy);
                }
                Ok(comp)
            })
            .collect::<Result<Vec<_>, Report<StoreError>>>()?;

        Ok(components)
    }

    async fn get_price_component_by_id(
        &self,
        tenant_id: TenantId,
        price_component_id: PriceComponentId,
    ) -> StoreResult<PriceComponent> {
        let mut conn = self.get_conn().await?;

        PriceComponentRow::get_by_id(&mut conn, tenant_id, price_component_id)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn create_price_component(
        &self,
        price_component: PriceComponentNew,
        tenant_id: TenantId,
    ) -> StoreResult<PriceComponent> {
        let price_component: PriceComponentRowNew = price_component.try_into()?;
        self.transaction(|conn| {
            async move {
                lock_draft_version_tx(conn, price_component.plan_version_id, tenant_id).await?;
                PriceComponentRow::insert(conn, price_component)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .try_into()
            }
            .scope_boxed()
        })
        .await
    }

    async fn create_price_component_with_prices(
        &self,
        price_component: PriceComponentNew,
        prices: Vec<PriceInput>,
        tenant_id: TenantId,
    ) -> StoreResult<PriceComponent> {
        use diesel_models::products::ProductRow;

        let product_id = price_component.product_id.ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "product_id is required when creating prices".to_string(),
            ))
        })?;

        let component_row_new: PriceComponentRowNew = price_component.try_into()?;

        let price_rows_new: Vec<PriceRowNew> = prices
            .iter()
            .map(|pi| {
                crate::domain::prices::validate_pricing(&pi.pricing, &pi.currency)?;
                crate::domain::prices::validate_price_credit_unit(
                    &pi.pricing,
                    pi.credit_unit.as_ref(),
                )?;
                let pricing_json = serde_json::to_value(&pi.pricing).map_err(|e| {
                    Report::new(StoreError::SerdeError(
                        "Failed to serialize pricing".to_string(),
                        e,
                    ))
                })?;
                Ok(PriceRowNew {
                    id: PriceId::new(),
                    product_id,
                    cadence: pi.cadence.into(),
                    currency: pi.currency.clone(),
                    pricing: pricing_json,
                    tenant_id,
                    catalog: true,
                    credit_unit_id: pi.credit_unit.as_ref().map(|unit| unit.unit_id),
                    credit_unit_rate: pi.credit_unit.as_ref().map(|unit| unit.rate),
                })
            })
            .collect::<Result<Vec<_>, Report<StoreError>>>()?;

        self.transaction(|conn| {
            async move {
                // Validate product belongs to tenant
                ProductRow::find_by_id_and_tenant_id(conn, product_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                ensure_credit_units_usable(
                    conn,
                    tenant_id,
                    price_rows_new.iter().filter_map(|row| row.credit_unit_id),
                )
                .await?;
                for price in &prices {
                    crate::services::credit::unit_wallets::ensure_credit_packs_usable(
                        conn,
                        tenant_id,
                        &price.currency,
                        [&price.pricing],
                    )
                    .await?;
                }

                // Validate price currencies match plan version
                let pv = lock_draft_version_tx(conn, component_row_new.plan_version_id, tenant_id)
                    .await?;
                for pi in &prices {
                    if pi.currency != pv.currency {
                        return Err(Report::new(StoreError::InvalidArgument(format!(
                            "Price currency '{}' does not match plan version currency '{}'",
                            pi.currency, pv.currency
                        ))));
                    }
                }

                // Insert the price component
                let inserted = PriceComponentRow::insert(conn, component_row_new)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Create Price entities
                let price_rows = PriceRowNew::insert_batch(conn, &price_rows_new)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Create plan_component_price join rows
                let pcp_rows_new: Vec<PlanComponentPriceRowNew> = price_rows
                    .iter()
                    .map(|pr| PlanComponentPriceRowNew {
                        plan_component_id: inserted.id,
                        price_id: pr.id,
                    })
                    .collect();

                PlanComponentPriceRowNew::insert_batch(conn, &pcp_rows_new)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Convert to domain
                let mut component: PriceComponent = inserted.try_into()?;
                component.prices = price_rows
                    .into_iter()
                    .map(Price::try_from)
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(component)
            }
            .scope_boxed()
        })
        .await
    }

    async fn create_price_component_batch(
        &self,
        price_components: Vec<PriceComponentNew>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<PriceComponent>> {
        let price_components: Vec<PriceComponentRowNew> = price_components
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;
        self.transaction(|conn| {
            async move {
                let mut versions: Vec<PlanVersionId> = price_components
                    .iter()
                    .map(|component| component.plan_version_id)
                    .collect();
                versions.sort_by_key(|version| version.as_uuid());
                versions.dedup();
                for version in versions {
                    lock_draft_version_tx(conn, version, tenant_id).await?;
                }
                PriceComponentRow::insert_batch(conn, price_components)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<_>, _>>()
            }
            .scope_boxed()
        })
        .await
    }

    async fn delete_price_component(
        &self,
        component_id: PriceComponentId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.transaction_bumping_tenant(tenant_id, |conn| {
            async move {
                let component = PriceComponentRow::get_by_id(conn, tenant_id, component_id).await?;
                // The delete cascades to the subscription components priced by it: only a draft
                // has none.
                lock_draft_version_tx(conn, component.plan_version_id, tenant_id).await?;
                PriceComponentRow::delete_by_id_and_tenant(conn, component_id, tenant_id).await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn update_price_component_with_prices(
        &self,
        price_component: PriceComponent,
        prices: Vec<PriceInput>,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
    ) -> StoreResult<PriceComponent> {
        use diesel_models::products::ProductRow;

        let product_id = price_component.product_id.ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "product_id is required when updating prices".to_string(),
            ))
        })?;

        let component_id = price_component.id;
        let pc_row = PriceComponentRow {
            id: component_id,
            plan_version_id,
            name: price_component.name,
            product_id: price_component.product_id,
            legacy_fee: None,
            billable_metric_id: None,
        };

        let price_rows_new: Vec<PriceRowNew> = prices
            .iter()
            .map(|pi| {
                crate::domain::prices::validate_pricing(&pi.pricing, &pi.currency)?;
                crate::domain::prices::validate_price_credit_unit(
                    &pi.pricing,
                    pi.credit_unit.as_ref(),
                )?;
                let pricing_json = serde_json::to_value(&pi.pricing).map_err(|e| {
                    Report::new(StoreError::SerdeError(
                        "Failed to serialize pricing".to_string(),
                        e,
                    ))
                })?;
                Ok(PriceRowNew {
                    id: PriceId::new(),
                    product_id,
                    cadence: pi.cadence.into(),
                    currency: pi.currency.clone(),
                    pricing: pricing_json,
                    tenant_id,
                    catalog: true,
                    credit_unit_id: pi.credit_unit.as_ref().map(|unit| unit.unit_id),
                    credit_unit_rate: pi.credit_unit.as_ref().map(|unit| unit.rate),
                })
            })
            .collect::<Result<Vec<_>, Report<StoreError>>>()?;

        self.transaction(|conn| {
            async move {
                // Validate product belongs to tenant
                ProductRow::find_by_id_and_tenant_id(conn, product_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                ensure_credit_units_usable(
                    conn,
                    tenant_id,
                    price_rows_new.iter().filter_map(|row| row.credit_unit_id),
                )
                .await?;
                for price in &prices {
                    crate::services::credit::unit_wallets::ensure_credit_packs_usable(
                        conn,
                        tenant_id,
                        &price.currency,
                        [&price.pricing],
                    )
                    .await?;
                }

                // Validate price currencies match plan version
                let current = PriceComponentRow::get_by_id(conn, tenant_id, component_id).await?;
                if current.plan_version_id != plan_version_id {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "the price component belongs to another plan version".to_string(),
                    )));
                }
                let pv = lock_draft_version_tx(conn, plan_version_id, tenant_id).await?;
                for pi in &prices {
                    if pi.currency != pv.currency {
                        return Err(Report::new(StoreError::InvalidArgument(format!(
                            "Price currency '{}' does not match plan version currency '{}'",
                            pi.currency, pv.currency
                        ))));
                    }
                }

                let updated = pc_row
                    .update(conn, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .ok_or_else(|| {
                        Report::new(StoreError::InvalidArgument(
                            "Price component not found".to_string(),
                        ))
                    })?;

                // Delete old join rows for this component
                PlanComponentPriceRow::delete_by_component_id(conn, component_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Create new Price entities
                let price_rows = PriceRowNew::insert_batch(conn, &price_rows_new)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Create new join rows
                let pcp_rows_new: Vec<PlanComponentPriceRowNew> = price_rows
                    .iter()
                    .map(|pr| PlanComponentPriceRowNew {
                        plan_component_id: updated.id,
                        price_id: pr.id,
                    })
                    .collect();

                PlanComponentPriceRowNew::insert_batch(conn, &pcp_rows_new)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let mut component: PriceComponent = updated.try_into()?;
                component.prices = price_rows
                    .into_iter()
                    .map(Price::try_from)
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(component)
            }
            .scope_boxed()
        })
        .await
    }

    async fn create_price_component_from_ref(
        &self,
        name: String,
        product_ref: ProductRef,
        price_entries: Vec<PriceEntry>,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
    ) -> StoreResult<PriceComponent> {
        let internal = PriceComponentNewInternal {
            name,
            product_ref,
            prices: price_entries,
        };

        self.transaction(|conn| {
            async move {
                let plan_version = lock_draft_version_tx(conn, plan_version_id, tenant_id).await?;
                let product_family_id =
                    PlanVersionRow::get_product_family_id(conn, plan_version_id, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                let (product_id, price_ids) = resolve_component_internal(
                    conn,
                    &internal,
                    tenant_id,
                    product_family_id,
                    &plan_version.currency,
                    true,
                )
                .await?;

                // Insert the price component row
                let component_row_new = PriceComponentRowNew {
                    id: PriceComponentId::new(),
                    name: internal.name,
                    legacy_fee: None,
                    plan_version_id,
                    product_id: Some(product_id),
                    billable_metric_id: None,
                };
                let inserted = component_row_new
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Create plan_component_price join rows
                let pcp_rows_new: Vec<PlanComponentPriceRowNew> = price_ids
                    .iter()
                    .map(|pid| PlanComponentPriceRowNew {
                        plan_component_id: inserted.id,
                        price_id: *pid,
                    })
                    .collect();

                PlanComponentPriceRowNew::insert_batch(conn, &pcp_rows_new)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Load the prices to return them
                let price_rows = PriceRow::list_by_ids(conn, &price_ids, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let mut component: PriceComponent = inserted.try_into()?;
                component.prices = price_rows
                    .into_iter()
                    .map(Price::try_from)
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(component)
            }
            .scope_boxed()
        })
        .await
    }
}
