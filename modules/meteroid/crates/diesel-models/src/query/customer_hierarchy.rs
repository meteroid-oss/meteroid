use crate::customers::CustomerBriefRow;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::{CustomerId, TenantId};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper, debug_query, sql_types};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

#[derive(diesel::QueryableByName, Debug, Clone)]
pub struct CustomerAncestorRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub id: CustomerId,
    #[diesel(sql_type = sql_types::Text)]
    pub name: String,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Text>)]
    pub alias: Option<String>,
}

/// The customer's parent, its parent's parent and so on, nearest first. Bounded by `max`.
pub async fn list_ancestors(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    max: i32,
) -> DbResult<Vec<CustomerAncestorRow>> {
    diesel::sql_query(
        r#"
        WITH RECURSIVE ancestors AS (
            SELECT p.id, p.name, p.alias, p.parent_customer_id, 1 AS depth
            FROM customer c
            JOIN customer p ON p.id = c.parent_customer_id AND p.tenant_id = $1
            WHERE c.tenant_id = $1 AND c.id = $2
          UNION ALL
            SELECT p.id, p.name, p.alias, p.parent_customer_id, a.depth + 1
            FROM ancestors a
            JOIN customer p ON p.id = a.parent_customer_id AND p.tenant_id = $1
            WHERE a.depth < $3
        )
        SELECT id, name, alias FROM ancestors ORDER BY depth
        "#,
    )
    .bind::<sql_types::Uuid, _>(tenant_id)
    .bind::<sql_types::Uuid, _>(customer_id)
    .bind::<sql_types::Integer, _>(max)
    .get_results(conn)
    .await
    .attach("Error while listing customer ancestors")
    .into_db_result()
}

pub async fn list_children(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> DbResult<Vec<CustomerBriefRow>> {
    use crate::schema::customer::dsl as c_dsl;

    let query = c_dsl::customer
        .filter(c_dsl::tenant_id.eq(tenant_id))
        .filter(c_dsl::parent_customer_id.eq(customer_id))
        .filter(c_dsl::archived_at.is_null())
        .order(c_dsl::name.asc())
        .select(CustomerBriefRow::as_select());
    log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

    query
        .get_results(conn)
        .await
        .attach("Error while listing customer children")
        .into_db_result()
}
