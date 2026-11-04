use crate::errors::IntoDbResult;
use crate::promo_code_attempts::PromoCodeAttemptRowNew;
use crate::{DbResult, PgConn};
use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, TenantId};
use diesel::{ExpressionMethods, QueryDsl, sql_types};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

/// Rows of other customers pruned by one try: bounded so a try never waits on a large delete.
const PRUNE_BATCH: i64 = 100;

impl PromoCodeAttemptRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::promo_code_attempt::dsl as a;
        diesel::insert_into(a::promo_code_attempt)
            .values(self)
            .execute(conn)
            .await
            .attach("Error while recording a promo code attempt")
            .into_db_result()
            .map(|_| ())
    }
}

/// Serialises one customer's promo-code tries until the transaction ends, so counting and
/// recording a try cannot race. Its own key, never a customer row: a try waits on no billing.
pub async fn lock_customer_attempts(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> DbResult<()> {
    diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind::<sql_types::Text, _>(format!("promo_code_attempt:{tenant_id}:{customer_id}"))
        .execute(conn)
        .await
        .attach("Error while locking a customer's promo code tries")
        .into_db_result()
        .map(|_| ())
}

/// The customer's tries since `since`, oldest first. Tries older than that are deleted, the
/// customer's own all, and a bounded batch of anyone's.
pub async fn attempts_since(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    since: DateTime<Utc>,
) -> DbResult<Vec<DateTime<Utc>>> {
    use crate::schema::promo_code_attempt::dsl as a;
    diesel::delete(
        a::promo_code_attempt
            .filter(a::tenant_id.eq(tenant_id))
            .filter(a::customer_id.eq(customer_id))
            .filter(a::attempted_at.lt(since)),
    )
    .execute(conn)
    .await
    .attach("Error while pruning promo code attempts")
    .into_db_result()?;
    let stale = a::promo_code_attempt
        .filter(a::attempted_at.lt(since))
        .select(a::id)
        .limit(PRUNE_BATCH)
        .load::<Uuid>(conn)
        .await
        .attach("Error while listing stale promo code attempts")
        .into_db_result()?;
    if !stale.is_empty() {
        diesel::delete(a::promo_code_attempt.filter(a::id.eq_any(stale)))
            .execute(conn)
            .await
            .attach("Error while pruning stale promo code attempts")
            .into_db_result()?;
    }
    a::promo_code_attempt
        .filter(a::tenant_id.eq(tenant_id))
        .filter(a::customer_id.eq(customer_id))
        .order(a::attempted_at.asc())
        .select(a::attempted_at)
        .load(conn)
        .await
        .attach("Error while counting promo code attempts")
        .into_db_result()
}

/// Forgets a try whose code proved usable.
pub async fn delete_attempt(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    id: Uuid,
) -> DbResult<()> {
    use crate::schema::promo_code_attempt::dsl as a;
    diesel::delete(
        a::promo_code_attempt
            .filter(a::id.eq(id))
            .filter(a::tenant_id.eq(tenant_id))
            .filter(a::customer_id.eq(customer_id)),
    )
    .execute(conn)
    .await
    .attach("Error while deleting a promo code attempt")
    .into_db_result()
    .map(|_| ())
}
