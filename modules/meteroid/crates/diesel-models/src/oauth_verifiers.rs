use chrono::NaiveDateTime;
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Queryable, Debug, Identifiable, Insertable, Selectable)]
#[diesel(table_name = crate::schema::oauth_verifier)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OauthVerifierRow {
    pub id: Uuid,
    pub csrf_token: String,
    pub pkce_verifier: String,
    pub created_at: NaiveDateTime,
    pub data: Option<serde_json::Value>,
}
