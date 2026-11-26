use crate::domain::add_ons::{
    AddOn, AddOnNew, AddOnPatch, AddOnPriceChange, ensure_one_price_per_cell, instances_allowed,
    same_cell,
};
use crate::domain::entitlements::{Entitlement, EntitlementSpec};
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::price_components::{
    PriceComponentNewInternal, PriceEntry, PriceInput, ProductRef,
};
use crate::domain::prices::FeeStructure;
use crate::domain::{PaginatedVec, PaginationRequest, Price};
use crate::errors::StoreError;
use crate::repositories::entitlements::{insert_entitlement_specs, replace_entitlements_tx};
use crate::repositories::price_components::{
    charge_structure_tx, ensure_prices_fit_charge, resolve_component_internal,
};
use crate::store::PgConn;
use crate::{Store, StoreResult};
use common_domain::ids::{
    AddOnId, BaseId, EntitlementEntityId, PlanVersionId, PriceId, ProductFamilyId, ProductId,
    TenantId,
};
use diesel_models::add_ons::{AddOnPriceRow, AddOnRow, AddOnRowNew, AddOnRowPatch};
use diesel_models::entitlements::EntitlementRow;
use diesel_models::prices::PriceRow;
use diesel_models::products::ProductRow;
use error_stack::Report;
use meteroid_store_macros::with_conn_delegate;
use scoped_futures::ScopedFutureExt;

#[with_conn_delegate]
#[async_trait::async_trait]
pub trait AddOnInterface {
    async fn list_add_ons(
        &self,
        tenant_id: TenantId,
        plan_version_id: Option<PlanVersionId>,
        pagination: PaginationRequest,
        search: Option<String>,
        currency: Option<String>,
        include_archived: bool,
        order_by: Option<String>,
    ) -> StoreResult<PaginatedVec<AddOn>>;

    /// The tenant's add-ons after `after` in id order, at most `limit`, loaded as `list_add_ons`
    /// loads them: a keyset page of API v2.
    async fn list_add_ons_after(
        &self,
        tenant_id: TenantId,
        include_archived: bool,
        after: Option<AddOnId>,
        limit: usize,
    ) -> StoreResult<Vec<AddOn>>;

    #[delegated]
    async fn list_add_ons_by_ids(
        &self,
        tenant_id: TenantId,
        ids: Vec<AddOnId>,
    ) -> StoreResult<Vec<AddOn>>;

    async fn get_add_on_by_id(&self, tenant_id: TenantId, id: AddOnId) -> StoreResult<AddOn>;

    async fn create_add_on(&self, actor: Actor, add_on: AddOnNew) -> StoreResult<AddOn>;

    #[allow(clippy::too_many_arguments)]
    async fn create_add_on_from_ref(
        &self,
        name: String,
        product_ref: ProductRef,
        price_entry: PriceEntry,
        description: Option<String>,
        self_serviceable: bool,
        max_instances_per_subscription: Option<i32>,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        entitlements: Vec<EntitlementSpec>,
    ) -> StoreResult<AddOn>;

    /// `create_add_on_from_ref` with a price per currency and billing period, the first one its
    /// `price_id`, and the add-on's structure (None: the new product's). `create_add_on_from_ref`
    /// bills as its new product's structure says.
    #[allow(clippy::too_many_arguments)]
    async fn create_add_on_with_prices(
        &self,
        name: String,
        product_ref: ProductRef,
        fee_structure: Option<FeeStructure>,
        prices: Vec<PriceEntry>,
        description: Option<String>,
        self_serviceable: bool,
        max_instances_per_subscription: Option<i32>,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        entitlements: Vec<EntitlementSpec>,
    ) -> StoreResult<AddOn>;

    /// `create_add_on_with_prices` under `code`, unique per tenant (ADR-0013); none: its name's
    /// slug.
    #[allow(clippy::too_many_arguments)]
    async fn create_add_on_with_code(
        &self,
        name: String,
        code: Option<String>,
        product_ref: ProductRef,
        fee_structure: Option<FeeStructure>,
        prices: Vec<PriceEntry>,
        description: Option<String>,
        self_serviceable: bool,
        max_instances_per_subscription: Option<i32>,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        entitlements: Vec<EntitlementSpec>,
    ) -> StoreResult<AddOn>;

