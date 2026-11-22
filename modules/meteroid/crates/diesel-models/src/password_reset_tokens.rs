use chrono::{DateTime, Utc};
use common_domain::ids::UserId;
use diesel::{Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Queryable, Selectable, Debug, Clone)]
#[diesel(table_name = crate::schema::password_reset_token)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PasswordResetTokenRow {
    pub id: Uuid,
    pub user_id: UserId,
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::password_reset_token)]
pub struct PasswordResetTokenRowNew {
    pub id: Uuid,
    pub user_id: UserId,
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}
