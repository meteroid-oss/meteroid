use crate::domain::CouponLineItem;
use crate::domain::eligibility::Eligibility;
use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use chrono::{Months, NaiveDate, NaiveDateTime};
use common_domain::ids::{AddOnId, BaseId, CouponId, PlanId, TenantId};
use diesel_models::coupons::{
    CouponFilter as CouponFilterDb, CouponRow, CouponRowNew, CouponRowPatch, CouponStatusRowPatch,
};
use error_stack::Report;
use o2o::o2o;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Coupon {
    pub id: CouponId,
    pub code: String,
    pub description: String,
    pub tenant_id: TenantId,
    pub discount: CouponDiscount,
    pub expires_at: Option<NaiveDateTime>,
    pub redemption_limit: Option<i32>, // max number of subscriptions it can be applied to
    /// Invoices it discounts on a subscription: what coupons made before `duration_months` count.
    pub recurring_value: Option<i32>,
    /// Discounts the billing periods starting within this many months of the first paid period;
    /// neither this nor `recurring_value`: forever. A redemption keeps the months it was made with.
    pub duration_months: Option<i32>,
    pub reusable: bool, // can it be applied to multiple subscriptions of the same customer
    pub disabled: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub last_redemption_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub redemption_count: i32,
    pub plan_ids: Vec<PlanId>, // restrict coupon to specific plans (empty = all plans)
    /// Redeemable with these add-ons too; neither these nor `plan_ids`: anything.
    pub add_on_ids: Vec<AddOnId>,
    /// Who may redeem it; `None`: anyone with the code.
    pub eligibility: Option<Eligibility>,
    /// Not redeemable before.
    pub valid_from: Option<NaiveDateTime>,
    /// A fixed discount in currencies other than its own, by ISO 4217 code.
    pub fixed_amounts: BTreeMap<String, Decimal>,
}

/// Validation error for coupon usage
#[derive(Debug, Clone)]
pub enum CouponValidationError {
    NotYetValid,
    Expired,
    Archived,
    Disabled,
    RedemptionLimitReached,
    CurrencyMismatch {
        coupon_currency: String,
        expected: String,
    },
    /// A fixed discount with no amount in the subscription's currency.
    NoAmountIn(String),
}

impl CouponValidationError {
    pub fn to_error_message(&self, coupon_code: &str) -> String {
        match self {
            CouponValidationError::NotYetValid => {
                format!("Coupon '{}' is not valid yet", coupon_code)
            }
            CouponValidationError::Expired => {
                format!("Coupon '{}' has expired", coupon_code)
            }
            CouponValidationError::Archived => {
                format!("Coupon '{}' is no longer available", coupon_code)
            }
            CouponValidationError::Disabled => {
                format!("Coupon '{}' is disabled", coupon_code)
            }
            CouponValidationError::RedemptionLimitReached => {
                format!("Coupon '{}' has reached its redemption limit", coupon_code)
            }
            CouponValidationError::CurrencyMismatch {
                coupon_currency,
                expected,
            } => {
                format!(
                    "Coupon '{}' currency ({}) does not match subscription currency ({})",
                    coupon_code, coupon_currency, expected
                )
            }
            CouponValidationError::NoAmountIn(currency) => {
                format!(
                    "Coupon '{}' has no amount in {}, the subscription's currency",
                    coupon_code, currency
                )
            }
        }
    }
}

/// How long a coupon discounts a subscription it is redeemed on, Stripe's way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CouponDuration {
    Forever,
    /// The billing periods starting within this many months of the first paid period.
    Months(u32),
    /// The first invoices: how coupons made before durations in months count, kept as they are.
    Invoices(u32),
}

impl CouponDuration {
    /// `duration_months` before `recurring_value`; a count below one is none.
    pub fn of(recurring_value: Option<i32>, duration_months: Option<i32>) -> Self {
        let positive = |n: i32| u32::try_from(n).unwrap_or(0);
        match (duration_months, recurring_value) {
            (Some(months), _) => CouponDuration::Months(positive(months)),
            (None, Some(invoices)) => CouponDuration::Invoices(positive(invoices)),
            (None, None) => CouponDuration::Forever,
        }
    }
}

impl Coupon {
    pub fn duration(&self) -> CouponDuration {
        CouponDuration::of(self.recurring_value, self.duration_months)
    }