    /// Edits an add-on; `price_entry` replaces the price of its cell.
    async fn update_add_on(
        &self,
        actor: Actor,
        patch: AddOnPatch,
        price_entry: Option<PriceEntry>,
    ) -> StoreResult<AddOn>;

    async fn update_add_on_with_prices(
        &self,
        actor: Actor,
        patch: AddOnPatch,
        prices: Option<AddOnPriceChange>,
    ) -> StoreResult<AddOn>;

    /// `update_add_on_with_prices` and, when `entitlements` is set, `replace_entitlements` of
    /// the add-on, in one transaction: a refusal of either writes neither.
    async fn update_add_on_with_entitlements(
        &self,
        actor: Actor,
        patch: AddOnPatch,
        prices: Option<AddOnPriceChange>,
        entitlements: Option<Vec<EntitlementSpec>>,
    ) -> StoreResult<AddOn>;

    async fn archive_add_on(
        &self,
        actor: Actor,
        id: AddOnId,
        tenant_id: TenantId,
    ) -> StoreResult<()>;
    async fn unarchive_add_on(&self, id: AddOnId, tenant_id: TenantId) -> StoreResult<()>;
}

/// Eagerly load prices, product appearance and entitlements for a list of add-on rows
pub(crate) async fn enrich_add_ons(
    conn: &mut crate::store::PgConn,
    rows: Vec<AddOnRow>,
    tenant_id: TenantId,
) -> StoreResult<Vec<AddOn>> {
    use std::collections::HashMap;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let product_ids: Vec<ProductId> = rows.iter().map(|r| r.product_id).collect();
    let add_on_ids: Vec<AddOnId> = rows.iter().map(|r| r.id).collect();

    let mut linked = AddOnPriceRow::list_by_add_on_ids(conn, &add_on_ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let price_ids: Vec<PriceId> = rows
        .iter()
        .map(|r| r.price_id)
        .chain(linked.values().flatten().copied())
        .collect();

    let product_rows = ProductRow::list_by_ids(conn, &product_ids, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let products: HashMap<_, _> = product_rows.into_iter().map(|p| (p.id, p)).collect();

    let prices = load_prices(conn, &price_ids, tenant_id).await?;
    let prices: HashMap<PriceId, Price> = prices.into_iter().map(|p| (p.id, p)).collect();

    let add_on_entities: Vec<EntitlementEntityId> = rows
        .iter()
        .map(|r| EntitlementEntityId::AddOn(r.id))
        .collect();
    let entitlement_rows =
        EntitlementRow::list_by_entity_ids(conn, tenant_id, &add_on_entities, None)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    let mut entitlements_by_entity: HashMap<uuid::Uuid, Vec<Entitlement>> = HashMap::new();
    for row in entitlement_rows {
        let entity_id = row.entity_id;
        let e: Entitlement = row.try_into()?;
        entitlements_by_entity.entry(entity_id).or_default().push(e);
    }

    let add_ons = rows
        .into_iter()
        .map(|row| {
            let product = products.get(&row.product_id);
            let product_icon = product.and_then(|p| p.icon.clone());
            let product_color = product.and_then(|p| p.color.clone());
            let price = prices.get(&row.price_id).cloned();
            let mut own: Vec<Price> = linked
                .remove(&row.id)
                .unwrap_or_default()
                .into_iter()
                .filter(|id| *id != row.price_id)
                .filter_map(|id| prices.get(&id).cloned())
                .collect();
            own.sort_by(|a, b| (&a.currency, a.cadence).cmp(&(&b.currency, b.cadence)));
            // Its first price leads, as `price` reads it.
            let all: Vec<Price> = price.iter().cloned().chain(own).collect();
            let entitlements = entitlements_by_entity.remove(&*row.id).unwrap_or_default();
            let mut addon: AddOn = row.into();
            addon.product_icon = product_icon;
            addon.product_color = product_color;
            addon.price = price;
            addon.prices = all;
            addon.entitlements = entitlements;
            addon
        })
        .collect();

    Ok(add_ons)
}

async fn load_prices(
    conn: &mut PgConn,
    ids: &[PriceId],
    tenant_id: TenantId,
) -> StoreResult<Vec<Price>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    PriceRow::list_by_ids(conn, ids, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(Price::try_from)
        .collect()
}

/// The prices `entries` give an add-on billing as `fee_structure` says, under `product_ref`
/// (created by the first entry when new), each validated in its own currency and against the
/// structure.
async fn resolve_add_on_prices(
    conn: &mut PgConn,
    name: &str,
    product_ref: ProductRef,
    fee_structure: &FeeStructure,
    entries: Vec<PriceEntry>,
    tenant_id: TenantId,
    product_family_id: ProductFamilyId,
) -> StoreResult<(ProductId, Vec<PriceId>)> {
    if entries.is_empty() {
        return Err(Report::new(StoreError::InvalidArgument(
            "At least one price entry is required".into(),
        )));
    }
    let mut product_ref = product_ref;
    let mut product_id = None;
    let mut price_ids = Vec::new();
    for entry in entries {
        let currency = match &entry {
            PriceEntry::New(input) => input.currency.clone(),
            PriceEntry::Existing(pid) => {
                PriceRow::find_by_id_and_tenant_id(conn, *pid, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .currency
            }
        };
        let internal = PriceComponentNewInternal {
            name: name.to_owned(),
            product_ref: product_ref.clone(),
            fee_structure: Some(fee_structure.clone()),
            prices: vec![entry],
        };
        let charge = resolve_component_internal(
            conn,
            &internal,
            tenant_id,
            product_family_id,
            &currency,
            true,
        )
        .await?;
        product_ref = ProductRef::Existing(charge.product_id);
        product_id = Some(charge.product_id);
        price_ids.extend(charge.price_ids);
    }
    let product_id = product_id.ok_or_else(|| {
        Report::new(StoreError::InvalidArgument(
            "No price resolved for add-on".into(),
        ))
    })?;
    Ok((product_id, price_ids))
}

/// A new catalog price of `product_id`, validated as a charge's.
async fn insert_add_on_price(
    conn: &mut PgConn,
    tenant_id: TenantId,
    product_id: ProductId,
    input: PriceInput,
) -> StoreResult<PriceId> {
    crate::domain::prices::validate_pricing(&input.pricing, &input.currency)?;
    crate::domain::prices::validate_price_credit_unit(&input.pricing, input.credit_unit.as_ref())?;
    crate::repositories::price_components::ensure_credit_units_usable(
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

    let price_row = diesel_models::prices::PriceRowNew {
        id: PriceId::new(),
        product_id,
        cadence: input.cadence.into(),
        currency: input.currency,
        pricing: pricing_json,
        tenant_id,
        catalog: true,
        credit_unit_id: input.credit_unit.as_ref().map(|unit| unit.unit_id),
        credit_unit_rate: input.credit_unit.as_ref().map(|unit| unit.rate),
    }
    .insert(conn)
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    Ok(price_row.id)
}

/// Whether `price` already is what `input` asks: kept rather than priced anew.
fn same_price(price: &Price, input: &PriceInput) -> bool {
    price.currency.eq_ignore_ascii_case(input.currency.trim())
        && price.cadence == input.cadence
        && price.credit_unit == input.credit_unit
        && price.archived_at.is_none()
        && serde_json::to_value(&price.pricing).ok() == serde_json::to_value(&input.pricing).ok()
}

/// Links `price_ids`, every price of `add_on_id`, once checked one per cell.
async fn link_add_on_prices(
    conn: &mut PgConn,
    add_on_id: AddOnId,
    name: &str,
    price_ids: &[PriceId],
    tenant_id: TenantId,
) -> StoreResult<()> {
    let prices = load_prices(conn, price_ids, tenant_id).await?;
    ensure_one_price_per_cell(name, &prices)?;
    let rows: Vec<AddOnPriceRow> = price_ids
        .iter()
        .map(|price_id| AddOnPriceRow {
            add_on_id,
            price_id: *price_id,
        })
        .collect();
    AddOnPriceRow::insert_batch(conn, &rows)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(())
}

/// Leaves `add_on` with the prices `change` asks, linked, and returns its first. A price left
/// unchanged keeps its id; one no longer sold stays a price row, as subscriptions bill it.
async fn reprice_add_on(
    conn: &mut PgConn,
    add_on: &AddOnRow,
    change: AddOnPriceChange,
    tenant_id: TenantId,
) -> StoreResult<PriceId> {
    let mut current_ids = AddOnPriceRow::list_by_add_on_ids(conn, &[add_on.id])
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .remove(&add_on.id)
        .unwrap_or_default();
    if !current_ids.contains(&add_on.price_id) {
        current_ids.insert(0, add_on.price_id);
    }
    let current = load_prices(conn, &current_ids, tenant_id).await?;

    let (mut kept, first) = match change {
        AddOnPriceChange::All(inputs) => {
            if inputs.is_empty() {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "{} needs at least one price",
                    add_on.name
                ))));
            }
            let mut ids = Vec::with_capacity(inputs.len());
            for input in inputs {
                let id = match current.iter().find(|p| same_price(p, &input)) {
                    Some(price) => price.id,
                    None => insert_add_on_price(conn, tenant_id, add_on.product_id, input).await?,
                };
                ids.push(id);
            }
            let first = if ids.contains(&add_on.price_id) {
                add_on.price_id
            } else {
                ids[0]
            };
            (ids, first)
        }
        AddOnPriceChange::One(entry) => {
            let price_id = match entry {
                PriceEntry::Existing(pid) => {
                    let row = PriceRow::find_by_id_and_tenant_id(conn, pid, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if row.product_id != add_on.product_id {
                        return Err(Report::new(StoreError::InvalidArgument(format!(
                            "Price {} belongs to product {}, not {}",
                            pid, row.product_id, add_on.product_id
                        ))));
                    }
                    pid
                }
                PriceEntry::New(input) => {
                    insert_add_on_price(conn, tenant_id, add_on.product_id, input).await?
                }
            };
            let price = load_prices(conn, &[price_id], tenant_id)
                .await?
                .pop()
                .ok_or_else(|| Report::new(StoreError::ValueNotFound("price".into())))?;
            // It replaces the price of its cell; a single-price add-on's, of its currency, on any.
            let first_price = current.iter().find(|p| p.id == add_on.price_id);
            let replaced = current
                .iter()
                .find(|p| same_cell(p, &price))
                .or(first_price.filter(|p| p.currency.eq_ignore_ascii_case(&price.currency)))
                .ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(format!(
                        "Price currency '{}' does not match add-on currency '{}'",
                        price.currency,
                        first_price.map(|p| p.currency.as_str()).unwrap_or_default()
                    )))
                })?;
            let ids: Vec<PriceId> = current
                .iter()
                .map(|p| if p.id == replaced.id { price.id } else { p.id })
                .collect();
            let first = if replaced.id == add_on.price_id {
                price.id
            } else {
                add_on.price_id
            };
            (ids, first)
        }
    };
    kept.sort();
    kept.dedup();
    let structure: FeeStructure =
        serde_json::from_value(add_on.fee_structure.clone()).map_err(|e| {
            Report::new(StoreError::SerdeError(
                "Failed to deserialize FeeStructure".to_string(),
                e,
            ))
        })?;
    let priced = load_prices(conn, &kept, tenant_id).await?;
    ensure_prices_fit_charge(
        conn,
        tenant_id,
        &structure,
        priced.iter().map(|p| &p.pricing),
    )
    .await?;

    AddOnPriceRow::delete_except(conn, add_on.id, &kept)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    link_add_on_prices(conn, add_on.id, &add_on.name, &kept, tenant_id).await?;
    Ok(first)
}

