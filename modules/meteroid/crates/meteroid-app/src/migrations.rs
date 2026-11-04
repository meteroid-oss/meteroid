use diesel::RunQueryDsl;
use diesel::connection::SimpleConnection;
use diesel::dsl::sql;
use diesel::migration::{Migration, MigrationSource};
use diesel::pg::Pg;
use diesel::sql_types::{BigInt, Bool, Integer, Text};
use diesel_async::AsyncPgConnection;
use diesel_async::async_connection_wrapper::AsyncConnectionWrapper;
use diesel_migrations::MigrationHarness;
use meteroid_store::store::{PgConn, PgPool};
use std::error::Error;
use std::time::Duration;
use thiserror::Error;

mod diesel_migrations_source {
    use diesel_migrations::{EmbeddedMigrations, embed_migrations};
    pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("../../migrations/diesel");
}

/// Moves databases migrated by the legacy public repository onto this migration history.
const OSS_BRIDGE_SQL: &str = include_str!("../../../migrations/bridge/oss_bridge.sql");
// What the bridge can change: version rows, the Connect schema and the two re-applied migrations.
const BRIDGE_STATE: &str = r#"SELECT count(*),
        to_regclass('connect_oauth_app') IS NOT NULL,
        EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass('"user"')
                AND attname = 'preferences' AND NOT attisdropped),
        to_regclass('idx_entity_activity_user_created') IS NOT NULL
    FROM __diesel_schema_migrations"#;

const MIGRATIONS_TABLE_EXISTS: &str =
    "SELECT to_regclass('__diesel_schema_migrations') IS NOT NULL";

// The one-argument (bigint) form, as the bridge's own pg_advisory_xact_lock.
const TRY_LOCK: &str = "SELECT pg_try_advisory_lock(hashtext('meteroid.migrations'))";
const UNLOCK: &str = "SELECT pg_advisory_unlock(hashtext('meteroid.migrations'))";
const LOCK_RETRY: Duration = Duration::from_millis(500);
// The migrations call uuidv7(), built into PostgreSQL 18.
const MIN_SERVER_VERSION_NUM: i32 = 180000;
const UPGRADE_GUIDE: &str = "https://docs.meteroid.com/self-hosting/upgrading-to-2-0";
// Pending migrations that drop a queue with its messages, its consumer gone (0.19 to 1.0.0-rc3).
const DROPPED_QUEUES: &[(&str, &str)] = &[
    ("202602022117360000", "billable_metric_sync"),
    ("20260712130000", "outbox_event"),
];

type Conn = AsyncConnectionWrapper<AsyncPgConnection>;

/// Runs the legacy-OSS bridge then the pending migrations, serialized across processes.
pub async fn run(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    // Detached from the pool: on a panic the connection, and the session lock it holds, closes.
    let conn = PgConn::take(pool.get().await?);
    let mut conn: Conn = AsyncConnectionWrapper::from(conn);

    tokio::task::spawn_blocking(move || -> Result<(), DieselMigrationError> {
        check_server_version(&mut conn)?;
        acquire_lock(&mut conn)?;
        check_dropped_queues(&mut conn)?;
        bridge(&mut conn)?;

        let applied = conn
            .run_pending_migrations(diesel_migrations_source::MIGRATIONS)
            .map_err(DieselMigrationError::ApplyError)?;

        if applied.is_empty() {
            tracing::info!("Migrations up to date");
        } else {
            for migration in &applied {
                tracing::info!("Migration applied: {}", migration);
            }
        }

        if let Ok(mut all) = conn.applied_migrations() {
            all.sort();
            if let Some(last) = all.last() {
                tracing::info!("Latest migration: {}", last);
            }
        }

        // Dropping the detached connection releases the lock too.
        if let Err(error) = conn.batch_execute(UNLOCK) {
            tracing::warn!(%error, "Could not release the migration lock");
        }
        Ok(())
    })
    .await??;

    Ok(())
}

/// The migrations of this binary the database has not applied, by name. Writes nothing.
pub async fn pending(pool: &PgPool) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let conn = PgConn::take(pool.get().await?);
    let mut conn: Conn = AsyncConnectionWrapper::from(conn);

    let pending =
        tokio::task::spawn_blocking(move || -> Result<Vec<String>, DieselMigrationError> {
            let table_exists: bool = sql::<Bool>(MIGRATIONS_TABLE_EXISTS)
                .get_result(&mut conn)
                .map_err(DieselMigrationError::Status)?;
            // `pending_migrations` would create the version table on a fresh database.
            let names = if table_exists {
                conn.pending_migrations(diesel_migrations_source::MIGRATIONS)
                    .map(|migrations| migration_names(&migrations))
            } else {
                MigrationSource::<Pg>::migrations(&diesel_migrations_source::MIGRATIONS)
                    .map(|migrations| migration_names(&migrations))
            };
            names.map_err(DieselMigrationError::ApplyError)
        })
        .await??;

    Ok(pending)
}

fn migration_names<M: Migration<Pg> + ?Sized>(migrations: &[Box<M>]) -> Vec<String> {
    migrations.iter().map(|m| m.name().to_string()).collect()
}

/// Refuses before anything is written: on an older server the migrations would stop halfway.
fn check_server_version(conn: &mut Conn) -> Result<(), DieselMigrationError> {
    let (num, version): (i32, String) = sql::<(Integer, Text)>(
        "SELECT current_setting('server_version_num')::int, current_setting('server_version')",
    )
    .get_result(conn)
    .map_err(DieselMigrationError::ServerVersion)?;
    supported_server_version(num, &version)
}

