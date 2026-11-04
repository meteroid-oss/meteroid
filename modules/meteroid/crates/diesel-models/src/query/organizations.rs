use crate::errors::IntoDbResult;
use crate::organizations::{OrganizationRow, OrganizationRowNew};
use common_domain::ids::{OrganizationId, TenantId};

use crate::{DbResult, PgConn};

use diesel::{ExpressionMethods, JoinOnDsl, QueryDsl, SelectableHelper};
use error_stack::ResultExt;
use tap::TapFallible;

impl OrganizationRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<OrganizationRow> {
        use crate::schema::organization::dsl::organization;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(organization).values(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting organization")
            .into_db_result()
    }
}

impl OrganizationRow {
    pub async fn exists(conn: &mut PgConn) -> DbResult<bool> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::dsl::select(diesel::dsl::exists(o_dsl::organization.limit(1)));

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while counting all organizations")
            .into_db_result()
    }

    pub async fn count(conn: &mut PgConn) -> DbResult<i64> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        o_dsl::organization
            .count()
            .get_result(conn)
            .await
            .attach("Error while counting organizations")
            .into_db_result()
    }

    pub async fn get_by_id(conn: &mut PgConn, id: OrganizationId) -> DbResult<OrganizationRow> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = o_dsl::organization.filter(o_dsl::id.eq(id));

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding organization by id")
            .into_db_result()
    }

    pub async fn get_by_tenant_id(conn: &mut PgConn, id: &TenantId) -> DbResult<OrganizationRow> {
        use crate::schema::organization::dsl as o_dsl;
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        let query = o_dsl::organization
            .inner_join(t_dsl::tenant.on(o_dsl::id.eq(t_dsl::organization_id)))
            .filter(t_dsl::id.eq(id))
            .select(OrganizationRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding organization by id")
            .into_db_result()
    }

    pub async fn find_by_slug(conn: &mut PgConn, slug: String) -> DbResult<OrganizationRow> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        // Archived (pending-deletion) organizations are not resolvable — this
        // blocks access/login to them.
        let query = o_dsl::organization
            .filter(o_dsl::slug.eq(slug))
            .filter(o_dsl::archived_at.is_null());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding organization by slug")
            .into_db_result()
    }

    pub async fn update_trade_name(
        conn: &mut PgConn,
        param_id: OrganizationId,
        new_trade_name: &String,
    ) -> DbResult<usize> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(o_dsl::organization)
            .filter(o_dsl::id.eq(param_id))
            .set(o_dsl::trade_name.eq(new_trade_name));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error while updating organization: {e:?}"))
            .attach("Error while updating organization")
            .into_db_result()
    }

    pub async fn update_mfa_required(
        conn: &mut PgConn,
        param_id: OrganizationId,
        required: bool,
    ) -> DbResult<usize> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(o_dsl::organization)
            .filter(o_dsl::id.eq(param_id))
            .set(o_dsl::mfa_required.eq(required));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error while updating organization mfa_required: {e:?}"))
            .attach("Error while updating organization mfa_required")
            .into_db_result()
    }

    pub async fn list_by_user_id(
        conn: &mut PgConn,
        user_id: uuid::Uuid,
    ) -> DbResult<Vec<OrganizationRow>> {
        use crate::schema::organization::dsl as o_dsl;
        use crate::schema::organization_member::dsl as om_dsl;
        use diesel_async::RunQueryDsl;

        let query = o_dsl::organization
            .inner_join(om_dsl::organization_member.on(o_dsl::id.eq(om_dsl::organization_id)))
            .filter(om_dsl::user_id.eq(user_id))
            // Hide organizations that are archived / pending deletion.
            .filter(o_dsl::archived_at.is_null())
            .select(OrganizationRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing organizations by user id")
            .into_db_result()
    }

    /// Hard-delete an organization. Every tenant- and organization-scoped row
    /// cascades away via the ON DELETE CASCADE foreign keys.
    pub async fn delete(conn: &mut PgConn, id: OrganizationId) -> DbResult<usize> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(o_dsl::organization).filter(o_dsl::id.eq(id));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error while deleting organization: {e:?}"))
            .attach("Error while deleting organization")
            .into_db_result()
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        ids: &[OrganizationId],
    ) -> DbResult<Vec<OrganizationRow>> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = o_dsl::organization
            .filter(o_dsl::id.eq_any(ids))
            .select(OrganizationRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing organizations by ids")
            .into_db_result()
    }

    /// Mark an organization as archived / pending deletion.
    pub async fn set_archived_at(
        conn: &mut PgConn,
        id: OrganizationId,
        at: chrono::NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        diesel::update(o_dsl::organization.filter(o_dsl::id.eq(id)))
            .set(o_dsl::archived_at.eq(Some(at)))
            .execute(conn)
            .await
            .attach("Error while archiving organization")
            .into_db_result()
    }

    /// Hard-delete every archived organization that no longer has any tenant —
    /// the final step once its tenants have all been purged.
    pub async fn delete_empty_archived(conn: &mut PgConn) -> DbResult<usize> {
        use diesel_async::RunQueryDsl;

        diesel::sql_query(
            "DELETE FROM organization o \
             WHERE o.archived_at IS NOT NULL \
             AND NOT EXISTS (SELECT 1 FROM tenant t WHERE t.organization_id = o.id)",
        )
        .execute(conn)
        .await
        .attach("Error while deleting empty archived organizations")
        .into_db_result()
    }
}

/// The organization that has proven ownership of `domain` (at most one: partial unique index).
pub async fn verified_domain_owner(
    conn: &mut PgConn,
    domain: &str,
) -> DbResult<Option<OrganizationId>> {
    use crate::schema::organization_domain::dsl as d;
    use diesel::OptionalExtension;
    use diesel_async::RunQueryDsl;

    let query = d::organization_domain
        .filter(d::domain.eq(domain))
        .filter(d::verified_at.is_not_null())
        .select(d::organization_id);
    crate::debug_sql(&query);

    query
        .first(conn)
        .await
        .optional()
        .attach("Error while resolving verified domain owner")
        .into_db_result()
}
