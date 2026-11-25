use crate::StoreResult;
use crate::domain::Price;
use crate::domain::price_cells::option_cell;
use crate::domain::price_components::{
    PriceComponent, PriceComponentNew, PriceComponentNewInternal, PriceEntry, ProductRef,
    UsagePricingModel,
};
use crate::domain::prices::{FeeStructure, LegacyPricingData, Pricing, extract_legacy_pricing};
use crate::errors::StoreError;
use crate::repositories::plan_options::version_options;
use crate::store::{PgConn, Store};
use common_domain::ids::{
    BaseId, CreditUnitId, PlanOptionId, PlanVersionId, PriceComponentId, PriceId, ProductFamilyId,
    ProductId, TenantId,
};
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::credit_units::CreditUnitRow;
use diesel_models::enums::BillingMetricAggregateEnum;
use diesel_models::plan_component_prices::{PlanComponentPriceRow, PlanComponentPriceRowNew};
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::price_components::{PriceComponentRow, PriceComponentRowNew};
use diesel_models::prices::{PriceRow, PriceRowNew};
use diesel_models::products::{ProductRow, ProductRowNew};
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

/// Whether a metric of `aggregation` adds up over the groups it is read by: a sum or a count.
/// Rates by dimension and rate cards add the dimensions they rate to its grouping, which its other
/// prices fold back by summing; a maximum or a distinct count would be billed as a sum of them.
pub(crate) fn adds_up(aggregation: BillingMetricAggregateEnum) -> bool {
    matches!(
        aggregation,
        BillingMetricAggregateEnum::Sum | BillingMetricAggregateEnum::Count
    )
}

/// Refuses rates by dimension among `pricings` of a fee of `structure` whose metric does not add
/// up (see [`adds_up`]), as a rate card refuses it.
pub(crate) async fn ensure_dimension_rates_add_up<'a>(
    conn: &mut PgConn,
    tenant_id: TenantId,
    structure: &FeeStructure,
    pricings: impl IntoIterator<Item = &'a Pricing>,
) -> StoreResult<()> {
    let FeeStructure::Usage { metric_id, .. } = structure else {
        return Ok(());
    };
    if !pricings.into_iter().any(rates_by_dimension) {
        return Ok(());
    }
    let metric = BillableMetricRow::find_by_id(conn, *metric_id, tenant_id).await?;
    if adds_up(metric.aggregation_type) {
        return Ok(());
    }
    Err(Report::new(StoreError::InvalidArgument(format!(
        "rates by dimension price a metric that adds up, a sum or a count: '{}' does not",
        metric.name
    ))))
}

