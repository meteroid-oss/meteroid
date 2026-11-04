//! The only writers of `credit_customer_inputs`, `credit_tenant_inputs` and `credit_input_change`
//! (check-credit-bumps.sh). A customer bump runs under its `CustomerLock`, so no bumper of that
//! row ever waits; the tenant row has no lock, so its bump must be the transaction's last statement.
//! A write bumps only what a live-credit reader reads: payment routing alone bumps nothing.

use crate::credit_inputs::{CreditFeedCursor, CreditInputChangeRow};
use crate::enums::CreditInputChangeKind;
use crate::errors::IntoDbResult;
use crate::lock::CustomerLock;
use crate::schema::sql_types::CreditInputChangeKindEnum;
use crate::{DbResult, PgConn};
use chrono::NaiveDate;
use common_domain::ids::{BaseId, CustomerId, TenantId};
use diesel::{QueryableByName, sql_query, sql_types};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use itertools::Itertools;
use uuid::Uuid;

const PARTITION_PREFIX: &str = "credit_input_change_p";
const DEFAULT_PARTITION: &str = "credit_input_change_default";
// A waiting DETACH blocks every feed writer behind it: keep the wait short and retry instead.
const DETACH_LOCK_TIMEOUT: &str = "200ms";
const ATTACH_LOCK_TIMEOUT: &str = "1s";
const GUARD_PREFIX: &str = "credit_input_change_default_not_";

pub async fn bump_customer(conn: &mut PgConn, lock: &CustomerLock) -> DbResult<()> {
    bump_customers(conn, &[lock]).await
}

/// Customers of one or several tenants, deduplicated and in key order: one statement per table.
pub async fn bump_customers(conn: &mut PgConn, locks: &[&CustomerLock]) -> DbResult<()> {
    for lock in locks {
        lock.on(conn);
    }
    let keys = sorted_keys(locks);
    if keys.is_empty() {
        return Ok(());
    }
    let (tenants, customers): (Vec<Uuid>, Vec<Uuid>) = keys.iter().copied().unzip();
    sql_query(
        "INSERT INTO credit_customer_inputs (tenant_id, customer_id, inputs_token)
         SELECT k.tenant_id, k.customer_id, credit_input_token()
         FROM UNNEST($1::uuid[], $2::uuid[]) WITH ORDINALITY AS k (tenant_id, customer_id, n)
         ORDER BY k.n
         ON CONFLICT (tenant_id, customer_id) DO UPDATE SET inputs_token = excluded.inputs_token",
    )
    .bind::<sql_types::Array<sql_types::Uuid>, _>(&tenants)
    .bind::<sql_types::Array<sql_types::Uuid>, _>(&customers)
    .execute(conn)
    .await
    .attach("Error while bumping customers' credit input tokens")
    .into_db_result()?;
    record_customers(conn, &tenants, &customers, CreditInputChangeKind::Inputs).await
}

