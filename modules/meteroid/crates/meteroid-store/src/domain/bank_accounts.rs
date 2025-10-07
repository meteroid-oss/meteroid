pub use crate::domain::enums::BankAccountFormat;
use common_domain::country::CountryCode;
use common_domain::ids::{BankAccountId, TenantId};
use diesel_models::bank_accounts::{BankAccountRow, BankAccountRowNew, BankAccountRowPatch};
use o2o::o2o;
use uuid::Uuid;

#[derive(Clone, Debug, o2o)]
#[from_owned(BankAccountRow)]
pub struct BankAccount {
    pub id: BankAccountId,
    pub tenant_id: TenantId,
    pub currency: String,
    pub country: CountryCode,
    pub bank_name: String,
    #[map(~.into())]
    pub format: BankAccountFormat,
    pub account_numbers: String,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(BankAccountRowNew)]
pub struct BankAccountNew {
    pub id: BankAccountId,
    pub tenant_id: TenantId,
    pub created_by: Uuid,
    pub currency: String,
    pub country: CountryCode,
    pub bank_name: String,
    #[map(~.into())]
    pub format: BankAccountFormat,
    pub account_numbers: String,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(BankAccountRowPatch)]
pub struct BankAccountPatch {
    pub id: BankAccountId,
    pub tenant_id: TenantId,
    pub currency: Option<String>,
    pub country: Option<CountryCode>,
    pub bank_name: Option<String>,
    #[map(~.map(|x| x.into()))]
    pub format: Option<BankAccountFormat>,
    pub account_numbers: Option<String>,
}
