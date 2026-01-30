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
        let now = chrono::Utc::now().naive_utc();

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
    Fixed { currency: String, amount: Decimal },
}

json_value_serde!(CouponDiscount);

impl CouponDiscount {
    pub fn currency(&self) -> Option<&str> {
        match self {
            CouponDiscount::Fixed { currency, .. } => Some(currency),
            _ => None,
        }
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
    pub updated_at: NaiveDateTime,
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
            updated_at: self.updated_at,
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

pub struct AppliedCouponsDiscount {
    pub discount_subunit: i64,
    pub applied_coupons: Vec<CouponLineItem>,
}
