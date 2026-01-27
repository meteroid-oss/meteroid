use diesel_async::async_connection_wrapper::AsyncConnectionWrapper;
use diesel_migrations::MigrationHarness;
use meteroid_store::store::{PgConn, PgPool};
use std::error::Error;
use thiserror::Error;

mod diesel {
    use diesel_migrations::{EmbeddedMigrations, embed_migrations};
    pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations/diesel");
}

pub async fn run(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get().await?;
    let mut async_wrapper: AsyncConnectionWrapper<PgConn> = AsyncConnectionWrapper::from(conn);

    tokio::task::spawn_blocking(move || {
        let applied = async_wrapper
            .run_pending_migrations(diesel::MIGRATIONS)
            .map_err(DieselMigrationError::ApplyError)
            .expect("Error running migrations");

        if applied.is_empty() {
            tracing::info!("Migrations up to date");
        } else {
            for migration in &applied {
                tracing::info!("Migration applied: {}", migration);
            }
        }

        if let Ok(mut all) = async_wrapper.applied_migrations() {
            all.sort();
            if let Some(last) = all.last() {
                tracing::info!("Latest migration: {}", last);
            }
        }
    })
    .await?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum DieselMigrationError {
    #[error("ApplyError: {0}")]
    ApplyError(#[source] Box<dyn Error + Send + Sync>),
}