    pub fn is_infinite(&self) -> bool {
        self.duration() == CouponDuration::Forever
    }

    /// Its months counted from `months_from`, the first paid period's start, the day they end: it
    /// discounts the periods starting before it. None unless it lasts months.
    pub fn lasts_until(&self, months_from: NaiveDate) -> Option<NaiveDate> {
        match self.duration() {
            CouponDuration::Months(months) => Some(
                months_from
                    .checked_add_months(Months::new(months))
                    .unwrap_or(NaiveDate::MAX),
            ),
            CouponDuration::Forever | CouponDuration::Invoices(_) => None,
        }
    }

    pub fn is_expired(&self, now: NaiveDateTime) -> bool {
        self.expires_at.is_some_and(|x| x <= now)
    }

    pub fn currency(&self) -> Option<&str> {
        self.discount.currency()
    }

    pub fn is_valid_yet(&self, now: NaiveDateTime) -> bool {
        self.valid_from.is_none_or(|from| from <= now)
    }

    /// The amount a fixed discount takes off in `currency`, if it has one there.
    pub fn fixed_amount_in(&self, currency: &str) -> Option<Decimal> {
        match &self.discount {
            CouponDiscount::Fixed {
                currency: own,
                amount,
            } if own == currency => Some(*amount),
            CouponDiscount::Fixed { .. } => self.fixed_amounts.get(currency).copied(),
            _ => None,
        }
    }

    /// The discount as a subscription in `currency` gets it: a fixed one in its amount there.
    /// One with no amount there is left as it is, in a currency no invoice of it bills in.
    pub fn discount_in(&self, currency: &str) -> CouponDiscount {
        match (&self.discount, self.fixed_amount_in(currency)) {
            (CouponDiscount::Fixed { .. }, Some(amount)) => CouponDiscount::Fixed {
                currency: currency.to_owned(),
                amount,
            },
            (discount, _) => discount.clone(),
        }
    }

    /// Whether a subscription to `plan_id` with `add_on_ids` may redeem it.
    pub fn applies_to(&self, plan_id: Option<PlanId>, add_on_ids: &[AddOnId]) -> bool {
        if self.plan_ids.is_empty() && self.add_on_ids.is_empty() {
            return true;
        }
        plan_id.is_some_and(|plan_id| self.plan_ids.contains(&plan_id))
            || add_on_ids.iter().any(|id| self.add_on_ids.contains(id))
    }

    /// The add-ons whose lines alone it discounts on a subscription to `plan_id`; `None`: the
    /// whole invoice, as when unscoped, scoped to plans only, or to that plan.
    pub fn discounted_add_ons(&self, plan_id: Option<PlanId>) -> Option<&[AddOnId]> {
        let on_its_plan = plan_id.is_some_and(|plan_id| self.plan_ids.contains(&plan_id));
        (!self.add_on_ids.is_empty() && !on_its_plan).then_some(self.add_on_ids.as_slice())
    }

    /// The coupon as a redemption bills it: the fixed amount and the add-ons it discounts as
    /// they were when it was redeemed (`None` amount: read the coupon; `None` add-ons: all).
    pub fn as_redeemed(
        mut self,
        fixed: Option<(String, Decimal)>,
        add_on_ids: Option<Vec<AddOnId>>,
    ) -> Self {
        if let Some((currency, amount)) = fixed
            && !self.discount.is_credit()
        {
            self.discount = CouponDiscount::Fixed { currency, amount };
            self.fixed_amounts = BTreeMap::new();
        }
        self.plan_ids = Vec::new();
        self.add_on_ids = add_on_ids.unwrap_or_default();
        self
    }

    /// Validates that a coupon can be used for a new subscription.
    /// This performs all standard validation checks:
    /// - Not expired
    /// - Not archived
    /// - Not disabled
    /// - Redemption limit not reached
    /// - Valid already (`valid_from`)
    /// - Currency compatible: a fixed discount has an amount in it, a credit is in it
    ///
    /// Note: This does NOT check the `reusable` flag (whether the same customer
    /// can use the coupon multiple times). That check requires a database lookup
    /// and should be done separately when needed.
    pub fn validate_for_use(
        &self,
        subscription_currency: &str,
    ) -> Result<(), CouponValidationError> {
        self.validate_for_use_at(common_clock::now().naive_utc(), subscription_currency)
    }

