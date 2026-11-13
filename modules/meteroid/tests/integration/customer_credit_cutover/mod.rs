//! Upgrades a shipped (main) billing schema holding scalar balances, and checks every minor unit
//! lands exactly once in a customer credit grant. The shipped balance tables stay as they were but
//! frozen against the previous release's writers. Rolling back sets the scalar to the live credit and
//! the upgrade stays repeatable.
use diesel::migration::{Migration, MigrationSource};
use diesel::pg::Pg;
use diesel::{Connection, PgConnection};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use testcontainers::core::WaitFor;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio_postgres::{Client, NoTls};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations/diesel");
const LEDGER_MIGRATION: &str = "20261008110347";
const FIXTURE: &str = include_str!("shipped_fixture.sql");

async fn postgres() -> (ContainerAsync<GenericImage>, String) {
    let container = crate::meteroid_it::container::start_with_retry("start postgres", || {
        GenericImage::new("ghcr.io/meteroid-oss/meteroid-postgres", "18.3-standard")
            .with_wait_for(WaitFor::log(LogWaitStrategy::stdout(
                "database system is ready to accept connections",
            )))
            .with_wait_for(WaitFor::log(LogWaitStrategy::stderr(
                "database system is ready to accept connections",
            )))
            .with_env_var("POSTGRES_DB", "postgres")
            .with_env_var("POSTGRES_USER", "postgres")
            .with_env_var("POSTGRES_PASSWORD", "postgres")
            // Throwaway database: skip fsyncs, which saturate the host disk when tests run in parallel.
            .with_env_var("POSTGRES_INITDB_ARGS", "--no-sync")
            .with_cmd([
                "postgres",
                "-c",
                "fsync=off",
                "-c",
                "synchronous_commit=off",
                "-c",
                "full_page_writes=off",
            ])
            .with_container_name(format!("cc-gfix-credit-cutover-{}", uuid::Uuid::now_v7()))
            .start()
    })
    .await;
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("postgres port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    (container, url)
}

fn sorted_migrations() -> Vec<Box<dyn Migration<Pg>>> {
    let mut migrations =
        MigrationSource::<Pg>::migrations(&MIGRATIONS).expect("embedded migrations");
    migrations.sort_by_key(|m| m.name().version().to_string());
    migrations
}

/// Applies every migration with a version in `[from, until)`; each runs in its own transaction.
async fn migrate(url: &str, from: &str, until: Option<&str>) -> Result<(), String> {
    let (url, from, until) = (url.to_owned(), from.to_owned(), until.map(str::to_owned));
    tokio::task::spawn_blocking(move || {
        let mut conn = PgConnection::establish(&url).map_err(|e| e.to_string())?;
        conn.applied_migrations().map_err(|e| e.to_string())?;
        for migration in sorted_migrations() {
            let version = migration.name().version().to_string();
            if version.as_str() < from.as_str() {
                continue;
            }
            if until
                .as_deref()
                .is_some_and(|until| version.as_str() >= until)
            {
                break;
            }
            conn.run_migration(migration.as_ref())
                .map_err(|e| format!("{}: {e}", migration.name()))?;
        }
        Ok(())
    })
    .await
    .expect("migration task")
}

/// Reverts newest-first down to and including `version`, as a real rollback of that migration.
async fn revert_through(url: &str, version: &str) -> Result<(), String> {
    let (url, version) = (url.to_owned(), version.to_owned());
    tokio::task::spawn_blocking(move || {
        let mut conn = PgConnection::establish(&url).map_err(|e| e.to_string())?;
        loop {
            let reverted = conn
                .revert_last_migration(MIGRATIONS)
                .map_err(|e| e.to_string())?;
            if reverted.to_string() <= version {
                return Ok(());
            }
        }
    })
    .await
    .expect("revert task")
}

async fn client(url: &str) -> Client {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await.expect("connect");
    tokio::spawn(connection);
    client
}

async fn shipped_database(url: &str) -> Client {
    migrate(url, "", Some(LEDGER_MIGRATION))
        .await
        .expect("shipped schema");
    let client = client(url).await;
    client
        .batch_execute(FIXTURE)
        .await
        .expect("shipped fixture");
    client
}

const TENANT: &str = "00000000-0000-0000-0000-00000000a001";
const C1: &str = "00000000-0000-0000-0000-0000000000c1";
const SHIPPED_BALANCES: i64 = 1234 + 500 + 42;

async fn texts(client: &Client, sql: &str) -> Result<Vec<String>, tokio_postgres::Error> {
    Ok(client
        .query(sql, &[])
        .await?
        .into_iter()
        .map(|r| r.get::<_, String>(0))
        .collect())
}

#[tokio::test]
async fn shipped_balances_land_exactly_once() -> TestResult {
    let (_container, url) = postgres().await;
    let client = shipped_database(&url).await;
    migrate(&url, LEDGER_MIGRATION, None).await?;

    // Only the finalized note that credited the scalar is marked; its value lives in the balance.
    let retired = texts(
        &client,
        "SELECT right(id::text, 3) FROM credit_note WHERE credited_retired_balance ORDER BY 1",
    )
    .await?;
    assert_eq!(retired, ["303"]);

    let grants = client
        .query(
            "SELECT right(g.customer_id::text, 2), g.source_kind, right(g.source_id::text, 3),
                    g.amount, g.remaining, a.currency, g.application, g.classification,
                    g.cost_minor, g.cost_currency
             FROM customer_credit_grant g JOIN customer_credit_account a ON a.id = g.account_id
             ORDER BY 1",
            &[],
        )
        .await?;
    let expected = [
        ("c1", 1234_i64, "USD"),
        ("c2", 500, "EUR"),
        ("c3", 42, "JPY"),
    ];
    assert_eq!(grants.len(), expected.len());
    for (grant, (customer, amount, currency)) in grants.iter().zip(expected) {
        assert_eq!(grant.get::<_, String>(0), customer);
        assert_eq!(grant.get::<_, String>(1), "CUSTOMER_BALANCE");
        assert_eq!(grant.get::<_, String>(2), format!("0{customer}"));
        assert_eq!(grant.get::<_, i64>(3), amount);
        assert_eq!(
            grant.get::<_, i64>(4),
            amount,
            "carried-over credit is unspent"
        );
        assert_eq!(
            grant.get::<_, String>(5),
            currency,
            "a balance lives in a wallet of its customer's currency"
        );
        assert_eq!(grant.get::<_, String>(6), "INVOICE");
        assert_eq!(
            grant.get::<_, String>(7),
            "PAID",
            "the scalar held paid value"
        );
        assert_eq!(
            grant.get::<_, Option<i64>>(8),
            Some(amount),
            "customer money costs its face"
        );
        assert_eq!(grant.get::<_, Option<String>>(9).as_deref(), Some(currency));
    }

    // Every grant has exactly one FUND entry of its amount and cost: the ledger replays to its balance.
    let unbalanced = client
        .query_one(
            "SELECT count(*) FROM customer_credit_grant g
             WHERE g.remaining <> (SELECT coalesce(sum(e.amount), 0) FROM customer_credit_entry e WHERE e.grant_id = g.id)
                OR (SELECT count(*) FROM customer_credit_entry e
                    WHERE e.grant_id = g.id AND e.kind = 'FUND' AND e.cost_minor = g.amount) <> 1",
            &[],
        )
        .await?;
    assert_eq!(unbalanced.get::<_, i64>(0), 0);

    // The scalar is left as shipped, frozen, and equals the balance grants.
    let scalar = client
        .query_one("SELECT sum(balance_value_cents)::bigint FROM customer", &[])
        .await?;
    assert_eq!(scalar.get::<_, i64>(0), SHIPPED_BALANCES);
    let ledger_equals_scalar = client
        .query_one(
            "SELECT count(*) FROM customer c
             WHERE c.balance_value_cents <> coalesce((SELECT sum(g.remaining) FROM customer_credit_grant g
                                                      WHERE g.customer_id = c.id), 0)",
            &[],
        )
        .await?;
    assert_eq!(ledger_equals_scalar.get::<_, i64>(0), 0);

    // Only debt-cancellation notes cancelled debt on main.
    let debts = texts(
        &client,
        "SELECT right(id::text, 3) || ':' || debt_cancelled_amount_cents FROM credit_note ORDER BY 1",
    )
    .await?;
    assert_eq!(debts, ["302:0", "303:0", "309:1000", "312:0"]);

    // History stays readable and keeps the currency each movement was taken in.
    let history = client
        .query_one(
            "SELECT (SELECT sum(amount_cents)::bigint FROM customer_balance_tx WHERE currency = 'USD')",
            &[],
        )
        .await?;
    assert_eq!(history.get::<_, i64>(0), 1234);
    let stamped = texts(
        &client,
        "SELECT currency FROM customer_balance_tx WHERE right(customer_id::text, 2) IN ('c2', 'c3')
         ORDER BY customer_id",
    )
    .await?;
    assert_eq!(
        stamped,
        ["EUR", "JPY"],
        "every historical movement is stamped with the customer's current currency, \
         even one that later invoiced in another currency"
    );

    // Spend part of c1's grant, then roll back: the scalar holds exactly the live remainder, and
    // upgrading again carries that remainder once.
    client
        .batch_execute(&format!(
            "UPDATE customer_credit_grant SET remaining = remaining - 1000 WHERE customer_id = '{C1}';
             INSERT INTO customer_credit_entry (id, tenant_id, customer_id, grant_id, kind, amount, source_id, source_kind, created_at)
             SELECT gen_random_uuid(), tenant_id, customer_id, id, 'DRAW', -1000, gen_random_uuid(), 'INVOICE', now()
             FROM customer_credit_grant WHERE customer_id = '{C1}';"
        ))
        .await?;
    revert_through(&url, LEDGER_MIGRATION).await?;
    let restored = client
        .query_one(
            &format!(
                "SELECT (SELECT balance_value_cents FROM customer WHERE id = '{C1}'),
                        (SELECT sum(balance_value_cents)::bigint FROM customer)"
            ),
            &[],
        )
        .await?;
    assert_eq!(restored.get::<_, i64>(0), 1234 - 1000);
    assert_eq!(restored.get::<_, i64>(1), SHIPPED_BALANCES - 1000);
    migrate(&url, LEDGER_MIGRATION, None).await?;
    let reapplied = client
        .query_one(
            "SELECT count(*), sum(amount)::bigint FROM customer_credit_grant",
            &[],
        )
        .await?;
    assert_eq!(reapplied.get::<_, i64>(0), 3);
    assert_eq!(reapplied.get::<_, i64>(1), SHIPPED_BALANCES - 1000);
    Ok(())
}

#[tokio::test]
async fn a_shipped_credit_purchase_aborts_the_upgrade() -> TestResult {
    let (_container, url) = postgres().await;
    let client = shipped_database(&url).await;
    client
        .batch_execute(&format!(
            "INSERT INTO customer_balance_pending_tx (id, amount_cents, invoice_id, tenant_id, customer_id, created_by)
             VALUES ('00000000-0000-0000-0000-000000000201', 5000, '00000000-0000-0000-0000-000000000101',
                     '{TENANT}', '{C1}', '00000000-0000-0000-0000-00000000a004')"
        ))
        .await?;
    let error = migrate(&url, LEDGER_MIGRATION, None)
        .await
        .expect_err("a credit purchase must abort the upgrade");
    assert!(error.contains("credit purchases exist"), "{error}");
    let untouched = client
        .query_one(
            "SELECT to_regclass('customer_credit_grant') IS NULL,
                    (SELECT sum(balance_value_cents)::bigint FROM customer)",
            &[],
        )
        .await?;
    assert!(untouched.get::<_, bool>(0));
    assert_eq!(untouched.get::<_, i64>(1), SHIPPED_BALANCES);
    Ok(())
}

#[tokio::test]
async fn rollback_refuses_credit_that_would_lose_its_terms() -> TestResult {
    let (_container, url) = postgres().await;
    let client = shipped_database(&url).await;
    migrate(&url, LEDGER_MIGRATION, None).await?;
    for (label, classification, effective, expires) in [
        ("promotional", "PROMOTIONAL", "now()", "NULL"),
        ("scheduled", "PAID", "now() + interval '1 day'", "NULL"),
        ("expiring", "PAID", "now()", "now() + interval '30 days'"),
    ] {
        client
            .batch_execute(&format!(
                "DELETE FROM customer_credit_entry e USING customer_credit_grant g
                 WHERE e.grant_id = g.id AND g.reason = 'rollback probe';
                 DELETE FROM customer_credit_grant WHERE reason = 'rollback probe';
                 INSERT INTO customer_credit_grant (id, application, tenant_id, customer_id, account_id, amount,
                                                    remaining, priority, classification, effective_at, expires_at,
                                                    source_kind, source_id, reason, created_at)
                 SELECT gen_random_uuid(), 'INVOICE', a.tenant_id, a.customer_id, a.id, 100, 100, 100, '{classification}',
                        {effective}, {expires}, 'MANUAL', gen_random_uuid(), 'rollback probe', now()
                 FROM customer_credit_account a
                 WHERE a.customer_id = '{C1}' AND a.currency = 'USD'"
            ))
            .await?;
        let error = revert_through(&url, LEDGER_MIGRATION)
            .await
            .expect_err("rollback must not turn this credit into plain paid balance");
        assert!(
            error.contains("promotional, scheduled or expiring"),
            "{label}: {error}"
        );
        let balance = client
            .query_one("SELECT sum(balance_value_cents)::bigint FROM customer", &[])
            .await?;
        assert_eq!(
            balance.get::<_, i64>(0),
            SHIPPED_BALANCES,
            "{label}: scalar untouched"
        );
    }
    Ok(())
}

#[tokio::test]
async fn rollback_refuses_credit_the_scalar_cannot_hold() -> TestResult {
    let (_container, url) = postgres().await;
    let client = shipped_database(&url).await;
    migrate(&url, LEDGER_MIGRATION, None).await?;
    // c1 is a USD customer; EUR invoice credit has no place in its scalar balance.
    client
        .batch_execute(&format!(
            "WITH a AS (INSERT INTO customer_credit_account (id, tenant_id, customer_id, currency)
                        VALUES (gen_random_uuid(), '{TENANT}', '{C1}', 'EUR') RETURNING id)
             INSERT INTO customer_credit_grant (id, application, tenant_id, customer_id, account_id, amount, remaining, priority,
                                                classification, effective_at, source_kind, source_id, reason, created_at)
             SELECT gen_random_uuid(), 'INVOICE', '{TENANT}', '{C1}', a.id, 100, 100, 100, 'PAID', now(), 'MANUAL',
                    gen_random_uuid(), 'foreign credit', now() FROM a;"
        ))
        .await?;
    let error = revert_through(&url, LEDGER_MIGRATION)
        .await
        .expect_err("rollback must not drop foreign-currency credit");
    assert!(error.contains("cannot hold"), "{error}");
    Ok(())
}

