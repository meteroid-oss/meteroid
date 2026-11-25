//! Catalog codes in the database (ADR-0013 §4): the backfill numbers the names a tenant repeats,
//! oldest first, and the trigger filling a missing code never waits for an insert of another
//! transaction, never takes a code an explicit write is setting, and leaves no lock order for two
//! transactions inserting into two tables to invert. A metric's own code is backfilled and filled
//! the same way from its event code, which several metrics may share.
use meteroid_testkit::release::{client, migrate, postgres};
use std::sync::Arc;
use std::time::Duration;
use tokio_postgres::Client;
use tokio_postgres::error::SqlState;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const CATALOG_CODES: &str = "20261125142357";
const METRIC_CATALOG_CODE: &str = "20261125221749";

async fn tenant(db: &Client, organization: Uuid) -> Uuid {
    let tenant = Uuid::now_v7();
    db.batch_execute(&format!(
        "INSERT INTO tenant (id, name, slug, organization_id, reporting_currency, status,
             environment)
         VALUES ('{tenant}', 't-{tenant}', 't-{tenant}', '{organization}', 'EUR', 'ACTIVE',
                 'PRODUCTION')"
    ))
    .await
    .expect("a tenant");
    tenant
}

/// A family created `minutes_ago`, before codes.
async fn old_family(db: &Client, tenant: Uuid, name: &str, minutes_ago: i32) -> Uuid {
    let id = Uuid::now_v7();
    db.batch_execute(&format!(
        "INSERT INTO product_family (id, name, tenant_id, created_at)
         VALUES ('{id}', '{name}', '{tenant}', now() - interval '{minutes_ago} minutes')"
    ))
    .await
    .expect("a family");
    id
}

async fn code_of(db: &Client, id: Uuid) -> String {
    db.query_one(
        &format!("SELECT code FROM product_family WHERE id = '{id}'"),
        &[],
    )
    .await
    .expect("the family")
    .get("code")
}

/// A family inserted without a code: the one the trigger gave it.
async fn family(db: &Client, tenant: Uuid, name: &str) -> Result<String, tokio_postgres::Error> {
    let row = db
        .query_one(
            &format!(
                "INSERT INTO product_family (id, name, tenant_id)
                 VALUES ('{}', '{name}', '{tenant}') RETURNING code",
                Uuid::now_v7()
            ),
            &[],
        )
        .await?;
    Ok(row.get("code"))
}

/// A product of `family` inserted without a code: the one the trigger gave it.
async fn product(
    db: &Client,
    tenant: Uuid,
    family: Uuid,
    name: &str,
) -> Result<String, tokio_postgres::Error> {
    let row = db
        .query_one(
            &format!(
                "INSERT INTO product (id, name, tenant_id, product_family_id, catalog)
                 VALUES ('{}', '{name}', '{tenant}', '{family}', true) RETURNING code",
                Uuid::now_v7()
            ),
            &[],
        )
        .await?;
    Ok(row.get("code"))
}

/// What `write` answers, within moments: it waited on no other transaction.
async fn without_waiting<T>(write: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), write)
        .await
        .expect("the write waited on another transaction")
}

