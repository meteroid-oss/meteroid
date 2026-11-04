//! `migrations::run` brings databases migrated by the legacy public repository, by this repository
//! before the bridge, and fresh ones to the same schema, and does nothing on a second run.
use crate::release_migrations::{catalog, client, migrate, postgres};
use diesel::migration::{Migration, MigrationSource};
use diesel::pg::Pg;
use diesel::{Connection, PgConnection};
use diesel_migrations::{FileBasedMigrations, MigrationHarness};
use meteroid::migrations;
use meteroid_store::store::{PgConfig, PgPool, diesel_make_pg_pool};
use std::collections::{BTreeSet, HashMap};
use tokio_postgres::Client;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations/diesel");
const PUBLIC_FIXTURES_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/integration/fixtures/oss_migrations"
);
const PUBLIC_HISTORY: &str = include_str!("fixtures/oss_migrations/public_history.txt");
// Enterprise migrations whose version a public migration holds: the bridge applies them on legacy databases.
const USER_PREFERENCES: &str = "20260708000000";
const USER_PREFERENCES_COLUMN: &str = "column user.preferences ";
const USER_ACTOR_INDEX: &str = "index entity_activity idx_entity_activity_user_created ";
// The first migration of the bridge's release: an enterprise database before it has none of them.
const BRIDGE_RELEASE: &str = "20261101100305";
// Public versions no migration of this repository has; the bridge keeps their rows.
const PUBLIC_ONLY_VERSIONS: [&str; 8] = [
    "202603272117360000",
    "20260611000000",
    "20260611120000",
    "20260611120001",
    "20260630120000",
    "20260820000004",
    "20260821000001",
    "20260827220329",
];

/// The public repository's migration set: its folders, read from the vendored fixtures when they
/// differ from (or are absent in) this repository, from this repository's directory otherwise.
struct PublicHistory {
    until: Option<&'static str>,
}

impl MigrationSource<Pg> for PublicHistory {
    fn migrations(&self) -> diesel::migration::Result<Vec<Box<dyn Migration<Pg>>>> {
        let mut by_name: HashMap<String, Box<dyn Migration<Pg>>> = HashMap::new();
        for dir in [MIGRATIONS_DIR, PUBLIC_FIXTURES_DIR] {
            let source = FileBasedMigrations::from_path(dir)?;
            for migration in MigrationSource::<Pg>::migrations(&source)? {
                // Vendored folders come second and replace a same-named folder.
                by_name.insert(migration.name().to_string(), migration);
            }
        }
        let mut migrations = Vec::new();
        for name in public_history() {
            let migration = by_name
                .remove(name)
                .ok_or_else(|| format!("public migration {name} is neither vendored nor here"))?;
            let version = migration.name().version().to_string();
            if self.until.is_some_and(|until| version.as_str() >= until) {
                continue;
            }
            migrations.push(migration);
        }
        Ok(migrations)
    }
}

fn public_history() -> impl Iterator<Item = &'static str> {
    PUBLIC_HISTORY
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

/// Migrates like a public binary did: Diesel's harness, pending migrations in version order, each
/// in its own transaction, version recorded. Returns how many ran.
async fn migrate_public_history(url: &str, until: Option<&'static str>) -> usize {
    let url = url.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut conn = PgConnection::establish(&url).expect("connect");
        conn.run_pending_migrations(PublicHistory { until })
            .expect("public history migrates")
            .len()
    })
    .await
    .expect("public history task")
}

async fn create_database(admin: &Client, base_url: &str, name: &str) -> String {
    admin
        .batch_execute(&format!("CREATE DATABASE {name}"))
        .await
        .expect("create database");
    let base = base_url
        .strip_suffix("/postgres")
        .expect("url ends with the postgres database");
    format!("{base}/{name}")
}

fn pool(url: &str) -> PgPool {
    diesel_make_pg_pool(PgConfig::new(url.to_owned())).expect("pool")
}

async fn versions(client: &Client) -> BTreeSet<String> {
    client
        .query("SELECT version FROM __diesel_schema_migrations", &[])
        .await
        .expect("versions")
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect()
}

struct Snapshot {
    catalog: Vec<String>,
    versions: BTreeSet<String>,
}

async fn snapshot(url: &str) -> Snapshot {
    let client = client(url).await;
    Snapshot {
        catalog: catalog(&client).await,
        versions: versions(&client).await,
    }
}

/// Runs `migrations::run` a second time and checks that it changes nothing.
async fn assert_second_run_is_noop(url: &str, label: &str) -> TestResult {
    let first = snapshot(url).await;
    migrations::run(&pool(url)).await?;
    let second = snapshot(url).await;
    assert_eq!(
        second.versions, first.versions,
        "{label}: second run records nothing"
    );
    assert_eq!(
        second.catalog, first.catalog,
        "{label}: second run changes nothing"
    );
    Ok(())
}

