use chrono::NaiveDate;
use common_domain::ids::{
    BillableMetricId, CreditUnitId, RateCardId, string_serde, string_serde_opt,
};
use meteroid_rest_common::datetime::ApiDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

/// What a card's rates count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ToSchema, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RateCardDenomination {
    /// Money: each rate is in a currency, a price rated in its own currency's rates.
    Money,
    /// One credit unit (`credit_unit_id`): rates carry no currency.
    CreditUnit,
}

/// What usage no key rates does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, ToSchema, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RateCardUnmatched {
    /// The invoice waits until the usage is rated.
    #[default]
    Hold,
    /// The usage is left unbilled, on purpose.
    Ignore,
}

/// The rates of a metric's usage by its dimensions (such as model and token type), independent
/// of the plans that price it: a usage price names a card and a multiplier.
#[derive(Clone, Debug, ToSchema, Serialize)]
pub struct RateCard {
    /// The card's id (`rc_…`).
    #[serde(with = "string_serde")]
    #[schema(value_type = String)]
    pub id: RateCardId,
    pub code: String,
    pub name: String,
    /// The metric the card rates: a sum or a count, without a matrix.
    #[serde(with = "string_serde")]
    #[schema(value_type = String)]
    pub metric_id: BillableMetricId,
    /// The dimensions a key has a pattern for, in the order that decides between keys equally
    /// specific on an earlier one.
    pub dimensions: Vec<String>,
    pub denomination: RateCardDenomination,
    /// Set when `denomination` is `CREDIT_UNIT`.
    #[serde(with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub credit_unit_id: Option<CreditUnitId>,
    /// Rates are per this many units of the metric, as a decimal string ("1000000": per 1M
    /// tokens).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub unit_size: Decimal,
    pub on_unmatched: RateCardUnmatched,
    pub created_at: ApiDateTime,
}

/// A key's rate in a currency, in force from `effective_from`.
#[derive(Clone, Debug, ToSchema, Serialize)]
pub struct RateCardRate {
    /// A pattern per dimension of the card: a value, a prefix ending in `*` (`anthropic/*`), or
    /// `*` for any value, a missing one included.
    pub key: Vec<String>,
    /// The key is `*` on every dimension: it rates what no other key does.
    pub any_other: bool,
    /// ISO 4217 code; absent on a credit unit card.
    pub currency: Option<String>,
    /// Per `unit_size` units, in `currency` (or the card's credit unit), as a decimal string.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub rate: Decimal,
    pub effective_from: NaiveDate,
    pub label: Option<String>,
}

/// A change taking effect after `at`: a key's new rate, or its removal (`rate` null).
#[derive(Clone, Debug, ToSchema, Serialize)]
pub struct RateCardScheduledChange {
    pub key: Vec<String>,
    pub any_other: bool,
    pub currency: Option<String>,
    /// Absent: the key is removed in `currency` from `effective_from`.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub rate: Option<Decimal>,
    pub effective_from: NaiveDate,
    pub label: Option<String>,
}

/// A card with its rates in force on `at`, and the changes scheduled after it.
#[derive(Clone, Debug, ToSchema, Serialize)]
pub struct RateCardWithRates {
    #[serde(flatten)]
    pub card: RateCard,
    pub at: NaiveDate,
    /// By key, then currency; removed keys left out.
    pub rates: Vec<RateCardRate>,
    /// By day, then key.
    pub scheduled: Vec<RateCardScheduledChange>,
}

#[derive(Clone, Debug, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RateCardListRequest {
    /// Only the cards that rate this metric.
    #[serde(default, with = "string_serde_opt")]
    pub metric_id: Option<BillableMetricId>,
}

/// The tenant's rate cards, by code, archived ones left out.
#[derive(Clone, Debug, ToSchema, Serialize)]
pub struct RateCardListResponse {
    pub data: Vec<RateCard>,
}

#[derive(Clone, Debug, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RateCardGetRequest {
    /// The day the rates are read on (YYYY-MM-DD); today when absent.
    pub at: Option<NaiveDate>,
}

/// A new rate card. Its rates are set with `PATCH /api/v1/rate-cards/{id_or_code}/rates`.
#[derive(Debug, ToSchema, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct RateCardCreateRequest {
    /// Unique in the tenant, 1 to 64 characters.
    #[validate(length(min = 1, max = 64))]
    pub code: String,
    #[validate(length(min = 1, max = 256))]
    pub name: String,
    #[serde(with = "string_serde")]
    #[schema(value_type = String)]
    pub metric_id: BillableMetricId,
    /// One to four distinct event properties, e.g. `["model", "token_type"]`.
    #[validate(length(min = 1, max = 4))]
    pub dimensions: Vec<String>,
    pub denomination: RateCardDenomination,
    /// Required when `denomination` is `CREDIT_UNIT`, refused otherwise.
    #[serde(default, with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub credit_unit_id: Option<CreditUnitId>,
    /// Positive decimal string; "1" when absent.
    #[serde(default, with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub unit_size: Option<Decimal>,
    #[serde(default)]
    pub on_unmatched: RateCardUnmatched,
}

/// A key's rate in a currency.
#[derive(Debug, ToSchema, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct RateCardRateSet {
    /// A pattern per dimension of the card: a value, a prefix ending in `*`, or `*`. A key of
    /// `*` on every dimension is the "Any other" rate.
    pub key: Vec<String>,
    /// ISO 4217 code: required on a money card, refused on a credit unit card.
    pub currency: Option<String>,
    /// Non-negative decimal string, per `unit_size` units.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub rate: Decimal,
    #[validate(length(max = 256))]
    pub label: Option<String>,
}

/// A key no longer rated in a currency.
#[derive(Debug, ToSchema, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct RateCardKeyRemove {
    pub key: Vec<String>,
    /// ISO 4217 code: required on a money card, refused on a credit unit card.
    pub currency: Option<String>,
}

/// Rates set and keys removed together, all from `effective_from`.
#[derive(Debug, ToSchema, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct RateCardRatesSetRequest {
    /// The day the whole batch takes effect (YYYY-MM-DD); today when absent.
    pub effective_from: Option<NaiveDate>,
    #[serde(default)]
    #[validate(nested)]
    pub rates: Vec<RateCardRateSet>,
    #[serde(default)]
    #[validate(nested)]
    pub remove: Vec<RateCardKeyRemove>,
}

/// What the batch did, then the card as of the batch's day.
#[derive(Clone, Debug, ToSchema, Serialize)]
pub struct RateCardRatesSetResponse {
    /// Rates added.
    pub inserted: usize,
    /// Keys removed.
    pub removed: usize,
    /// Rates and removals already in force: a repeated sync changes nothing.
    pub unchanged: usize,
    /// Scheduled changes replaced or withdrawn.
    pub replaced: usize,
    pub rate_card: RateCardWithRates,
}
