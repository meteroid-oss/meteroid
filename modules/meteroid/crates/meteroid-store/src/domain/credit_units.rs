//! Tenant-defined credit units: denominations that price usage in credits instead of money.
//! Amounts in a unit are minor units of 10^-precision credit.
use crate::StoreResult;
use crate::errors::StoreError;
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, CreditUnitId, CreditUnitPriceId, ProductId, TenantId};
use diesel_models::credit_units::{
    CreditUnitPriceRow, CreditUnitRow, CreditUnitRowNew, CreditUnitRowPatch,
};
use error_stack::Report;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

pub const CREDIT_UNIT_MAX_PRECISION: u8 = 6;
pub const CREDIT_UNIT_NAME_MAX_LEN: usize = 64;
pub const CREDIT_UNIT_SYMBOL_MAX_LEN: usize = 8;

fn invalid(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

/// Trims and checks against the same bounds as the `name`/`plural_name` CHECKs.
fn validate_label(value: &str, field: &str, max_len: usize) -> StoreResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > max_len {
        return Err(invalid(format!(
            "{field} must be 1 to {max_len} characters"
        )));
    }
    Ok(trimmed.to_owned())
}

/// Trims; an empty symbol normalizes to `None` (the DB stores NULL, never `''`).
fn validate_symbol(value: Option<&str>) -> StoreResult<Option<String>> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(symbol) => {
            if symbol.len() > CREDIT_UNIT_SYMBOL_MAX_LEN {
                return Err(invalid(format!(
                    "symbol must be at most {CREDIT_UNIT_SYMBOL_MAX_LEN} characters"
                )));
            }
            Ok(Some(symbol.to_owned()))
        }
    }
}

/// `^[A-Z][A-Z0-9_]{3,15}$`: four characters or more, so a unit code is never an ISO 4217 code
/// and one string column can key both money and unit wallets.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CreditUnitCode(String);

impl CreditUnitCode {
    pub const MIN_LEN: usize = 4;
    pub const MAX_LEN: usize = 16;

    /// Trims and upper-cases before validating.
    pub fn new(value: impl AsRef<str>) -> StoreResult<Self> {
        let code = value.as_ref().trim().to_ascii_uppercase();
        let mut chars = code.chars();
        let valid = (Self::MIN_LEN..=Self::MAX_LEN).contains(&code.len())
            && chars.next().is_some_and(|c| c.is_ascii_uppercase())
            && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if !valid {
            return Err(invalid(format!(
                "a credit unit code is {} to {} characters A-Z, 0-9 or _, starting with a letter",
                Self::MIN_LEN,
                Self::MAX_LEN
            )));
        }
        Ok(Self(code))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for CreditUnitCode {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value).map_err(|e| e.current_context().to_string())
    }
}

impl From<CreditUnitCode> for String {
    fn from(code: CreditUnitCode) -> Self {
        code.0
    }
}

impl Display for CreditUnitCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a wallet, budget or ledger amount is denominated in, as stored in their `currency` column:
/// three letters for an ISO currency, otherwise a credit unit code.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DenominationCode {
    Currency(String),
    Unit(CreditUnitCode),
}