fn with(versions: &BTreeSet<String>, extra: &[&str]) -> BTreeSet<String> {
    let mut all = versions.clone();
    all.extend(extra.iter().map(|v| (*v).to_owned()));
    all
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn migration_bridge_converges_every_database_shape() -> TestResult {
    let (_container, base_url) = postgres("migration-bridge").await;
    let admin = client(&base_url).await;

    let fresh = create_database(&admin, &base_url, "fresh").await;
    migrations::run(&pool(&fresh)).await?;
    let expected = snapshot(&fresh).await;
    for line in [
        "table connect_oauth_app ",
        USER_PREFERENCES_COLUMN,
        USER_ACTOR_INDEX,
    ] {
        assert!(
            expected.catalog.iter().any(|l| l.starts_with(line)),
            "fresh: {line}"
        );
    }
    assert_second_run_is_noop(&fresh, "fresh").await?;

    // This repository's history before the bridge's release (Cloud, enterprise self-hosted).
    let enterprise = create_database(&admin, &base_url, "enterprise").await;
    migrate(&enterprise, "", Some(BRIDGE_RELEASE)).await?;
    migrations::run(&pool(&enterprise)).await?;
    let migrated = snapshot(&enterprise).await;
    assert_eq!(migrated.versions, expected.versions, "enterprise versions");
    assert_eq!(migrated.catalog, expected.catalog, "enterprise schema");
    assert_second_run_is_noop(&enterprise, "enterprise").await?;

    // Older than user_preferences: the bridge must not add its column, whose migration is not idempotent.
    let enterprise_older = create_database(&admin, &base_url, "enterprise_older").await;
    migrate(&enterprise_older, "", Some(USER_PREFERENCES)).await?;
    migrations::run(&pool(&enterprise_older)).await?;
    let migrated = snapshot(&enterprise_older).await;
    assert_eq!(
        migrated.versions, expected.versions,
        "older enterprise versions"
    );
    assert_eq!(
        migrated.catalog, expected.catalog,
        "older enterprise schema"
    );
    assert_second_run_is_noop(&enterprise_older, "older enterprise").await?;

    // The whole public history. Two processes start together, as the API and the scheduler do.
    let legacy = create_database(&admin, &base_url, "legacy").await;
    let public_count = public_history().count();
    assert_eq!(migrate_public_history(&legacy, None).await, public_count);
    let before = snapshot(&legacy).await;
    assert!(
        !before
            .catalog
            .iter()
            .any(|l| l.starts_with("table connect_oauth_app ")),
        "the public 2026-03-12-000001_meteroid_connect is the placeholder"
    );
    for line in [USER_PREFERENCES_COLUMN, USER_ACTOR_INDEX] {
        assert!(
            !before.catalog.iter().any(|l| l.starts_with(line)),
            "no public migration creates {line}"
        );
    }
    let (a, b) = (pool(&legacy), pool(&legacy));
    let (first, second) = tokio::join!(migrations::run(&a), migrations::run(&b));
    first?;
    second?;
    let migrated = snapshot(&legacy).await;
    assert!(
        before.versions.is_subset(&migrated.versions),
        "public rows stay"
    );
    assert_eq!(
        migrated.versions,
        with(&expected.versions, &PUBLIC_ONLY_VERSIONS),
        "legacy versions"
    );
    assert_eq!(migrated.catalog, expected.catalog, "legacy schema");
    for line in [USER_PREFERENCES_COLUMN, USER_ACTOR_INDEX] {
        assert!(
            migrated.catalog.iter().any(|l| l.starts_with(line)),
            "legacy: {line}"
        );
    }
    assert_second_run_is_noop(&legacy, "legacy").await?;

    // An older public release: only the Connect placeholder and remove_bm_sync_queue of the 11.
    let older = create_database(&admin, &base_url, "legacy_older").await;
    migrate_public_history(&older, Some("20260611000000")).await;
    migrations::run(&pool(&older)).await?;
    let migrated = snapshot(&older).await;
    assert_eq!(
        migrated.versions,
        with(&expected.versions, &["202603272117360000"]),
        "older legacy versions"
    );
    assert_eq!(migrated.catalog, expected.catalog, "older legacy schema");
    assert_second_run_is_noop(&older, "older legacy").await?;

    Ok(())
}

#[tokio::test]
async fn migration_bridge_refuses_a_recorded_version_without_its_schema() -> TestResult {
    let (_container, base_url) = postgres("migration-bridge-refuses").await;
    let admin = client(&base_url).await;

    let legacy = create_database(&admin, &base_url, "legacy").await;
    migrate_public_history(&legacy, None).await;
    client(&legacy)
        .await
        .batch_execute("ALTER TABLE payment_transaction DROP COLUMN initiated_by_customer_id")
        .await?;
    let before = snapshot(&legacy).await;

    let error = migrations::run(&pool(&legacy))
        .await
        .expect_err("the bridge refuses")
        .to_string();
    assert!(
        error.contains("20260820000004") && error.contains("schema changes are missing"),
        "{error}"
    );
    let after = snapshot(&legacy).await;
    assert_eq!(after.versions, before.versions, "nothing recorded");
    assert_eq!(after.catalog, before.catalog, "nothing changed");
    Ok(())
}

#[tokio::test]
async fn migrations_refuse_while_a_dropped_queue_holds_messages() -> TestResult {
    let (_container, base_url) = postgres("migration-bridge-queue").await;
    let admin = client(&base_url).await;

    let legacy = create_database(&admin, &base_url, "legacy").await;
    migrate_public_history(&legacy, None).await;
    let legacy_client = client(&legacy).await;
    legacy_client
        .batch_execute("SELECT pgmq.send('outbox_event', '{\"pending\": true}'::jsonb)")
        .await?;
    let before = snapshot(&legacy).await;

    let error = migrations::run(&pool(&legacy))
        .await
        .expect_err("a message would be dropped")
        .to_string();
    assert!(
        error.contains("1 message(s) wait in the `outbox_event` queue"),
        "{error}"
    );
    let after = snapshot(&legacy).await;
    assert_eq!(after.versions, before.versions, "nothing recorded");
    assert_eq!(after.catalog, before.catalog, "nothing changed");

    legacy_client
        .batch_execute("SELECT pgmq.purge_queue('outbox_event')")
        .await?;
    migrations::run(&pool(&legacy)).await?;
    Ok(())
}
