use crate::domain::connectors::ConnectionMeta;
use crate::domain::enums::PaymentMethodTypeEnum;
use crate::errors::StoreError;
use crate::errors::StoreErrorReport;
use crate::json_value_serde;
use chrono::NaiveDateTime;
use common_domain::ids::{
    AliasOr, BankAccountId, BaseId, ConnectorId, CustomerConnectionId, CustomerId,
    CustomerPaymentMethodId, InvoicingEntityId, TenantId,
};
use diesel_models::customer_connection::CustomerConnectionRow;
use diesel_models::customers::CustomerRow;
use diesel_models::customers::{CustomerBriefRow, CustomerRowNew, CustomerRowPatch};
use error_stack::Report;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, o2o)]
#[try_from_owned(CustomerRow, StoreErrorReport)]
pub struct Customer {
    pub id: CustomerId,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub updated_by: Option<Uuid>,
    pub archived_at: Option<NaiveDateTime>,
    pub archived_by: Option<Uuid>,
    pub tenant_id: TenantId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i64,
    pub currency: String,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub billing_address: Option<Address>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub shipping_address: Option<ShippingAddress>,
    pub bank_account_id: Option<BankAccountId>,
    pub current_payment_method_id: Option<CustomerPaymentMethodId>,
    pub card_provider_id: Option<ConnectorId>,
    pub direct_debit_provider_id: Option<ConnectorId>,
    pub vat_number: Option<String>,
    #[map(~.into_iter().flatten().collect())]
    pub invoicing_emails: Vec<String>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub conn_meta: Option<ConnectionMeta>,
    pub is_tax_exempt: bool,
    pub custom_tax_rate: Option<rust_decimal::Decimal>,
    pub vat_number_format_valid: bool,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(CustomerBriefRow)]
#[owned_into(CustomerBriefRow)]
pub struct CustomerBrief {
    pub id: CustomerId,
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CustomerNew {
    pub name: String,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Vec<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i64,
    pub currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub created_by: Uuid,
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    // for seeding
    pub force_created_date: Option<NaiveDateTime>,
    pub bank_account_id: Option<BankAccountId>,
    pub vat_number: Option<String>,
    pub custom_tax_rate: Option<rust_decimal::Decimal>,
    pub is_tax_exempt: bool,
}

impl CustomerNew {
    pub fn is_valid_vat_number_format(&self) -> bool {
        match self.vat_number {
            Some(ref vat_number) => {
                meteroid_tax::validation::validate_vat_number_format(vat_number)
            }
            None => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CustomerNewWrapper {
    pub inner: CustomerNew,
    pub tenant_id: TenantId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub vat_number_format_valid: bool,
}

impl TryInto<CustomerRowNew> for CustomerNewWrapper {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<CustomerRowNew, Self::Error> {
        Ok(CustomerRowNew {
            id: CustomerId::new(),
            name: self.inner.name,
            created_by: self.inner.created_by,
            tenant_id: self.tenant_id,
            invoicing_entity_id: self.invoicing_entity_id,
            alias: self.inner.alias,
            billing_email: self.inner.billing_email,
            invoicing_emails: self.inner.invoicing_emails.into_iter().map(Some).collect(),
            phone: self.inner.phone,
            balance_value_cents: self.inner.balance_value_cents,
            currency: self.inner.currency,
            billing_address: self
                .inner
                .billing_address
                .map(|v| v.try_into())
                .transpose()?,
            shipping_address: self
                .inner
                .shipping_address
                .map(|v| v.try_into())
                .transpose()?,
            created_at: self.inner.force_created_date,
            bank_account_id: self.inner.bank_account_id,
            current_payment_method_id: None,
            direct_debit_provider_id: None,
            card_provider_id: None,
            vat_number: self.inner.vat_number,
            custom_tax_rate: self.inner.custom_tax_rate,
            is_tax_exempt: self.inner.is_tax_exempt,
            vat_number_format_valid: self.vat_number_format_valid,
        })
    }
}

#[derive(Clone, Debug, o2o)]
#[owned_try_into(CustomerRowPatch, StoreErrorReport)]
#[ghosts(vat_number_format_valid: None)]
pub struct CustomerPatch {
    pub id: CustomerId,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    #[map(~.map(|v| v.into_iter().map(|t| Some(t.into())).collect()))]
    pub invoicing_emails: Option<Vec<String>>,
    pub phone: Option<String>,
    pub balance_value_cents: Option<i64>,
    pub currency: Option<String>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub billing_address: Option<Address>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub shipping_address: Option<ShippingAddress>,
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    pub vat_number: Option<Option<String>>,
    pub custom_tax_rate: Option<Option<rust_decimal::Decimal>>,
    pub bank_account_id: Option<Option<BankAccountId>>,
    pub is_tax_exempt: Option<bool>,
}

impl CustomerPatch {
    pub fn is_valid_vat_number_format(&self) -> Option<bool> {
        match self.vat_number.as_ref() {
            Some(Some(vat_number)) => Some(meteroid_tax::validation::validate_vat_number_format(
                vat_number,
            )),
            Some(None) => Some(false),
            None => None,
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Address {
    pub line1: Option<String>,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub country: Option<String>, // TODO mandatory ?
    pub state: Option<String>,
    pub zip_code: Option<String>,
}

json_value_serde!(Address);

impl From<Address> for meteroid_tax::Address {
    fn from(val: Address) -> Self {
        meteroid_tax::Address {
            line1: val.line1,
            city: val.city,
            country: val.country,
            region: val.state,
            postal_code: val.zip_code,
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShippingAddress {
    pub address: Option<Address>,
    pub same_as_billing: bool,
}

json_value_serde!(ShippingAddress);

#[derive(Clone, Debug)]
pub struct CustomerTopUpBalance {
    pub created_by: Uuid,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub cents: i64,
    pub notes: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CustomerBuyCredits {
    pub created_by: Uuid,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub cents: i64,
    pub notes: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CustomerUpdate {
    pub id_or_alias: AliasOr<CustomerId>,
    pub name: String,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Vec<String>,
    pub phone: Option<String>,
    pub currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub invoicing_entity_id: InvoicingEntityId,
    pub vat_number: Option<String>,
    pub custom_tax_rate: Option<rust_decimal::Decimal>,
    pub bank_account_id: Option<BankAccountId>,
    pub is_tax_exempt: bool,
}

impl CustomerUpdate {
    pub fn is_valid_vat_number_format(&self) -> bool {
        match self.vat_number {
            Some(ref vat_number) => {
                meteroid_tax::validation::validate_vat_number_format(vat_number)
            }
            None => false,
        }
    }
}

#[derive(Clone, Debug, o2o)]
#[from_owned(CustomerConnectionRow)]
#[owned_into(CustomerConnectionRow)]
pub struct CustomerConnection {
    pub id: CustomerConnectionId,
    pub customer_id: CustomerId,
    pub connector_id: ConnectorId,
    #[into(~.map(|v| v.into_iter().map(|t| Some(t.into())).collect()))]
    #[from(~.map(|v| v.into_iter().flatten().map(|t| t.into()).collect()))]
    pub supported_payment_types: Option<Vec<PaymentMethodTypeEnum>>,
    pub external_customer_id: String,
}
