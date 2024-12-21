use utoipa::ToSchema;

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
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

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct ShippingAddress {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
    pub same_as_billing: bool,
}
