use meteroid_store::domain;
use rust_decimal::Decimal;
use std::str::FromStr;

use super::model::*;
use crate::errors::RestApiError;

pub fn coupon_to_rest(coupon: domain::coupons::Coupon) -> Coupon {
    Coupon {
        id: coupon.id,
        code: coupon.code,
        description: if coupon.description.is_empty() {
            None
        } else {
            Some(coupon.description)
        },
        discount: coupon_discount_to_rest(&coupon.discount),
        expires_at: coupon.expires_at,
        redemption_limit: coupon.redemption_limit,
        recurring_value: coupon.recurring_value,
        reusable: coupon.reusable,
        disabled: coupon.disabled,
        created_at: coupon.created_at,
        archived_at: coupon.archived_at,
        redemption_count: coupon.redemption_count,
        plan_ids: coupon.plan_ids,
    }
}

fn coupon_discount_to_rest(discount: &domain::coupons::CouponDiscount) -> CouponDiscountRest {
    match discount {
        domain::coupons::CouponDiscount::Percentage(pct) => CouponDiscountRest::Percentage {
            percentage: pct.to_string(),
        },
        domain::coupons::CouponDiscount::Fixed { currency, amount } => CouponDiscountRest::Fixed {
            currency: currency.clone(),
            amount: amount.to_string(),
        },
    }
}

pub fn rest_discount_to_domain(
    discount: &CouponDiscountRest,
) -> Result<domain::coupons::CouponDiscount, RestApiError> {
    match discount {
        CouponDiscountRest::Percentage { percentage } => {
            let pct = Decimal::from_str(percentage)
                .map_err(|_| RestApiError::InvalidInput("Invalid percentage value".to_string()))?;
            if pct <= Decimal::ZERO || pct > Decimal::from(100) {
                return Err(RestApiError::InvalidInput(
                    "Percentage must be between 0 (exclusive) and 100 (inclusive)".to_string(),
                ));
            }
            Ok(domain::coupons::CouponDiscount::Percentage(pct))
        }
        CouponDiscountRest::Fixed { currency, amount } => {
            let amt = Decimal::from_str(amount)
                .map_err(|_| RestApiError::InvalidInput("Invalid amount value".to_string()))?;
            if amt <= Decimal::ZERO {
                return Err(RestApiError::InvalidInput(
                    "Amount must be greater than 0".to_string(),
                ));
            }
            Ok(domain::coupons::CouponDiscount::Fixed {
                currency: currency.clone(),
                amount: amt,
            })
        }
    }
}
