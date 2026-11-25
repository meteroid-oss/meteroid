//! CRUD for the tenant's credit unit catalog. Code and precision are immutable once
//! created (`CreditUnitPatch` carries neither); only name,
//! plural name, symbol and `archived_at` can change.
use crate::StoreResult;
use crate::domain::credit_units::{
    CreditUnit, CreditUnitNew, CreditUnitPatch, CreditUnitPrice, CreditUnitsById,
    credit_unit_price_in,
};
use crate::errors::StoreError;
use crate::store::{PgConn, Store};
use common_domain::ids::{BaseId, CreditUnitId, CreditUnitPriceId, ProductId, TenantId};
use diesel_models::credit_units::{
    CreditUnitPriceRow, CreditUnitPriceRowNew, CreditUnitRow, CreditUnitRowNew, CreditUnitRowPatch,
};
use diesel_models::errors::DatabaseError;
use diesel_models::products::ProductRow;
use error_stack::Report;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;

#[async_trait::async_trait]
pub trait CreditUnitsInterface {
    async fn create_credit_unit(&self, new: CreditUnitNew) -> StoreResult<CreditUnit>;

    /// `create_credit_unit` with the product its sales are booked under and its selling prices
    /// (currency, price), in one transaction: a refusal of any writes none.
    async fn create_credit_unit_with_prices(
        &self,
        new: CreditUnitNew,
        product_id: Option<ProductId>,
        prices: Vec<(String, Decimal)>,
    ) -> StoreResult<CreditUnit>;

    /// Ordered by code; archived units are included only when asked.
    async fn list_credit_units(
        &self,
        tenant_id: TenantId,
        include_archived: bool,
    ) -> StoreResult<Vec<CreditUnit>>;

    async fn get_credit_unit(
        &self,
        tenant_id: TenantId,
        id: CreditUnitId,
    ) -> StoreResult<CreditUnit>;

    async fn update_credit_unit(&self, patch: CreditUnitPatch) -> StoreResult<CreditUnit>;

    /// `update_credit_unit` when `patch` is set, then each price (currency, price) set as
    /// `set_credit_unit_price` sets it or, `None`, the currency's active price archived, in one
    /// transaction: a refusal of any writes none.
    async fn update_credit_unit_with_prices(
        &self,
        tenant_id: TenantId,
        id: CreditUnitId,
        patch: Option<CreditUnitPatch>,
        prices: Vec<(String, Option<Decimal>)>,
    ) -> StoreResult<CreditUnit>;

    /// A page of the tenant's units by keyset (API v2): after `after`, in id order, at most
    /// `limit`; archived ones only when asked.
    async fn list_credit_units_after(
        &self,
        tenant_id: TenantId,
        include_archived: bool,
        after: Option<CreditUnitId>,
        limit: usize,
    ) -> StoreResult<Vec<CreditUnit>>;

    /// The tenant's unit of `code` (trimmed, upper-cased), archived or not.
    async fn find_credit_unit_by_code(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<CreditUnit>;

    /// The units fee resolution needs (`Price::credit_unit_ids`), archived ones included.
    async fn get_credit_units_by_ids(
        &self,
        tenant_id: TenantId,
        ids: Vec<CreditUnitId>,
    ) -> StoreResult<CreditUnitsById>;

    /// The single place that refuses an archived (or unknown) unit for anything newly priced or
    /// granted in it, called before accepting a `credit_unit_id` on a price or pack.
    /// Existing prices, packs and wallets keep resolving the unit through
    /// `repositories::credit::denomination::resolve_denomination`, archived or not.
    async fn ensure_credit_unit_active(
        &self,
        tenant_id: TenantId,
        id: CreditUnitId,
    ) -> StoreResult<CreditUnit>;

    /// Selling prices of the units, by unit then currency, newest first.
    async fn list_credit_unit_prices(
        &self,
        tenant_id: TenantId,
        unit_ids: Vec<CreditUnitId>,
        include_archived: bool,
    ) -> StoreResult<Vec<CreditUnitPrice>>;

    /// Replaces the unit's price in `currency`: the previous one is archived, never edited, so a
    /// sale that pinned it keeps its terms.
    async fn set_credit_unit_price(
        &self,
        tenant_id: TenantId,
        unit_id: CreditUnitId,
        currency: &str,
        unit_price: Decimal,
    ) -> StoreResult<CreditUnitPrice>;

    /// The unit can no longer be sold in that price's currency until a new price is set.
    async fn archive_credit_unit_price(
        &self,
        tenant_id: TenantId,
        unit_id: CreditUnitId,
        price_id: CreditUnitPriceId,
    ) -> StoreResult<CreditUnitPrice>;
}

#[async_trait::async_trait]
impl CreditUnitsInterface for Store {
    async fn create_credit_unit(&self, new: CreditUnitNew) -> StoreResult<CreditUnit> {
        let mut conn = self.get_conn().await?;
        insert_unit(&mut conn, new).await?.try_into()
    }