    /// `validate_for_use` as of `now`.
    pub fn validate_for_use_at(
        &self,
        now: NaiveDateTime,
        subscription_currency: &str,
    ) -> Result<(), CouponValidationError> {
        if !self.is_valid_yet(now) {
            return Err(CouponValidationError::NotYetValid);
        }

        if self.is_expired(now) {
            return Err(CouponValidationError::Expired);
        }

        if self.archived_at.is_some() {
            return Err(CouponValidationError::Archived);
        }

        if self.disabled {
            return Err(CouponValidationError::Disabled);
        }

        if let Some(limit) = self.redemption_limit
            && self.redemption_count >= limit
        {
            return Err(CouponValidationError::RedemptionLimitReached);
        }

        if matches!(self.discount, CouponDiscount::Fixed { .. }) {
            if self.fixed_amount_in(subscription_currency).is_none() {
                return Err(CouponValidationError::NoAmountIn(
                    subscription_currency.to_string(),
                ));
            }
        } else if let Some(coupon_currency) = self.currency()
            && coupon_currency != subscription_currency
        {
            return Err(CouponValidationError::CurrencyMismatch {
                coupon_currency: coupon_currency.to_string(),
                expected: subscription_currency.to_string(),
            });
        }

        Ok(())
    }

    /// Validates the coupon and returns an error message suitable for API responses.
    pub fn validate_for_use_with_message(&self, subscription_currency: &str) -> Result<(), String> {
        self.validate_for_use(subscription_currency)
            .map_err(|e| e.to_error_message(&self.code))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum CouponDiscount {
    Percentage(Decimal),
    Fixed {
        currency: String,
        amount: Decimal,
    },
    /// Promotional credit funded when the subscription it is redeemed on starts; never an invoice
    /// discount. Redeemed at most once per customer, whatever the coupon's `reusable`.
    Credit {
        currency: String,
        amount: Decimal,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_in_days: Option<u32>,
        #[serde(default)]
        scope: CouponCreditScope,
    },
}

json_value_serde!(CouponDiscount);

/// Which charges a promotional credit coupon's credit pays: any of the customer's, or only those
/// of the subscription it is redeemed on (ending with it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CouponCreditScope {
    #[default]
    Customer,
    Subscription,
}

/// Longest term of promo credit, so its expiry always resolves.
pub const MAX_COUPON_CREDIT_DAYS: u32 = 3_660;

impl CouponDiscount {
    pub fn currency(&self) -> Option<&str> {
        match self {
            CouponDiscount::Fixed { currency, .. } | CouponDiscount::Credit { currency, .. } => {
                Some(currency)
            }
            CouponDiscount::Percentage(_) => None,
        }
    }

    pub fn is_credit(&self) -> bool {
        matches!(self, CouponDiscount::Credit { .. })
    }

    /// The credit a redemption funds, in minor units of its currency.
    pub fn credit_minor(&self) -> Option<i64> {
        let CouponDiscount::Credit {
            currency, amount, ..
        } = self
        else {
            return None;
        };
        let precision = crate::constants::Currencies::resolve_currency_precision(currency)?;
        let minor = amount.checked_mul(Decimal::from(10_i64.checked_pow(u32::from(precision))?))?;
        if minor.fract() != Decimal::ZERO {
            return None;
        }
        i64::try_from(minor).ok()
    }

    /// Percentage and fixed discounts are checked where they are parsed.
    pub fn validate(&self) -> Result<(), StoreError> {
        let invalid = |message: &str| Err(StoreError::InvalidArgument(message.to_owned()));
        match self {
            CouponDiscount::Percentage(_) | CouponDiscount::Fixed { .. } => {}
            CouponDiscount::Credit {
                currency,
                amount,
                expires_in_days,
                ..
            } => {
                if crate::constants::Currencies::resolve_currency_precision(currency).is_none() {
                    return invalid("promotional credit must be in a supported currency");
                }
                if currency.trim() != currency || currency.to_uppercase() != *currency {
                    return invalid(
                        "promotional credit currency must be an uppercase ISO 4217 code",
                    );
                }
                if *amount <= Decimal::ZERO {
                    return invalid("promotional credit must be a positive amount");
                }
                if self.credit_minor().is_none() {
                    return invalid(
                        "promotional credit has more decimal places than its currency allows",
                    );
                }
                if expires_in_days.is_some_and(|days| !(1..=MAX_COUPON_CREDIT_DAYS).contains(&days))
                {
                    return invalid(
                        "promotional credit must expire 1 to 3660 days after redemption",
                    );
                }
            }
        }
        Ok(())
    }
}

impl TryInto<Coupon> for CouponRow {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<Coupon, Self::Error> {
        let discount: CouponDiscount = self.discount.try_into()?;

