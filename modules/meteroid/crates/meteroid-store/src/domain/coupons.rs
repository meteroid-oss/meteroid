use crate::errors::StoreError;
use crate::utils::local_id::{IdType, LocalId};
use chrono::NaiveDateTime;
use diesel_models::coupons::{
    CouponFilter as CouponFilterDb, CouponRow, CouponRowNew, CouponRowPatch, CouponStatusRowPatch,
};
use error_stack::Report;
use o2o::o2o;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Coupon {
    pub id: Uuid,
    pub local_id: String,
    pub code: String,
    pub description: String,
    pub tenant_id: Uuid,
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
}

impl Coupon {
    pub fn is_infinite(&self) -> bool {
        self.recurring_value.is_none()
    }

    pub fn is_expired(&self, now: NaiveDateTime) -> bool {
        self.expires_at.map(|x| x <= now).unwrap_or(false)
    }

    pub fn applies_once(&self) -> bool {
        self.redemption_limit.is_some_and(|x| x == 1)
    }

    pub fn currency(&self) -> Option<&str> {
        self.discount.currency()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum CouponDiscount {
    Percentage(Decimal),
    Fixed { currency: String, amount: Decimal },
}

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
        let discount: CouponDiscount = serde_json::from_value(self.discount)
            .map_err(|e| StoreError::SerdeError("coupon discount".to_string(), e))?;

        Ok(Coupon {
            id: self.id,
            local_id: self.local_id,
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
        })
    }
}

#[derive(Debug, Clone)]
pub struct CouponNew {
    pub code: String,
    pub description: String,
    pub tenant_id: Uuid,
    pub discount: CouponDiscount,
    pub expires_at: Option<NaiveDateTime>,
    pub redemption_limit: Option<i32>,
    pub recurring_value: Option<i32>,
    pub reusable: bool,
}

impl TryInto<CouponRowNew> for CouponNew {
    type Error = StoreError;

    fn try_into(self) -> Result<CouponRowNew, StoreError> {
        let json_discount = serde_json::to_value(&self.discount)
            .map_err(|e| StoreError::SerdeError("coupon discount".to_string(), e))?;

        Ok(CouponRowNew {
            id: Uuid::now_v7(),
            local_id: LocalId::generate_for(IdType::Coupon),
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
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub description: Option<String>,
    pub discount: Option<CouponDiscount>,
    pub updated_at: NaiveDateTime,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(CouponStatusRowPatch)]
pub struct CouponStatusPatch {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub archived_at: Option<Option<NaiveDateTime>>,
    pub disabled: Option<bool>,
}

impl TryInto<CouponRowPatch> for CouponPatch {
    type Error = StoreError;

    fn try_into(self) -> Result<CouponRowPatch, StoreError> {
        let json_discount = self
            .discount
            .map(|x| {
                serde_json::to_value(&x)
                    .map_err(|e| StoreError::SerdeError("coupon discount".to_string(), e))
            })
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
