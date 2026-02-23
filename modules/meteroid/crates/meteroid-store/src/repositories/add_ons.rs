use crate::domain::add_ons::{AddOn, AddOnNew, AddOnPatch};
use crate::domain::enums::FeeTypeEnum;
use crate::domain::price_components::{PriceComponentNewInternal, PriceEntry, ProductRef};
use crate::domain::{PaginatedVec, PaginationRequest, Price};
use crate::errors::StoreError;
use crate::repositories::price_components::resolve_component_internal;
use crate::{Store, StoreResult};
use common_domain::ids::{AddOnId, BaseId, PlanVersionId, ProductFamilyId, TenantId};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::add_ons::{AddOnRow, AddOnRowNew, AddOnRowPatch};
use diesel_models::prices::PriceRow;
use diesel_models::products::ProductRow;
use error_stack::Report;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait AddOnInterface {
    async fn list_add_ons(
        &self,
        tenant_id: TenantId,
        plan_version_id: Option<PlanVersionId>,
        pagination: PaginationRequest,
        search: Option<String>,
        currency: Option<String>,
    ) -> StoreResult<PaginatedVec<AddOn>>;

    async fn list_add_ons_by_ids(
        &self,
        tenant_id: TenantId,
        ids: Vec<AddOnId>,
    ) -> StoreResult<Vec<AddOn>>;

    async fn get_add_on_by_id(&self, tenant_id: TenantId, id: AddOnId) -> StoreResult<AddOn>;

    async fn create_add_on(&self, add_on: AddOnNew) -> StoreResult<AddOn>;

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
        created_by: Uuid,
        product_family_id: ProductFamilyId,
    ) -> StoreResult<AddOn>;

    async fn update_add_on(
        &self,
        patch: AddOnPatch,
        price_entry: Option<PriceEntry>,
        created_by: Uuid,
    ) -> StoreResult<AddOn>;

    async fn archive_add_on(&self, id: AddOnId, tenant_id: TenantId) -> StoreResult<()>;
}