/// Paid, non-expiring credit restricted to a product passes every other guard, and the scalar
/// would let it pay any product.
#[tokio::test]
async fn rollback_refuses_product_restricted_credit() -> TestResult {
    let (_container, url) = postgres().await;
    let client = shipped_database(&url).await;
    migrate(&url, LEDGER_MIGRATION, None).await?;
    client
        .batch_execute(&format!(
            "INSERT INTO product_family (id, name, created_at, tenant_id)
             VALUES ('00000000-0000-0000-0000-0000000000f1', 'Default', now(), '{TENANT}');
             INSERT INTO product (id, name, created_at, tenant_id, product_family_id, fee_type,
                                  fee_structure, catalog, store_committed_at)
             VALUES ('00000000-0000-0000-0000-0000000000a1', 'Seats', now(), '{TENANT}',
                     '00000000-0000-0000-0000-0000000000f1', 'RATE', '{{}}', true, now());
             WITH g AS (INSERT INTO customer_credit_grant (id, application, tenant_id, customer_id, account_id, amount,
                                                           remaining, priority, classification, effective_at,
                                                           source_kind, source_id, reason, created_at)
                        SELECT gen_random_uuid(), 'INVOICE', a.tenant_id, a.customer_id, a.id, 100, 100, 100, 'PAID',
                               now(), 'MANUAL', gen_random_uuid(), 'seats only', now()
                        FROM customer_credit_account a
                        WHERE a.customer_id = '{C1}' AND a.currency = 'USD'
                        RETURNING id, tenant_id)
             INSERT INTO customer_credit_grant_product (tenant_id, grant_id, product_id)
             SELECT tenant_id, id, '00000000-0000-0000-0000-0000000000a1' FROM g;"
        ))
        .await?;
    let error = revert_through(&url, LEDGER_MIGRATION)
        .await
        .expect_err("rollback must not widen product-restricted credit to every product");
    assert!(error.contains("product-restricted"), "{error}");
    let balance = client
        .query_one("SELECT sum(balance_value_cents)::bigint FROM customer", &[])
        .await?;
    assert_eq!(
        balance.get::<_, i64>(0),
        SHIPPED_BALANCES,
        "scalar untouched"
    );
    Ok(())
}

