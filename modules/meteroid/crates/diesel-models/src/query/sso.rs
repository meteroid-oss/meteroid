use crate::errors::IntoDbResult;
use crate::sso::OrganizationSsoConnectionRow;
use crate::{DbResult, PgConn};

use common_domain::ids::{OrganizationId, SsoConnectionId};
use diesel::SelectableHelper;
use diesel::prelude::{ExpressionMethods, OptionalExtension, QueryDsl};
use error_stack::ResultExt;

impl OrganizationSsoConnectionRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        id: SsoConnectionId,
    ) -> DbResult<Option<OrganizationSsoConnectionRow>> {
        use crate::schema::organization_sso_connection::dsl as s_dsl;
        use diesel_async::RunQueryDsl;

        let query = s_dsl::organization_sso_connection
            .filter(s_dsl::id.eq(id))
            .select(OrganizationSsoConnectionRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching SSO connection by id")
            .into_db_result()
    }

    /// All enabled connections. The caller matches an email domain in-memory —
    /// The enabled connection for an org, if any (one connection per org).
    pub async fn find_enabled_by_org(
        conn: &mut PgConn,
        org_id: OrganizationId,
    ) -> DbResult<Option<OrganizationSsoConnectionRow>> {
        use crate::schema::organization_sso_connection::dsl as s_dsl;
        use diesel_async::RunQueryDsl;

        let query = s_dsl::organization_sso_connection
            .filter(s_dsl::organization_id.eq(org_id))
            .filter(s_dsl::enabled.eq(true))
            .select(OrganizationSsoConnectionRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching enabled SSO connection")
            .into_db_result()
    }
}
