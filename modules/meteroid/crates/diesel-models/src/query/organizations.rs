use crate::errors::IntoDbResult;
use crate::organizations::{OrganizationRow, OrganizationRowNew};
use common_domain::ids::{OrganizationId, TenantId};

use crate::{DbResult, PgConn};

use diesel::{ExpressionMethods, JoinOnDsl, QueryDsl, SelectableHelper, debug_query};
use error_stack::ResultExt;
use tap::TapFallible;

impl OrganizationRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<OrganizationRow> {
        use crate::schema::organization::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(organization).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting organization")
            .into_db_result()
    }
}

impl OrganizationRow {
    pub async fn exists(conn: &mut PgConn) -> DbResult<bool> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::dsl::select(diesel::dsl::exists(o_dsl::organization.limit(1)));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while counting all organizations")
            .into_db_result()
    }

    pub async fn find_by_invite_link(
        conn: &mut PgConn,
        invite_link_hash: String,
    ) -> DbResult<OrganizationRow> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = o_dsl::organization.filter(o_dsl::invite_link_hash.eq(invite_link_hash));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding organization by invite_link_hash")
            .into_db_result()
    }

    pub async fn get_by_id(conn: &mut PgConn, id: OrganizationId) -> DbResult<OrganizationRow> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = o_dsl::organization.filter(o_dsl::id.eq(id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding organization by id")
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding organization by id")
            .into_db_result()
    }

    pub async fn find_by_slug(conn: &mut PgConn, slug: String) -> DbResult<OrganizationRow> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = o_dsl::organization.filter(o_dsl::slug.eq(slug));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding organization by slug")
            .into_db_result()
    }

    pub async fn update_invite_link(
        conn: &mut PgConn,
        param_id: OrganizationId,
        new_invite_hash_link: &String,
    ) -> DbResult<usize> {
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(o_dsl::organization)
            .filter(o_dsl::id.eq(param_id))
            .set(o_dsl::invite_link_hash.eq(new_invite_hash_link));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error while updating organization: {:?}", e))
            .attach_printable("Error while updating organization")
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error while updating organization: {:?}", e))
            .attach_printable("Error while updating organization")
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
            .select(OrganizationRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing organizations by user id")
            .into_db_result()
    }
}
