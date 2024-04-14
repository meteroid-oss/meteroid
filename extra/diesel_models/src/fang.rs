use crate::enums::FangTaskState;
use chrono::offset::Utc;
use chrono::DateTime;
use diesel::sql_types::{Bpchar, Nullable};
use diesel::{Identifiable, Queryable};
use uuid::Uuid;

#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::fang_tasks)]
pub struct FangTask {
    pub id: Uuid,
    pub metadata: serde_json::Value,
    pub error_message: Option<String>,
    pub state: FangTaskState,
    pub task_type: String,
    pub uniq_hash: Option<Nullable<Bpchar>>,
    pub retries: i32,
    pub scheduled_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::fang_tasks_archive)]
pub struct FangTasksArchive {
    pub id: Uuid,
    pub metadata: serde_json::Value,
    pub error_message: Option<String>,
    pub state: FangTaskState,
    pub task_type: String,
    pub uniq_hash: Option<Nullable<Bpchar>>,
    pub retries: i32,
    pub scheduled_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: DateTime<Utc>,
}
