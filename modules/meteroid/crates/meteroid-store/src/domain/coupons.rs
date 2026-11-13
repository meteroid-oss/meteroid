use crate::domain::CouponLineItem;
use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, CouponId, PlanId, TenantId};
use diesel_models::coupons::{
    CouponFilter as CouponFilterDb, CouponRow, CouponRowNew, CouponRowPatch, CouponStatusRowPatch,
};
use error_stack::Report;
use o2o::o2o;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Coupon {
    pub id: CouponId,
    pub code: String,
    pub description: String,
    pub tenant_id: TenantId,
    pub discount: CouponDiscount,
    pub expires_at: Option<NaiveDateTime>,
    pub redemption_limit: Option<i32>, // max number of subscriptions it can be applied to
    pub recurring_value: Option<i32>, // max number of times can be applied on recurring invoices for a single subscription.
    pub reusable: bool, // can it be applied to multiple subscriptions of the same customer
    pub disabled: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub last_redemption_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub redemption_count: i32,
    pub plan_ids: Vec<PlanId>, // restrict coupon to specific plans (empty = all plans)
}

/// Validation error for coupon usage
#[derive(Debug, Clone)]
pub enum CouponValidationError {
    Expired,
    Archived,
    Disabled,
    RedemptionLimitReached,
    CurrencyMismatch {
        coupon_currency: String,
        expected: String,
    },
}

impl CouponValidationError {
    pub fn to_error_message(&self, coupon_code: &str) -> String {
        match self {
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
        }
    }
}

impl Coupon {
    pub fn is_infinite(&self) -> bool {
        self.recurring_value.is_none()
    }

    pub fn is_expired(&self, now: NaiveDateTime) -> bool {
        self.expires_at.is_some_and(|x| x <= now)
    }

    pub fn applies_once(&self) -> bool {
        self.redemption_limit.is_some_and(|x| x == 1)
    }

    pub fn currency(&self) -> Option<&str> {
        self.discount.currency()
    }

    /// Validates that a coupon can be used for a new subscription.
    /// This performs all standard validation checks:
    /// - Not expired
    /// - Not archived
    /// - Not disabled
    /// - Redemption limit not reached
    /// - Currency compatible (if coupon has fixed currency)
    ///
    /// Note: This does NOT check the `reusable` flag (whether the same customer
    /// can use the coupon multiple times). That check requires a database lookup
    /// and should be done separately when needed.
    pub fn validate_for_use(
        &self,
        subscription_currency: &str,
    ) -> Result<(), CouponValidationError> {
        let now = common_domain::clock::now().naive_utc();

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

        if let Some(coupon_currency) = self.currency()
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
            reusable: self.reusable,
            disabled: self.disabled,
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_redemption_at: self.last_redemption_at,
            archived_at: self.archived_at,
            redemption_count: self.redemption_count,
            plan_ids: Vec::new(), // populated separately from coupon_plan table
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
        })
    }
}

#[derive(Debug, Clone)]
pub struct CouponPatch {
    pub id: CouponId,
    pub tenant_id: TenantId,
    pub description: Option<String>,
    pub discount: Option<CouponDiscount>,
    pub plan_ids: Option<Vec<PlanId>>,
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
            updated_at: common_domain::clock::now().naive_utc(),
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
