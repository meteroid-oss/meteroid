use crate::enums::StoredObjectKindEnum;
use chrono::NaiveDateTime;
use common_domain::ids::{StoredDocumentId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Clone, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::stored_object)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct StoredObjectRow {
    pub id: StoredDocumentId,
    pub tenant_id: TenantId,
    pub kind: StoredObjectKindEnum,
    pub key: String,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::stored_object)]
pub struct StoredObjectRowNew {
    pub id: StoredDocumentId,
    pub tenant_id: TenantId,
    pub kind: StoredObjectKindEnum,
    pub key: String,
}
