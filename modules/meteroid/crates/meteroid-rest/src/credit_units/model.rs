use common_domain::ids::{
    CreditUnitId, CreditUnitPriceId, ProductId, string_serde, string_serde_opt,
};
use meteroid_rest_common::datetime::ApiDateTime;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

/// A tenant-defined denomination usage can be priced or credit granted in, instead of money.
/// `code` and `precision` are immutable once created.
#[derive(Clone, Debug, ToSchema, serde::Serialize)]
pub struct CreditUnit {
    /// The unit's id (`cun_…`).
    #[serde(with = "string_serde")]
    #[schema(value_type = String)]
    pub id: CreditUnitId,
    /// Upper-case code naming the unit in wallets, grants and webhooks, e.g. `CREDITS`.
    pub code: String,
    /// Singular display name, e.g. "Credit".
    pub name: String,
    /// Plural display name, e.g. "Credits".
    pub plural_name: String,
    /// Short display symbol, e.g. "cr".
    pub symbol: Option<String>,
    /// Minor-unit exponent: amounts of this unit are integers of 10^-precision credit.
    pub precision: u8,
    /// Archived units take no new prices, packs or grants; existing ones keep working.
    pub archived: bool,
    /// The product sales of this unit are booked under: their tax category and accounting code.
    /// The unit can be sold (top-ups) only once it is set.
    #[serde(with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub product_id: Option<ProductId>,
    /// The active selling prices, one per currency.
    pub prices: Vec<CreditUnitPrice>,
}

/// What one credit sells for in a currency. A price is never edited: setting a new one for the
/// same currency archives it.
#[derive(Clone, Debug, ToSchema, serde::Serialize)]
pub struct CreditUnitPrice {
    /// The price's id (`cup_…`).
    #[serde(with = "string_serde")]
    #[schema(value_type = String)]
    pub id: CreditUnitPriceId,
    /// ISO 4217 code of the currency the unit sells in.
    pub currency: String,
    /// Major units of `currency` per credit, as a decimal string (e.g. "0.001").
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub unit_price: rust_decimal::Decimal,
    /// When the price was set.
    pub created_at: ApiDateTime,
    /// When a newer price, or its removal, replaced it; absent while active.
    pub archived_at: Option<ApiDateTime>,
}

/// The new price of one credit in a currency.
#[derive(Debug, ToSchema, serde::Serialize, serde::Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct CreditUnitPriceSetRequest {
    /// Major units of the currency per credit, as a decimal string: positive, at most 12 decimal
    /// places.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub unit_price: rust_decimal::Decimal,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    ToSchema,
    IntoParams,
    serde::Serialize,
    serde::Deserialize,
    Validate,
)]
#[into_params(parameter_in = Query)]
pub struct CreditUnitPriceListRequest {
    /// Include archived prices. Defaults to false.
    pub include_archived: Option<bool>,
}

/// A credit unit's selling prices, newest first.
#[derive(Debug, ToSchema, serde::Serialize)]
pub struct CreditUnitPriceListResponse {
    pub data: Vec<CreditUnitPrice>,
}

/// A new credit unit.
#[derive(Debug, ToSchema, serde::Serialize, serde::Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct CreditUnitCreateRequest {
    /// Immutable once created: 4 to 16 characters, A-Z/0-9/_, starting with a letter. Never an
    /// ISO currency code (those are always 3 letters).
    #[validate(length(min = 4, max = 16))]
    pub code: String,
    /// Singular display name, 1 to 64 characters, e.g. "Credit".
    #[validate(length(min = 1, max = 64))]
    pub name: String,
    /// Plural display name, 1 to 64 characters, e.g. "Credits".
    #[validate(length(min = 1, max = 64))]
    pub plural_name: String,
    /// Short display symbol, at most 8 characters.
    #[validate(length(max = 8))]
    pub symbol: Option<String>,
    /// Immutable once created: 0 to 6.
    #[validate(range(min = 0, max = 6))]
    pub precision: u8,
}

/// Partial update: an absent field is left unchanged. `code` and `precision` have no field here,
/// they are immutable. `symbol` set to `null` clears it.
#[derive(Debug, Default, ToSchema, serde::Serialize, serde::Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct CreditUnitUpdateRequest {
    /// Singular display name, 1 to 64 characters.
    #[validate(length(min = 1, max = 64))]
    pub name: Option<String>,
    /// Plural display name, 1 to 64 characters.
    #[validate(length(min = 1, max = 64))]
    pub plural_name: Option<String>,
    /// Validated by the store (trimmed, at most 8 characters) rather than here: `validator` has
    /// no built-in support for a double `Option`.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub symbol: Option<Option<String>>,
    /// Blocks the unit from new prices, packs and grants; existing ones keep working.
    pub archived: Option<bool>,
    /// The product sales of the unit are booked under; `null` clears it, which stops the unit
    /// from being sold.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub product_id: Option<Option<ProductId>>,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    ToSchema,
    IntoParams,
    serde::Serialize,
    serde::Deserialize,
    Validate,
)]
#[into_params(parameter_in = Query)]
pub struct CreditUnitListRequest {
    /// Include archived units. Defaults to false.
    pub include_archived: Option<bool>,
}

/// The tenant's credit units.
#[derive(Debug, ToSchema, serde::Serialize)]
pub struct CreditUnitListResponse {
    pub data: Vec<CreditUnit>,
}