impl DenominationCode {
    /// Trims and upper-cases; refuses an unsupported currency or a malformed unit code.
    pub fn parse(value: &str) -> StoreResult<Self> {
        let code = value.trim().to_ascii_uppercase();
        if code.len() == 3 {
            if crate::constants::Currencies::resolve_currency_precision(&code).is_none() {
                return Err(invalid(format!("unsupported currency {code}")));
            }
            return Ok(Self::Currency(code));
        }
        CreditUnitCode::new(code).map(Self::Unit)
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Currency(code) => code,
            Self::Unit(code) => code.as_str(),
        }
    }

    pub fn is_unit(&self) -> bool {
        matches!(self, Self::Unit(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditUnit {
    pub id: CreditUnitId,
    pub tenant_id: TenantId,
    pub code: CreditUnitCode,
    pub name: String,
    pub plural_name: String,
    pub symbol: Option<String>,
    pub precision: u8,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    /// The product sales of the unit are booked under: their tax category and accounting code.
    pub product_id: Option<ProductId>,
}

impl CreditUnit {
    pub fn reference(&self) -> CreditUnitRef {
        CreditUnitRef {
            unit_id: self.id,
            code: self.code.as_str().to_owned(),
            precision: self.precision,
            product_id: self.product_id,
        }
    }

    /// Refuses an archived unit for anything newly priced or granted in it. Existing prices,
    /// packs and wallets keep resolving it (`resolve_denomination`) so they keep working.
    pub fn ensure_active_for_new_pricing(&self) -> StoreResult<()> {
        if self.archived_at.is_some() {
            return Err(Report::new(StoreError::FailedPrecondition(format!(
                "credit unit {} is archived and cannot be used for new pricing",
                self.code
            ))));
        }
        Ok(())
    }
}

impl TryFrom<CreditUnitRow> for CreditUnit {
    type Error = Report<StoreError>;

    fn try_from(row: CreditUnitRow) -> Result<Self, Self::Error> {
        let precision = u8::try_from(row.precision)
            .ok()
            .filter(|p| *p <= CREDIT_UNIT_MAX_PRECISION)
            .ok_or_else(|| invalid(format!("credit unit precision {}", row.precision)))?;
        Ok(Self {
            id: row.id,
            tenant_id: row.tenant_id,
            code: CreditUnitCode::new(&row.code)?,
            name: row.name,
            plural_name: row.plural_name,
            symbol: row.symbol,
            precision,
            created_at: row.created_at,
            updated_at: row.updated_at,
            archived_at: row.archived_at,
            product_id: row.product_id,
        })
    }
}

/// A credit unit to create. Code and precision become immutable once stored (D11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditUnitNew {
    pub id: CreditUnitId,
    pub tenant_id: TenantId,
    pub code: CreditUnitCode,
    pub name: String,
    pub plural_name: String,
    pub symbol: Option<String>,
    pub precision: u8,
}

impl CreditUnitNew {
    pub fn new(
        tenant_id: TenantId,
        code: impl AsRef<str>,
        name: impl AsRef<str>,
        plural_name: impl AsRef<str>,
        symbol: Option<&str>,
        precision: u8,
    ) -> StoreResult<Self> {
        if precision > CREDIT_UNIT_MAX_PRECISION {
            return Err(invalid(format!(
                "credit unit precision must be 0 to {CREDIT_UNIT_MAX_PRECISION}"
            )));
        }
        Ok(Self {
            id: CreditUnitId::new(),
            tenant_id,
            code: CreditUnitCode::new(code)?,
            name: validate_label(name.as_ref(), "name", CREDIT_UNIT_NAME_MAX_LEN)?,
            plural_name: validate_label(
                plural_name.as_ref(),
                "plural_name",
                CREDIT_UNIT_NAME_MAX_LEN,
            )?,
            symbol: validate_symbol(symbol)?,
            precision,
        })
    }
}

impl From<CreditUnitNew> for CreditUnitRowNew {
    fn from(value: CreditUnitNew) -> Self {
        Self {
            id: value.id,
            tenant_id: value.tenant_id,
            code: value.code.into_inner(),
            name: value.name,
            plural_name: value.plural_name,
            symbol: value.symbol,
            precision: i16::from(value.precision),
        }
    }
}

/// Partial update: a `None` field is left unchanged. `archived` toggles `archived_at`; code and
/// precision have no field here, they are immutable (D11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditUnitPatch {
    pub tenant_id: TenantId,
    pub id: CreditUnitId,
    pub name: Option<String>,
    pub plural_name: Option<String>,
    pub symbol: Option<Option<String>>,
    pub archived: Option<bool>,
    /// `Some(None)` clears the product, which stops the unit from being sold.
    pub product_id: Option<Option<ProductId>>,
}

impl CreditUnitPatch {
    pub fn new(
        tenant_id: TenantId,
        id: CreditUnitId,
        name: Option<String>,
        plural_name: Option<String>,
        symbol: Option<Option<String>>,
        archived: Option<bool>,
    ) -> StoreResult<Self> {
        let name = name
            .map(|v| validate_label(&v, "name", CREDIT_UNIT_NAME_MAX_LEN))
            .transpose()?;
        let plural_name = plural_name
            .map(|v| validate_label(&v, "plural_name", CREDIT_UNIT_NAME_MAX_LEN))
            .transpose()?;
        let symbol = symbol.map(|v| validate_symbol(v.as_deref())).transpose()?;
        Ok(Self {
            tenant_id,
            id,
            name,
            plural_name,
            symbol,
            archived,
            product_id: None,
        })
    }

    pub fn with_product_id(self, product_id: Option<ProductId>) -> Self {
        Self {
            product_id: Some(product_id),
            ..self
        }
    }
}

impl From<&CreditUnitPatch> for CreditUnitRowPatch {
    fn from(patch: &CreditUnitPatch) -> Self {
        Self {
            name: patch.name.clone(),
            plural_name: patch.plural_name.clone(),
            symbol: patch.symbol.clone(),
            archived_at: patch.archived.map(|archived| archived.then(Utc::now)),
            product_id: patch.product_id,
        }
    }
}

/// The rate credits used beyond the balance are billed at, where that usage is invoiced past
/// zero: the customer's agreed price, else the price's own rate, else the unit's list price in the
/// invoice currency. Usage whose access is denied at zero keeps the price's rate.
pub fn overage_rate(
    price_rate: Decimal,
    invoiced_past_zero: bool,
    agreed: Option<Decimal>,
    list: Option<Decimal>,
) -> Decimal {
    if !invoiced_past_zero {
        return price_rate;
    }
    agreed
        .into_iter()
        .chain([price_rate])
        .chain(list)
        .find(|rate| *rate > Decimal::ZERO)
        .unwrap_or(price_rate)
}

/// Money owed for credits used beyond the balance, at the rate of the price they were used under:
/// nothing when the rate is empty (zero), as where access is denied at zero.
pub fn beyond_balance_money_minor(
    credits_minor: i64,
    unit_precision: u8,
    rate: Decimal,
    currency_precision: u8,
) -> StoreResult<i64> {
    if rate.is_zero() && credits_minor >= 0 {
        return Ok(0);
    }
    credits_to_money_minor(credits_minor, unit_precision, rate, currency_precision)
}

/// `credits_minor` of a unit with `unit_precision` decimals at `rate` major units of a currency
/// each, in that currency's minor units: rounded once, halves away from zero. Credit overage and
/// credit sales both price through here.
pub fn credits_to_money_minor(
    credits_minor: i64,
    unit_precision: u8,
    rate: Decimal,
    currency_precision: u8,
) -> StoreResult<i64> {
    if credits_minor < 0 || rate <= Decimal::ZERO {
        return Err(invalid(
            "credits are priced in positive amounts at a positive rate",
        ));
    }
    let exponent = i32::from(currency_precision) - i32::from(unit_precision);
    let scale = Decimal::from(10_i64.pow(exponent.unsigned_abs()));
    let credits = Decimal::from(credits_minor);
    let money = if exponent >= 0 {
        credits.checked_mul(rate).and_then(|v| v.checked_mul(scale))
    } else {
        credits.checked_mul(rate).and_then(|v| v.checked_div(scale))
    };
    money
        .map(|money| {
            money.round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
        })
        .and_then(|money| rust_decimal::prelude::ToPrimitive::to_i64(&money))
        .ok_or_else(|| invalid("the price of these credits exceeds the supported range"))
}

/// What one credit of a unit sells for in one currency, in major units of that currency. Never
/// edited: a new price replaces it, so a sale that pinned it keeps its terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditUnitPrice {
    pub id: CreditUnitPriceId,
    pub credit_unit_id: CreditUnitId,
    pub currency: String,
    pub unit_price: Decimal,
    pub created_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

impl From<CreditUnitPriceRow> for CreditUnitPrice {
    fn from(row: CreditUnitPriceRow) -> Self {
        Self {
            id: row.id,
            credit_unit_id: row.credit_unit_id,
            currency: row.currency,
            unit_price: row.unit_price.normalize(),
            created_at: row.created_at,
            archived_at: row.archived_at,
        }
    }
}

/// Most decimal places a selling price may carry: a credit can be worth a tiny fraction of a cent.
pub const CREDIT_UNIT_PRICE_MAX_SCALE: u32 = 12;

/// Refuses a selling price that is not positive or finer than `CREDIT_UNIT_PRICE_MAX_SCALE`.
pub fn validate_credit_unit_price(unit_price: Decimal) -> StoreResult<Decimal> {
    let unit_price = unit_price.normalize();
    if unit_price <= Decimal::ZERO {
        return Err(invalid("a credit's price must be positive"));
    }
    if unit_price.scale() > CREDIT_UNIT_PRICE_MAX_SCALE {
        return Err(invalid(format!(
            "a credit's price has at most {CREDIT_UNIT_PRICE_MAX_SCALE} decimal places"
        )));
    }
    Ok(unit_price)
}

/// A credit unit frozen where it is used (a pack's fee and line, a grant's funding), so a later
/// rename never changes what was sold. Code and precision are immutable anyway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditUnitRef {
    pub unit_id: CreditUnitId,
    pub code: String,
    pub precision: u8,
    /// The unit's tax product when resolved: a plan's pack of the unit is taxed and booked under
    /// it, as the unit's direct sales are (ADR-0008 §1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_id: Option<ProductId>,
}

