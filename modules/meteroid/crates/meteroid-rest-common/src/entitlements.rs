//! The REST models core entitlements and the modules' entitlement answers share.
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A number config value (decimal, encoded as a string).
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct NumberConfigValue {
    #[schema(value_type = String, format = "decimal")]
    pub value: Decimal,
}

/// A boolean config value.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct BooleanConfigValue {
    pub value: bool,
}

/// A text config value.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct TextConfigValue {
    pub value: String,
}

/// A structured (JSON) config value — the "metadata" case, several fields in one entitlement.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct JsonConfigValue {
    #[schema(value_type = serde_json::Value)]
    pub value: serde_json::Value,
}

/// A static, typed configuration value carried by a Config entitlement. Resolved synchronously
/// through the entitlement hierarchy — no metric, no usage counter.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConfigValue {
    Number(NumberConfigValue),
    Boolean(BooleanConfigValue),
    Text(TextConfigValue),
    Json(JsonConfigValue),
}

impl From<meteroid_store::domain::entitlements::ConfigValue> for ConfigValue {
    fn from(v: meteroid_store::domain::entitlements::ConfigValue) -> Self {
        use meteroid_store::domain::entitlements::ConfigValue as D;
        match v {
            D::Number(d) => ConfigValue::Number(NumberConfigValue { value: d }),
            D::Boolean(b) => ConfigValue::Boolean(BooleanConfigValue { value: b }),
            D::Text(s) => ConfigValue::Text(TextConfigValue { value: s }),
            D::Json(j) => ConfigValue::Json(JsonConfigValue { value: j }),
        }
    }
}
