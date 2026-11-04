use crate::enums::EmailSendClaimStateEnum;
use chrono::NaiveDateTime;
use common_domain::ids::{EntityActivityId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

/// Claim-before-send: one row per dedupe key, upserted before SMTP.
#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::email_send_claim)]
#[diesel(primary_key(dedupe_key))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EmailSendClaimRow {
    pub dedupe_key: String,
    pub tenant_id: TenantId,
    pub state: EmailSendClaimStateEnum,
    pub claimed_at: NaiveDateTime,
    pub finished_at: Option<NaiveDateTime>,
    pub sent_email_id: Option<EntityActivityId>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::email_send_claim)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EmailSendClaimRowNew {
    pub dedupe_key: String,
    pub tenant_id: TenantId,
    pub state: EmailSendClaimStateEnum,
    pub claimed_at: NaiveDateTime,
}
