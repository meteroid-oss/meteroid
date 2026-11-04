//! Proves a whole-tenant / whole-organization hard delete removes *everything*.
//!
//! The fixture seeds one row in every tenant- and organization-scoped table, all
//! linked together, then we `DELETE FROM tenant` / `DELETE FROM organization` and
//! assert nothing is left behind (and that the delete isn't blocked by a FK). This
//! guards the `ON DELETE` graph that the deletion worker relies on.

use crate::meteroid_it;
use crate::meteroid_it::container::TestDatabase;
use diesel_async::SimpleAsyncConnection;
use meteroid_store::store::{PgConfig, PgConn, diesel_make_pg_pool};

const FIXTURE: &str = include_str!("fixtures/full_graph_fixture.sql");

const TENANT_ID: &str = "10000000-0000-0000-0000-000000000002";
const ORG_ID: &str = "10000000-0000-0000-0000-000000000001";
const USER_ID: &str = "10000000-0000-0000-0000-000000000003";
const HIST_RATE_ID: &str = "10000000-0000-0000-0000-0000000000f0";
const OAUTH_VERIFIER_ID: &str = "10000000-0000-0000-0000-0000000000f1";

/// Global (non-scoped) fixture rows that must be untouched by any delete — proves
/// the cascade doesn't over-reach.
fn assert_globals_survive_sql() -> String {
    format!(
        "DO $$ BEGIN \
         IF (SELECT count(*) FROM historical_rates_from_usd WHERE id = '{HIST_RATE_ID}') <> 1 THEN RAISE EXCEPTION 'historical_rates row should survive'; END IF; \
         IF (SELECT count(*) FROM oauth_verifier WHERE id = '{OAUTH_VERIFIER_ID}') <> 1 THEN RAISE EXCEPTION 'oauth_verifier row should survive'; END IF; \
         END $$;"
    )
}

/// Tables that carry no `tenant_id`/`organization_id` of their own and are only
/// reached transitively — they must be emptied by the cascade too. A migrations-only
/// database never seeds these, so any leftover row means the cascade missed them.
const TRANSITIVE_TABLES: &[&str] = &[
    "quote_coupon",
    "quote_component",
    "quote_add_on",
    "quote_signature",
    "quote_adjustment",
    "quote_adjustment_component",
    "subscription_component",
    "subscription_add_on",
    "subscription_event",
    "subscription_adjustment",
    "subscription_adjustment_component",
    "scheduled_event",
    "slot_transaction",
    "applied_coupon",
    "plan_component_price",
    "plan_version_adjustment",
    "plan_version_adjustment_component",
    "plan_version_annex_template",
    "schedule",
    "coupon_plan",
    "customer_connection",
    "custom_tax",
    "product_accounting",
    "product_custom_tax",
    "webhook_in_event",
];

async fn seed(conn: &mut diesel_async::AsyncPgConnection) {
    conn.batch_execute(FIXTURE)
        .await
        .expect("seed full-graph fixture");
}

/// `DO` block that raises (failing `batch_execute`) if any table still holds rows
/// for `column = id`, naming the offenders.
fn assert_scoped_empty_sql(column: &str, id: &str) -> String {
    format!(
        r#"DO $$
        DECLARE r record; c bigint; bad text := '';
        BEGIN
          FOR r IN SELECT table_name FROM information_schema.columns
                   WHERE column_name = '{column}' AND table_schema = 'public' LOOP
            EXECUTE format('SELECT count(*) FROM %I WHERE {column} = %L', r.table_name, '{id}') INTO c;
            IF c > 0 THEN bad := bad || r.table_name || ' '; END IF;
          END LOOP;
          IF bad <> '' THEN RAISE EXCEPTION '% rows left in: %', '{column}', bad; END IF;
        END $$;"#
    )
}

fn assert_transitive_empty_sql() -> String {
    let checks: Vec<String> = TRANSITIVE_TABLES
        .iter()
        .map(|t| format!("IF (SELECT count(*) FROM {t}) > 0 THEN bad := bad || '{t} '; END IF;"))
        .collect();
    format!(
        "DO $$ DECLARE bad text := ''; BEGIN {} IF bad <> '' THEN RAISE EXCEPTION 'transitive rows left in: %', bad; END IF; END $$;",
        checks.join(" ")
    )
}

async fn fresh_conn() -> (TestDatabase, PgConn) {
    let db = meteroid_it::container::create_test_database().await;
    let pool = diesel_make_pg_pool(PgConfig::new(db.url.clone())).expect("pool");
    // The pooled connection keeps the pool alive internally, so dropping `pool` is fine.
    let conn = pool.get().await.expect("db connection");
    (db, conn)
}

#[tokio::test]
async fn test_full_graph_tenant_deletion_removes_everything() {
    crate::helpers::init::logging();
    let (_db, mut conn) = fresh_conn().await;
    seed(&mut conn).await;

    conn.batch_execute(&format!("DELETE FROM tenant WHERE id = '{TENANT_ID}';"))
        .await
        .expect("tenant delete must not be blocked by any foreign key");

    conn.batch_execute(&assert_scoped_empty_sql("tenant_id", TENANT_ID))
        .await
        .expect("every tenant_id-scoped table must be empty");
    conn.batch_execute(&assert_transitive_empty_sql())
        .await
        .expect("every transitively-scoped table must be empty");

    // The organization (and user) outlive a tenant delete.
    conn.batch_execute(&format!(
        "DO $$ BEGIN IF (SELECT count(*) FROM organization WHERE id = '{ORG_ID}') <> 1 THEN RAISE EXCEPTION 'org should survive'; END IF; END $$;"
    ))
    .await
    .expect("organization must survive a tenant delete");

    conn.batch_execute(&assert_globals_survive_sql())
        .await
        .expect("global tables must survive a tenant delete");
}

#[tokio::test]
async fn test_full_graph_organization_deletion_removes_everything() {
    crate::helpers::init::logging();
    let (_db, mut conn) = fresh_conn().await;
    seed(&mut conn).await;

    conn.batch_execute(&format!("DELETE FROM organization WHERE id = '{ORG_ID}';"))
        .await
        .expect("organization delete must not be blocked by any foreign key");

    conn.batch_execute(&assert_scoped_empty_sql("tenant_id", TENANT_ID))
        .await
        .expect("every tenant_id-scoped table must be empty");
    conn.batch_execute(&assert_scoped_empty_sql("organization_id", ORG_ID))
        .await
        .expect("every organization_id-scoped table must be empty");
    conn.batch_execute(&assert_transitive_empty_sql())
        .await
        .expect("every transitively-scoped table must be empty");

    // The user is not organization-scoped and outlives the delete.
    conn.batch_execute(&format!(
        "DO $$ BEGIN IF (SELECT count(*) FROM \"user\" WHERE id = '{USER_ID}') <> 1 THEN RAISE EXCEPTION 'user should survive'; END IF; END $$;"
    ))
    .await
    .expect("user must survive an organization delete");

    conn.batch_execute(&assert_globals_survive_sql())
        .await
        .expect("global tables must survive an organization delete");
}