        Ok(Coupon {
            id: self.id,
            code: self.code,
            description: self.description,
            tenant_id: self.tenant_id,
            discount,
            expires_at: self.expires_at,
            redemption_limit: self.redemption_limit,
            recurring_value: self.recurring_value,
            duration_months: self.duration_months,
            reusable: self.reusable,
            disabled: self.disabled,
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_redemption_at: self.last_redemption_at,
            archived_at: self.archived_at,
            redemption_count: self.redemption_count,
            plan_ids: Vec::new(), // populated separately from coupon_plan table
            add_on_ids: Vec::new(), // populated separately from coupon_add_on table
            eligibility: Eligibility::from_row(self.eligibility),
            valid_from: self.valid_from,
            fixed_amounts: fixed_amounts_from_row(self.fixed_amounts)?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CouponNew {
    pub code: String,
    pub description: String,
    pub tenant_id: TenantId,
    pub discount: CouponDiscount,
    pub expires_at: Option<NaiveDateTime>,
    pub redemption_limit: Option<i32>,
    pub recurring_value: Option<i32>,
    pub reusable: bool,
    pub plan_ids: Vec<PlanId>,
}

impl TryInto<CouponRowNew> for CouponNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<CouponRowNew, Self::Error> {
        let json_discount: serde_json::Value = (&self.discount).try_into()?;

        Ok(CouponRowNew {
            id: CouponId::new(),
            code: self.code,
            description: self.description,
            tenant_id: self.tenant_id,
            discount: json_discount,
            expires_at: self.expires_at,
            redemption_limit: self.redemption_limit,
            recurring_value: self.recurring_value,
            reusable: self.reusable,
            eligibility: None,
            valid_from: None,
            fixed_amounts: None,
            duration_months: None,
        })
    }
}

/// What a coupon is, beyond `CouponNew`: who may redeem it, from when, with what add-ons, a
/// fixed discount's amounts in other currencies, and the months it lasts (instead of invoices).
#[derive(Debug, Clone, Default)]
pub struct CouponTerms {
    pub add_on_ids: Vec<AddOnId>,
    pub eligibility: Option<Eligibility>,
    pub valid_from: Option<NaiveDateTime>,
    pub fixed_amounts: BTreeMap<String, Decimal>,
    pub duration_months: Option<i32>,
}

/// Longest a coupon lasts in months: ten years.
pub const MAX_COUPON_DURATION_MONTHS: i32 = 120;

/// Refuses a duration in months below one or past ten years, or one beside a count of invoices.
pub(crate) fn validate_duration_months(
    duration_months: Option<i32>,
    recurring_value: Option<i32>,
) -> Result<(), StoreError> {
    let Some(months) = duration_months else {
        return Ok(());
    };
    let invalid = |message: &str| Err(StoreError::InvalidArgument(message.to_owned()));
    if recurring_value.is_some() {
        return invalid("a coupon lasts either a number of months or of invoices, not both");
    }
    if !(1..=MAX_COUPON_DURATION_MONTHS).contains(&months) {
        return invalid("a coupon lasts 1 to 120 months");
    }
    Ok(())
}

fn fixed_amounts_from_row(
    value: Option<serde_json::Value>,
) -> Result<BTreeMap<String, Decimal>, Report<StoreError>> {
    match value.filter(|v| !v.is_null()) {
        None => Ok(BTreeMap::new()),
        Some(value) => serde_json::from_value(value).map_err(|e| {
            Report::new(StoreError::SerdeError(
                "Failed to deserialize coupon fixed amounts".to_owned(),
                e,
            ))
        }),
    }
}

pub(crate) fn fixed_amounts_to_row(
    amounts: &BTreeMap<String, Decimal>,
) -> Result<Option<serde_json::Value>, Report<StoreError>> {
    if amounts.is_empty() {
        return Ok(None);
    }
    serde_json::to_value(amounts).map(Some).map_err(|e| {
        Report::new(StoreError::SerdeError(
            "Failed to serialize coupon fixed amounts".to_owned(),
            e,
        ))
    })
}

/// Refuses amounts in other currencies but on a fixed discount, or not each a positive amount of
/// a supported currency other than the discount's own.
pub(crate) fn validate_fixed_amounts(
    discount: &CouponDiscount,
    amounts: &BTreeMap<String, Decimal>,
) -> Result<(), StoreError> {
    let invalid = |message: String| Err(StoreError::InvalidArgument(message));
    if amounts.is_empty() {
        return Ok(());
    }
    let CouponDiscount::Fixed { currency: own, .. } = discount else {
        return invalid("only a fixed discount has amounts in other currencies".to_owned());
    };
    for (currency, amount) in amounts {
        let Some(precision) = crate::constants::Currencies::resolve_currency_precision(currency)
        else {
            return invalid(format!("{currency} is not a supported currency"));
        };
        if currency.to_uppercase() != *currency || currency.trim() != currency {
            return invalid(format!("{currency} must be an uppercase ISO 4217 code"));
        }
        if currency == own {
            return invalid(format!("the discount is already in {currency}"));
        }
        if *amount <= Decimal::ZERO {
            return invalid(format!("the amount in {currency} must be positive"));
        }
        if amount.normalize().scale() > u32::from(precision) {
            return invalid(format!(
                "the amount in {currency} has more decimal places than {currency} allows"
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct CouponPatch {
    pub id: CouponId,
    pub tenant_id: TenantId,
    pub description: Option<String>,
    pub discount: Option<CouponDiscount>,
    pub plan_ids: Option<Vec<PlanId>>,
    /// A field left `None` stays.
    pub add_on_ids: Option<Vec<AddOnId>>,
    pub eligibility: Option<Option<Eligibility>>,
    pub valid_from: Option<Option<NaiveDateTime>>,
    pub expires_at: Option<Option<NaiveDateTime>>,
    pub fixed_amounts: Option<BTreeMap<String, Decimal>>,
    /// `Some(None)`: forever; `Some(Some(n))`: n months. Past redemptions keep theirs.
    pub duration_months: Option<Option<i32>>,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(CouponStatusRowPatch)]
pub struct CouponStatusPatch {
    pub id: CouponId,
    pub tenant_id: TenantId,
    pub archived_at: Option<Option<NaiveDateTime>>,
    pub disabled: Option<bool>,
}

impl TryInto<CouponRowPatch> for CouponPatch {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<CouponRowPatch, Self::Error> {
        let json_discount = self
            .discount
            .map(std::convert::TryInto::try_into)
            .transpose()?;

        Ok(CouponRowPatch {
            id: self.id,
            tenant_id: self.tenant_id,
            description: self.description,
            discount: json_discount,
            updated_at: common_clock::now().naive_utc(),
            eligibility: self
                .eligibility
                .map(|rule| Eligibility::to_row(rule.as_ref())),
            valid_from: self.valid_from,
            expires_at: self.expires_at,
            fixed_amounts: self
                .fixed_amounts
                .as_ref()
                .map(fixed_amounts_to_row)
                .transpose()?,
            duration_months: self.duration_months,
        })
    }
}

#[derive(Debug, Clone, o2o)]
#[owned_into(CouponFilterDb)]
pub enum CouponFilter {
    ALL,
    ACTIVE,
    INACTIVE,
    ARCHIVED,
}

/// A started subscription a promo code applying to one subscription may be redeemed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedeemableSubscription {
    pub id: common_domain::ids::SubscriptionId,
    pub plan_name: String,
    pub start_date: chrono::NaiveDate,
}

/// Attached to the refusal of a code that applies to one subscription when several qualify and
/// none was named: the subscriptions it may be redeemed on.
#[derive(Debug, Clone)]
pub struct SubscriptionRequired(pub Vec<RedeemableSubscription>);

/// The credit one redemption of a promotional credit coupon added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CouponCreditRedemption {
    pub applied_coupon_id: common_domain::ids::AppliedCouponId,
    pub amount_minor: i64,
    pub currency: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct AppliedCouponsDiscount {
    pub discount_subunit: i64,
    pub applied_coupons: Vec<CouponLineItem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coupon(discount: CouponDiscount) -> Coupon {
        let now = common_clock::now().naive_utc();
        Coupon {
            id: CouponId::new(),
            code: "SUPPORT50".to_owned(),
            description: String::new(),
            tenant_id: TenantId::new(),
            discount,
            expires_at: None,
            redemption_limit: None,
            recurring_value: None,
            duration_months: None,
            reusable: true,
            disabled: false,
            created_at: now,
            updated_at: now,
            last_redemption_at: None,
            archived_at: None,
            redemption_count: 0,
            plan_ids: vec![],
            add_on_ids: vec![],
            eligibility: None,
            valid_from: None,
            fixed_amounts: BTreeMap::new(),
        }
    }

    /// Scoped to add-ons, only their lines; to plans, the whole invoice; to both, the whole
    /// invoice on one of its plans, else only the add-ons' lines.
    #[test]
    fn a_coupon_scoped_to_add_ons_discounts_their_lines_alone_off_its_plans() {
        let (pro, starter, support) = (PlanId::new(), PlanId::new(), AddOnId::new());
        let mut on_support = coupon(CouponDiscount::Percentage(Decimal::from(50)));
        on_support.add_on_ids = vec![support];
        assert_eq!(
            on_support.discounted_add_ons(Some(pro)),
            Some([support].as_slice())
        );

        let mut on_pro = coupon(CouponDiscount::Percentage(Decimal::from(50)));
        on_pro.plan_ids = vec![pro];
        assert_eq!(on_pro.discounted_add_ons(Some(starter)), None);

        let mut both = on_support.clone();
        both.plan_ids = vec![pro];
        assert_eq!(both.discounted_add_ons(Some(pro)), None);
        assert_eq!(
            both.discounted_add_ons(Some(starter)),
            Some([support].as_slice())
        );
        assert_eq!(
            coupon(CouponDiscount::Percentage(Decimal::ONE)).discounted_add_ons(None),
            None
        );
    }

    /// Months before invoices: a coupon made with months never counts invoices; one made before
    /// durations in months keeps counting them.
    #[test]
    fn a_coupon_lasts_months_invoices_or_forever() {
        assert_eq!(
            CouponDuration::of(None, Some(12)),
            CouponDuration::Months(12)
        );
        assert_eq!(
            CouponDuration::of(Some(3), None),
            CouponDuration::Invoices(3)
        );
        assert_eq!(CouponDuration::of(None, None), CouponDuration::Forever);
        assert!(validate_duration_months(Some(12), None).is_ok());
        assert!(validate_duration_months(Some(12), Some(1)).is_err());
        assert!(validate_duration_months(Some(0), None).is_err());
        assert!(validate_duration_months(Some(121), None).is_err());

        let mut year = coupon(CouponDiscount::Percentage(Decimal::from(50)));
        year.duration_months = Some(12);
        let redeemed = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        assert_eq!(
            year.lasts_until(redeemed),
            NaiveDate::from_ymd_opt(2027, 1, 31)
        );
        assert!(!year.is_infinite());
    }

    /// Redeemed in EUR at 9.00: its amount there removed or changed later, it still bills 9.00.
    #[test]
    fn a_redemption_bills_the_fixed_amount_it_was_granted() {
        let edited = coupon(CouponDiscount::Fixed {
            currency: "USD".to_owned(),
            amount: Decimal::from(10),
        });
        assert_eq!(edited.fixed_amount_in("EUR"), None);
        let redeemed = edited.as_redeemed(Some(("EUR".to_owned(), Decimal::from(9))), None);
        assert_eq!(
            redeemed.discount_in("EUR"),
            CouponDiscount::Fixed {
                currency: "EUR".to_owned(),
                amount: Decimal::from(9),
            }
        );

        let percentage = coupon(CouponDiscount::Percentage(Decimal::from(20)));
        let support = AddOnId::new();
        let redeemed = percentage.as_redeemed(None, Some(vec![support]));
        assert_eq!(
            redeemed.discount,
            CouponDiscount::Percentage(Decimal::from(20))
        );
        assert_eq!(
            redeemed.discounted_add_ons(Some(PlanId::new())),
            Some([support].as_slice())
        );
    }
}
