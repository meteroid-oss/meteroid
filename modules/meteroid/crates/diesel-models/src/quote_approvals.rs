use crate::enums::QuoteApprovalStatusEnum;
use chrono::NaiveDateTime;
use common_domain::ids::{QuoteApprovalId, QuoteApprovalRuleId, QuoteId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Debug, Clone, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::quote_approval_rule)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteApprovalRuleRow {
    pub id: QuoteApprovalRuleId,
    pub tenant_id: TenantId,
    pub name: String,
    pub min_discount_percent: Option<rust_decimal::Decimal>,
    pub max_net_terms_days: Option<i32>,
    pub max_trial_days: Option<i32>,
    pub always: bool,
    pub approver_user_ids: Vec<Option<Uuid>>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::quote_approval_rule)]
pub struct QuoteApprovalRuleRowNew {
    pub id: QuoteApprovalRuleId,
    pub tenant_id: TenantId,
    pub name: String,
    pub min_discount_percent: Option<rust_decimal::Decimal>,
    pub max_net_terms_days: Option<i32>,
    pub max_trial_days: Option<i32>,
    pub always: bool,
    pub approver_user_ids: Vec<Option<Uuid>>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::quote_approval_rule)]
#[diesel(treat_none_as_null = true)]
pub struct QuoteApprovalRuleRowPatch {
    pub name: String,
    pub min_discount_percent: Option<rust_decimal::Decimal>,
    pub max_net_terms_days: Option<i32>,
    pub max_trial_days: Option<i32>,
    pub always: bool,
    pub approver_user_ids: Vec<Option<Uuid>>,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::quote_approval)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteApprovalRow {
    pub id: QuoteApprovalId,
    pub tenant_id: TenantId,
    pub quote_id: QuoteId,
    pub revision: i32,
    pub document_hash: String,
    pub rule_ids: Vec<Option<Uuid>>,
    pub status: QuoteApprovalStatusEnum,
    pub requested_by: Option<Uuid>,
    pub requested_at: NaiveDateTime,
    pub request_note: Option<String>,
    pub decided_by: Option<Uuid>,
    pub decided_at: Option<NaiveDateTime>,
    pub decision_note: Option<String>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::quote_approval)]
pub struct QuoteApprovalRowNew {
    pub id: QuoteApprovalId,
    pub tenant_id: TenantId,
    pub quote_id: QuoteId,
    pub revision: i32,
    pub document_hash: String,
    pub rule_ids: Vec<Option<Uuid>>,
    pub status: QuoteApprovalStatusEnum,
    pub requested_by: Option<Uuid>,
    pub request_note: Option<String>,
}