#[async_trait::async_trait]
impl AddOnInterface for Store {
    async fn list_add_ons(
        &self,
        tenant_id: TenantId,
        plan_version_id: Option<PlanVersionId>,
        pagination: PaginationRequest,
        search: Option<String>,
        currency: Option<String>,
        include_archived: bool,
        order_by: Option<String>,
    ) -> StoreResult<PaginatedVec<AddOn>> {
        let mut conn = self.get_conn().await?;

        let paginated = AddOnRow::list_by_tenant_id(
            &mut conn,
            tenant_id,
            plan_version_id,
            pagination.into(),
            search,
            currency,
            include_archived,
            order_by.as_deref(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let items = enrich_add_ons(&mut conn, paginated.items, tenant_id).await?;

        Ok(PaginatedVec {
            items,
            total_pages: paginated.total_pages,
            total_results: paginated.total_results,
        })
    }

    async fn list_add_ons_after(
        &self,
        tenant_id: TenantId,
        include_archived: bool,
        after: Option<AddOnId>,
        limit: usize,
    ) -> StoreResult<Vec<AddOn>> {
        let mut conn = self.get_conn().await?;
        let rows = AddOnRow::list_page(
            &mut conn,
            tenant_id,
            include_archived,
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        enrich_add_ons(&mut conn, rows, tenant_id).await
    }

    async fn list_add_ons_by_ids_with_conn(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        ids: Vec<AddOnId>,
    ) -> StoreResult<Vec<AddOn>> {
        let rows = AddOnRow::list_by_ids(conn, &ids, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        enrich_add_ons(conn, rows, tenant_id).await
    }

    async fn get_add_on_by_id(&self, tenant_id: TenantId, id: AddOnId) -> StoreResult<AddOn> {
        let mut conn = self.get_conn().await?;

        let row = AddOnRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut enriched = enrich_add_ons(&mut conn, vec![row], tenant_id).await?;
        enriched
            .pop()
            .ok_or_else(|| Report::new(StoreError::InvalidArgument("Add-on not found".into())))
    }

    async fn create_add_on(&self, actor: Actor, add_on: AddOnNew) -> StoreResult<AddOn> {
        let mut conn = self.get_conn().await?;

        let price_row =
            PriceRow::find_by_id_and_tenant_id(&mut conn, add_on.price_id, add_on.tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        if price_row.product_id != add_on.product_id {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "Price {} belongs to product {}, not {}",
                add_on.price_id, price_row.product_id, add_on.product_id
            ))));
        }

        let tenant_id = add_on.tenant_id;
        let entitlements = add_on.entitlements.clone();
        let price = Price::try_from(price_row)?;
        let mut add_on = add_on;
        add_on.max_instances_per_subscription =
            instances_allowed(&add_on.fee_structure, add_on.max_instances_per_subscription)?;

        self.transaction_with(&mut conn, |conn| {
            let actor = &actor;
            async move {
                add_on.fee_structure =
                    charge_structure_tx(conn, tenant_id, add_on.fee_structure).await?;
                ensure_prices_fit_charge(conn, tenant_id, &add_on.fee_structure, [&price.pricing])
                    .await?;
                let row_new = AddOnRowNew::try_from(add_on)?;
                let row = row_new
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                link_add_on_prices(conn, row.id, &row.name, &[row.price_id], tenant_id).await?;

                let mut enriched = enrich_add_ons(conn, vec![row], tenant_id).await?;
                let mut addon = enriched.pop().ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument("Add-on not found".into()))
                })?;

                if !entitlements.is_empty() {
                    addon.entitlements = insert_entitlement_specs(
                        conn,
                        entitlements,
                        EntitlementEntityId::AddOn(addon.id),
                        tenant_id,
                    )
                    .await?;
                }

                self.internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        actor,
                        vec![OutboxEvent::add_on_created(addon.clone().into())],
                    )
                    .await?;

                Ok(addon)
            }
            .scope_boxed()
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_add_on_from_ref(
        &self,
        name: String,
        product_ref: ProductRef,
        price_entry: PriceEntry,
        description: Option<String>,
        self_serviceable: bool,
        max_instances_per_subscription: Option<i32>,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        entitlements: Vec<EntitlementSpec>,
    ) -> StoreResult<AddOn> {
        self.create_add_on_with_prices(
            name,
            product_ref,
            None,
            vec![price_entry],
            description,
            self_serviceable,
            max_instances_per_subscription,
            tenant_id,
            product_family_id,
            entitlements,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_add_on_with_prices(
        &self,
        name: String,
        product_ref: ProductRef,
        fee_structure: Option<FeeStructure>,
        prices: Vec<PriceEntry>,
        description: Option<String>,
        self_serviceable: bool,
        max_instances_per_subscription: Option<i32>,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        entitlements: Vec<EntitlementSpec>,
    ) -> StoreResult<AddOn> {
        self.create_add_on_with_code(
            name,
            None,
            product_ref,
            fee_structure,
            prices,
            description,
            self_serviceable,
            max_instances_per_subscription,
            tenant_id,
            product_family_id,
            entitlements,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_add_on_with_code(
        &self,
        name: String,
        code: Option<String>,
        product_ref: ProductRef,
        fee_structure: Option<FeeStructure>,
        prices: Vec<PriceEntry>,
        description: Option<String>,
        self_serviceable: bool,
        max_instances_per_subscription: Option<i32>,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        entitlements: Vec<EntitlementSpec>,
    ) -> StoreResult<AddOn> {
        if let Some(code) = code.as_deref() {
            crate::domain::catalog_codes::validate_code(code, AddOnId::PREFIX)
                .map_err(Report::new)?;
        }
        let fee_structure = fee_structure
            .or_else(|| product_ref.new_product_structure().cloned())
            .ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(format!(
                    "{name}: an add-on names how it bills its product"
                )))
            })?;
        let max_instances_per_subscription =
            instances_allowed(&fee_structure, max_instances_per_subscription)?;
        self.transaction(|conn| {
            async move {
                let fee_structure = charge_structure_tx(conn, tenant_id, fee_structure).await?;
                let (product_id, price_ids) = resolve_add_on_prices(
                    conn,
                    &name,
                    product_ref,
                    &fee_structure,
                    prices,
                    tenant_id,
                    product_family_id,
                )
                .await?;
                let price_id = *price_ids.first().ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(
                        "No price resolved for add-on".into(),
                    ))
                })?;

                let row_new = AddOnRowNew {
                    id: AddOnId::new(),
                    name,
                    tenant_id,
                    product_id,
                    price_id,
                    description,
                    self_serviceable,
                    max_instances_per_subscription,
                    fee_structure: serde_json::to_value(&fee_structure).map_err(|e| {
                        Report::new(StoreError::SerdeError(
                            "Failed to serialize fee_structure".to_string(),
                            e,
                        ))
                    })?,
                };

                let mut row = row_new
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if let Some(code) = code {
                    diesel_models::query::catalog_codes::set_add_on_code(
                        conn, tenant_id, row.id, &code,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    row.code = code;
                }
                link_add_on_prices(conn, row.id, &row.name, &price_ids, tenant_id).await?;

                let mut enriched = enrich_add_ons(conn, vec![row], tenant_id).await?;
                let mut addon = enriched.pop().ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument("Add-on not found".into()))
                })?;

