use crate::enums::InvoiceAnnexKindEnum;
use chrono::NaiveDateTime;
use common_domain::ids::{InvoiceAnnexTemplateId, PlanVersionId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable, Clone)]
#[diesel(table_name = crate::schema::invoice_annex_template)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoiceAnnexTemplateRow {
    pub id: InvoiceAnnexTemplateId,
    pub tenant_id: TenantId,
    pub name: String,
    pub kind: InvoiceAnnexKindEnum,
    pub definition: serde_json::Value,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::plan_version_annex_template)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanVersionAnnexTemplateRowNew {
    pub plan_version_id: PlanVersionId,
    pub annex_template_id: InvoiceAnnexTemplateId,
    pub tenant_id: TenantId,
}
