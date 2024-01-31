use error_stack::{Result, ResultExt};
use refinery::AsyncMigrate;
use regex::Regex;
use thiserror::Error;

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("./refinery/migrations");
}
pub fn file_match_re() -> Regex {
    Regex::new(r"^([U|V])(\d+(?:\.\d+)?)__(\w+)").unwrap()
}

lazy_static::lazy_static! {
    static ref RE: regex::Regex = file_match_re();
}

pub async fn run_migrations<C>(client: &mut C) -> Result<(), MigrationError>
where
    C: AsyncMigrate + Send,
{
    let migration_report = embedded::migrations::runner()
        .run_async(client)
        .await
        .change_context(MigrationError)
        .attach_printable("Failed to run migrations")?;

    for migration in migration_report.applied_migrations() {
        tracing::info!(
            "Migration Applied -  Name: {}, Version: {}",
            migration.name(),
            migration.version()
        );
    }

    Ok(())
}

#[derive(Debug, Error)]
#[error("Migration Error")]
pub struct MigrationError;
