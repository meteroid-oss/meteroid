use secrecy::SecretString;

use crate::StoreResult;
use crate::domain::connectors::{Connector, ConnectorMeta};
use crate::domain::{Address, BankAccount};
use common_domain::ids::{
    BankAccountId, ConnectorId, InvoicingEntityId, StoredDocumentId, TenantId,
};
use diesel_models::invoicing_entities::{
    InvoicingEntityProvidersRow, InvoicingEntityRow, InvoicingEntityRowPatch,
    InvoicingEntityRowProvidersPatch,
};
use o2o::o2o;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, o2o)]
#[map_owned(InvoicingEntityRow)]
pub struct InvoicingEntity {
    pub id: InvoicingEntityId,
    pub is_default: bool,

    pub legal_name: String,

    pub invoice_number_pattern: String,
    pub next_invoice_number: i64,
    pub next_credit_note_number: i64,

    pub grace_period_hours: i32,
    pub net_terms: i32,
    pub invoice_footer_info: Option<String>,
    pub invoice_footer_legal: Option<String>,
    pub logo_attachment_id: Option<StoredDocumentId>,
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
    pub tenant_id: TenantId,

    pub card_provider_id: Option<ConnectorId>,
    pub direct_debit_provider_id: Option<ConnectorId>,
    pub bank_account_id: Option<BankAccountId>,
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
    pub logo_attachment_id: Option<StoredDocumentId>,
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
    pub id: InvoicingEntityId,
    pub legal_name: Option<String>,
    pub invoice_number_pattern: Option<String>,
    pub grace_period_hours: Option<i32>,
    pub net_terms: Option<i32>,
    pub invoice_footer_info: Option<String>,
    pub invoice_footer_legal: Option<String>,
    pub logo_attachment_id: Option<Option<StoredDocumentId>>,
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
    pub id: InvoicingEntityId,
    pub card_provider_id: Option<Option<ConnectorId>>,
    pub direct_debit_provider_id: Option<Option<ConnectorId>>,
    pub bank_account_id: Option<Option<BankAccountId>>,
}

#[derive(Clone, Debug)]
pub struct InvoicingEntityProviders {
    pub id: InvoicingEntityId,
    pub bank_account: Option<BankAccount>,
    pub card_provider: Option<ConnectorMeta>,
    pub direct_debit_provider: Option<ConnectorMeta>,
}

impl From<InvoicingEntityProvidersRow> for InvoicingEntityProviders {
    fn from(row: InvoicingEntityProvidersRow) -> Self {
        Self {
            id: row.entity.id,
            bank_account: row.bank_account.map(BankAccount::from),
            card_provider: row.card_provider.map(ConnectorMeta::from),
            direct_debit_provider: row.direct_debit_provider.map(ConnectorMeta::from),
        }
    }
}

#[derive(Clone, Debug)]
pub struct InvoicingEntityProviderSensitive {
    pub id: InvoicingEntityId,
    pub bank_account: Option<BankAccount>,
    pub card_provider: Option<Connector>,
    pub direct_debit_provider: Option<Connector>,
}

impl InvoicingEntityProviderSensitive {
    pub fn from_row(row: InvoicingEntityProvidersRow, key: &SecretString) -> StoreResult<Self> {
        Ok(Self {
            id: row.entity.id,
            bank_account: row.bank_account.map(BankAccount::from),
            card_provider: row
                .card_provider
                .map(|p| Connector::from_row(key, p))
                .transpose()?,
            direct_debit_provider: row
                .direct_debit_provider
                .map(|p| Connector::from_row(key, p))
                .transpose()?,
        })
    }
}
