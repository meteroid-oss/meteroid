use std::collections::HashMap;

use crate::errors::IntoDbResult;
use crate::managed_quantities::{
    ManagedQuantityRow, ManagedQuantityRowNew, ManagedQuantityRowPatch,
};
use crate::{DbResult, PgConn};
use common_domain::ids::{BaseId, ManagedQuantityId, TenantId};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl ManagedQuantityRow {
    /// The quantities of `codes`, created under their code as name when the tenant has none yet.
    pub async fn ensure_codes(
        conn: &mut PgConn,
        tenant_id: TenantId,
        codes: &[&str],
    ) -> DbResult<HashMap<String, ManagedQuantityId>> {
        use crate::schema::managed_quantity::dsl as q;

        if codes.is_empty() {
            return Ok(HashMap::new());
        }
        let new: Vec<ManagedQuantityRowNew> = codes
            .iter()
            .map(|code| ManagedQuantityRowNew {
                id: ManagedQuantityId::new(),
                tenant_id,
                code: (*code).to_owned(),
                name: (*code).to_owned(),
            })
            .collect();
        let insert = diesel::insert_into(q::managed_quantity)
            .values(&new)
            .on_conflict((q::tenant_id, q::code))
            .do_nothing();
        crate::debug_sql(&insert);
        insert
            .execute(conn)
            .await
            .attach("Error while creating managed quantities")
            .into_db_result()?;

        let select = q::managed_quantity
            .filter(q::tenant_id.eq(tenant_id))
            .filter(q::code.eq_any(codes))
            .select((q::code, q::id));
        crate::debug_sql(&select);
        select
            .load::<(String, ManagedQuantityId)>(conn)
            .await
            .attach("Error while reading managed quantities")
            .into_db_result()
            .map(|rows| rows.into_iter().collect())
    }

    /// The tenant's quantities of `codes`, by code; a code without one is absent.
    pub async fn ids_of_codes(
        conn: &mut PgConn,
        tenant_id: TenantId,
        codes: &[&str],
    ) -> DbResult<HashMap<String, ManagedQuantityId>> {
        use crate::schema::managed_quantity::dsl as q;

        if codes.is_empty() {
            return Ok(HashMap::new());
        }
        let select = q::managed_quantity
            .filter(q::tenant_id.eq(tenant_id))
            .filter(q::code.eq_any(codes))
            .select((q::code, q::id));
        crate::debug_sql(&select);
        select
            .load::<(String, ManagedQuantityId)>(conn)
            .await
            .attach("Error while reading managed quantities")
            .into_db_result()
            .map(|rows| rows.into_iter().collect())
    }

    /// The tenant's quantities after `after` in id order, at most `limit` (API v2 keyset pages),
    /// the archived ones only when asked.
    pub async fn list_page(
        conn: &mut PgConn,
        tenant_id: TenantId,
        include_archived: bool,
        after: Option<ManagedQuantityId>,
        limit: i64,
    ) -> DbResult<Vec<ManagedQuantityRow>> {
        use crate::schema::managed_quantity::dsl as q;

        let mut query = q::managed_quantity
            .filter(q::tenant_id.eq(tenant_id))
            .into_boxed();
        if !include_archived {
            query = query.filter(q::archived_at.is_null());
        }
        if let Some(after) = after {
            query = query.filter(q::id.gt(after));
        }
        let query = query
            .order(q::id.asc())
            .limit(limit)
            .select(ManagedQuantityRow::as_select());
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while fetching a page of managed quantities")
            .into_db_result()
    }

    pub async fn list(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<Vec<ManagedQuantityRow>> {
        use crate::schema::managed_quantity::dsl as q;

        let query = q::managed_quantity
            .filter(q::tenant_id.eq(tenant_id))
            .order(q::code.asc())
            .select(ManagedQuantityRow::as_select());
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while listing managed quantities")
            .into_db_result()
    }

    pub async fn find_by_code(
        conn: &mut PgConn,
        tenant_id: TenantId,
        code: &str,
    ) -> DbResult<Option<ManagedQuantityRow>> {
        use crate::schema::managed_quantity::dsl as q;

        let query = q::managed_quantity
            .filter(q::tenant_id.eq(tenant_id))
            .filter(q::code.eq(code))
            .select(ManagedQuantityRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding managed quantity")
            .into_db_result()
    }

    pub async fn find(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: ManagedQuantityId,
    ) -> DbResult<Option<ManagedQuantityRow>> {
        use crate::schema::managed_quantity::dsl as q;

        let query = q::managed_quantity
            .filter(q::tenant_id.eq(tenant_id))
            .filter(q::id.eq(id))
            .select(ManagedQuantityRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding managed quantity")
            .into_db_result()
    }

    /// The codes of `ids`, of the tenant's quantities.
    pub async fn codes_of(
        conn: &mut PgConn,
        tenant_id: TenantId,
        ids: &[ManagedQuantityId],
    ) -> DbResult<HashMap<ManagedQuantityId, String>> {
        use crate::schema::managed_quantity::dsl as q;

        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let query = q::managed_quantity
            .filter(q::tenant_id.eq(tenant_id))
            .filter(q::id.eq_any(ids))
            .select((q::id, q::code));
        crate::debug_sql(&query);
        query
            .load::<(ManagedQuantityId, String)>(conn)
            .await
            .attach("Error while reading managed quantity codes")
            .into_db_result()
            .map(|rows| rows.into_iter().collect())
    }

    /// A new quantity; None when the tenant has one of its code.
    pub async fn insert(
        conn: &mut PgConn,
        new: &ManagedQuantityRowNew,
    ) -> DbResult<Option<ManagedQuantityRow>> {
        use crate::schema::managed_quantity::dsl as q;

        let query = diesel::insert_into(q::managed_quantity)
            .values(new)
            .on_conflict((q::tenant_id, q::code))
            .do_nothing()
            .returning(ManagedQuantityRow::as_returning());
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while creating a managed quantity")
            .into_db_result()
    }

    /// Archives the quantity (`Some`) or restores it (`None`); None when the tenant has no such
    /// quantity.
    pub async fn set_archived(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: ManagedQuantityId,
        archived_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> DbResult<Option<ManagedQuantityRow>> {
        use crate::schema::managed_quantity::dsl as q;

        let query = diesel::update(q::managed_quantity)
            .filter(q::tenant_id.eq(tenant_id))
            .filter(q::id.eq(id))
            .set((
                q::archived_at.eq(archived_at),
                q::updated_at.eq(common_clock::now()),
            ))
            .returning(ManagedQuantityRow::as_returning());
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while archiving managed quantity")
            .into_db_result()
    }

    pub async fn update(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: ManagedQuantityId,
        patch: &ManagedQuantityRowPatch,
    ) -> DbResult<ManagedQuantityRow> {
        use crate::schema::managed_quantity::dsl as q;

        let query = diesel::update(q::managed_quantity)
            .filter(q::tenant_id.eq(tenant_id))
            .filter(q::id.eq(id))
            .set((patch, q::updated_at.eq(common_clock::now())))
            .returning(ManagedQuantityRow::as_returning());
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .attach("Error while updating managed quantity")
            .into_db_result()
    }
}

/// The managed quantity each of `features` counts its limit per unit of.
pub async fn feature_limit_quantities(
    conn: &mut PgConn,
    tenant_id: TenantId,
    features: &[common_domain::ids::FeatureId],
) -> DbResult<HashMap<common_domain::ids::FeatureId, ManagedQuantityId>> {
    use crate::schema::feature_quantity_limit::dsl as l;
    use crate::schema::managed_quantity::dsl as q;

    if features.is_empty() {
        return Ok(HashMap::new());
    }
    let query = l::feature_quantity_limit
        .inner_join(q::managed_quantity)
        .filter(l::feature_id.eq_any(features))
        .filter(q::tenant_id.eq(tenant_id))
        .select((l::feature_id, l::managed_quantity_id));
    crate::debug_sql(&query);
    query
        .load::<(common_domain::ids::FeatureId, ManagedQuantityId)>(conn)
        .await
        .attach("Error while reading feature limits per quantity")
        .into_db_result()
        .map(|rows| rows.into_iter().collect())
}

/// Counts `feature`'s limit per unit of `quantity`, or per subscription again when none.
pub async fn set_feature_limit_quantity(
    conn: &mut PgConn,
    feature: common_domain::ids::FeatureId,
    quantity: Option<ManagedQuantityId>,
) -> DbResult<()> {
    use crate::schema::feature_quantity_limit::dsl as l;

    let result = match quantity {
        Some(quantity) => {
            let query = diesel::insert_into(l::feature_quantity_limit)
                .values((
                    l::feature_id.eq(feature),
                    l::managed_quantity_id.eq(quantity),
                ))
                .on_conflict(l::feature_id)
                .do_update()
                .set(l::managed_quantity_id.eq(quantity));
            crate::debug_sql(&query);
            query.execute(conn).await
        }
        None => {
            let query = diesel::delete(l::feature_quantity_limit).filter(l::feature_id.eq(feature));
            crate::debug_sql(&query);
            query.execute(conn).await
        }
    };
    result
        .map(|_| ())
        .attach("Error while setting a feature's limit per quantity")
        .into_db_result()
}

/// The tenant's metered features whose limit counts per unit of a quantity, by quantity.
pub async fn list_feature_limits(
    conn: &mut PgConn,
    tenant_id: TenantId,
) -> DbResult<Vec<(ManagedQuantityId, common_domain::ids::FeatureId)>> {
    use crate::schema::feature_quantity_limit::dsl as l;
    use crate::schema::managed_quantity::dsl as q;

    let query = l::feature_quantity_limit
        .inner_join(q::managed_quantity)
        .filter(q::tenant_id.eq(tenant_id))
        .select((l::managed_quantity_id, l::feature_id));
    crate::debug_sql(&query);
    query
        .load(conn)
        .await
        .attach("Error while listing feature limits per quantity")
        .into_db_result()
}
