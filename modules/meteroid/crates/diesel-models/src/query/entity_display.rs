//! Batched `(id, display_name)` lookups for the activity API.

use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::TenantId;
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

pub async fn customer_names(
    conn: &mut PgConn,
    tenant: TenantId,
    ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String)>> {
    use crate::schema::customer::dsl as c;
    if ids.is_empty() {
        return Ok(vec![]);
    }
    c::customer
        .filter(c::tenant_id.eq(tenant))
        .filter(c::id.eq_any(ids))
        .select((c::id, c::name))
        .load(conn)
        .await
        .attach("customer_names")
        .into_db_result()
}

pub async fn invoice_numbers(
    conn: &mut PgConn,
    tenant: TenantId,
    ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String)>> {
    use crate::schema::invoice::dsl as i;
    if ids.is_empty() {
        return Ok(vec![]);
    }
    i::invoice
        .filter(i::tenant_id.eq(tenant))
        .filter(i::id.eq_any(ids))
        .select((i::id, i::invoice_number))
        .load(conn)
        .await
        .attach("invoice_numbers")
        .into_db_result()
}

pub async fn quote_numbers(
    conn: &mut PgConn,
    tenant: TenantId,
    ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String)>> {
    use crate::schema::quote::dsl as q;
    if ids.is_empty() {
        return Ok(vec![]);
    }
    q::quote
        .filter(q::tenant_id.eq(tenant))
        .filter(q::id.eq_any(ids))
        .select((q::id, q::quote_number))
        .load(conn)
        .await
        .attach("quote_numbers")
        .into_db_result()
}

pub async fn plan_names(
    conn: &mut PgConn,
    tenant: TenantId,
    ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String)>> {
    use crate::schema::plan::dsl as p;
    if ids.is_empty() {
        return Ok(vec![]);
    }
    p::plan
        .filter(p::tenant_id.eq(tenant))
        .filter(p::id.eq_any(ids))
        .select((p::id, p::name))
        .load(conn)
        .await
        .attach("plan_names")
        .into_db_result()
}

pub async fn product_names(
    conn: &mut PgConn,
    tenant: TenantId,
    ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String)>> {
    use crate::schema::product::dsl as p;
    if ids.is_empty() {
        return Ok(vec![]);
    }
    p::product
        .filter(p::tenant_id.eq(tenant))
        .filter(p::id.eq_any(ids))
        .select((p::id, p::name))
        .load(conn)
        .await
        .attach("product_names")
        .into_db_result()
}

pub async fn add_on_names(
    conn: &mut PgConn,
    tenant: TenantId,
    ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String)>> {
    use crate::schema::add_on::dsl as a;
    if ids.is_empty() {
        return Ok(vec![]);
    }
    a::add_on
        .filter(a::tenant_id.eq(tenant))
        .filter(a::id.eq_any(ids))
        .select((a::id, a::name))
        .load(conn)
        .await
        .attach("add_on_names")
        .into_db_result()
}

pub async fn coupon_codes(
    conn: &mut PgConn,
    tenant: TenantId,
    ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String)>> {
    use crate::schema::coupon::dsl as c;
    if ids.is_empty() {
        return Ok(vec![]);
    }
    c::coupon
        .filter(c::tenant_id.eq(tenant))
        .filter(c::id.eq_any(ids))
        .select((c::id, c::code))
        .load(conn)
        .await
        .attach("coupon_codes")
        .into_db_result()
}

pub async fn billable_metric_names(
    conn: &mut PgConn,
    tenant: TenantId,
    ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String)>> {
    use crate::schema::billable_metric::dsl as b;
    if ids.is_empty() {
        return Ok(vec![]);
    }
    b::billable_metric
        .filter(b::tenant_id.eq(tenant))
        .filter(b::id.eq_any(ids))
        .select((b::id, b::name))
        .load(conn)
        .await
        .attach("billable_metric_names")
        .into_db_result()
}

pub async fn credit_note_numbers(
    conn: &mut PgConn,
    tenant: TenantId,
    ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String)>> {
    use crate::schema::credit_note::dsl as c;
    if ids.is_empty() {
        return Ok(vec![]);
    }
    c::credit_note
        .filter(c::tenant_id.eq(tenant))
        .filter(c::id.eq_any(ids))
        .select((c::id, c::credit_note_number))
        .load(conn)
        .await
        .attach("credit_note_numbers")
        .into_db_result()
}

/// Subscription display = the plan name (joined through plan_version).
pub async fn subscription_plan_names(
    conn: &mut PgConn,
    tenant: TenantId,
    ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String)>> {
    use crate::schema::plan::dsl as p;
    use crate::schema::plan_version::dsl as pv;
    use crate::schema::subscription::dsl as s;
    use diesel::JoinOnDsl;

    if ids.is_empty() {
        return Ok(vec![]);
    }
    s::subscription
        .inner_join(pv::plan_version.on(pv::id.eq(s::plan_version_id)))
        .inner_join(p::plan.on(p::id.eq(pv::plan_id)))
        .filter(s::tenant_id.eq(tenant))
        .filter(s::id.eq_any(ids))
        .select((s::id, p::name))
        .load(conn)
        .await
        .attach("subscription_plan_names")
        .into_db_result()
}
