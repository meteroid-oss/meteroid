//! The release's migrations apply, revert to exactly the schema they started from (main's), and
//! apply again to exactly the schema they first produced.
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
const FIRST_RELEASE_MIGRATION: &str = "20261008110347";

pub(crate) async fn postgres(name: &str) -> (ContainerAsync<GenericImage>, String) {
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
            .with_env_var("POSTGRES_INITDB_ARGS", "--no-sync")
            .with_cmd([
                "postgres",
                "-c",
                "fsync=off",
                "-c",
                "synchronous_commit=off",
            ])
            .with_container_name(format!(
                "{}{name}-{}",
                std::env::var("IT_CONTAINER_PREFIX").unwrap_or_default(),
                uuid::Uuid::now_v7()
            ))
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

/// Applies every migration with a version in `[from, until)`; each runs in its own transaction
/// unless its `metadata.toml` opts out (a `CREATE INDEX CONCURRENTLY`).
pub(crate) async fn migrate(url: &str, from: &str, until: Option<&str>) -> Result<(), String> {
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

/// Reverts the newest migration and returns its version.
async fn revert_last(url: &str) -> Result<String, String> {
    let url = url.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut conn = PgConnection::establish(&url).map_err(|e| e.to_string())?;
        conn.revert_last_migration(MIGRATIONS)
            .map(|version| version.to_string())
            .map_err(|e| e.to_string())
    })
    .await
    .expect("revert task")
}

pub(crate) async fn client(url: &str) -> Client {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await.expect("connect");
    tokio::spawn(connection);
    client
}

/// The whole public schema, as sorted lines: columns, constraints, indexes, enums, storage options
/// and persistence of every table, functions and triggers.
pub(crate) async fn catalog(client: &Client) -> Vec<String> {
    let queries = [
        "SELECT 'column ' || table_name || '.' || column_name || ' ' || data_type || ' ' || is_nullable
                || ' ' || coalesce(column_default, '')
         FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name <> '__diesel_schema_migrations'",
        "SELECT 'constraint ' || conrelid::regclass || ' ' || conname || ' ' || convalidated
                || ' ' || pg_get_constraintdef(oid)
         FROM pg_constraint
         WHERE connamespace = 'public'::regnamespace",
        "SELECT 'index ' || tablename || ' ' || indexname || ' ' || indexdef
         FROM pg_indexes
         WHERE schemaname = 'public'",
        "SELECT 'table ' || relname || ' ' || relpersistence::text || ' ' || coalesce(reloptions::text, '')
         FROM pg_class
         WHERE relkind = 'r' AND relnamespace = 'public'::regnamespace",
        "SELECT 'enum ' || t.typname || ' ' || e.enumsortorder || ' ' || e.enumlabel
         FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
         WHERE t.typnamespace = 'public'::regnamespace",
        "SELECT 'function ' || p.proname || ' ' || md5(pg_get_functiondef(p.oid))
         FROM pg_proc p
         WHERE p.pronamespace = 'public'::regnamespace AND p.prokind = 'f'",
        "SELECT 'trigger ' || tgrelid::regclass || ' ' || tgname || ' ' || tgenabled::text
                || ' ' || pg_get_triggerdef(oid)
         FROM pg_trigger
         WHERE NOT tgisinternal",
    ];
    let mut lines = Vec::new();
    for query in queries {
        for row in client.query(query, &[]).await.expect("catalog query") {
            lines.push(row.get::<_, String>(0));
        }
    }
    lines.sort();
    lines
}

#[tokio::test]
async fn release_migrations_revert_exactly_and_reapply_identically() -> TestResult {
    let (_container, url) = postgres("release-migrations").await;
    migrate(&url, "", Some(FIRST_RELEASE_MIGRATION)).await?;
    let client = client(&url).await;
    let before = catalog(&client).await;
    assert!(
        !before
            .iter()
            .any(|l| l.starts_with("table customer_credit_grant "))
    );

    migrate(&url, FIRST_RELEASE_MIGRATION, None).await?;
    let applied = catalog(&client).await;
    for expected in [
        "column usage_fold.grouping ",
        "column usage_fold.window_start ",
        "table usage_monitor_state u {fillfactor=80}",
        "table customer_budget p ",
        "column customer_budget.subscription_id ",
        "table credit_unit p ",
        "table payment_reversal_movement p ",
        "column invoice.store_xid ",
        "index usage_monitor_state usage_monitor_state_heartbeat_idx ",
        "index subscription subscription_live_customer_idx ",
    ] {
        assert!(
            applied.iter().any(|l| l.starts_with(expected)),
            "missing {expected}\n{applied:#?}"
        );
    }
    assert!(
        !applied
            .iter()
            .any(|l| l.starts_with("column usage_fold.period_start "))
    );
    assert!(
        !applied.iter().any(|l| l.starts_with("table usage_limit ")),
        "usage limits were folded into entitlements before the release"
    );
    assert!(
        applied
            .iter()
            .filter(|l| l.starts_with("constraint ") && !before.contains(l))
            .all(|l| l.split(' ').nth(3) == Some("true")),
        "every constraint the release adds is validated"
    );

    loop {
        let reverted = revert_last(&url).await?;
        if reverted.as_str() <= FIRST_RELEASE_MIGRATION {
            break;
        }
    }
    // PostgreSQL cannot drop enum values: the ones the release adds stay, unused.
    let kept = [
        "enum CheckoutTypeEnum 5 TOP_UP",
        "enum DeclineKindEnum 9 DO_NOT_RETRY",
        "enum DeclineKindEnum 10 UNCONFIRMED",
        "enum FeeTypeEnum 7 CREDIT",
        "enum PaymentTypeEnum 3 TOP_UP",
        "enum PaymentTypeEnum 4 AUTO_TOPUP",
    ];
    // Appended after the type's own labels, whatever their count.
    let final_invoice = |l: &str| l.starts_with("enum InvoiceType ") && l.ends_with(" FINAL");
    let reverted = catalog(&client).await;
    assert!(kept.iter().all(|value| reverted.iter().any(|l| l == value)));
    assert!(reverted.iter().any(|l| final_invoice(l)));
    assert_eq!(
        reverted
            .into_iter()
            .filter(|l| !kept.contains(&l.as_str()) && !final_invoice(l))
            .collect::<Vec<_>>(),
        before,
        "the down migrations restore main's schema"
    );

    migrate(&url, FIRST_RELEASE_MIGRATION, None).await?;
    assert_eq!(catalog(&client).await, applied);
    Ok(())
}
