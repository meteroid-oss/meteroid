use common_domain::country::CountryCode;
use serde_with::skip_serializing_none;
use utoipa::ToSchema;

#[skip_serializing_none]
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct Address {
    pub line1: Option<String>,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub country: Option<CountryCode>, // TODO mandatory ?
    pub state: Option<String>,
    pub zip_code: Option<String>,
}

#[skip_serializing_none]
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct ShippingAddress {
    pub address: Option<Address>,
    pub same_as_billing: bool,
}