#[tokio::test]
async fn rollback_refuses_credit_purchases() -> TestResult {
    let (_container, url) = postgres().await;
    let client = shipped_database(&url).await;
    migrate(&url, LEDGER_MIGRATION, None).await?;
    client
        .batch_execute(&format!(
            "INSERT INTO customer_credit_purchase (id, amount_cents, invoice_id, tenant_id, customer_id, created_by, created_at)
             VALUES (gen_random_uuid(), 700, '00000000-0000-0000-0000-000000000101', '{TENANT}', '{C1}',
                     '00000000-0000-0000-0000-00000000a004', now())"
        ))
        .await?;
    let error = revert_through(&url, LEDGER_MIGRATION)
        .await
        .expect_err("main has no place for a purchase sold on the ledger");
    assert!(error.contains("credit purchases exist"), "{error}");
    let still_upgraded = client
        .query_one("SELECT count(*) FROM customer_credit_grant", &[])
        .await?;
    assert_eq!(still_upgraded.get::<_, i64>(0), 3);
    Ok(())
}

#[tokio::test]
async fn grant_remaining_is_bounded_by_its_constraints() -> TestResult {
    let (_container, url) = postgres().await;
    let client = shipped_database(&url).await;
    migrate(&url, LEDGER_MIGRATION, None).await?;
    client
        .batch_execute(
            "UPDATE customer_credit_grant SET remaining = remaining - 1 WHERE remaining > 0",
        )
        .await?;
    let overdrawn = client
        .batch_execute("UPDATE customer_credit_grant SET remaining = -1")
        .await
        .expect_err("a grant never goes negative");
    assert!(format!("{overdrawn:?}").contains("violates check constraint"));
    let overfilled = client
        .batch_execute("UPDATE customer_credit_grant SET remaining = amount + 1")
        .await
        .expect_err("a grant never holds more than it was granted");
    assert!(format!("{overfilled:?}").contains("violates check constraint"));
    Ok(())
}

