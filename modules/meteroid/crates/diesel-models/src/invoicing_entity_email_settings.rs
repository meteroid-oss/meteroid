use chrono::NaiveDateTime;
use common_domain::ids::{InvoicingEntityId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

/// 1:1 child of `invoicing_entity`, created lazily. An absent row means defaults.
#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::invoicing_entity_email_settings)]
#[diesel(primary_key(invoicing_entity_id))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoicingEntityEmailSettingsRow {
    pub invoicing_entity_id: InvoicingEntityId,
    pub tenant_id: TenantId,
    pub sender_name: Option<String>,
    pub reply_to: Option<String>,
    /// Element-level Option<> is forced by diesel's TEXT[] mapping; elements are never NULL.
    pub bcc: Vec<Option<String>>,
    /// EmailKind snake_case values switched off for this entity.
    pub disabled_kinds: Vec<Option<String>>,
    /// IANA zone for the reminder send hour; NULL = UTC.
    pub timezone: Option<String>,
    pub reminder_send_hour: i16,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::invoicing_entity_email_settings)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoicingEntityEmailSettingsRowNew {
    pub invoicing_entity_id: InvoicingEntityId,
    pub tenant_id: TenantId,
    pub sender_name: Option<String>,
    pub reply_to: Option<String>,
    pub bcc: Vec<Option<String>>,
    pub disabled_kinds: Vec<Option<String>>,
    pub timezone: Option<String>,
    pub reminder_send_hour: i16,
}
