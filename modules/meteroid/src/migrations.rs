use ::diesel::pg::Pg;
use diesel_migrations::MigrationHarness;
use std::error::Error;
use thiserror::Error;

mod diesel {
    use diesel_migrations::{embed_migrations, EmbeddedMigrations};
    pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations/diesel");
}

pub fn run(conn: &mut impl MigrationHarness<Pg>) -> error_stack::Result<(), DieselMigrationError> {
    conn.run_pending_migrations(diesel::MIGRATIONS)
        .map_err(|e| DieselMigrationError::ApplyError(e))?;

    let all_migrations = conn
        .applied_migrations()
        .map_err(|e| DieselMigrationError::GetMigrationsError(e))?;

    for migration in all_migrations {
        tracing::info!("Migration Applied - {}", migration);
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum DieselMigrationError {
    #[error("ApplyError: {0}")]
    ApplyError(#[source] Box<dyn Error + Send + Sync>),
    #[error("GetMigrationsError: {0}")]
    GetMigrationsError(#[source] Box<dyn Error + Send + Sync>),
}
