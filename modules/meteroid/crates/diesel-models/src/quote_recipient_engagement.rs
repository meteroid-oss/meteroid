use chrono::NaiveDateTime;
use common_domain::ids::{QuoteId, QuoteRecipientEngagementId, TenantId};
use diesel::{Identifiable, Queryable, Selectable};

#[derive(Debug, Clone, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::quote_recipient_engagement)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteRecipientEngagementRow {
    pub id: QuoteRecipientEngagementId,
    pub tenant_id: TenantId,
    pub quote_id: QuoteId,
    pub email: String,
    pub view_count: i32,
    pub first_viewed_at: NaiveDateTime,
    pub last_viewed_at: NaiveDateTime,
    pub last_viewed_revision: i32,
}