    async fn create_credit_unit_with_prices(
        &self,
        new: CreditUnitNew,
        product_id: Option<ProductId>,
        prices: Vec<(String, Decimal)>,
    ) -> StoreResult<CreditUnit> {
        let tenant_id = new.tenant_id;
        let prices = prices
            .into_iter()
            .map(|(currency, unit_price)| credit_unit_price_in(&currency, unit_price))
            .collect::<StoreResult<Vec<_>>>()?;
        // A new unit has no wallet yet: no automatic top-up to stop.
        self.transaction_bumping_tenant(tenant_id, |conn| {
            async move {
                let mut row = insert_unit(conn, new).await?;
                let id = row.id;
                if let Some(product_id) = product_id {
                    ensure_sellable_product(conn, tenant_id, product_id).await?;
                    let patch = CreditUnitRowPatch {
                        name: None,
                        plural_name: None,
                        symbol: None,
                        archived_at: None,
                        product_id: Some(Some(product_id)),
                    };
                    row = CreditUnitRow::update(conn, tenant_id, id, &patch)
                        .await
                        .map_err(|err| Report::new(StoreError::DatabaseError(err.error)))?
                        .ok_or_else(|| {
                            Report::new(StoreError::ValueNotFound(format!("credit unit {id}")))
                        })?;
                }
                for (currency, unit_price) in prices {
                    CreditUnitPriceRowNew {
                        id: CreditUnitPriceId::new(),
                        tenant_id,
                        credit_unit_id: id,
                        currency,
                        unit_price,
                    }
                    .insert(conn)
                    .await?;
                }
                row.try_into()
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_credit_units(
        &self,
        tenant_id: TenantId,
        include_archived: bool,
    ) -> StoreResult<Vec<CreditUnit>> {
        let mut conn = self.get_conn().await?;

        CreditUnitRow::list_for_tenant(&mut conn, tenant_id, include_archived)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }

    async fn get_credit_unit(
        &self,
        tenant_id: TenantId,
        id: CreditUnitId,
    ) -> StoreResult<CreditUnit> {
        let mut conn = self.get_conn().await?;

        CreditUnitRow::find_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?
            .ok_or_else(|| Report::new(StoreError::ValueNotFound(format!("credit unit {id}"))))?
            .try_into()
    }

    async fn update_credit_unit(&self, patch: CreditUnitPatch) -> StoreResult<CreditUnit> {
        let mut conn = self.get_conn().await?;
        if let Some(Some(product_id)) = patch.product_id {
            ensure_sellable_product(&mut conn, patch.tenant_id, product_id).await?;
        }
        let row_patch: CreditUnitRowPatch = (&patch).into();

        let (tenant_id, id) = (patch.tenant_id, patch.id);
        drop(conn);
        let unit: CreditUnit = self
            .transaction_bumping_tenant(tenant_id, |conn| {
                async move {
                    CreditUnitRow::update(conn, tenant_id, id, &row_patch)
                        .await
                        .map_err(|err| Report::new(StoreError::DatabaseError(err.error)))
                }
                .scope_boxed()
            })
            .await?
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(format!(
                    "credit unit {}",
                    patch.id
                )))
            })?
            .try_into()?;
        // Clearing its product (or archiving it) leaves the unit unsellable.
        self.stop_unit_auto_top_ups(patch.tenant_id, patch.id)
            .await?;
        Ok(unit)
    }

    async fn update_credit_unit_with_prices(
        &self,
        tenant_id: TenantId,
        id: CreditUnitId,
        patch: Option<CreditUnitPatch>,
        prices: Vec<(String, Option<Decimal>)>,
    ) -> StoreResult<CreditUnit> {
        if patch.is_none() && prices.is_empty() {
            return self.get_credit_unit(tenant_id, id).await;
        }
        let prices = prices
            .into_iter()
            .map(|(currency, unit_price)| match unit_price {
                Some(unit_price) => credit_unit_price_in(&currency, unit_price)
                    .map(|(currency, unit_price)| (currency, Some(unit_price))),
                None => Ok((currency.trim().to_ascii_uppercase(), None)),
            })
            .collect::<StoreResult<Vec<_>>>()?;
        let unit: CreditUnit = self
            .transaction_bumping_tenant(tenant_id, |conn| {
                async move {
                    if let Some(patch) = &patch {
                        if let Some(Some(product_id)) = patch.product_id {
                            ensure_sellable_product(conn, tenant_id, product_id).await?;
                        }
                        let row_patch: CreditUnitRowPatch = patch.into();
                        CreditUnitRow::update(conn, tenant_id, id, &row_patch)
                            .await
                            .map_err(|err| Report::new(StoreError::DatabaseError(err.error)))?;
                    }
                    let unit: CreditUnit = CreditUnitRow::lock_by_id(conn, tenant_id, id)
                        .await?
                        .ok_or_else(|| {
                            Report::new(StoreError::ValueNotFound(format!("credit unit {id}")))
                        })?
                        .try_into()?;
                    for (currency, unit_price) in prices {
                        CreditUnitPriceRow::archive_active(conn, tenant_id, id, &currency).await?;
                        if let Some(unit_price) = unit_price {
                            unit.ensure_active_for_new_pricing()?;
                            CreditUnitPriceRowNew {
                                id: CreditUnitPriceId::new(),
                                tenant_id,
                                credit_unit_id: id,
                                currency,
                                unit_price,
                            }
                            .insert(conn)
                            .await?;
                        }
                    }
                    Ok(unit)
                }
                .scope_boxed()
            })
            .await?;
        // Its product cleared or a price changed: the unit's automatic top-ups are checked again.
        self.stop_unit_auto_top_ups(tenant_id, id).await?;
        Ok(unit)
    }

    async fn list_credit_units_after(
        &self,
        tenant_id: TenantId,
        include_archived: bool,
        after: Option<CreditUnitId>,
        limit: usize,
    ) -> StoreResult<Vec<CreditUnit>> {
        let mut conn = self.get_conn().await?;
        CreditUnitRow::list_page(
            &mut conn,
            tenant_id,
            include_archived,
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn find_credit_unit_by_code(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<CreditUnit> {
        let code = code.trim().to_ascii_uppercase();
        let mut conn = self.get_conn().await?;
        CreditUnitRow::find_by_code(&mut conn, tenant_id, &code)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?
            .ok_or_else(|| Report::new(StoreError::ValueNotFound(format!("credit unit {code}"))))?
            .try_into()
    }

    async fn get_credit_units_by_ids(
        &self,
        tenant_id: TenantId,
        ids: Vec<CreditUnitId>,
    ) -> StoreResult<CreditUnitsById> {
        let mut conn = self.get_conn().await?;
        load_credit_units(&mut conn, tenant_id, ids).await
    }

    async fn ensure_credit_unit_active(
        &self,
        tenant_id: TenantId,
        id: CreditUnitId,
    ) -> StoreResult<CreditUnit> {
        let unit = self.get_credit_unit(tenant_id, id).await?;
        unit.ensure_active_for_new_pricing()?;
        Ok(unit)
    }

    async fn list_credit_unit_prices(
        &self,
        tenant_id: TenantId,
        unit_ids: Vec<CreditUnitId>,
        include_archived: bool,
    ) -> StoreResult<Vec<CreditUnitPrice>> {
        let mut conn = self.get_conn().await?;
        Ok(
            CreditUnitPriceRow::list_by_units(&mut conn, tenant_id, &unit_ids, include_archived)
                .await?
                .into_iter()
                .map(CreditUnitPrice::from)
                .collect(),
        )
    }

    async fn set_credit_unit_price(
        &self,
        tenant_id: TenantId,
        unit_id: CreditUnitId,
        currency: &str,
        unit_price: Decimal,
    ) -> StoreResult<CreditUnitPrice> {
        let (currency, unit_price) = credit_unit_price_in(currency, unit_price)?;
        let price = self
            .transaction_bumping_tenant(tenant_id, |conn| {
                async move {
                    let unit: CreditUnit = CreditUnitRow::lock_by_id(conn, tenant_id, unit_id)
                        .await?
                        .ok_or_else(|| {
                            Report::new(StoreError::ValueNotFound(format!("credit unit {unit_id}")))
                        })?
                        .try_into()?;
                    unit.ensure_active_for_new_pricing()?;
                    CreditUnitPriceRow::archive_active(conn, tenant_id, unit_id, &currency).await?;
                    let row = CreditUnitPriceRowNew {
                        id: CreditUnitPriceId::new(),
                        tenant_id,
                        credit_unit_id: unit_id,
                        currency,
                        unit_price,
                    }
                    .insert(conn)
                    .await?;
                    Ok(CreditUnitPrice::from(row))
                }
                .scope_boxed()
            })
            .await?;
        self.stop_unit_auto_top_ups(tenant_id, unit_id).await?;
        Ok(price)
    }

    async fn archive_credit_unit_price(
        &self,
        tenant_id: TenantId,
        unit_id: CreditUnitId,
        price_id: CreditUnitPriceId,
    ) -> StoreResult<CreditUnitPrice> {
        let price = self
            .transaction_bumping_tenant(tenant_id, |conn| {
                async move {
                    CreditUnitRow::lock_by_id(conn, tenant_id, unit_id)
                        .await?
                        .ok_or_else(|| {
                            Report::new(StoreError::ValueNotFound(format!("credit unit {unit_id}")))
                        })?;
                    let price = CreditUnitPriceRow::find_by_id(conn, tenant_id, price_id)
                        .await?
                        .filter(|price| price.credit_unit_id == unit_id)
                        .ok_or_else(|| {
                            Report::new(StoreError::ValueNotFound(format!(
                                "credit unit price {price_id}"
                            )))
                        })?;
                    if price.archived_at.is_some() {
                        return Ok(CreditUnitPrice::from(price));
                    }
                    CreditUnitPriceRow::archive_active(conn, tenant_id, unit_id, &price.currency)
                        .await?
                        .map(CreditUnitPrice::from)
                        .ok_or_else(|| {
                            Report::new(StoreError::ValueNotFound(format!(
                                "credit unit price {price_id}"
                            )))
                        })
                }
                .scope_boxed()
            })
            .await?;
        self.stop_unit_auto_top_ups(tenant_id, unit_id).await?;
        Ok(price)
    }
}

impl Store {
    /// A unit's price or sellability changed: the automatic top-ups of its wallets that can no
    /// longer charge what they were authorized at stop now (customer locks are taken after the
    /// unit's transaction, never under the unit row).
    async fn stop_unit_auto_top_ups(
        &self,
        tenant_id: TenantId,
        unit_id: CreditUnitId,
    ) -> StoreResult<()> {
        self.hooks
            .credit
            .credit_unit_price_changed(self, tenant_id, unit_id)
            .await
    }
}

/// Inserts the unit, its code taken read as a duplicate.
async fn insert_unit(conn: &mut PgConn, new: CreditUnitNew) -> StoreResult<CreditUnitRow> {
    let code = new.code.as_str().to_owned();
    let insertable: CreditUnitRowNew = new.into();
    insertable.insert(conn).await.map_err(|err| {
        if matches!(err.error.current_context(), DatabaseError::UniqueViolation) {
            Report::new(StoreError::DuplicateValue {
                entity: "credit unit",
                key: Some(code),
            })
        } else {
            Into::<Report<StoreError>>::into(err)
        }
    })
}

/// The product unit sales are booked under: the tenant's own, still in the catalog.
async fn ensure_sellable_product(
    conn: &mut PgConn,
    tenant_id: TenantId,
    product_id: ProductId,
) -> StoreResult<()> {
    let product = ProductRow::find_by_id_and_tenant_id(conn, product_id, tenant_id).await?;
    if product.archived_at.is_some() {
        return Err(Report::new(StoreError::FailedPrecondition(format!(
            "product {product_id} is archived"
        ))));
    }
    Ok(())
}

/// See [`CreditUnitsInterface::get_credit_units_by_ids`].
pub(crate) async fn load_credit_units(
    conn: &mut PgConn,
    tenant_id: TenantId,
    ids: impl IntoIterator<Item = CreditUnitId>,
) -> StoreResult<CreditUnitsById> {
    let mut ids: Vec<CreditUnitId> = ids.into_iter().collect();
    ids.sort_unstable_by_key(|id| id.as_uuid());
    ids.dedup();
    if ids.is_empty() {
        return Ok(CreditUnitsById::new());
    }
    CreditUnitRow::list_by_ids(conn, tenant_id, &ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(|row| CreditUnit::try_from(row).map(|unit| (unit.id, unit)))
        .collect()
}

/// The credit units add-on requests resolve against: their catalog prices' and their overrides'.
pub(crate) async fn load_add_on_credit_units(
    conn: &mut PgConn,
    tenant_id: TenantId,
    prices_by_id: &std::collections::HashMap<common_domain::ids::PriceId, crate::domain::Price>,
    requests: &[crate::domain::subscription_add_ons::CreateSubscriptionAddOn],
) -> StoreResult<CreditUnitsById> {
    load_credit_units(
        conn,
        tenant_id,
        prices_by_id
            .values()
            .flat_map(crate::domain::Price::credit_unit_ids)
            .chain(
                requests
                    .iter()
                    .flat_map(|request| request.customization.new_credit_unit_ids()),
            )
            .collect::<Vec<_>>(),
    )
    .await
}
