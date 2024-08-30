use ::diesel::pg::Pg;
use diesel_migrations::MigrationHarness;
use thiserror::Error;

mod diesel {
    use diesel_migrations::{embed_migrations, EmbeddedMigrations};
    pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations/diesel");
}

pub fn run(conn: &mut impl MigrationHarness<Pg>) -> error_stack::Result<(), DieselMigrationError> {
    conn.run_pending_migrations(diesel::MIGRATIONS)
        .map_err(|_| DieselMigrationError)?;

    let all_migrations = conn
        .applied_migrations()
        .map_err(|_| DieselMigrationError)?;

    for migration in all_migrations {
        tracing::info!("Migration Applied - {}", migration);
    }

    Ok(())
}

#[derive(Debug, Error)]
#[error("Migration Error")]
pub struct DieselMigrationError;