/// [`ensure_dimension_rates_add_up`] for prices of `product_id`, loaded only when one rates by
/// dimension.
pub(crate) async fn ensure_product_dimension_rates_add_up<'a>(
    conn: &mut PgConn,
    tenant_id: TenantId,
    product_id: ProductId,
    pricings: impl IntoIterator<Item = &'a Pricing>,
) -> StoreResult<()> {
    let pricings: Vec<&Pricing> = pricings.into_iter().collect();
    if !pricings.iter().copied().any(rates_by_dimension) {
        return Ok(());
    }
    let product = ProductRow::find_by_id_and_tenant_id(conn, product_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let structure: FeeStructure = serde_json::from_value(product.fee_structure).map_err(|e| {
        Report::new(StoreError::SerdeError(
            "Failed to deserialize FeeStructure".to_string(),
            e,
        ))
    })?;
    ensure_dimension_rates_add_up(conn, tenant_id, &structure, pricings).await
}

fn rates_by_dimension(pricing: &Pricing) -> bool {
    matches!(
        pricing,
        Pricing::Usage(UsagePricingModel::DimensionRates(_))
    )
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
                icon: None,
                color: None,
            }
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
            crate::repositories::products::register_quantity(conn, tenant_id, fee_structure)
                .await?;
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
                ensure_product_dimension_rates_add_up(
                    conn,
                    tenant_id,
                    product_id,
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
pub(crate) async fn lock_draft_version_tx(
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

/// Attaches to each component its links and their prices, read in two queries.
pub(crate) async fn attach_prices(
    conn: &mut PgConn,
    tenant_id: TenantId,
    components: &mut [PriceComponent],
) -> StoreResult<()> {
    let ids: Vec<PriceComponentId> = components.iter().map(|c| c.id).collect();
    if ids.is_empty() {
        return Ok(());
    }
    let links = PlanComponentPriceRow::list_by_component_ids(conn, &ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    if links.is_empty() {
        return Ok(());
    }
    let mut price_ids: Vec<PriceId> = links.iter().map(|link| link.price_id).collect();
    price_ids.sort_by_key(|id| id.as_uuid());
    price_ids.dedup();
    let prices_by_id: HashMap<PriceId, Price> = PriceRow::list_by_ids(conn, &price_ids, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(|row| Price::try_from(row).map(|price| (price.id, price)))
        .collect::<Result<_, _>>()?;
    for component in components {
        component.attach_links(&links, &prices_by_id);
    }
    Ok(())
}

/// Links each of `price_ids` into the cells of every one of `options`.
pub(crate) async fn link_prices(
    conn: &mut PgConn,
    component_id: PriceComponentId,
    price_ids: &[PriceId],
    options: &[PlanOptionId],
) -> StoreResult<()> {
    let links: Vec<PlanComponentPriceRowNew> = options
        .iter()
        .flat_map(|option_id| {
            price_ids
                .iter()
                .map(move |price_id| PlanComponentPriceRowNew {
                    plan_component_id: component_id,
                    price_id: *price_id,
                    option_id: *option_id,
                })
        })
        .collect();
    PlanComponentPriceRowNew::insert_batch(conn, &links)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(())
}

/// Every option of the version: a new charge's prices are the same in each.
async fn every_option(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_version_id: PlanVersionId,
) -> StoreResult<Vec<PlanOptionId>> {
    Ok(version_options(conn, tenant_id, plan_version_id)
        .await?
        .into_iter()
        .map(|option| option.id)
        .collect())
}

/// The options a cell write names, all of the version's when none is; refused for another's.
async fn named_options(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_version_id: PlanVersionId,
    asked: Vec<PlanOptionId>,
) -> StoreResult<Vec<PlanOptionId>> {
    let every = every_option(conn, tenant_id, plan_version_id).await?;
    if asked.is_empty() {
        return Ok(every);
    }
    if asked.iter().any(|id| !every.contains(id)) {
        return Err(Report::new(StoreError::InvalidArgument(
            "An option named is not of this plan version".to_string(),
        )));
    }
    Ok(every.into_iter().filter(|id| asked.contains(id)).collect())
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

    /// Update a price component and replace its prices but `keep`, its own prices it keeps
    /// as they are (another currency's, untouched by the edit).
    async fn update_price_component_with_prices(
        &self,
        price_component: PriceComponent,
        prices: Vec<PriceInput>,
        keep: Vec<PriceId>,
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

    /// Prices the cells of the price's currency in `options` (none: every option) with one new
    /// price, linked in each; the prices it replaces leave those cells, never edited. A capacity
    /// price replaces only the level of its commitment, else adds one.
    async fn set_component_price(
        &self,
        tenant_id: TenantId,
        component_id: PriceComponentId,
        options: Vec<PlanOptionId>,
        price: PriceInput,
    ) -> StoreResult<PriceComponent>;

    /// Prices the cells of the prices' currency in `options` (none: every option) with exactly
    /// `prices`: one price, or a capacity charge's levels (one per commitment, on one cadence).
    /// Every price the cells held before leaves them, never edited.
    async fn set_component_prices(
        &self,
        tenant_id: TenantId,
        component_id: PriceComponentId,
        options: Vec<PlanOptionId>,
        prices: Vec<PriceInput>,
    ) -> StoreResult<PriceComponent>;

    /// Empties the cells of `currency` in `options` (none: every option).
    async fn clear_component_price(
        &self,
        tenant_id: TenantId,
        component_id: PriceComponentId,
        options: Vec<PlanOptionId>,
        currency: String,
    ) -> StoreResult<PriceComponent>;
}

/// Locks the draft version of `component_id`, and loads the component with its prices.
async fn lock_draft_component(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    component_id: PriceComponentId,
) -> StoreResult<(PlanVersionId, PriceComponent)> {
    let row = PriceComponentRow::get_by_id(conn, tenant_id, component_id).await?;
    lock_draft_version_tx(conn, row.plan_version_id, tenant_id).await?;
    let component =
        component_with_prices(store, conn, tenant_id, row.plan_version_id, component_id).await?;
    Ok((row.plan_version_id, component))
}

async fn component_with_prices(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_version_id: PlanVersionId,
    component_id: PriceComponentId,
) -> StoreResult<PriceComponent> {
    store
        .list_price_components_with_conn(conn, plan_version_id, tenant_id)
        .await?
        .into_iter()
        .find(|component| component.id == component_id)
        .ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(
                "This price component does not exist".to_owned(),
            ))
        })
}

/// What a cell write does to the prices a cell held.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CellWrite {
    /// One price replaces the cell; a capacity price only the level of its commitment.
    Level,
    /// The prices written are the whole cell.
    Whole,
}

/// Refuses prices that cannot share a cell: another currency, or several that are not one
/// capacity charge's levels (each its own commitment, all on one cadence).
fn check_cell_prices(prices: &[PriceInput], currency: &str) -> StoreResult<()> {
    let invalid = |message: &str| Err(Report::new(StoreError::InvalidArgument(message.to_owned())));
    if prices.is_empty() {
        return invalid("a price is required");
    }
    if prices
        .iter()
        .any(|price| !price.currency.trim().eq_ignore_ascii_case(currency))
    {
        return invalid("A cell's prices are all in one currency");
    }
    if prices.len() == 1 {
        return Ok(());
    }
    let mut levels: Vec<u64> = Vec::with_capacity(prices.len());
    for price in prices {
        let Pricing::Capacity { included, .. } = price.pricing else {
            return invalid("Only a capacity charge's levels share a cell");
        };
        levels.push(included);
    }
    levels.sort_unstable();
    if levels.windows(2).any(|pair| pair[0] == pair[1]) {
        return invalid("Two levels include the same amount");
    }
    if prices
        .iter()
        .any(|price| price.cadence != prices[0].cadence)
    {
        return invalid("A charge's levels bill on one cadence");
    }
    Ok(())
}

/// Writes `prices` into the cells of their currency in `options` (none: every option), as
/// `write` says; the prices replaced leave those cells, never edited.
async fn write_component_cell(
    store: &Store,
    tenant_id: TenantId,
    component_id: PriceComponentId,
    options: Vec<PlanOptionId>,
    prices: Vec<PriceInput>,
    write: CellWrite,
) -> StoreResult<PriceComponent> {
    let Some(first) = prices.first() else {
        return Err(Report::new(StoreError::InvalidArgument(
            "a price is required".to_owned(),
        )));
    };
    let currency = first.currency.trim().to_ascii_uppercase();
    if crate::constants::Currencies::resolve_currency(&currency).is_none() {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "unknown currency '{currency}'"
        ))));
    }
    check_cell_prices(&prices, &currency)?;
    let mut pricing_jsons = Vec::with_capacity(prices.len());
    for price in &prices {
        crate::domain::prices::validate_pricing(&price.pricing, &price.currency)?;
        crate::domain::prices::validate_price_credit_unit(
            &price.pricing,
            price.credit_unit.as_ref(),
        )?;
        let pricing_json = serde_json::to_value(&price.pricing).map_err(|e| {
            Report::new(StoreError::SerdeError(
                "Failed to serialize pricing".to_string(),
                e,
            ))
        })?;
        pricing_jsons.push(pricing_json);
    }

    store
        .transaction(|conn| {
            async move {
                let (plan_version_id, component) =
                    lock_draft_component(store, conn, tenant_id, component_id).await?;
                let product_id = component.product_id.ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(
                        "A charge without a product cannot be priced by currency".to_string(),
                    ))
                })?;

                let product = ProductRow::find_by_id_and_tenant_id(conn, product_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let structure: FeeStructure = serde_json::from_value(product.fee_structure)
                    .map_err(|e| {
                        Report::new(StoreError::SerdeError(
                            "Failed to deserialize FeeStructure".to_string(),
                            e,
                        ))
                    })?;
                for price in &prices {
                    crate::domain::prices::validate_pricing_fits(&structure, &price.pricing)?;
                }
                // `check_cell_prices` made the prices written all of one kind.
                let kind = std::mem::discriminant(&prices[0].pricing);
                if component
                    .prices
                    .iter()
                    .any(|other| std::mem::discriminant(&other.pricing) != kind)
                {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "A charge's prices are all of one pricing kind".to_string(),
                    )));
                }
                ensure_credit_units_usable(
                    conn,
                    tenant_id,
                    prices
                        .iter()
                        .filter_map(|price| price.credit_unit.as_ref().map(|unit| unit.unit_id)),
                )
                .await?;
                crate::services::credit::unit_wallets::ensure_credit_packs_usable(
                    conn,
                    tenant_id,
                    &currency,
                    prices.iter().map(|price| &price.pricing),
                )
                .await?;
                ensure_dimension_rates_add_up(
                    conn,
                    tenant_id,
                    &structure,
                    prices.iter().map(|price| &price.pricing),
                )
                .await?;

                // The prices written replace each cell; one capacity price only the level of its
                // commitment, else it adds a level.
                let options = named_options(conn, tenant_id, plan_version_id, options).await?;
                for option in &options {
                    let replaced: Vec<PriceId> = option_cell(&component, *option, &currency)
                        .into_iter()
                        .filter(|old| match (write, &prices[0].pricing, &old.pricing) {
                            (
                                CellWrite::Level,
                                Pricing::Capacity { included, .. },
                                Pricing::Capacity {
                                    included: level, ..
                                },
                            ) => included == level,
                            _ => true,
                        })
                        .map(|old| old.id)
                        .collect();
                    PlanComponentPriceRow::delete_by_component_option_and_prices(
                        conn,
                        component_id,
                        *option,
                        &replaced,
                    )
                    .await?;
                }

                let mut inserted_ids = Vec::with_capacity(prices.len());
                for (price, pricing_json) in prices.iter().zip(pricing_jsons) {
                    let inserted = PriceRowNew {
                        id: PriceId::new(),
                        product_id,
                        cadence: price.cadence.into(),
                        currency: currency.clone(),
                        pricing: pricing_json,
                        tenant_id,
                        catalog: true,
                        credit_unit_id: price.credit_unit.as_ref().map(|unit| unit.unit_id),
                        credit_unit_rate: price.credit_unit.as_ref().map(|unit| unit.rate),
                    }
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    inserted_ids.push(inserted.id);
                }
                link_prices(conn, component_id, &inserted_ids, &options).await?;

                component_with_prices(store, conn, tenant_id, plan_version_id, component_id).await
            }
            .scope_boxed()
        })
        .await
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

        let legacy_fees: Vec<Option<serde_json::Value>> = component_rows
            .iter()
            .map(|row| row.legacy_fee.clone())
            .collect();
        let mut components = component_rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<PriceComponent>, Report<StoreError>>>()?;
        attach_prices(conn, tenant_id, &mut components).await?;

        // Extract legacy pricing data for v1 components (no fake IDs)
        let has_v1 = components
            .iter()
            .zip(&legacy_fees)
            .any(|(c, legacy)| c.prices.is_empty() && legacy.is_some());
        if has_v1 {
            let pv = PlanVersionRow::find_by_id_and_tenant_id(conn, plan_version_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            for (component, legacy) in components.iter_mut().zip(&legacy_fees) {
                if component.prices.is_empty()
                    && let Some(legacy_json) = legacy
                {
                    let legacy: LegacyPricingData =
                        extract_legacy_pricing(legacy_json, pv.currency.clone())?;
                    component.legacy_pricing = Some(legacy);
                }
            }
        }

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
                ensure_product_dimension_rates_add_up(
                    conn,
                    tenant_id,
                    product_id,
                    prices.iter().map(|price| &price.pricing),
                )
                .await?;

                // Validate price currencies match plan version
                let pv = lock_draft_version_tx(conn, component_row_new.plan_version_id, tenant_id)
                    .await?;
                for pi in &prices {
                    if crate::constants::Currencies::resolve_currency(&pi.currency).is_none() {
                        return Err(Report::new(StoreError::InvalidArgument(format!(
                            "unknown currency '{}' (plan version in {})",
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

                // A new charge's prices are the same in every option.
                let options = every_option(conn, tenant_id, pv.id).await?;
                let price_ids: Vec<PriceId> = price_rows.iter().map(|pr| pr.id).collect();
                link_prices(conn, inserted.id, &price_ids, &options).await?;

                let mut component: PriceComponent = inserted.try_into()?;
                attach_prices(conn, tenant_id, std::slice::from_mut(&mut component)).await?;
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
        keep: Vec<PriceId>,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
    ) -> StoreResult<PriceComponent> {
        // No price, none kept: the product is included, its features only.
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
                ensure_product_dimension_rates_add_up(
                    conn,
                    tenant_id,
                    product_id,
                    prices.iter().map(|price| &price.pricing),
                )
                .await?;

                // Validate price currencies match plan version
                let current = PriceComponentRow::get_by_id(conn, tenant_id, component_id).await?;
                if current.plan_version_id != plan_version_id {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "the price component belongs to another plan version".to_string(),
                    )));
                }
                let pv = lock_draft_version_tx(conn, plan_version_id, tenant_id).await?;
                for pi in &prices {
                    if crate::constants::Currencies::resolve_currency(&pi.currency).is_none() {
                        return Err(Report::new(StoreError::InvalidArgument(format!(
                            "unknown currency '{}' (plan version in {})",
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

                let own =
                    PlanComponentPriceRow::list_by_component_ids(conn, &[component_id]).await?;
                if keep
                    .iter()
                    .any(|id| !own.iter().any(|link| link.price_id == *id))
                {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "a price kept is one of the component's own".to_string(),
                    )));
                }
                PlanComponentPriceRow::delete_by_component_except(conn, component_id, &keep)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Create new Price entities
                let price_rows = if price_rows_new.is_empty() {
                    Vec::new()
                } else {
                    PriceRowNew::insert_batch(conn, &price_rows_new)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                };

                // New prices are the same in every option; the kept keep their options.
                let options = every_option(conn, tenant_id, pv.id).await?;
                let price_ids: Vec<PriceId> = price_rows.iter().map(|pr| pr.id).collect();
                link_prices(conn, updated.id, &price_ids, &options).await?;

                let mut component: PriceComponent = updated.try_into()?;
                attach_prices(conn, tenant_id, std::slice::from_mut(&mut component)).await?;
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

                // A new charge's prices are the same in every option.
                let options = every_option(conn, tenant_id, plan_version_id).await?;
                link_prices(conn, inserted.id, &price_ids, &options).await?;

                let mut component: PriceComponent = inserted.try_into()?;
                attach_prices(conn, tenant_id, std::slice::from_mut(&mut component)).await?;
                Ok(component)
            }
            .scope_boxed()
        })
        .await
    }

    async fn set_component_price(
        &self,
        tenant_id: TenantId,
        component_id: PriceComponentId,
        options: Vec<PlanOptionId>,
        price: PriceInput,
    ) -> StoreResult<PriceComponent> {
        write_component_cell(
            self,
            tenant_id,
            component_id,
            options,
            vec![price],
            CellWrite::Level,
        )
        .await
    }

    async fn set_component_prices(
        &self,
        tenant_id: TenantId,
        component_id: PriceComponentId,
        options: Vec<PlanOptionId>,
        prices: Vec<PriceInput>,
    ) -> StoreResult<PriceComponent> {
        write_component_cell(
            self,
            tenant_id,
            component_id,
            options,
            prices,
            CellWrite::Whole,
        )
        .await
    }

    async fn clear_component_price(
        &self,
        tenant_id: TenantId,
        component_id: PriceComponentId,
        options: Vec<PlanOptionId>,
        currency: String,
    ) -> StoreResult<PriceComponent> {
        self.transaction(|conn| {
            async move {
                let (plan_version_id, component) =
                    lock_draft_component(self, conn, tenant_id, component_id).await?;
                let options = named_options(conn, tenant_id, plan_version_id, options).await?;
                let cleared: Vec<(PlanOptionId, Vec<PriceId>)> = options
                    .iter()
                    .map(|option| {
                        let prices = option_cell(&component, *option, &currency)
                            .into_iter()
                            .map(|price| price.id)
                            .collect();
                        (*option, prices)
                    })
                    .collect();
                // Without a price the charge would silently become included.
                let left = component.option_prices.iter().any(|link| {
                    !cleared.iter().any(|(option, prices)| {
                        link.option_id == *option && prices.contains(&link.price_id)
                    })
                });
                if !component.option_prices.is_empty() && !left {
                    return Err(Report::new(StoreError::FailedPrecondition(
                        "This is the charge's last price. Remove the charge instead.".to_string(),
                    )));
                }
                for (option, prices) in &cleared {
                    PlanComponentPriceRow::delete_by_component_option_and_prices(
                        conn,
                        component_id,
                        *option,
                        prices,
                    )
                    .await?;
                }

                component_with_prices(self, conn, tenant_id, plan_version_id, component_id).await
            }
            .scope_boxed()
        })
        .await
    }
}