fn supported_server_version(num: i32, version: &str) -> Result<(), DieselMigrationError> {
    if num < MIN_SERVER_VERSION_NUM {
        return Err(DieselMigrationError::UnsupportedPostgres(
            version.to_string(),
        ));
    }
    Ok(())
}

/// Refuses before anything is written while a queue a pending migration drops holds messages.
fn check_dropped_queues(conn: &mut Conn) -> Result<(), DieselMigrationError> {
    for (version, queue) in DROPPED_QUEUES {
        // Separate statements: a query naming a missing table fails at parse time.
        let queue_exists: bool =
            sql::<Bool>(&format!("SELECT to_regclass('pgmq.q_{queue}') IS NOT NULL"))
                .get_result(conn)
                .map_err(DieselMigrationError::Status)?;
        if !queue_exists {
            continue;
        }
        let history_exists: bool = sql::<Bool>(MIGRATIONS_TABLE_EXISTS)
            .get_result(conn)
            .map_err(DieselMigrationError::Status)?;
        if history_exists {
            let applied: bool = sql::<Bool>(&format!(
                "SELECT EXISTS (SELECT 1 FROM __diesel_schema_migrations WHERE version = '{version}')"
            ))
            .get_result(conn)
            .map_err(DieselMigrationError::Status)?;
            if applied {
                continue;
            }
        }
        let messages: i64 = sql::<BigInt>(&format!("SELECT count(*) FROM pgmq.q_{queue}"))
            .get_result(conn)
            .map_err(DieselMigrationError::Status)?;
        if messages > 0 {
            return Err(DieselMigrationError::QueueNotDrained(
                queue.to_string(),
                messages,
            ));
        }
    }
    Ok(())
}

/// Polls instead of blocking in pg_advisory_lock: a session waiting inside a statement holds a
/// snapshot, which a `CREATE INDEX CONCURRENTLY` run by the lock holder would wait for (deadlock).
fn acquire_lock(conn: &mut Conn) -> Result<(), DieselMigrationError> {
    let mut logged = false;
    loop {
        let acquired: bool = sql::<Bool>(TRY_LOCK)
            .get_result(conn)
            .map_err(DieselMigrationError::Lock)?;
        if acquired {
            return Ok(());
        }
        if !logged {
            tracing::info!("Waiting for another process to finish running migrations");
            logged = true;
        }
        std::thread::sleep(LOCK_RETRY);
    }
}

fn bridge(conn: &mut Conn) -> Result<(), DieselMigrationError> {
    let before = bridge_state(conn).map_err(DieselMigrationError::Bridge)?;
    conn.batch_execute(OSS_BRIDGE_SQL)
        .map_err(DieselMigrationError::Bridge)?;

    // The bridge returns without writing on a fresh database, so there is nothing to compare.
    if let Some(before) = before
        && let Some(after) = bridge_state(conn).map_err(DieselMigrationError::Bridge)?
        && after != before
    {
        tracing::info!(
            mapped_versions = after.versions - before.versions,
            connect_schema_created = !before.connect && after.connect,
            user_preferences_added = !before.user_preferences && after.user_preferences,
            user_actor_index_created = !before.user_actor_index && after.user_actor_index,
            "Legacy OSS migration history moved onto the current history"
        );
    }
    Ok(())
}

#[derive(PartialEq, Eq)]
struct BridgeState {
    versions: i64,
    connect: bool,
    user_preferences: bool,
    user_actor_index: bool,
}

/// `None` when Diesel's version table does not exist yet (fresh database).
fn bridge_state(conn: &mut Conn) -> diesel::QueryResult<Option<BridgeState>> {
    let table_exists: bool = sql::<Bool>(MIGRATIONS_TABLE_EXISTS).get_result(conn)?;
    if !table_exists {
        return Ok(None);
    }
    let (versions, connect, user_preferences, user_actor_index): (i64, bool, bool, bool) =
        sql::<(BigInt, Bool, Bool, Bool)>(BRIDGE_STATE).get_result(conn)?;
    Ok(Some(BridgeState {
        versions,
        connect,
        user_preferences,
        user_actor_index,
    }))
}

#[derive(Debug, Error)]
pub enum DieselMigrationError {
    #[error("Migration lock: {0}")]
    Lock(#[source] diesel::result::Error),
    #[error(
        "Meteroid needs PostgreSQL 18 or later, and this server runs {0}. Nothing was changed: upgrade PostgreSQL first, see {UPGRADE_GUIDE}"
    )]
    UnsupportedPostgres(String),
    #[error(
        "{1} message(s) wait in the `{0}` queue, which this version removes. Nothing was changed: start your previous version until the queue is empty, then upgrade, see {UPGRADE_GUIDE}"
    )]
    QueueNotDrained(String, i64),
    #[error("PostgreSQL version: {0}")]
    ServerVersion(#[source] diesel::result::Error),
    #[error("Legacy OSS migration bridge: {0}")]
    Bridge(#[source] diesel::result::Error),
    #[error("Migration status: {0}")]
    Status(#[source] diesel::result::Error),
    #[error("ApplyError: {0}")]
    ApplyError(#[source] Box<dyn Error + Send + Sync>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_18_and_later_are_supported() {
        assert!(supported_server_version(180000, "18.0").is_ok());
        assert!(supported_server_version(180003, "18.3").is_ok());
        assert!(supported_server_version(190001, "19.1").is_ok());
    }

    #[test]
    fn older_servers_are_refused_with_their_version() {
        let error = supported_server_version(170004, "17.4").unwrap_err();
        assert!(matches!(
            error,
            DieselMigrationError::UnsupportedPostgres(_)
        ));
        assert!(error.to_string().contains("this server runs 17.4"));
    }
}