/// Rules: ADR-0013 §4
#[tokio::test]
async fn catalog_codes_backfilled_then_filled_without_waiting() -> TestResult {
    let (_container, url) = postgres("catalog-codes").await;
    migrate(&url, "", Some(CATALOG_CODES)).await?;
    let db = client(&url).await;
    let organization = Uuid::now_v7();
    db.batch_execute(&format!(
        "INSERT INTO organization (id, trade_name, slug, default_country)
         VALUES ('{organization}', 'O', 'o-{organization}', 'FR')"
    ))
    .await?;
    let (a, b) = (
        tenant(&db, organization).await,
        tenant(&db, organization).await,
    );
    let mut families = Vec::new();
    for (name, minutes_ago) in [
        ("Pro", 60),
        ("Pro", 50),
        ("Pro 2", 40),
        ("Pro", 30),
        ("!!!", 20),
        ("Family", 10),
    ] {
        families.push(old_family(&db, a, name, minutes_ago).await);
    }
    let other_tenant = old_family(&db, b, "Pro", 5).await;

    migrate(&url, CATALOG_CODES, None).await?;

    // The oldest of a slug keeps it bare, the others are numbered in creation order; the second
    // `Pro` would be `Pro 2`'s, so it takes the next number free. Codes are per tenant.
    let mut codes = Vec::new();
    for id in &families {
        codes.push(code_of(&db, *id).await);
    }
    assert_eq!(
        codes,
        ["pro", "pro-4", "pro-2", "pro-3", "family", "family-2"]
    );
    assert_eq!(code_of(&db, other_tenant).await, "pro");

    let one = client(&url).await;
    let two = client(&url).await;

    // Two inserts of one name, neither committed: the second passes over the code the first
    // holds instead of waiting for it.
    one.batch_execute("BEGIN").await?;
    two.batch_execute("BEGIN").await?;
    assert_eq!(family(&one, a, "Launch").await?, "launch");
    assert_eq!(
        without_waiting(family(&two, a, "Launch")).await?,
        "launch-2"
    );
    one.batch_execute("COMMIT").await?;
    two.batch_execute("COMMIT").await?;

    // A code an explicit write is setting, uncommitted (v2 inserts, then sets its code): a filled
    // insert passes over it, rather than failing on it once the explicit write commits.
    one.batch_execute("BEGIN").await?;
    two.batch_execute("BEGIN").await?;
    let placeholder = family(&one, a, "Placeholder").await?;
    one.batch_execute(&format!(
        "UPDATE product_family SET code = 'edge' WHERE tenant_id = '{a}' AND code = '{placeholder}'"
    ))
    .await?;
    assert_eq!(without_waiting(family(&two, a, "Edge")).await?, "edge-2");
    one.batch_execute("COMMIT").await?;
    two.batch_execute("COMMIT").await?;

    // Two transactions inserting a family and a product in opposite orders, under one name: each
    // passes over the other's code, so neither waits and they cannot deadlock.
    let parent = families[0];
    one.batch_execute("BEGIN").await?;
    two.batch_execute("BEGIN").await?;
    assert_eq!(family(&one, a, "Crossed").await?, "crossed");
    assert_eq!(product(&two, a, parent, "Crossed").await?, "crossed");
    assert_eq!(
        without_waiting(product(&one, a, parent, "Crossed")).await?,
        "crossed-2"
    );
    assert_eq!(
        without_waiting(family(&two, a, "Crossed")).await?,
        "crossed-2"
    );
    one.batch_execute("COMMIT").await?;
    two.batch_execute("COMMIT").await?;

    // An explicit code a filled insert holds, uncommitted: the explicit write waits for it, as its
    // unique index would, then is refused as a duplicate.
    one.batch_execute("BEGIN").await?;
    two.batch_execute("BEGIN").await?;
    assert_eq!(family(&one, a, "Corner").await?, "corner");
    let elsewhere = family(&two, a, "Elsewhere").await?;
    let two = Arc::new(two);
    let setter = {
        let two = two.clone();
        tokio::spawn(async move {
            two.batch_execute(&format!(
                "UPDATE product_family SET code = 'corner' \
                 WHERE tenant_id = '{a}' AND code = '{elsewhere}'"
            ))
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !setter.is_finished(),
        "the explicit code waits for the transaction holding it"
    );
    one.batch_execute("COMMIT").await?;
    let refused = setter.await?.expect_err("a code the tenant has");
    assert_eq!(refused.code(), Some(&SqlState::UNIQUE_VIOLATION));
    two.batch_execute("ROLLBACK").await?;
    Ok(())
}

/// A metric of event code `code` created `minutes_ago` (archived when `archived`), before metrics
/// had a code of their own.
async fn old_metric(
    db: &Client,
    tenant: Uuid,
    family: Uuid,
    code: &str,
    minutes_ago: i32,
    archived: bool,
) -> Uuid {
    let id = Uuid::now_v7();
    let archived_at = if archived { "now()" } else { "NULL" };
    db.batch_execute(&format!(
        "INSERT INTO billable_metric (id, name, code, tenant_id, product_family_id, created_at,
             archived_at)
         VALUES ('{id}', 'M', '{code}', '{tenant}', '{family}',
                 now() - interval '{minutes_ago} minutes', {archived_at})"
    ))
    .await
    .expect("a metric");
    id
}

async fn metric_code_of(db: &Client, id: Uuid) -> String {
    db.query_one(
        &format!("SELECT catalog_code FROM billable_metric WHERE id = '{id}'"),
        &[],
    )
    .await
    .expect("the metric")
    .get("catalog_code")
}

/// A metric of event code `code` inserted with `catalog_code`, or without one: the code it got.
async fn metric(
    db: &Client,
    tenant: Uuid,
    family: Uuid,
    code: &str,
    catalog_code: Option<&str>,
) -> Result<String, tokio_postgres::Error> {
    let catalog_code = catalog_code.map_or("NULL".to_owned(), |code| format!("'{code}'"));
    let row = db
        .query_one(
            &format!(
                "INSERT INTO billable_metric (id, name, code, tenant_id, product_family_id,
                     catalog_code)
                 VALUES ('{}', 'M', '{code}', '{tenant}', '{family}', {catalog_code})
                 RETURNING catalog_code",
                Uuid::now_v7()
            ),
            &[],
        )
        .await?;
    Ok(row.get("catalog_code"))
}

