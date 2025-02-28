use crate::enums::BankAccountFormat;
use chrono::NaiveDateTime;
use common_domain::ids::{BankAccountId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::bank_account)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BankAccountRow {
    pub id: BankAccountId,
    pub tenant_id: TenantId,
    pub currency: String,
    pub country: String,
    pub bank_name: String,
    pub format: BankAccountFormat,
    pub account_numbers: String,
    pub created_by: Uuid,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::bank_account)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BankAccountRowNew {
    pub id: BankAccountId,
    pub tenant_id: TenantId,
    pub created_by: Uuid,
    pub currency: String,
    pub country: String,
    pub bank_name: String,
    pub format: BankAccountFormat,
    pub account_numbers: String,
}

#[derive(AsChangeset)]
#[diesel(table_name = crate::schema::bank_account)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct BankAccountRowPatch {
    pub id: BankAccountId,
    pub tenant_id: TenantId,
    pub currency: Option<String>,
    pub country: Option<String>,
    pub bank_name: Option<String>,
    pub format: Option<BankAccountFormat>,
    pub account_numbers: Option<String>,
}
