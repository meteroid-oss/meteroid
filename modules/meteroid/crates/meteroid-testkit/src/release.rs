//! Databases for the release's migration tests: a Postgres of their own, migrated to any version
//! and compared by catalog, and a database at main's schema with changes pending as scheduled
//! events, as main leaves them.
use chrono::{Datelike, Months, NaiveDate};
use diesel::migration::{Migration, MigrationSource};
use diesel::pg::Pg;
use diesel::{Connection, PgConnection};
use diesel_migrations::{FileBasedMigrations, MigrationHarness};
use testcontainers::core::WaitFor;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio_postgres::{Client, NoTls};
use uuid::Uuid;

const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations/diesel");

/// The first migration main does not have.
pub const FIRST_AFTER_MAIN: &str = "20261116171538";

pub async fn postgres(name: &str) -> (ContainerAsync<GenericImage>, String) {
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
                Uuid::now_v7()
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

/// The server `METEROID_TEST_PG_URL` names, or a container of its own (kept while it is held).
pub async fn server(name: &str) -> (Option<ContainerAsync<GenericImage>>, String) {
    match std::env::var("METEROID_TEST_PG_URL") {
        Ok(url) => (None, url),
        Err(_) => {
            let (container, url) = postgres(name).await;
            (Some(container), url)
        }
    }
}

fn migrations() -> FileBasedMigrations {
    FileBasedMigrations::from_path(MIGRATIONS_DIR).expect("the migrations directory")
}

fn sorted_migrations() -> Vec<Box<dyn Migration<Pg>>> {
    let mut migrations = MigrationSource::<Pg>::migrations(&migrations()).expect("the migrations");
    migrations.sort_by_key(|m| m.name().version().to_string());
    migrations
}

/// Applies every migration with a version in `[from, until)`; each runs in its own transaction
/// unless its `metadata.toml` opts out (a `CREATE INDEX CONCURRENTLY`).
pub async fn migrate(url: &str, from: &str, until: Option<&str>) -> Result<(), String> {
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
pub async fn revert_last(url: &str) -> Result<String, String> {
    let url = url.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut conn = PgConnection::establish(&url).map_err(|e| e.to_string())?;
        conn.revert_last_migration(migrations())
            .map(|version| version.to_string())
            .map_err(|e| e.to_string())
    })
    .await
    .expect("revert task")
}

pub async fn client(url: &str) -> Client {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await.expect("connect");
    tokio::spawn(connection);
    client
}

