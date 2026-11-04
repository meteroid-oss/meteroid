use crate::errors::IntoDbResult;
use crate::org_security_audit_log::{OrgSecurityAuditLogRow, OrgSecurityAuditLogRowNew};
use crate::{DbResult, PgConn};
use error_stack::ResultExt;

impl OrgSecurityAuditLogRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<OrgSecurityAuditLogRow> {
        use crate::schema::org_security_audit_log::dsl::org_security_audit_log;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(org_security_audit_log).values(self);
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting org security audit entry")
            .into_db_result()
    }

    pub async fn insert_batch(rows: &[Self], conn: &mut PgConn) -> DbResult<usize> {
        use crate::schema::org_security_audit_log::dsl::org_security_audit_log;
        use diesel_async::RunQueryDsl;

        if rows.is_empty() {
            return Ok(0);
        }

        let query = diesel::insert_into(org_security_audit_log).values(rows);
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while batch-inserting org security audit entries")
            .into_db_result()
    }
}
