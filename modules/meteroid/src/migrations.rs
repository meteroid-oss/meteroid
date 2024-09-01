use diesel_async::async_connection_wrapper::AsyncConnectionWrapper;
use diesel_migrations::MigrationHarness;
use meteroid_store::store::PgConn;
use std::error::Error;
use thiserror::Error;

mod diesel {
    use diesel_migrations::{embed_migrations, EmbeddedMigrations};
    pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations/diesel");
}

pub async fn run(conn: PgConn) -> Result<(), Box<dyn std::error::Error>> {
    let mut async_wrapper: AsyncConnectionWrapper<PgConn> = AsyncConnectionWrapper::from(conn);

    tokio::task::spawn_blocking(move || {
        async_wrapper
            .run_pending_migrations(diesel::MIGRATIONS)
            .map_err(|e| DieselMigrationError::ApplyError(e))
            .expect("Error running migrations");

        let all_migrations = async_wrapper
            .applied_migrations()
            .map_err(|e| DieselMigrationError::GetMigrationsError(e))
            .expect("Error getting migrations");

        for migration in all_migrations {
            tracing::info!("Migration Applied - {}", migration);
        }
    })
    .await?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum DieselMigrationError {
    #[error("ApplyError: {0}")]
    ApplyError(#[source] Box<dyn Error + Send + Sync>),
    #[error("GetMigrationsError: {0}")]
    GetMigrationsError(#[source] Box<dyn Error + Send + Sync>),
}