/// The whole public schema, as sorted lines: columns, constraints, indexes, enums, storage options
/// and persistence of every table, functions and triggers.
pub async fn catalog(client: &Client) -> Vec<String> {
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
        // pgcrypto, created by the entitlements migration, stays installed on a revert.
        "SELECT 'function ' || p.proname || ' ' || md5(pg_get_functiondef(p.oid))
         FROM pg_proc p
         WHERE p.pronamespace = 'public'::regnamespace AND p.prokind = 'f'
           AND NOT EXISTS (SELECT 1 FROM pg_depend d JOIN pg_extension x ON x.oid = d.refobjid
                           WHERE d.objid = p.oid AND d.deptype = 'e' AND x.extname = 'pgcrypto')",
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

pub struct Seeded {
    pub tenant: Uuid,
    pub entity: Uuid,
    pub customer: Uuid,
    pub price: Uuid,
    pub subscription: Uuid,
    pub component: Uuid,
    pub amendment: Uuid,
}

/// One tenant of `status` in `environment` with a monthly subscription started last month, its
/// one component, and an amendment closing that component at the next period, pending.
async fn seed_tenant(
    client: &Client,
    organization: Uuid,
    status: &str,
    environment: &str,
    period_start: NaiveDate,
) -> Seeded {
    let [
        tenant,
        entity,
        customer,
        family,
        product,
        price,
        plan,
        version,
        price_component,
    ] = [(); 9].map(|()| Uuid::now_v7());
    let (subscription, component, amendment) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let start = period_start - Months::new(1);
    let next = period_start + Months::new(1);
    let archived = if status == "ARCHIVED" {
        "now()"
    } else {
        "NULL"
    };
    client
        .batch_execute(&format!(
            "INSERT INTO tenant (id, name, slug, organization_id, reporting_currency, status,
                 environment, archived_at)
             VALUES ('{tenant}', 't-{tenant}', 't-{tenant}', '{organization}', 'EUR', '{status}',
                     '{environment}', {archived});
             INSERT INTO invoicing_entity (id, is_default, legal_name, invoice_number_pattern,
                 next_invoice_number, next_credit_note_number, grace_period_hours, net_terms,
                 country, accounting_currency, tenant_id)
             VALUES ('{entity}', true, 'E', 'INV-{{number}}', 1, 1, 0, 30, 'FR', 'EUR', '{tenant}');
             INSERT INTO customer (id, name, tenant_id, currency, invoicing_entity_id,
                 balance_value_cents, invoicing_emails, vat_number_format_valid, custom_taxes,
                 store_committed_at, custom_properties, customer_type, tax_status,
                 preferred_locales, payment_reminders_opt_out, created_at)
             SELECT '{customer}', 'C', '{tenant}', 'EUR', '{entity}', 0, '{{}}', false, '[]',
                 now(), '{{}}', enum_first(NULL::\"CustomerTypeEnum\"),
                 enum_first(NULL::\"CustomerTaxStatusEnum\"), '{{}}', false, now();
             INSERT INTO product_family (id, name, tenant_id) VALUES ('{family}', 'F', '{tenant}');
             INSERT INTO product (id, name, tenant_id, product_family_id, fee_type, fee_structure)
             VALUES ('{product}', 'Platform', '{tenant}', '{family}', 'RATE', '{{\"type\": \"Rate\"}}');
             INSERT INTO price (id, product_id, cadence, currency, pricing, tenant_id)
             VALUES ('{price}', '{product}', 'MONTHLY', 'EUR',
                     '{{\"rate\": \"10.00\", \"type\": \"Rate\"}}', '{tenant}');
             INSERT INTO plan (id, name, tenant_id, product_family_id, plan_type, status)
             VALUES ('{plan}', 'P', '{tenant}', '{family}', 'STANDARD', 'ACTIVE');
             INSERT INTO plan_version (id, is_draft_version, plan_id, version, tenant_id, net_terms,
                 currency, created_at, trial_is_free, uses_product_pricing, tax_inclusive,
                 credit_exhausted_caps)
             VALUES ('{version}', false, '{plan}', 1, '{tenant}', 30, 'EUR', now(), true, true,
                     false, '{{}}');
             INSERT INTO price_component (id, name, plan_version_id, product_id)
             VALUES ('{price_component}', 'Platform', '{version}', '{product}');
             INSERT INTO subscription (id, customer_id, billing_day_anchor, tenant_id, start_date,
                 plan_version_id, created_at, net_terms, period, status, current_period_start,
                 current_period_end, currency, activated_at, billing_start_date,
                 activation_condition)
             VALUES ('{subscription}', '{customer}', 1, '{tenant}', '{start}', '{version}', now(),
                     30, 'MONTHLY', 'ACTIVE', '{period_start}', '{next}', 'EUR', '{start}',
                     '{start}', 'ON_START');
             INSERT INTO subscription_component (id, name, subscription_id, price_component_id,
                 product_id, period, price_id, effective_from)
             VALUES ('{component}', 'Platform', '{subscription}', '{price_component}',
                     '{product}', 'MONTHLY', '{price}', '{start}');
             INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type,
                 scheduled_time, priority, event_data, created_at, status, source)
             VALUES ('{amendment}', '{subscription}', '{tenant}', 'APPLY_AMENDMENT',
                     '{next} 00:00:00', 0,
                     '{{\"ApplyAmendment\": {{\"component_close\": [\"{component}\"],
                        \"component_insert\": [], \"addon_close\": [], \"addon_insert\": []}}}}',
                     now(), 'PENDING', 'api');"
        ))
        .await
        .expect("a tenant at main's schema");
    Seeded {
        tenant,
        entity,
        customer,
        price,
        subscription,
        component,
        amendment,
    }
}

async fn database(base_url: &str, name: &str) -> String {
    let admin = client(base_url).await;
    admin
        .batch_execute(&format!("CREATE DATABASE {name}"))
        .await
        .expect("create database");
    let (base, _) = base_url.rsplit_once('/').expect("a database in the url");
    format!("{base}/{name}")
}

pub struct AtMain {
    pub url: String,
    pub db: Client,
    pub next: NaiveDate,
    pub active: Seeded,
    pub archived: Seeded,
    pub sandbox: Seeded,
    /// Amendments naming a row that is gone, on the archived and the sandbox tenants.
    pub unresolvable: [Uuid; 2],
}

/// An amendment of `subscription` pending for `next`, closing a fee row that is gone.
pub async fn pending_unresolvable(db: &Client, subscription: Uuid, next: NaiveDate) -> Uuid {
    let id = Uuid::now_v7();
    db.batch_execute(&format!(
        "INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type, scheduled_time,
             priority, event_data, created_at, status, source)
         SELECT '{id}', s.id, s.tenant_id, 'APPLY_AMENDMENT', '{next} 00:00:00', 0,
                '{{\"ApplyAmendment\": {{\"component_close\": [\"{gone}\"],
                   \"component_insert\": [], \"addon_close\": [], \"addon_insert\": []}}}}',
                now(), 'PENDING', 'api'
         FROM subscription s WHERE s.id = '{subscription}'",
        gone = Uuid::now_v7(),
    ))
    .await
    .expect("an unresolvable amendment");
    id
}

/// A new database of the server at `base_url`, at main's schema: an active production tenant, an
/// archived production one and a sandbox one, each with an amendment pending for the next period;
/// the archived and sandbox tenants have one more, naming a row that is gone.
pub async fn at_main(base_url: &str) -> Result<AtMain, Box<dyn std::error::Error>> {
    let url = database(
        base_url,
        &format!("pending_changes_{}", Uuid::now_v7().simple()),
    )
    .await;
    migrate(&url, "", Some(FIRST_AFTER_MAIN)).await?;

    let today = common_clock::now().date_naive();
    let period_start = today.with_day(1).expect("the month's first day");
    let next = period_start + Months::new(1);
    let db = client(&url).await;
    let organization = Uuid::now_v7();
    db.batch_execute(&format!(
        "INSERT INTO organization (id, trade_name, slug, default_country)
         VALUES ('{organization}', 'O', 'o-{organization}', 'FR')"
    ))
    .await?;
    let active = seed_tenant(&db, organization, "ACTIVE", "PRODUCTION", period_start).await;
    let archived = seed_tenant(&db, organization, "ARCHIVED", "PRODUCTION", period_start).await;
    let sandbox = seed_tenant(&db, organization, "ACTIVE", "SANDBOX", period_start).await;
    // Tenants not live: a change whose row is gone is cancelled with its reason.
    let unresolvable = [
        pending_unresolvable(&db, archived.subscription, next).await,
        pending_unresolvable(&db, sandbox.subscription, next).await,
    ];
    Ok(AtMain {
        url,
        db,
        next,
        active,
        archived,
        sandbox,
        unresolvable,
    })
}
