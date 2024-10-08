use diesel_async::async_connection_wrapper::AsyncConnectionWrapper;
use diesel_migrations::MigrationHarness;
use meteroid_store::store::{PgConn, PgPool};
use std::error::Error;
use thiserror::Error;

mod diesel {
    use diesel_migrations::{embed_migrations, EmbeddedMigrations};
    pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations/diesel");
}

pub async fn run(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get().await?;
    let mut async_wrapper: AsyncConnectionWrapper<PgConn> = AsyncConnectionWrapper::from(conn);

    tokio::task::spawn_blocking(move || {
        async_wrapper
            .run_pending_migrations(diesel::MIGRATIONS)
            .map_err(DieselMigrationError::ApplyError)
            .expect("Error running migrations");

        let mut all_migrations = async_wrapper
            .applied_migrations()
            .map_err(DieselMigrationError::GetMigrationsError)
            .expect("Error getting migrations");

        all_migrations.sort();

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
