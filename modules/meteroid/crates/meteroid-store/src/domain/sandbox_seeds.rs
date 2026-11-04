use chrono::NaiveDateTime;
use diesel_models::enums::SandboxSeedStatusEnum;
use diesel_models::sandbox_seed::SandboxSeedRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxSeedStatus {
    InProgress,
    Completed,
    Failed,
}

/// Whether a seeded sandbox's sample data finished, and why it stopped if it did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxSeed {
    pub status: SandboxSeedStatus,
    /// Shown to the user; never carries internal error details.
    pub failure_reason: Option<String>,
    pub started_at: NaiveDateTime,
    pub finished_at: Option<NaiveDateTime>,
}

impl From<SandboxSeedRow> for SandboxSeed {
    fn from(row: SandboxSeedRow) -> Self {
        SandboxSeed {
            status: match row.status {
                SandboxSeedStatusEnum::InProgress => SandboxSeedStatus::InProgress,
                SandboxSeedStatusEnum::Completed => SandboxSeedStatus::Completed,
                SandboxSeedStatusEnum::Failed => SandboxSeedStatus::Failed,
            },
            failure_reason: row.failure_reason,
            started_at: row.started_at,
            finished_at: row.finished_at,
        }
    }
}
