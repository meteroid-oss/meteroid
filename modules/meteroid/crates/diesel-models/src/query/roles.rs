use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

use common_domain::ids::{OrganizationId, RoleId};

use crate::errors::IntoDbResult;
use crate::roles::RoleRow;
use crate::{DbResult, PgConn};

impl RoleRow {
    pub async fn find_by_id_and_org(
        conn: &mut PgConn,
        id: RoleId,
        org_id: OrganizationId,
    ) -> DbResult<RoleRow> {
        use crate::schema::role::dsl as r;
        let query = r::role
            .filter(r::id.eq(id))
            .filter(r::organization_id.eq(org_id))
            .select(RoleRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .attach("Error finding role by id")
            .into_db_result()
    }

    pub async fn list_by_org(conn: &mut PgConn, org_id: OrganizationId) -> DbResult<Vec<RoleRow>> {
        use crate::schema::role::dsl as r;
        let query = r::role
            .filter(r::organization_id.eq(org_id))
            .order(r::name.asc())
            .select(RoleRow::as_select());
        crate::debug_sql(&query);
        query
            .get_results(conn)
            .await
            .attach("Error listing roles")
            .into_db_result()
    }
}