/// Eagerly load fee_type and price for a list of add-on rows
pub(crate) async fn enrich_add_ons(
    conn: &mut crate::store::PgConn,
    rows: Vec<AddOnRow>,
    tenant_id: TenantId,
) -> StoreResult<Vec<AddOn>> {
    use std::collections::HashMap;

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let product_ids: Vec<_> = rows.iter().map(|r| r.product_id).collect();
    let price_ids: Vec<_> = rows.iter().map(|r| r.price_id).collect();

    let product_rows = ProductRow::list_by_ids(conn, &product_ids, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let products: HashMap<_, _> = product_rows.into_iter().map(|p| (p.id, p)).collect();

    let price_rows = PriceRow::list_by_ids(conn, &price_ids, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let prices: HashMap<_, _> = price_rows
        .into_iter()
        .map(|r| {
            let id = r.id;
            Price::try_from(r).map(|p| (id, p))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;

    let add_ons = rows
        .into_iter()
        .map(|row| {
            let product = products.get(&row.product_id);
            let fee_type: Option<FeeTypeEnum> = product.map(|p| p.fee_type.clone().into());
            let fee_structure =
                product.and_then(|p| serde_json::from_value(p.fee_structure.clone()).ok());
            let price = prices.get(&row.price_id).cloned();
            let mut addon: AddOn = row.into();
            addon.fee_type = fee_type;
            addon.fee_structure = fee_structure;
            addon.price = price;
            addon
        })
        .collect();

    Ok(add_ons)
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
    ) -> StoreResult<PaginatedVec<AddOn>> {
        let mut conn = self.get_conn().await?;

        let paginated = AddOnRow::list_by_tenant_id(
            &mut conn,
            tenant_id,
            plan_version_id,
            pagination.into(),
            search,
            currency,
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

    async fn list_add_ons_by_ids(
        &self,
        tenant_id: TenantId,
        ids: Vec<AddOnId>,
    ) -> StoreResult<Vec<AddOn>> {
        let mut conn = self.get_conn().await?;

        let rows = AddOnRow::list_by_ids(&mut conn, &ids, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        enrich_add_ons(&mut conn, rows, tenant_id).await
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

    async fn create_add_on(&self, add_on: AddOnNew) -> StoreResult<AddOn> {
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
        let row_new: AddOnRowNew = add_on.into();

        let row = row_new
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut enriched = enrich_add_ons(&mut conn, vec![row], tenant_id).await?;
        enriched
            .pop()
            .ok_or_else(|| Report::new(StoreError::InvalidArgument("Add-on not found".into())))
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
        created_by: Uuid,
        product_family_id: ProductFamilyId,
    ) -> StoreResult<AddOn> {
        let internal = PriceComponentNewInternal {
            name: name.clone(),
            product_ref,
            prices: vec![price_entry],
        };

        self.transaction(|conn| {
            async move {
                let currency = match &internal.prices.first() {
                    Some(PriceEntry::New(input)) => input.currency.clone(),
                    Some(PriceEntry::Existing(pid)) => {
                        PriceRow::find_by_id_and_tenant_id(conn, *pid, tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                            .currency
                    }
                    None => {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "At least one price entry is required".into(),
                        )));
                    }
                };

                let (product_id, price_ids) = resolve_component_internal(
                    conn,
                    &internal,
                    tenant_id,
                    created_by,
                    product_family_id,
                    &currency,
                    true,
                )
                .await?;

                let price_id = price_ids.into_iter().next().ok_or_else(|| {
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
                };

                let row = row_new
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let mut enriched = enrich_add_ons(conn, vec![row], tenant_id).await?;
                enriched.pop().ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument("Add-on not found".into()))
                })
            }
            .scope_boxed()
        })
        .await
    }

    async fn update_add_on(
        &self,
        patch: AddOnPatch,
        price_entry: Option<PriceEntry>,
        created_by: Uuid,
    ) -> StoreResult<AddOn> {
        let tenant_id = patch.tenant_id;
        let add_on_id = patch.id;

        self.transaction(|conn| {
            async move {
                let existing = AddOnRow::get_by_id(conn, tenant_id, add_on_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let existing_price =
                    PriceRow::find_by_id_and_tenant_id(conn, existing.price_id, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                let new_price_id = if let Some(entry) = price_entry {
                    match entry {
                        PriceEntry::Existing(pid) => {
                            let price_row =
                                PriceRow::find_by_id_and_tenant_id(conn, pid, tenant_id)
                                    .await
                                    .map_err(Into::<Report<StoreError>>::into)?;
                            if price_row.product_id != existing.product_id {
                                return Err(Report::new(StoreError::InvalidArgument(format!(
                                    "Price {} belongs to product {}, not {}",
                                    pid, price_row.product_id, existing.product_id
                                ))));
                            }
                            if price_row.currency != existing_price.currency {
                                return Err(Report::new(StoreError::InvalidArgument(format!(
                                    "Price {} currency '{}' does not match add-on currency '{}'",
                                    pid, price_row.currency, existing_price.currency
                                ))));
                            }
                            Some(pid)
                        }
                        PriceEntry::New(input) => {
                            if input.currency != existing_price.currency {
                                return Err(Report::new(StoreError::InvalidArgument(format!(
                                    "Price currency '{}' does not match add-on currency '{}'",
                                    input.currency, existing_price.currency
                                ))));
                            }

                            let pricing_json =
                                serde_json::to_value(&input.pricing).map_err(|e| {
                                    Report::new(StoreError::SerdeError(
                                        "Failed to serialize pricing".to_string(),
                                        e,
                                    ))
                                })?;

                            let price_row = diesel_models::prices::PriceRowNew {
                                id: common_domain::ids::PriceId::new(),
                                product_id: existing.product_id,
                                cadence: input.cadence.into(),
                                currency: input.currency,
                                pricing: pricing_json,
                                tenant_id,
                                created_by,
                                catalog: true,
                            }
                            .insert(conn)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                            Some(price_row.id)
                        }
                    }
                } else {
                    None
                };

                let row_patch: AddOnRowPatch = patch.into_row_patch(new_price_id);

                let row = row_patch
                    .patch(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let mut enriched = enrich_add_ons(conn, vec![row], tenant_id).await?;
                enriched.pop().ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument("Add-on not found".into()))
                })
            }
            .scope_boxed()
        })
        .await
    }

    async fn archive_add_on(&self, id: AddOnId, tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        AddOnRow::archive(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}
