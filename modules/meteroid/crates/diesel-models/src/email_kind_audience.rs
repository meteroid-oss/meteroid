use crate::enums::EmailKindAudienceModeEnum;
use chrono::NaiveDateTime;
use common_domain::ids::{InvoicingEntityId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

/// "Send to" rule of one email kind for one invoicing entity; an absent row means everyone.
#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Insertable)]
#[diesel(table_name = crate::schema::email_kind_audience)]
#[diesel(primary_key(invoicing_entity_id, kind))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EmailKindAudienceRow {
    pub invoicing_entity_id: InvoicingEntityId,
    pub tenant_id: TenantId,
    /// EmailKind snake_case value.
    pub kind: String,
    pub mode: EmailKindAudienceModeEnum,
    pub audience: serde_json::Value,
    pub updated_at: NaiveDateTime,
}