/// The unit a price is expressed in: its `pricing` amounts are credits, each worth `rate` major
/// units of the price currency when billed as overage. Versioned with the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceCreditUnit {
    pub unit_id: CreditUnitId,
    pub rate: Decimal,
}

/// The credit units fee resolution may meet, by id: a unit missing from it refuses the fee
/// rather than resolving its credits as money.
pub type CreditUnitsById = std::collections::HashMap<CreditUnitId, CreditUnit>;

/// `PriceCreditUnit` resolved onto a subscription fee.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeCreditUnit {
    pub unit_id: CreditUnitId,
    pub code: String,
    pub precision: u8,
    /// Invoice-currency major units per credit.
    pub rate: Decimal,
}

/// A denomination with its minor-unit exponent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denomination {
    Currency { code: String, precision: u8 },
    Unit(CreditUnit),
}

impl Denomination {
    pub fn code(&self) -> &str {
        match self {
            Self::Currency { code, .. } => code,
            Self::Unit(unit) => unit.code.as_str(),
        }
    }

    pub fn precision(&self) -> u8 {
        match self {
            Self::Currency { precision, .. } => *precision,
            Self::Unit(unit) => unit.precision,
        }
    }

    pub fn credit_unit_id(&self) -> Option<CreditUnitId> {
        match self {
            Self::Currency { .. } => None,
            Self::Unit(unit) => Some(unit.id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant() -> TenantId {
        TenantId::new()
    }

    #[test]
    fn a_new_unit_validates_code_name_and_precision() {
        assert!(CreditUnitNew::new(tenant(), "CREDITS", "Credit", "Credits", Some("C"), 2).is_ok());
        // code too short (an ISO clash is structurally impossible: codes are 4-16 chars)
        assert!(CreditUnitNew::new(tenant(), "USD", "Credit", "Credits", None, 2).is_err());
        // precision above the max
        assert!(CreditUnitNew::new(tenant(), "CREDITS", "Credit", "Credits", None, 7).is_err());
        // blank name
        assert!(CreditUnitNew::new(tenant(), "CREDITS", "  ", "Credits", None, 2).is_err());
    }

    #[test]
    fn a_blank_symbol_normalizes_to_none_but_an_overlong_one_is_refused() {
        let unit =
            CreditUnitNew::new(tenant(), "CREDITS", "Credit", "Credits", Some("  "), 2).unwrap();
        assert_eq!(unit.symbol, None);
        assert!(
            CreditUnitNew::new(
                tenant(),
                "CREDITS",
                "Credit",
                "Credits",
                Some("TOOLONGSYM"),
                2
            )
            .is_err()
        );
    }

    #[test]
    fn a_patch_validates_present_fields_and_leaves_absent_ones_as_none() {
        let id = CreditUnitId::new();
        let patch = CreditUnitPatch::new(tenant(), id, None, None, None, Some(true)).unwrap();
        assert!(patch.name.is_none() && patch.plural_name.is_none() && patch.symbol.is_none());
        let row_patch = CreditUnitRowPatch::from(&patch);
        assert!(row_patch.archived_at.flatten().is_some());

        assert!(
            CreditUnitPatch::new(tenant(), id, Some("  ".to_owned()), None, None, None).is_err()
        );
    }

    #[test]
    fn an_archived_unit_refuses_new_pricing() {
        let archived = CreditUnit {
            id: CreditUnitId::new(),
            tenant_id: tenant(),
            code: CreditUnitCode::new("CREDITS").unwrap(),
            name: "Credit".into(),
            plural_name: "Credits".into(),
            symbol: None,
            precision: 2,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: Some(Utc::now()),
            product_id: None,
        };
        assert!(archived.ensure_active_for_new_pricing().is_err());
        let mut active = archived.clone();
        active.archived_at = None;
        assert!(active.ensure_active_for_new_pricing().is_ok());
    }

    #[test]
    fn a_unit_code_is_never_an_iso_currency() {
        assert_eq!(
            CreditUnitCode::new(" credits ").unwrap().as_str(),
            "CREDITS"
        );
        assert_eq!(
            CreditUnitCode::new("GPU_HOURS").unwrap().as_str(),
            "GPU_HOURS"
        );
        assert_eq!(CreditUnitCode::new("TOK1").unwrap().as_str(), "TOK1");
        for refused in [
            "USD",
            "EUR",
            "CRD",
            "1TOK",
            "_TOK",
            "TOK-EN",
            "TOKÉN",
            "",
            "ABCDEFGHIJKLMNOPQ",
        ] {
            assert!(CreditUnitCode::new(refused).is_err(), "{refused}");
        }
        assert!(CreditUnitCode::new("ABCDEFGHIJKLMNOP").is_ok());
    }

    #[test]
    fn a_denomination_code_is_a_currency_at_three_letters_and_a_unit_otherwise() {
        assert_eq!(
            DenominationCode::parse("usd").unwrap(),
            DenominationCode::Currency("USD".to_owned())
        );
        assert_eq!(
            DenominationCode::parse("Credits").unwrap(),
            DenominationCode::Unit(CreditUnitCode::new("CREDITS").unwrap())
        );
        assert!(DenominationCode::parse("ZZZ").is_err());
        assert!(DenominationCode::parse("US").is_err());
    }

    #[test]
    fn stored_fees_and_prices_read_without_unit_fields_and_write_back_unchanged() {
        use crate::domain::{Pricing, SubscriptionFee};
        let fees = [
            serde_json::json!({"Usage": {
                "metric_id": "00000000-0000-0000-0000-000000000001",
                "model": {"PerUnit": {"rate": "0.01"}}
            }}),
            serde_json::json!({"Capacity": {
                "rate": "50", "included": 1000, "overage_rate": "0.05",
                "metric_id": "00000000-0000-0000-0000-000000000001"
            }}),
            serde_json::json!({"Credit": {
                "rate": "0", "quantity": 1, "credit_amount": "500",
                "expiry": {"unit": "MONTH", "value": 1}
            }}),
        ];
        for stored in fees {
            let fee: SubscriptionFee = serde_json::from_value(stored.clone()).unwrap();
            match &fee {
                SubscriptionFee::Usage { credit_unit, .. }
                | SubscriptionFee::Capacity { credit_unit, .. } => assert!(credit_unit.is_none()),
                SubscriptionFee::Credit {
                    credit_unit,
                    rollover,
                    ..
                } => assert!(credit_unit.is_none() && rollover.is_none()),
                _ => unreachable!(),
            }
            assert_eq!(serde_json::to_value(&fee).unwrap(), stored);
        }

        let stored = serde_json::json!({
            "type": "Credit", "unit_price": "40", "quantity": 1, "credit_amount": "5000"
        });
        let pricing: Pricing = serde_json::from_value(stored.clone()).unwrap();
        let Pricing::Credit {
            credit_unit_id,
            rollover,
            ..
        } = &pricing
        else {
            unreachable!()
        };
        assert!(credit_unit_id.is_none() && rollover.is_none());
        assert_eq!(serde_json::to_value(&pricing).unwrap(), stored);
    }

    #[test]
    fn a_pack_in_a_unit_with_rollover_round_trips() {
        use crate::domain::Pricing;
        let stored = serde_json::json!({
            "type": "Credit", "unit_price": "0", "quantity": 1, "credit_amount": "500",
            "expiry": {"unit": "MONTH", "value": 1},
            "credit_unit_id": "00000000-0000-0000-0000-000000000002",
            "rollover": {"max_amount": "200", "expiry": {"unit": "MONTH", "value": 1}}
        });
        let pricing: Pricing = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(serde_json::to_value(&pricing).unwrap(), stored);

        let grant = crate::domain::credit::GrantRollover {
            max_minor: None,
            expiry: crate::domain::price_components::CreditExpiry::Day(3),
        };
        let json = grant.to_json().unwrap();
        assert_eq!(
            json,
            serde_json::json!({"expiry": {"unit": "DAY", "value": 3}})
        );
        assert_eq!(
            crate::domain::credit::GrantRollover::from_json(&json).unwrap(),
            grant
        );
    }

    #[test]
    fn the_agreed_price_comes_first_then_the_price_rate_then_the_list_price() {
        use rust_decimal_macros::dec;
        let (agreed, list) = (Some(dec!(0.008)), Some(dec!(0.01)));
        assert_eq!(overage_rate(dec!(0.02), true, agreed, list), dec!(0.008));
        assert_eq!(overage_rate(dec!(0.02), true, None, list), dec!(0.02));
        assert_eq!(overage_rate(Decimal::ZERO, true, agreed, list), dec!(0.008));
        assert_eq!(overage_rate(Decimal::ZERO, true, None, list), dec!(0.01));
        assert_eq!(
            overage_rate(Decimal::ZERO, true, Some(Decimal::ZERO), list),
            dec!(0.01)
        );
        assert_eq!(overage_rate(Decimal::ZERO, true, None, None), Decimal::ZERO);
        assert_eq!(
            overage_rate(Decimal::ZERO, false, agreed, list),
            Decimal::ZERO,
            "usage denied at zero keeps the price's empty rate"
        );
        assert_eq!(
            overage_rate(dec!(0.02), false, agreed, list),
            dec!(0.02),
            "and its own rate, whatever the customer agreed"
        );
    }

    #[test]
    fn a_unit_code_deserializes_only_when_valid() {
        let code: CreditUnitCode = serde_json::from_str("\"CREDITS\"").unwrap();
        assert_eq!(serde_json::to_string(&code).unwrap(), "\"CREDITS\"");
        assert!(serde_json::from_str::<CreditUnitCode>("\"USD\"").is_err());
    }
}