/// Must be the last statement of its transaction: the store's `transaction_bumping_tenant` is the
/// only caller. Nothing protects the tenant row, so a statement after it could wait while holding it.
pub async fn bump_tenant(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<()> {
    bump_tenants(conn, &[tenant_id]).await
}

/// `bump_tenant` for several tenants, deduplicated and in key order.
pub async fn bump_tenants(conn: &mut PgConn, tenant_ids: &[TenantId]) -> DbResult<()> {
    let tenants: Vec<Uuid> = tenant_ids
        .iter()
        .map(|id| id.as_uuid())
        .sorted()
        .dedup()
        .collect();
    if tenants.is_empty() {
        return Ok(());
    }
    sql_query(
        "INSERT INTO credit_tenant_inputs (tenant_id, inputs_token)
         SELECT t.tenant_id, credit_input_token()
         FROM UNNEST($1::uuid[]) WITH ORDINALITY AS t (tenant_id, n)
         ORDER BY t.n
         ON CONFLICT (tenant_id) DO UPDATE SET inputs_token = excluded.inputs_token",
    )
    .bind::<sql_types::Array<sql_types::Uuid>, _>(&tenants)
    .execute(conn)
    .await
    .attach("Error while bumping tenants' credit input tokens")
    .into_db_result()?;
    sql_query(
        "INSERT INTO credit_input_change (shard, tenant_id, customer_id, kind)
         SELECT credit_input_shard(t.tenant_id), t.tenant_id, NULL, $2
         FROM UNNEST($1::uuid[]) WITH ORDINALITY AS t (tenant_id, n)
         ORDER BY t.n
         ON CONFLICT (created_on, store_xid, tenant_id, customer_id, kind) DO NOTHING",
    )
    .bind::<sql_types::Array<sql_types::Uuid>, _>(&tenants)
    .bind::<CreditInputChangeKindEnum, _>(CreditInputChangeKind::Tenant)
    .execute(conn)
    .await
    .attach("Error while recording tenants' credit input changes")
    .into_db_result()
    .map(|_| ())
}

/// Once per transaction however many entries it writes: later calls insert nothing.
pub async fn record_ledger_change(conn: &mut PgConn, lock: &CustomerLock) -> DbResult<()> {
    record_one(conn, lock, CreditInputChangeKind::Ledger).await
}

pub async fn record_topup_runtime(conn: &mut PgConn, lock: &CustomerLock) -> DbResult<()> {
    record_one(conn, lock, CreditInputChangeKind::TopUpRuntime).await
}

async fn record_one(
    conn: &mut PgConn,
    lock: &CustomerLock,
    kind: CreditInputChangeKind,
) -> DbResult<()> {
    lock.on(conn);
    record_customers(
        conn,
        &[lock.tenant_id().as_uuid()],
        &[lock.customer_id().as_uuid()],
        kind,
    )
    .await
}

async fn record_customers(
    conn: &mut PgConn,
    tenants: &[Uuid],
    customers: &[Uuid],
    kind: CreditInputChangeKind,
) -> DbResult<()> {
    sql_query(
        "INSERT INTO credit_input_change (shard, tenant_id, customer_id, kind)
         SELECT credit_input_shard(k.tenant_id), k.tenant_id, k.customer_id, $3
         FROM UNNEST($1::uuid[], $2::uuid[]) WITH ORDINALITY AS k (tenant_id, customer_id, n)
         ORDER BY k.n
         ON CONFLICT (created_on, store_xid, tenant_id, customer_id, kind) DO NOTHING",
    )
    .bind::<sql_types::Array<sql_types::Uuid>, _>(tenants)
    .bind::<sql_types::Array<sql_types::Uuid>, _>(customers)
    .bind::<CreditInputChangeKindEnum, _>(kind)
    .execute(conn)
    .await
    .attach("Error while recording credit input changes")
    .into_db_result()
    .map(|_| ())
}

fn sorted_keys(locks: &[&CustomerLock]) -> Vec<(Uuid, Uuid)> {
    locks
        .iter()
        .map(|lock| (lock.tenant_id().as_uuid(), lock.customer_id().as_uuid()))
        .sorted()
        .dedup()
        .collect()
}

#[derive(QueryableByName)]
struct TokenRow {
    #[diesel(sql_type = sql_types::BigInt)]
    inputs_token: i64,
}

pub async fn customer_token(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> DbResult<Option<i64>> {
    sql_query(
        "SELECT inputs_token FROM credit_customer_inputs WHERE tenant_id = $1 AND customer_id = $2",
    )
    .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<sql_types::Uuid, _>(customer_id.as_uuid())
    .get_results::<TokenRow>(conn)
    .await
    .attach("Error while reading a customer's credit input token")
    .into_db_result()
    .map(|rows| rows.into_iter().next().map(|row| row.inputs_token))
}

pub async fn tenant_token(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<Option<i64>> {
    sql_query("SELECT inputs_token FROM credit_tenant_inputs WHERE tenant_id = $1")
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .get_results::<TokenRow>(conn)
        .await
        .attach("Error while reading a tenant's credit input token")
        .into_db_result()
        .map(|rows| rows.into_iter().next().map(|row| row.inputs_token))
}

const CHANGE_COLUMNS: &str = "id, store_xid::text::bigint AS store_xid, shard, tenant_id, \
                              customer_id, kind, created_on";

/// Rows after `cursor` committed below the oldest open transaction, in `(store_xid, id)` order: a
/// transaction still open is at or above `xmin`, so the cursor never passes a row it could miss.
/// On the primary only. Without a shard, each shard is read on its own index and merged.
pub async fn poll_changes(
    conn: &mut PgConn,
    shard: Option<i16>,
    cursor: CreditFeedCursor,
    limit: i64,
) -> DbResult<Vec<CreditInputChangeRow>> {
    let page = |shard: &str| {
        format!(
            "(SELECT {CHANGE_COLUMNS} FROM credit_input_change
              WHERE shard = {shard} AND (store_xid, id) > ($1::text::xid8, $2)
                AND store_xid < pg_snapshot_xmin(pg_current_snapshot())
              ORDER BY credit_input_change.store_xid, credit_input_change.id LIMIT $3)"
        )
    };
    let query = match shard {
        Some(_) => sql_query(page("$4")).into_boxed(),
        None => sql_query(format!(
            "SELECT * FROM ({}) feed ORDER BY store_xid, id LIMIT $3",
            (0..crate::credit_inputs::CREDIT_INPUT_SHARDS)
                .map(|shard| page(&shard.to_string()))
                .join(" UNION ALL ")
        ))
        .into_boxed(),
    };
    let query = query
        .bind::<sql_types::Text, _>(cursor.store_xid.to_string())
        .bind::<sql_types::BigInt, _>(cursor.id)
        .bind::<sql_types::BigInt, _>(limit);
    let query = match shard {
        Some(shard) => query.bind::<sql_types::SmallInt, _>(shard),
        None => query,
    };
    query
        .get_results(conn)
        .await
        .attach("Error while polling credit input changes")
        .into_db_result()
}

#[derive(QueryableByName)]
struct FloorRow {
    #[diesel(sql_type = sql_types::BigInt)]
    store_xid: i64,
    #[diesel(sql_type = sql_types::BigInt)]
    id: i64,
}

/// The last feed row dropped by retention.
pub async fn retention_floor(conn: &mut PgConn) -> DbResult<Option<CreditFeedCursor>> {
    sql_query(
        "SELECT max_store_xid::text::bigint AS store_xid, max_id AS id
         FROM credit_input_change_retention
         WHERE max_store_xid IS NOT NULL
         ORDER BY max_store_xid DESC, max_id DESC
         LIMIT 1",
    )
    .get_results::<FloorRow>(conn)
    .await
    .attach("Error while reading the credit input feed's retention floor")
    .into_db_result()
    .map(|rows| {
        rows.into_iter().next().map(|row| CreditFeedCursor {
            store_xid: row.store_xid,
            id: row.id,
        })
    })
}

/// Whether rows after `cursor` were dropped before it read them: its reader must sweep.
pub async fn cursor_missed_rows(conn: &mut PgConn, cursor: CreditFeedCursor) -> DbResult<bool> {
    Ok(retention_floor(conn)
        .await?
        .is_some_and(|floor| cursor < floor))
}

#[derive(QueryableByName)]
struct DayRow {
    #[diesel(sql_type = sql_types::Date)]
    day: NaiveDate,
}

async fn days_where(conn: &mut PgConn, attached: bool) -> DbResult<Vec<NaiveDate>> {
    let membership = if attached { "IN" } else { "NOT IN" };
    sql_query(format!(
        "SELECT to_date(substr(c.relname, {}), 'YYYYMMDD') AS day
         FROM pg_class c
         WHERE c.relkind = 'r'
           AND c.relnamespace = 'public'::regnamespace
           AND c.relname ~ '^{PARTITION_PREFIX}[0-9]{{8}}$'
           AND c.oid {membership} (SELECT inhrelid FROM pg_inherits
                                    WHERE inhparent = 'credit_input_change'::regclass)
         ORDER BY day",
        PARTITION_PREFIX.len() + 1
    ))
    .get_results::<DayRow>(conn)
    .await
    .attach("Error while listing credit input feed partitions")
    .into_db_result()
    .map(|rows| rows.into_iter().map(|row| row.day).collect())
}

/// Days with their own partition, ascending.
pub async fn partition_days(conn: &mut PgConn) -> DbResult<Vec<NaiveDate>> {
    days_where(conn, true).await
}

/// Partitions a drop detached but did not finish (crash, error after the DETACH committed).
pub async fn detached_partition_days(conn: &mut PgConn) -> DbResult<Vec<NaiveDate>> {
    days_where(conn, false).await
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = sql_types::BigInt)]
    count: i64,
}