/// Rules: ADR-0013 §4
#[tokio::test]
async fn metric_catalog_codes_backfilled_from_event_codes_then_filled() -> TestResult {
    let (_container, url) = postgres("metric-catalog-codes").await;
    migrate(&url, "", Some(METRIC_CATALOG_CODE)).await?;
    let db = client(&url).await;
    let organization = Uuid::now_v7();
    db.batch_execute(&format!(
        "INSERT INTO organization (id, trade_name, slug, default_country)
         VALUES ('{organization}', 'O', 'o-{organization}', 'FR')"
    ))
    .await?;
    let (a, b) = (
        tenant(&db, organization).await,
        tenant(&db, organization).await,
    );
    let (family_a, family_b) = (
        old_family(&db, a, "Family", 90).await,
        old_family(&db, b, "Family", 90).await,
    );
    let mut metrics = Vec::new();
    for (code, minutes_ago, archived) in [
        ("calls", 80, true),
        ("calls", 60, false),
        ("calls", 50, false),
        ("tokens", 45, false),
        ("tokens", 35, false),
        ("tokens-2", 40, false),
        ("API Calls", 30, false),
        ("api-calls", 20, false),
        ("bm_tokens", 10, false),
    ] {
        metrics.push(old_metric(&db, a, family_a, code, minutes_ago, archived).await);
    }
    let other_tenant = old_metric(&db, b, family_b, "calls", 5, false).await;

    migrate(&url, METRIC_CATALOG_CODE, None).await?;

    // Per base, live metrics first, then archived, oldest first: the oldest live `calls` keeps
    // it, the archived one is numbered after the live ones. The second `tokens` would be
    // `tokens-2`'s, so it takes the next number free. An event code that is no code (or reads as
    // a metric id) gives its slug. Codes are per tenant, and no event code changes.
    let mut codes = Vec::new();
    for id in &metrics {
        codes.push(metric_code_of(&db, *id).await);
    }
    assert_eq!(
        codes,
        [
            "calls-3",
            "calls",
            "calls-2",
            "tokens",
            "tokens-3",
            "tokens-2",
            "api-calls",
            "api-calls-2",
            "bm-tokens"
        ]
    );
    assert_eq!(metric_code_of(&db, other_tenant).await, "calls");
    let event_codes: i64 = db
        .query_one(
            &format!(
                "SELECT count(*) FROM billable_metric WHERE tenant_id = '{a}' AND code = 'calls'"
            ),
            &[],
        )
        .await?
        .get(0);
    assert_eq!(event_codes, 3);

    // A metric inserted without a code gets its event code's, past every code taken (archived
    // ones too); one inserted with a code taken is refused.
    assert_eq!(metric(&db, a, family_a, "calls", None).await?, "calls-4");
    assert_eq!(
        metric(&db, a, family_a, "calls", Some("calls-total")).await?,
        "calls-total"
    );
    let taken = metric(&db, a, family_a, "other", Some("calls-3"))
        .await
        .expect_err("a code the tenant has");
    assert_eq!(taken.code(), Some(&SqlState::UNIQUE_VIOLATION));

    // Two inserts of one event code, neither committed: the second passes over the code the
    // first holds instead of waiting for it.
    let one = client(&url).await;
    let two = client(&url).await;
    one.batch_execute("BEGIN").await?;
    two.batch_execute("BEGIN").await?;
    assert_eq!(metric(&one, a, family_a, "launch", None).await?, "launch");
    assert_eq!(
        without_waiting(metric(&two, a, family_a, "launch", None)).await?,
        "launch-2"
    );
    one.batch_execute("COMMIT").await?;
    two.batch_execute("COMMIT").await?;
    Ok(())
}
