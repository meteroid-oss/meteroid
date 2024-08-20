use chrono::NaiveDateTime;
use diesel_models::customers::CustomerRow;
use diesel_models::customers::{CustomerBriefRow, CustomerRowNew, CustomerRowPatch};
use error_stack::Report;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::errors::StoreError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Customer {
    pub id: Uuid,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub updated_by: Option<Uuid>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
    pub invoicing_entity_id: Uuid,
    pub billing_config: BillingConfig,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i32,
    pub balance_currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
}

impl TryFrom<CustomerRow> for Customer {
    type Error = Report<StoreError>;

    fn try_from(value: CustomerRow) -> Result<Self, Self::Error> {
        Ok(Customer {
            id: value.id,
            name: value.name,
            created_at: value.created_at,
            created_by: value.created_by,
            updated_at: value.updated_at,
            updated_by: value.updated_by,
            archived_at: value.archived_at,
            tenant_id: value.tenant_id,
            billing_config: value.billing_config.try_into()?,
            alias: value.alias,
            email: value.email,
            invoicing_email: value.invoicing_email,
            phone: value.phone,
            balance_value_cents: value.balance_value_cents,
            balance_currency: value.balance_currency,
            billing_address: value.billing_address.map(|v| v.try_into()).transpose()?,
            shipping_address: value.shipping_address.map(|v| v.try_into()).transpose()?,
            invoicing_entity_id: value.invoicing_entity_id,
        })
    }
}

impl TryInto<CustomerRow> for Customer {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<CustomerRow, Self::Error> {
        Ok(CustomerRow {
            id: self.id,
            name: self.name,
            created_at: self.created_at,
            created_by: self.created_by,
            updated_at: self.updated_at,
            updated_by: self.updated_by,
            archived_at: self.archived_at,
            tenant_id: self.tenant_id,
            billing_config: self.billing_config.try_into()?,
            alias: self.alias,
            email: self.email,
            invoicing_email: self.invoicing_email,
            phone: self.phone,
            balance_value_cents: self.balance_value_cents,
            balance_currency: self.balance_currency,
            billing_address: self.billing_address.map(|v| v.try_into()).transpose()?,
            shipping_address: self.shipping_address.map(|v| v.try_into()).transpose()?,
            invoicing_entity_id: self.invoicing_entity_id,
        })
    }
}

#[derive(Clone, Debug, o2o)]
#[from_owned(CustomerBriefRow)]
#[owned_into(CustomerBriefRow)]
pub struct CustomerBrief {
    pub id: Uuid,
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CustomerNew {
    pub name: String,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub invoicing_entity_id: Uuid,
    pub billing_config: BillingConfig,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i32,
    pub balance_currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub created_at: Option<NaiveDateTime>,
}

impl TryFrom<CustomerRowNew> for CustomerNew {
    type Error = Report<StoreError>;

    fn try_from(value: CustomerRowNew) -> Result<Self, Self::Error> {
        Ok(CustomerNew {
            name: value.name,
            created_at: value.created_at,
            created_by: value.created_by,
            tenant_id: value.tenant_id,
            invoicing_entity_id: value.invoicing_entity_id,
            billing_config: value.billing_config.try_into()?,
            alias: value.alias,
            email: value.email,
            invoicing_email: value.invoicing_email,
            phone: value.phone,
            balance_value_cents: value.balance_value_cents,
            balance_currency: value.balance_currency,
            billing_address: value.billing_address.map(|v| v.try_into()).transpose()?,
            shipping_address: value.shipping_address.map(|v| v.try_into()).transpose()?,
        })
    }
}

impl TryInto<CustomerRowNew> for CustomerNew {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<CustomerRowNew, Self::Error> {
        Ok(CustomerRowNew {
            id: Uuid::now_v7(),
            name: self.name,
            created_by: self.created_by,
            tenant_id: self.tenant_id,
            invoicing_entity_id: self.invoicing_entity_id,
            billing_config: self.billing_config.try_into()?,
            alias: self.alias,
            email: self.email,
            invoicing_email: self.invoicing_email,
            phone: self.phone,
            balance_value_cents: self.balance_value_cents,
            balance_currency: self.balance_currency,
            billing_address: self.billing_address.map(|v| v.try_into()).transpose()?,
            shipping_address: self.shipping_address.map(|v| v.try_into()).transpose()?,
            created_at: self.created_at,
        })
    }
}

#[derive(Clone, Debug, o2o)]
#[owned_into(CustomerRowPatch)]
pub struct CustomerPatch {
    pub id: Uuid,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: Option<i32>,
    pub balance_currency: Option<String>,
    pub billing_address: Option<serde_json::Value>, // TODO avoid json
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: Option<Uuid>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Address {
    pub line1: Option<String>,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub country: Option<String>,
    pub state: Option<String>,
    pub zip_code: Option<String>,
}

impl TryFrom<serde_json::Value> for Address {
    type Error = Report<StoreError>;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let address = serde_json::from_value::<Address>(value).map_err(|e| {
            StoreError::SerdeError("Failed to deserialize customer address".to_string(), e)
        })?;

        Ok(address)
    }
}

impl TryInto<serde_json::Value> for Address {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<Value, Self::Error> {
        let address_json = serde_json::to_value(self).map_err(|e| {
            StoreError::SerdeError("Failed to serialize customer address".to_string(), e)
        })?;

        Ok(address_json)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShippingAddress {
    pub address: Option<Address>,
    pub same_as_billing: bool,
}

impl TryFrom<serde_json::Value> for ShippingAddress {
    type Error = Report<StoreError>;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let shipping_address = serde_json::from_value::<ShippingAddress>(value).map_err(|e| {
            StoreError::SerdeError(
                "Failed to deserialize customer shipping address".to_string(),
                e,
            )
        })?;

        Ok(shipping_address)
    }
}

impl TryInto<serde_json::Value> for ShippingAddress {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<Value, Self::Error> {
        let shipping_address_json = serde_json::to_value(self).map_err(|e| {
            StoreError::SerdeError(
                "Failed to serialize customer shipping address".to_string(),
                e,
            )
        })?;

        Ok(shipping_address_json)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BillingConfig {
    Stripe(Stripe),
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Stripe {
    pub customer_id: String,
    pub collection_method: i32, // todo fix: models.proto : CollectionMethod
}

impl TryFrom<serde_json::Value> for BillingConfig {
    type Error = Report<StoreError>;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let billing_config = serde_json::from_value::<BillingConfig>(value).map_err(|e| {
            StoreError::SerdeError(
                "Failed to deserialize customer billing config".to_string(),
                e,
            )
        })?;

        Ok(billing_config)
    }
}

impl TryInto<serde_json::Value> for BillingConfig {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<Value, Self::Error> {
        let billing_config_json = serde_json::to_value(self).map_err(|e| {
            StoreError::SerdeError("Failed to serialize customer billing config".to_string(), e)
        })?;

        Ok(billing_config_json)
    }
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