pub async fn default_partition_rows(conn: &mut PgConn) -> DbResult<i64> {
    sql_query(format!("SELECT count(*) AS count FROM {DEFAULT_PARTITION}"))
        .get_result::<CountRow>(conn)
        .await
        .attach("Error while counting the credit input feed's default partition")
        .into_db_result()
        .map(|row| row.count)
}

fn partition_name(day: NaiveDate) -> String {
    format!("{PARTITION_PREFIX}{}", day.format("%Y%m%d"))
}

fn guard_name(day: NaiveDate) -> String {
    format!("{GUARD_PREFIX}{}", day.format("%Y%m%d"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionCreation {
    Created,
    /// The DEFAULT partition holds rows of the day: they stay there and age out with it.
    Skipped,
}

/// The statements in one transaction under a lock timeout.
async fn in_transaction(
    conn: &mut PgConn,
    lock_timeout: &str,
    statements: &[String],
) -> diesel::QueryResult<()> {
    use diesel_async::AsyncConnection;
    let lock_timeout = format!("SET LOCAL lock_timeout = '{lock_timeout}'");
    conn.transaction(async |conn| {
        sql_query(lock_timeout.as_str()).execute(conn).await?;
        for statement in statements {
            sql_query(statement.as_str()).execute(conn).await?;
        }
        Ok(())
    })
    .await
}

/// Attaches a new partition for `day`, each step its own short transaction. A validated CHECK
/// on DEFAULT excluding the day first lets the ATTACH skip scanning DEFAULT under its lock; the
/// validation scan itself takes SHARE UPDATE EXCLUSIVE, which writers do not wait on.
pub async fn create_partition(conn: &mut PgConn, day: NaiveDate) -> DbResult<PartitionCreation> {
    let name = partition_name(day);
    let guard = guard_name(day);
    let next = day + chrono::Days::new(1);
    let outcome = async {
        let in_default: CountRow = sql_query(format!(
            "SELECT count(*) AS count FROM (SELECT 1 FROM {DEFAULT_PARTITION}
                                            WHERE created_on = $1 LIMIT 1) d"
        ))
        .bind::<sql_types::Date, _>(day)
        .get_result(conn)
        .await?;
        if in_default.count > 0 {
            return Ok(PartitionCreation::Skipped);
        }
        in_transaction(
            conn,
            ATTACH_LOCK_TIMEOUT,
            &[format!(
                "ALTER TABLE {DEFAULT_PARTITION} ADD CONSTRAINT {guard}
                     CHECK (created_on < '{day}' OR created_on >= '{next}') NOT VALID"
            )],
        )
        .await?;
        let validated = in_transaction(
            conn,
            ATTACH_LOCK_TIMEOUT,
            &[format!(
                "ALTER TABLE {DEFAULT_PARTITION} VALIDATE CONSTRAINT {guard}"
            )],
        )
        .await;
        if let Err(error) = validated {
            // A row of the day reached DEFAULT since the check above: the day stays there.
            return match error {
                diesel::result::Error::DatabaseError(
                    diesel::result::DatabaseErrorKind::CheckViolation,
                    _,
                ) => Ok(PartitionCreation::Skipped),
                error => Err(error),
            };
        }
        in_transaction(
            conn,
            ATTACH_LOCK_TIMEOUT,
            &[
                format!(
                    "CREATE TABLE IF NOT EXISTS {name}
                     (LIKE credit_input_change INCLUDING DEFAULTS INCLUDING CONSTRAINTS)"
                ),
                format!(
                    "ALTER TABLE credit_input_change ATTACH PARTITION {name}
                     FOR VALUES FROM ('{day}') TO ('{next}')"
                ),
            ],
        )
        .await?;
        Ok(PartitionCreation::Created)
    }
    .await;
    // The guard must never outlive the attempt: a row of that day would then fail its insert.
    let dropped = drop_default_guard(conn, day).await;
    let outcome = outcome
        .attach("Error while creating a credit input feed partition")
        .into_db_result()?;
    dropped?;
    Ok(outcome)
}

async fn drop_default_guard(conn: &mut PgConn, day: NaiveDate) -> DbResult<()> {
    let guard = guard_name(day);
    in_transaction(
        conn,
        ATTACH_LOCK_TIMEOUT,
        &[format!(
            "ALTER TABLE {DEFAULT_PARTITION} DROP CONSTRAINT IF EXISTS {guard}"
        )],
    )
    .await
    .attach("Error while dropping a credit input feed partition guard")
    .into_db_result()
    .map(|_| ())
}

#[derive(QueryableByName)]
struct NameRow {
    #[diesel(sql_type = sql_types::Text)]
    name: String,
}

/// Guards left on DEFAULT by an interrupted `create_partition`, dropped before anything else.
pub async fn drop_leftover_default_guards(conn: &mut PgConn) -> DbResult<usize> {
    let guards: Vec<NameRow> = sql_query(format!(
        "SELECT conname::text AS name FROM pg_constraint
         WHERE conrelid = '{DEFAULT_PARTITION}'::regclass AND conname LIKE '{GUARD_PREFIX}%'"
    ))
    .get_results(conn)
    .await
    .attach("Error while listing credit input feed partition guards")
    .into_db_result()?;
    for guard in &guards {
        let day = NaiveDate::parse_from_str(&guard.name[GUARD_PREFIX.len()..], "%Y%m%d").map_err(
            |error| {
                crate::errors::DatabaseErrorContainer::from(crate::errors::DatabaseError::Others(
                    format!("unexpected guard {}: {error}", guard.name),
                ))
            },
        )?;
        drop_default_guard(conn, day).await?;
    }
    Ok(guards.len())
}

/// Detaches the day's partition in a transaction of its own: a plain DETACH (CONCURRENTLY is
/// refused with a DEFAULT partition) holds the parent, so the wait is bounded by a short lock
/// timeout and the caller retries. Then `finish_dropped_partition`.
pub async fn detach_partition(conn: &mut PgConn, day: NaiveDate) -> DbResult<()> {
    let name = partition_name(day);
    in_transaction(
        conn,
        DETACH_LOCK_TIMEOUT,
        &[format!(
            "ALTER TABLE credit_input_change DETACH PARTITION {name}"
        )],
    )
    .await
    .attach("Error while detaching a credit input feed partition")
    .into_db_result()
    .map(|_| ())
}

/// Records the last (store_xid, id) of a detached partition and drops it; no parent lock.
pub async fn finish_dropped_partition(conn: &mut PgConn, day: NaiveDate) -> DbResult<()> {
    use diesel_async::AsyncConnection;
    let name = partition_name(day);
    conn.transaction(async |conn| {
        sql_query(format!(
            "INSERT INTO credit_input_change_retention (partition_day, max_store_xid, max_id)
             SELECT $1, last.store_xid, last.id
             FROM (SELECT NULL) one
             LEFT JOIN (SELECT store_xid, id FROM {name}
                        ORDER BY store_xid DESC, id DESC LIMIT 1) last ON true
             {RETENTION_UPSERT}"
        ))
        .bind::<sql_types::Date, _>(day)
        .execute(conn)
        .await?;
        sql_query(format!("DROP TABLE {name}"))
            .execute(conn)
            .await?;
        Ok::<_, diesel::result::Error>(())
    })
    .await
    .attach("Error while dropping a credit input feed partition")
    .into_db_result()
}

/// Deletes DEFAULT-partition rows of days before `before`, recording each day's last row.
pub async fn purge_default_before(conn: &mut PgConn, before: NaiveDate) -> DbResult<usize> {
    sql_query(format!(
        "WITH gone AS (
             DELETE FROM {DEFAULT_PARTITION} WHERE created_on < $1
             RETURNING created_on, store_xid, id
         )
         INSERT INTO credit_input_change_retention (partition_day, max_store_xid, max_id)
         SELECT DISTINCT ON (created_on) created_on, store_xid, id
         FROM gone
         ORDER BY created_on, store_xid DESC, id DESC
         {RETENTION_UPSERT}"
    ))
    .bind::<sql_types::Date, _>(before)
    .execute(conn)
    .await
    .attach("Error while purging the credit input feed's default partition")
    .into_db_result()
}

// Keeps the later of the stored and the new last row of a day.
const RETENTION_UPSERT: &str = "ON CONFLICT (partition_day) DO UPDATE SET
    max_store_xid = CASE WHEN credit_input_change_retention.max_store_xid IS NULL
                           OR (excluded.max_store_xid, excluded.max_id)
                              > (credit_input_change_retention.max_store_xid,
                                 credit_input_change_retention.max_id)
                         THEN excluded.max_store_xid
                         ELSE credit_input_change_retention.max_store_xid END,
    max_id = CASE WHEN credit_input_change_retention.max_store_xid IS NULL
                    OR (excluded.max_store_xid, excluded.max_id)
                       > (credit_input_change_retention.max_store_xid,
                          credit_input_change_retention.max_id)
                  THEN excluded.max_id
                  ELSE credit_input_change_retention.max_id END,
    dropped_at = now()";
