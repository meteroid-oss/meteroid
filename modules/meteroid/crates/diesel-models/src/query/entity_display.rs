//! Batched `(id, display_name)` lookups for the activity API.

use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::{BaseId, TenantId};
use diesel::{
    ExpressionMethods, NullableExpressionMethods, QueryDsl, QueryableByName, sql_query, sql_types,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use std::collections::HashMap;
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

pub async fn rate_card_names(
    conn: &mut PgConn,
    tenant: TenantId,
    ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String)>> {
    use crate::schema::rate_card::dsl as r;
    if ids.is_empty() {
        return Ok(vec![]);
    }
    r::rate_card
        .filter(r::tenant_id.eq(tenant))
        .filter(r::id.eq_any(ids))
        .select((r::id, r::name))
        .load(conn)
        .await
        .attach("rate_card_names")
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
        .filter(c::code.is_not_null())
        .select((c::id, c::code.assume_not_null()))
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

/// The owners an entitlement explain names, by kind.
#[derive(Debug, Default)]
pub struct EntitlementSourceIds {
    pub features: Vec<Uuid>,
    pub plans: Vec<Uuid>,
    pub plan_versions: Vec<Uuid>,
    pub add_ons: Vec<Uuid>,
    pub subscriptions: Vec<Uuid>,
    pub resources: Vec<Uuid>,
}

impl EntitlementSourceIds {
    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
            && self.plans.is_empty()
            && self.plan_versions.is_empty()
            && self.add_ons.is_empty()
            && self.subscriptions.is_empty()
            && self.resources.is_empty()
    }
}

/// A plan version reads "<plan> v<n>", a subscription its plan's name, a resource its name or key.
#[derive(Debug, Default)]
pub struct EntitlementSourceNames {
    pub features: HashMap<Uuid, String>,
    pub plans: HashMap<Uuid, String>,
    pub plan_versions: HashMap<Uuid, String>,
    pub add_ons: HashMap<Uuid, String>,
    pub subscriptions: HashMap<Uuid, String>,
    pub resources: HashMap<Uuid, String>,
}

#[derive(QueryableByName)]
struct SourceNameRow {
    #[diesel(sql_type = sql_types::Text)]
    kind: String,
    #[diesel(sql_type = sql_types::Uuid)]
    id: Uuid,
    #[diesel(sql_type = sql_types::Text)]
    name: String,
}

pub async fn entitlement_source_names(
    conn: &mut PgConn,
    tenant: TenantId,
    ids: &EntitlementSourceIds,
) -> DbResult<EntitlementSourceNames> {
    let mut names = EntitlementSourceNames::default();
    if ids.is_empty() {
        return Ok(names);
    }
    let rows = sql_query(
        "SELECT 'feature' AS kind, f.id, f.name FROM feature f \
             WHERE f.tenant_id = $1 AND f.id = ANY($2) \
         UNION ALL SELECT 'plan', p.id, p.name FROM plan p \
             WHERE p.tenant_id = $1 AND p.id = ANY($3) \
         UNION ALL SELECT 'plan_version', pv.id, p.name || ' v' || pv.version \
             FROM plan_version pv JOIN plan p ON p.id = pv.plan_id \
             WHERE pv.tenant_id = $1 AND pv.id = ANY($4) \
         UNION ALL SELECT 'add_on', a.id, a.name FROM add_on a \
             WHERE a.tenant_id = $1 AND a.id = ANY($5) \
         UNION ALL SELECT 'subscription', s.id, p.name \
             FROM subscription s JOIN plan_version pv ON pv.id = s.plan_version_id \
             JOIN plan p ON p.id = pv.plan_id \
             WHERE s.tenant_id = $1 AND s.id = ANY($6) \
         UNION ALL SELECT 'resource', r.id, COALESCE(r.name, r.key_value) FROM resource r \
             WHERE r.tenant_id = $1 AND r.id = ANY($7)",
    )
    .bind::<sql_types::Uuid, _>(tenant.as_uuid())
    .bind::<sql_types::Array<sql_types::Uuid>, _>(&ids.features)
    .bind::<sql_types::Array<sql_types::Uuid>, _>(&ids.plans)
    .bind::<sql_types::Array<sql_types::Uuid>, _>(&ids.plan_versions)
    .bind::<sql_types::Array<sql_types::Uuid>, _>(&ids.add_ons)
    .bind::<sql_types::Array<sql_types::Uuid>, _>(&ids.subscriptions)
    .bind::<sql_types::Array<sql_types::Uuid>, _>(&ids.resources)
    .get_results::<SourceNameRow>(conn)
    .await
    .attach("entitlement_source_names")
    .into_db_result()?;
    for row in rows {
        let by_kind = match row.kind.as_str() {
            "feature" => &mut names.features,
            "plan" => &mut names.plans,
            "plan_version" => &mut names.plan_versions,
            "add_on" => &mut names.add_ons,
            "subscription" => &mut names.subscriptions,
            "resource" => &mut names.resources,
            _ => continue,
        };
        by_kind.insert(row.id, row.name);
    }
    Ok(names)
}
