pub use crate::domain::enums::BankAccountFormat;
use diesel_models::bank_accounts::{BankAccountRow, BankAccountRowNew, BankAccountRowPatch};
use o2o::o2o;
use uuid::Uuid;

#[derive(Clone, Debug, o2o)]
#[from_owned(BankAccountRow)]
pub struct BankAccount {
    pub id: Uuid,
    pub local_id: String,
    pub tenant_id: Uuid,
    pub currency: String,
    pub country: String,
    pub bank_name: String,
    #[map(~.into())]
    pub format: BankAccountFormat,
    pub account_numbers: String,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(BankAccountRowNew)]
pub struct BankAccountNew {
    pub id: Uuid,
    pub local_id: String,
    pub tenant_id: Uuid,
    pub created_by: Uuid,
    pub currency: String,
    pub country: String,
    pub bank_name: String,
    #[map(~.into())]
    pub format: BankAccountFormat,
    pub account_numbers: String,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(BankAccountRowPatch)]
pub struct BankAccountPatch {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub currency: Option<String>,
    pub country: Option<String>,
    pub bank_name: Option<String>,
    #[map(~.map(|x| x.into()))]
    pub format: Option<BankAccountFormat>,
    pub account_numbers: Option<String>,
}
