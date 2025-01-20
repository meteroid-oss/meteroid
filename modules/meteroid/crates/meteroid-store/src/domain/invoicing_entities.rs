use o2o::o2o;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::configs::ProviderConfigMeta;
use crate::domain::{Address, BankAccount};
use diesel_models::invoicing_entities::{
    InvoicingEntityProvidersRow, InvoicingEntityRow, InvoicingEntityRowPatch,
    InvoicingEntityRowProvidersPatch,
};

#[derive(Clone, Debug, Serialize, Deserialize, o2o)]
#[map_owned(InvoicingEntityRow)]
pub struct InvoicingEntity {
    pub id: Uuid,
    pub local_id: String,
    pub is_default: bool,

    pub legal_name: String,

    pub invoice_number_pattern: String,
    pub next_invoice_number: i64,
    pub next_credit_note_number: i64,

    pub grace_period_hours: i32,
    pub net_terms: i32,
    pub invoice_footer_info: Option<String>,
    pub invoice_footer_legal: Option<String>,
    pub logo_attachment_id: Option<String>,
    pub brand_color: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub zip_code: Option<String>,
    pub state: Option<String>,
    pub city: Option<String>,
    pub vat_number: Option<String>,

    // immutable
    pub country: String,
    // immutable
    pub accounting_currency: String,
    pub tenant_id: Uuid,

    pub cc_provider_id: Option<Uuid>,
    pub bank_account_id: Option<Uuid>,
}

impl InvoicingEntity {
    pub fn address(&self) -> Address {
        Address {
            line1: self.address_line1.clone(),
            line2: self.address_line2.clone(),
            zip_code: self.zip_code.clone(),
            state: self.state.clone(),
            city: self.city.clone(),
            country: Some(self.country.clone()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct InvoicingEntityNew {
    pub country: Option<String>,
    pub legal_name: Option<String>,
    pub invoice_number_pattern: Option<String>,
    pub next_invoice_number: Option<i64>,
    pub next_credit_note_number: Option<i64>,
    pub grace_period_hours: Option<i32>,
    pub net_terms: Option<i32>,
    pub invoice_footer_info: Option<String>,
    pub invoice_footer_legal: Option<String>,
    pub logo_attachment_id: Option<String>,
    pub brand_color: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub zip_code: Option<String>,
    pub state: Option<String>,
    pub city: Option<String>,
    pub vat_number: Option<String>,
}

#[derive(Clone, Debug, o2o, Default)]
#[owned_into(InvoicingEntityRowPatch)]
#[ghosts(accounting_currency: {None})]
pub struct InvoicingEntityPatch {
    pub id: Uuid,
    pub legal_name: Option<String>,
    pub invoice_number_pattern: Option<String>,
    pub grace_period_hours: Option<i32>,
    pub net_terms: Option<i32>,
    pub invoice_footer_info: Option<String>,
    pub invoice_footer_legal: Option<String>,
    pub logo_attachment_id: Option<Option<String>>,
    pub brand_color: Option<Option<String>>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub zip_code: Option<String>,
    pub state: Option<String>,
    pub city: Option<String>,
    pub vat_number: Option<String>,
    pub country: Option<String>,
}

#[derive(Clone, Debug, o2o, Default)]
#[owned_into(InvoicingEntityRowProvidersPatch)]
pub struct InvoicingEntityProvidersPatch {
    pub id: Uuid,
    pub cc_provider_id: Option<Uuid>,
    pub bank_account_id: Option<Uuid>,
}

pub struct InvoicingEntityProviders {
    pub bank_account: Option<BankAccount>,
    pub cc_provider: Option<ProviderConfigMeta>,
}

impl From<InvoicingEntityProvidersRow> for InvoicingEntityProviders {
    fn from(row: InvoicingEntityProvidersRow) -> Self {
        Self {
            bank_account: row.bank_account.map(BankAccount::from),
            cc_provider: row.cc_provider.map(ProviderConfigMeta::from),
        }
    }
}
