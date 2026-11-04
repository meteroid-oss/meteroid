//! Settings of a built-in signal that core screens read, kept when the edition loses its
//! alerting so the tenant's own thresholds survive a downgrade.

use common_domain::ids::TenantId;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use error_stack::ResultExt;

use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};

/// `None` when the built-in `code` is not seeded for the tenant, or has no settings.
pub async fn builtin_alert_settings(
    conn: &mut PgConn,
    tenant_id: TenantId,
    code: &str,
) -> DbResult<Option<serde_json::Value>> {
    use crate::schema::alert_definition::dsl as a;
    use diesel_async::RunQueryDsl;

    a::alert_definition
        .filter(a::tenant_id.eq(tenant_id))
        .filter(a::builtin_code.eq(code))
        .select(a::settings)
        .first::<Option<serde_json::Value>>(conn)
        .await
        .optional()
        .attach("Error while reading a built-in signal's settings")
        .into_db_result()
        .map(Option::flatten)
}