#[tokio::test]
async fn previous_release_writers_are_refused_after_the_upgrade() -> TestResult {
    let (_container, url) = postgres().await;
    let client = shipped_database(&url).await;
    migrate(&url, LEDGER_MIGRATION, None).await?;

    let refused = [
        format!(
            "UPDATE customer SET balance_value_cents = balance_value_cents - 100 WHERE id = '{C1}'"
        ),
        format!(
            "INSERT INTO customer_balance_tx (id, amount_cents, balance_cents_after, tenant_id, customer_id, currency)
             VALUES (gen_random_uuid(), -100, 1134, '{TENANT}', '{C1}', 'USD')"
        ),
        "UPDATE customer_balance_tx SET amount_cents = amount_cents + 1".to_owned(),
        "DELETE FROM customer_balance_tx".to_owned(),
        format!(
            "INSERT INTO customer_balance_pending_tx (id, amount_cents, invoice_id, tenant_id, customer_id, created_by)
             VALUES (gen_random_uuid(), 100, '00000000-0000-0000-0000-000000000101', '{TENANT}', '{C1}',
                     '00000000-0000-0000-0000-00000000a004')"
        ),
    ];
    for statement in &refused {
        let error = client
            .batch_execute(statement)
            .await
            .expect_err("a previous-release write must be refused");
        assert!(
            format!("{error:?}").contains(
                "customer balance moved to the credit ledger; retry on an upgraded instance"
            ),
            "{statement}: {error:?}"
        );
    }

    // Other customer columns stay writable, and nothing above changed a balance.
    client
        .batch_execute(&format!(
            "UPDATE customer SET name = 'Renamed' WHERE id = '{C1}'"
        ))
        .await?;
    let state = client
        .query_one(
            "SELECT (SELECT sum(balance_value_cents)::bigint FROM customer),
                    (SELECT count(*) FROM customer_balance_tx),
                    (SELECT count(*) FROM customer_balance_pending_tx),
                    (SELECT sum(remaining)::bigint FROM customer_credit_grant)",
            &[],
        )
        .await?;
    assert_eq!(state.get::<_, i64>(0), SHIPPED_BALANCES);
    assert_eq!(state.get::<_, i64>(1), 4);
    assert_eq!(state.get::<_, i64>(2), 0);
    assert_eq!(
        state.get::<_, i64>(3),
        state.get::<_, i64>(0),
        "the ledger balance equals the migrated balance"
    );

    // Rolling back lifts the freeze for the shipped release.
    revert_through(&url, LEDGER_MIGRATION).await?;
    client
        .batch_execute(&format!(
            "UPDATE customer SET balance_value_cents = balance_value_cents - 100 WHERE id = '{C1}'"
        ))
        .await?;
    Ok(())
}

