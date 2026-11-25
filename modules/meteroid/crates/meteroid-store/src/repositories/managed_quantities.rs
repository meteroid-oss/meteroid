//! The tenant's managed quantities. Created with the seat products and ledgers that use them;
//! their code is immutable, their name and policies can change.
use crate::StoreResult;
use crate::domain::managed_quantities::{
    ManagedQuantity, ManagedQuantityNew, ManagedQuantityPatch, validate_quantity_code,
};
use crate::errors::StoreError;
use crate::store::Store;
use common_domain::ids::{BaseId, FeatureId, ManagedQuantityId, TenantId};
use diesel_models::entitlements::FeatureRow;
use diesel_models::enums::FeatureTypeEnum;
use diesel_models::managed_quantities::{
    ManagedQuantityRow, ManagedQuantityRowNew, ManagedQuantityRowPatch,
};
use diesel_models::query::managed_quantities::{list_feature_limits, set_feature_limit_quantity};
use error_stack::Report;

#[async_trait::async_trait]
pub trait ManagedQuantitiesInterface {
    /// Ordered by code.
    async fn list_managed_quantities(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ManagedQuantity>>;

    /// A new quantity, refused when the tenant has one of its code.
    async fn create_managed_quantity(
        &self,
        new: ManagedQuantityNew,
    ) -> StoreResult<ManagedQuantity>;

    async fn update_managed_quantity(
        &self,
        patch: ManagedQuantityPatch,
    ) -> StoreResult<ManagedQuantity>;

    /// Counts the metered `feature`'s limit per unit of `quantity` a subscription holds, or per
    /// subscription again when none.
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

    async fn set_feature_limit_quantity(
        &self,
        tenant_id: TenantId,
        feature: FeatureId,
        quantity: Option<ManagedQuantityId>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let found = FeatureRow::find_by_id(&mut conn, feature, tenant_id).await?;
        if found.feature.feature_type != FeatureTypeEnum::Metered {
            return Err(Report::new(StoreError::InvalidArgument(
                "only a metered feature's limit counts per unit of a quantity".to_owned(),
            )));
        }
        if let Some(quantity) = quantity {
            let known = ManagedQuantityRow::list(&mut conn, tenant_id)
                .await?
                .iter()
                .any(|row| row.id == quantity && row.archived_at.is_none());
            if !known {
                return Err(Report::new(StoreError::ValueNotFound(format!(
                    "managed quantity {quantity}"
                ))));
            }
        }
        Ok(set_feature_limit_quantity(&mut conn, feature, quantity).await?)
    }
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
