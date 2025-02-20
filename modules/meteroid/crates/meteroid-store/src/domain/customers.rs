use crate::domain::Identity;
use crate::errors::StoreError;
use crate::errors::StoreErrorReport;
use crate::json_value_serde;
use crate::utils::local_id::{IdType, LocalId};
use chrono::NaiveDateTime;
use diesel_models::customers::{CustomerBriefRow, CustomerRowNew, CustomerRowPatch};
use diesel_models::customers::{CustomerForDisplayRow, CustomerRow};
use error_stack::Report;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, o2o)]
#[try_from_owned(CustomerRow, StoreErrorReport)]
pub struct Customer {
    pub id: Uuid,
    pub local_id: String,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub updated_by: Option<Uuid>,
    pub archived_at: Option<NaiveDateTime>,
    pub archived_by: Option<Uuid>,
    pub tenant_id: Uuid,
    pub invoicing_entity_id: Uuid,
    #[map(~.try_into()?)]
    pub billing_config: BillingConfig,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i32,
    pub currency: String,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub billing_address: Option<Address>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub shipping_address: Option<ShippingAddress>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(CustomerBriefRow)]
#[owned_into(CustomerBriefRow)]
pub struct CustomerBrief {
    pub id: Uuid,
    pub local_id: String,
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CustomerNew {
    pub name: String,
    pub billing_config: BillingConfig,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i32,
    pub currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub created_by: Uuid,
    pub invoicing_entity_id: Option<Identity>,
    // for seeding
    pub force_created_date: Option<NaiveDateTime>,
}

#[derive(Clone, Debug)]
pub struct CustomerNewWrapper {
    pub inner: CustomerNew,
    pub tenant_id: Uuid,
    pub invoicing_entity_id: Uuid,
}

impl TryInto<CustomerRowNew> for CustomerNewWrapper {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<CustomerRowNew, Self::Error> {
        Ok(CustomerRowNew {
            id: Uuid::now_v7(),
            local_id: LocalId::generate_for(IdType::Customer),
            name: self.inner.name,
            created_by: self.inner.created_by,
            tenant_id: self.tenant_id,
            invoicing_entity_id: self.invoicing_entity_id,
            billing_config: self.inner.billing_config.try_into()?,
            alias: self.inner.alias,
            email: self.inner.email,
            invoicing_email: self.inner.invoicing_email,
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
        })
    }
}

#[derive(Clone, Debug, o2o)]
#[owned_try_into(CustomerRowPatch, StoreErrorReport)]
pub struct CustomerPatch {
    pub id: Uuid,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: Option<i32>,
    pub currency: Option<String>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub billing_address: Option<Address>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub shipping_address: Option<ShippingAddress>,
    pub invoicing_entity_id: Option<Uuid>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Address {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line1: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line2: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>, // TODO mandatory ?
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zip_code: Option<String>,
}

json_value_serde!(Address);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShippingAddress {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
    pub same_as_billing: bool,
}

json_value_serde!(ShippingAddress);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BillingConfig {
    Stripe(StripeCustomerConfig),
    Manual,
}

json_value_serde!(BillingConfig);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StripeCustomerConfig {
    pub customer_id: String,
    pub collection_method: StripeCollectionMethod,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StripeCollectionMethod {
    ChargeAutomatically,
    SendInvoice,
}

#[derive(Clone, Debug)]
pub struct CustomerTopUpBalance {
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub cents: i32,
    pub notes: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CustomerBuyCredits {
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub cents: i32,
    pub notes: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, o2o)]
#[try_from_owned(CustomerForDisplayRow, StoreErrorReport)]
pub struct CustomerForDisplay {
    pub id: Uuid,
    pub local_id: String,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub updated_by: Option<Uuid>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
    pub invoicing_entity_id: Uuid,
    pub invoicing_entity_local_id: String,
    #[map(~.try_into()?)]
    pub billing_config: BillingConfig,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i32,
    pub currency: String,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub billing_address: Option<Address>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub shipping_address: Option<ShippingAddress>,
}

#[derive(Clone, Debug)]
pub struct CustomerUpdate {
    pub local_id_or_alias: String,
    pub name: String,
    pub billing_config: BillingConfig,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub invoicing_entity_id: Identity,
}