#[tokio::test]
async fn customers_cannot_be_created_with_a_balance_after_the_upgrade() -> TestResult {
    let (_container, url) = postgres().await;
    let client = shipped_database(&url).await;
    migrate(&url, LEDGER_MIGRATION, None).await?;
    client
        .batch_execute(&format!(
            "CREATE TEMP TABLE cust_copy AS SELECT * FROM customer WHERE id = '{C1}';
             UPDATE cust_copy SET id = gen_random_uuid(), alias = 'inserted-' || gen_random_uuid()::text,
                                  balance_value_cents = 250;"
        ))
        .await?;
    for statement in [
        "INSERT INTO customer SELECT * FROM cust_copy",
        "INSERT INTO customer SELECT * FROM cust_copy
         ON CONFLICT (id) DO UPDATE SET name = excluded.name",
    ] {
        let error = client
            .batch_execute(statement)
            .await
            .expect_err("a customer created with a balance must be refused");
        assert!(
            format!("{error:?}").contains(
                "customer balance moved to the credit ledger; retry on an upgraded instance"
            ),
            "{statement}: {error:?}"
        );
    }
    client
        .batch_execute("UPDATE cust_copy SET balance_value_cents = 0")
        .await?;
    client
        .batch_execute("INSERT INTO customer SELECT * FROM cust_copy")
        .await?;
    let created = client
        .query_one(
            "SELECT count(*) FROM customer WHERE alias LIKE 'inserted-%' AND balance_value_cents = 0",
            &[],
        )
        .await?;
    assert_eq!(created.get::<_, i64>(0), 1);

    revert_through(&url, LEDGER_MIGRATION).await?;
    client
        .batch_execute(
            "UPDATE cust_copy SET id = gen_random_uuid(), alias = 'again-' || gen_random_uuid()::text,
                                  balance_value_cents = 250;
             INSERT INTO customer SELECT * FROM cust_copy",
        )
        .await?;
    Ok(())
}