                if !entitlements.is_empty() {
                    addon.entitlements = insert_entitlement_specs(
                        conn,
                        entitlements,
                        EntitlementEntityId::AddOn(addon.id),
                        tenant_id,
                    )
                    .await?;
                }

                Ok(addon)
            }
            .scope_boxed()
        })
        .await
    }

    async fn update_add_on(
        &self,
        actor: Actor,
        patch: AddOnPatch,
        price_entry: Option<PriceEntry>,
    ) -> StoreResult<AddOn> {
        self.update_add_on_with_prices(actor, patch, price_entry.map(AddOnPriceChange::One))
            .await
    }

    async fn update_add_on_with_prices(
        &self,
        actor: Actor,
        patch: AddOnPatch,
        prices: Option<AddOnPriceChange>,
    ) -> StoreResult<AddOn> {
        self.update_add_on_with_entitlements(actor, patch, prices, None)
            .await
    }

    async fn update_add_on_with_entitlements(
        &self,
        actor: Actor,
        patch: AddOnPatch,
        prices: Option<AddOnPriceChange>,
        entitlements: Option<Vec<EntitlementSpec>>,
    ) -> StoreResult<AddOn> {
        let tenant_id = patch.tenant_id;
        let add_on_id = patch.id;

        self.transaction_bumping_tenant(tenant_id, |conn| {
            let actor = &actor;
            async move {
                let existing = AddOnRow::get_by_id(conn, tenant_id, add_on_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let mut patch = patch;
                if let Some(max) = patch.max_instances_per_subscription
                    && let Ok(structure) =
                        serde_json::from_value::<FeeStructure>(existing.fee_structure.clone())
                {
                    patch.max_instances_per_subscription =
                        Some(instances_allowed(&structure, max)?);
                }

                let new_price_id = match prices {
                    Some(change) => Some(reprice_add_on(conn, &existing, change, tenant_id).await?),
                    None => None,
                };

                let row_patch: AddOnRowPatch = patch.into_row_patch(new_price_id);

                let row = row_patch
                    .patch(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if let Some(specs) = entitlements {
                    replace_entitlements_tx(
                        conn,
                        Some(actor),
                        tenant_id,
                        EntitlementEntityId::AddOn(add_on_id),
                        specs,
                    )
                    .await?;
                }

                let mut enriched = enrich_add_ons(conn, vec![row], tenant_id).await?;
                let addon = enriched.pop().ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument("Add-on not found".into()))
                })?;

                self.internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        actor,
                        vec![OutboxEvent::add_on_updated(addon.clone().into())],
                    )
                    .await?;

                Ok(addon)
            }
            .scope_boxed()
        })
        .await
    }

    async fn archive_add_on(
        &self,
        actor: Actor,
        id: AddOnId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        self.transaction_with(&mut conn, |conn| {
            let actor = &actor;
            async move {
                AddOnRow::archive(conn, id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let row = AddOnRow::get_by_id(conn, tenant_id, id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let addon: AddOn = row.into();
                self.internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        actor,
                        vec![OutboxEvent::add_on_archived(addon.into())],
                    )
                    .await?;

                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn unarchive_add_on(&self, id: AddOnId, tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        AddOnRow::unarchive(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}

impl Store {
    /// How an add-on of `product_id` priced by `price_id` bills its product when its creator
    /// names no structure (REST v1, whose add-ons billed as their product): as the charges the
    /// price is linked to, when they agree; else as the product's stored structure, else its live
    /// charges', when it fits the price; else as the pricing alone tells it: a rate, a one-time
    /// fee, a credit pack, or per seat of the quantity the product's name names (normalised as a
    /// charge naming no quantity is, `ensure_quantity_of_unit`). None when it stays ambiguous (a
    /// usage, capacity or recurring price no charge tells).
    pub async fn add_on_structure_of_price(
        &self,
        tenant_id: TenantId,
        product_id: ProductId,
        price_id: PriceId,
    ) -> StoreResult<Option<FeeStructure>> {
        use crate::domain::price_components::{DowngradePolicy, UpgradePolicy};
        use crate::domain::prices::{Pricing, validate_pricing_fits};
        use crate::domain::products::single_structure;

        let mut conn = self.get_conn().await?;
        let price = Price::try_from(
            PriceRow::find_by_id_and_tenant_id(&mut conn, price_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?,
        )?;
        if price.product_id != product_id {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "Price {price_id} belongs to product {}, not {product_id}",
                price.product_id
            ))));
        }
        let fits =
            |structure: &FeeStructure| validate_pricing_fits(structure, &price.pricing).is_ok();

        let linked: Vec<FeeStructure> =
            diesel_models::query::price_components::charge_structures_of_prices(
                &mut conn,
                tenant_id,
                &[price_id],
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .filter_map(|(_, json)| serde_json::from_value(json).ok())
            .collect();
        if let Some(structure) = single_structure(&linked).filter(|s| fits(s)) {
            return Ok(Some(structure.clone()));
        }

        let product = ProductRow::find_by_id_and_tenant_id(&mut conn, product_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let stored: Option<FeeStructure> = product
            .fee_structure
            .clone()
            .and_then(|json| serde_json::from_value(json).ok());
        if let Some(structure) = stored.filter(|s| fits(s)) {
            return Ok(Some(structure));
        }
        let charges = crate::repositories::products::charge_structures_of_products(
            &mut conn,
            tenant_id,
            &[product_id],
        )
        .await?
        .remove(&product_id)
        .unwrap_or_default();
        if let Some(structure) = single_structure(&charges).filter(|s| fits(s)) {
            return Ok(Some(structure.clone()));
        }

        Ok(match price.pricing {
            Pricing::Rate { .. } => Some(FeeStructure::Rate {}),
            Pricing::OneTime { .. } => Some(FeeStructure::OneTime {}),
            Pricing::Credit { .. } => Some(FeeStructure::Credit {}),
            Pricing::Slot { .. } => Some(FeeStructure::Slot {
                unit_name: product.name,
                quantity_id: None,
                upgrade_policy: UpgradePolicy::Prorated,
                downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
            }),
            Pricing::Capacity { .. } | Pricing::Usage(_) | Pricing::ExtraRecurring { .. } => None,
        })
    }
}
