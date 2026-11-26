//! The tenant's managed quantities. Created with the seat products and ledgers that use them;
//! their code is immutable, their name and policies can change.
use crate::StoreResult;
use crate::domain::entitlements::{Feature, FeatureKind};
use crate::domain::enums::FeatureStatusEnum;
use crate::domain::managed_quantities::{
    ManagedQuantity, ManagedQuantityNew, ManagedQuantityPatch, validate_quantity_code,
};
use crate::errors::StoreError;
use crate::store::Store;
use common_domain::ids::{BaseId, FeatureId, ManagedQuantityId, TenantId};
use diesel_models::customer_credit::CreditRecomputeMarkRow;
use diesel_models::entitlements::FeatureRow;
use diesel_models::enums::EntitlementEntityTypeEnum;
use diesel_models::managed_quantities::{
    ManagedQuantityRow, ManagedQuantityRowNew, ManagedQuantityRowPatch,
};
use diesel_models::query::managed_quantities::{list_feature_limits, set_feature_limit_quantity};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

#[async_trait::async_trait]
pub trait ManagedQuantitiesInterface {
    /// Ordered by code.
    async fn list_managed_quantities(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ManagedQuantity>>;

    /// The tenant's quantities after `after` in id order, at most `limit`, read as
    /// `list_managed_quantities` reads them, the archived ones only when asked: a keyset page of
    /// API v2.
    async fn list_managed_quantities_after(
        &self,
        tenant_id: TenantId,
        include_archived: bool,
        after: Option<ManagedQuantityId>,
        limit: usize,
    ) -> StoreResult<Vec<ManagedQuantity>>;

    /// Off the catalog for new charges; the subscriptions holding it keep counting and changing
    /// it. Archiving an archived quantity changes nothing.
    async fn archive_managed_quantity(
        &self,
        tenant_id: TenantId,
        id: ManagedQuantityId,
    ) -> StoreResult<ManagedQuantity>;

    async fn unarchive_managed_quantity(
        &self,
        tenant_id: TenantId,
        id: ManagedQuantityId,
    ) -> StoreResult<ManagedQuantity>;

    /// A new quantity, refused when the tenant has one of its code.
    async fn create_managed_quantity(
        &self,
        new: ManagedQuantityNew,
    ) -> StoreResult<ManagedQuantity>;

    async fn update_managed_quantity(
        &self,
        patch: ManagedQuantityPatch,
    ) -> StoreResult<ManagedQuantity>;

    /// Counts the limit `feature` grants per unit of `quantity` a subscription holds, or per
    /// subscription again when none (R-133). Any limit: one counted from events or by the app.
    async fn set_feature_limit_quantity(
        &self,
        tenant_id: TenantId,
        feature: FeatureId,
        quantity: Option<ManagedQuantityId>,
    ) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl ManagedQuantitiesInterface for Store {
    async fn list_managed_quantities(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ManagedQuantity>> {
        let mut conn = self.get_conn().await?;
        let mut quantities: Vec<ManagedQuantity> = ManagedQuantityRow::list(&mut conn, tenant_id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
        for (quantity, feature) in list_feature_limits(&mut conn, tenant_id).await? {
            if let Some(found) = quantities.iter_mut().find(|q| q.id == quantity) {
                found.per_unit_features.push(feature);
            }
        }
        Ok(quantities)
    }

    async fn list_managed_quantities_after(
        &self,
        tenant_id: TenantId,
        include_archived: bool,
        after: Option<ManagedQuantityId>,
        limit: usize,
    ) -> StoreResult<Vec<ManagedQuantity>> {
        let mut conn = self.get_conn().await?;
        let mut quantities: Vec<ManagedQuantity> = ManagedQuantityRow::list_page(
            &mut conn,
            tenant_id,
            include_archived,
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
        if !quantities.is_empty() {
            for (quantity, feature) in list_feature_limits(&mut conn, tenant_id).await? {
                if let Some(found) = quantities.iter_mut().find(|q| q.id == quantity) {
                    found.per_unit_features.push(feature);
                }
            }
        }
        Ok(quantities)
    }

    async fn create_managed_quantity(
        &self,
        new: ManagedQuantityNew,
    ) -> StoreResult<ManagedQuantity> {
        let code = new.code.trim().to_owned();
        validate_quantity_code(&code)?;
        let name = new.name.trim().to_owned();
        if name.is_empty() {
            return Err(Report::new(StoreError::InvalidArgument(
                "A quantity has a name".to_owned(),
            )));
        }
        let mut conn = self.get_conn().await?;
        let row = ManagedQuantityRow::insert(
            &mut conn,
            &ManagedQuantityRowNew {
                id: ManagedQuantityId::new(),
                tenant_id: new.tenant_id,
                code: code.clone(),
                name,
            },
        )
        .await?
        .ok_or_else(|| {
            Report::new(StoreError::DuplicateValue {
                entity: "managed quantity",
                key: Some(code),
            })
        })?;
        // The policies are set past the defaults the row is inserted with.
        let patch = ManagedQuantityRowPatch {
            name: None,
            on_increase: Some(new.on_increase.into()),
            on_decrease: Some(new.on_decrease.into()),
        };
        Ok(
            ManagedQuantityRow::update(&mut conn, new.tenant_id, row.id, &patch)
                .await?
                .into(),
        )
    }

    async fn update_managed_quantity(
        &self,
        patch: ManagedQuantityPatch,
    ) -> StoreResult<ManagedQuantity> {
        let mut conn = self.get_conn().await?;
        let row_patch: ManagedQuantityRowPatch = (&patch).into();
        Ok(
            ManagedQuantityRow::update(&mut conn, patch.tenant_id, patch.id, &row_patch)
                .await?
                .into(),
        )
    }

    async fn archive_managed_quantity(
        &self,
        tenant_id: TenantId,
        id: ManagedQuantityId,
    ) -> StoreResult<ManagedQuantity> {
        set_quantity_archived(self, tenant_id, id, true).await
    }

    async fn unarchive_managed_quantity(
        &self,
        tenant_id: TenantId,
        id: ManagedQuantityId,
    ) -> StoreResult<ManagedQuantity> {
        set_quantity_archived(self, tenant_id, id, false).await
    }

    async fn set_feature_limit_quantity(
        &self,
        tenant_id: TenantId,
        feature: FeatureId,
        quantity: Option<ManagedQuantityId>,
    ) -> StoreResult<()> {
        // Every subscription's resolved limit moves: the tenant's inputs are bumped.
        self.transaction_bumping_tenant(tenant_id, |conn| {
            async move {
                let found: Feature = FeatureRow::find_by_id(conn, feature, tenant_id)
                    .await?
                    .try_into()?;
                if !matches!(found.kind, FeatureKind::Limit(_)) {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "only a limit counts per unit of a quantity".to_owned(),
                    )));
                }
                if found.status == FeatureStatusEnum::Archived {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "feature is archived; unarchive it first".to_owned(),
                    )));
                }
                if let Some(quantity) = quantity {
                    let known = ManagedQuantityRow::find(conn, tenant_id, quantity)
                        .await?
                        .is_some_and(|row| row.archived_at.is_none());
                    if !known {
                        return Err(Report::new(StoreError::ValueNotFound(format!(
                            "managed quantity {quantity}"
                        ))));
                    }
                }
                set_feature_limit_quantity(conn, feature, quantity).await?;
                CreditRecomputeMarkRow::mark_for_entitlement_owner(
                    conn,
                    tenant_id,
                    &EntitlementEntityTypeEnum::Feature,
                    feature.as_uuid(),
                )
                .await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }
}

/// Archives or restores the quantity, with the features counting their limit per unit of it;
/// one already so is read back unchanged.
async fn set_quantity_archived(
    store: &Store,
    tenant_id: TenantId,
    id: ManagedQuantityId,
    archived: bool,
) -> StoreResult<ManagedQuantity> {
    let mut conn = store.get_conn().await?;
    let not_found = || Report::new(StoreError::ValueNotFound(format!("managed quantity {id}")));
    let current = ManagedQuantityRow::find(&mut conn, tenant_id, id)
        .await?
        .ok_or_else(not_found)?;
    let row = if current.archived_at.is_some() == archived {
        current
    } else {
        ManagedQuantityRow::set_archived(&mut conn, tenant_id, id, archived.then(common_clock::now))
            .await?
            .ok_or_else(not_found)?
    };
    let mut quantity: ManagedQuantity = row.into();
    quantity.per_unit_features = list_feature_limits(&mut conn, tenant_id)
        .await?
        .into_iter()
        .filter(|(of, _)| *of == id)
        .map(|(_, feature)| feature)
        .collect();
    Ok(quantity)
}

/// The tenant's quantities a seat unit names, read once for many units: see
/// `domain::managed_quantities::quantity_of_unit`.
pub(crate) async fn quantities_of_units(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    units: &[&str],
) -> StoreResult<std::collections::HashMap<String, ManagedQuantityRow>> {
    if units.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let quantities = ManagedQuantityRow::list(conn, tenant_id).await?;
    Ok(units
        .iter()
        .filter_map(|unit| {
            crate::domain::managed_quantities::quantity_of_unit(&quantities, unit)
                .map(|row| ((*unit).to_owned(), row.clone()))
        })
        .collect())
}

/// The tenant's quantity a seat unit names, created as "New quantity…" would when none does: its
/// code made of the unit, its name the unit. A charge or a ledger row naming no quantity id
/// (REST v1, a client omitting it, a fee recorded before quantities were named) counts it, so
/// "Seats" and "seats" are one count.
pub(crate) async fn ensure_quantity_of_unit(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    unit: &str,
) -> StoreResult<ManagedQuantityRow> {
    if let Some(found) = quantities_of_units(conn, tenant_id, &[unit])
        .await?
        .remove(unit)
    {
        return Ok(found);
    }
    let code = crate::domain::managed_quantities::quantity_code(unit);
    validate_quantity_code(&code).map_err(|_| {
        Report::new(StoreError::InvalidArgument(format!(
            "a seat charge names the quantity it counts: {unit:?} names none"
        )))
    })?;
    let inserted = ManagedQuantityRow::insert(
        conn,
        &ManagedQuantityRowNew {
            id: ManagedQuantityId::new(),
            tenant_id,
            code: code.clone(),
            name: unit.trim().to_owned(),
        },
    )
    .await?;
    match inserted {
        Some(row) => Ok(row),
        // Made meanwhile by another request.
        None => ManagedQuantityRow::find_by_code(conn, tenant_id, &code)
            .await?
            .ok_or_else(|| Report::new(StoreError::ValueNotFound(format!("quantity {code}")))),
    }
}