const PREFLIGHT: &str = include_str!("../../../migrations/preflight/customer_credit_ledger.sql");

async fn preflight_issues(client: &Client) -> Result<Vec<(String, String)>, tokio_postgres::Error> {
    let mut issues = client
        .simple_query(PREFLIGHT)
        .await?
        .into_iter()
        .filter_map(|message| match message {
            tokio_postgres::SimpleQueryMessage::Row(row)
                if row.columns().iter().any(|c| c.name() == "issue") =>
            {
                Some((
                    row.get("issue").unwrap_or_default().to_owned(),
                    row.get("customer_id").unwrap_or_default().to_owned(),
                ))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    issues.sort();
    Ok(issues)
}

/// The preflight runs read-only on the shipped schema and lists what the upgrade refuses (a credit
/// purchase) or would credit at once (an invoice overpaid outside a sandbox).
#[tokio::test]
async fn preflight_lists_purchases_and_live_overpaid_invoices() -> TestResult {
    let (_container, url) = postgres().await;
    let client = shipped_database(&url).await;
    assert!(
        preflight_issues(&client).await?.is_empty(),
        "the fixture is clear"
    );

    client
        .batch_execute(&format!(
            "INSERT INTO customer_balance_pending_tx (id, amount_cents, invoice_id, tenant_id, customer_id, created_by)
             VALUES (gen_random_uuid(), 5000, '00000000-0000-0000-0000-000000000101', '{TENANT}', '{C1}',
                     '00000000-0000-0000-0000-00000000a004');
             INSERT INTO payment_transaction (id, tenant_id, invoice_id, amount, currency, status, payment_type, created_at)
             VALUES (gen_random_uuid(), '{TENANT}', '00000000-0000-0000-0000-000000000101', 5500, 'USD',
                     'SETTLED', 'PAYMENT', now());"
        ))
        .await?;
    assert_eq!(
        preflight_issues(&client).await?,
        [
            ("credit purchase".to_owned(), C1.to_owned()),
            ("overpaid invoice".to_owned(), C1.to_owned()),
        ]
    );

    // A sandbox's overpaid invoice is test money: it is not listed.
    client
        .batch_execute(&format!(
            "UPDATE tenant SET environment = 'SANDBOX' WHERE id = '{TENANT}'"
        ))
        .await?;
    assert_eq!(
        preflight_issues(&client).await?,
        [("credit purchase".to_owned(), C1.to_owned())]
    );
    Ok(())
}

#[tokio::test]
async fn rollback_refuses_credit_units_and_rollover() -> TestResult {
    let (_container, url) = postgres().await;
    let client = shipped_database(&url).await;
    migrate(&url, LEDGER_MIGRATION, None).await?;
    // A credit unit is only ever archived: some wallet, price, budget or pack may name it.
    client
        .batch_execute(&format!(
            "INSERT INTO credit_unit (id, tenant_id, code, name, plural_name, precision)
             VALUES (gen_random_uuid(), '{TENANT}', 'TOKENS', 'Token', 'Tokens', 0)"
        ))
        .await?;
    let error = revert_through(&url, LEDGER_MIGRATION)
        .await
        .expect_err("main has no column for what a credit unit prices, funds or caps");
    assert!(error.contains("credit units exist"), "{error}");
    client.batch_execute("DELETE FROM credit_unit").await?;

    // Rolled-over money credit that has lapsed holds nothing the scalar would miss, but the
    // rollover itself has no place in main.
    client
        .batch_execute(&format!(
            "WITH g AS (INSERT INTO customer_credit_grant (id, application, tenant_id, customer_id, account_id, amount,
                                                           remaining, priority, classification, effective_at, expires_at,
                                                           source_kind, source_id, reason, created_at)
                        SELECT gen_random_uuid(), 'INVOICE', a.tenant_id, a.customer_id, a.id, 100, 0, 100, 'PAID',
                               now() - interval '2 days', now() - interval '1 day', 'ROLLOVER', gen_random_uuid(),
                               'rolled over', now()
                        FROM customer_credit_account a WHERE a.customer_id = '{C1}'
                        RETURNING id, tenant_id, customer_id)
             INSERT INTO customer_credit_entry (id, tenant_id, customer_id, grant_id, kind, amount, source_id, created_at)
             SELECT gen_random_uuid(), tenant_id, customer_id, id, 'FUND', 100, gen_random_uuid(), now() FROM g"
        ))
        .await?;
    let error = revert_through(&url, LEDGER_MIGRATION)
        .await
        .expect_err("main cannot represent plan rollover");
    assert!(error.contains("rollover"), "{error}");
    Ok(())
}
