use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use crate::domain::{SubscriptionFee, SubscriptionFeeInterface};
use crate::errors::{StoreError, StoreErrorReport};
use chrono::NaiveDateTime;
use common_domain::ids::{
    AddOnId, BaseId, PriceComponentId, PriceId, ProductId, SubscriptionAddOnId, SubscriptionId,
    SubscriptionPriceComponentId,
};
use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SubscriptionAddOn {
    pub id: SubscriptionAddOnId,
    pub subscription_id: SubscriptionId,
    pub add_on_id: AddOnId,
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub created_at: NaiveDateTime,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
}

impl SubscriptionFeeInterface for SubscriptionAddOn {
    #[inline]
    fn price_component_id(&self) -> Option<PriceComponentId> {
        None
    }

    #[inline]
    fn product_id(&self) -> Option<ProductId> {
        self.product_id
    }

    #[inline]
    fn subscription_id(&self) -> SubscriptionId {
        self.subscription_id
    }

    #[inline]
    fn name_ref(&self) -> &String {
        &self.name
    }

    #[inline]
    fn period_ref(&self) -> &SubscriptionFeeBillingPeriod {
        &self.period
    }

    #[inline]
    fn fee_ref(&self) -> &SubscriptionFee {
        &self.fee
    }

    #[inline]
    fn sub_component_id(&self) -> Option<SubscriptionPriceComponentId> {
        None
    }

    #[inline]
    fn sub_add_on_id(&self) -> Option<SubscriptionAddOnId> {
        Some(self.id)
    }
}

impl TryInto<SubscriptionAddOn> for SubscriptionAddOnRow {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<SubscriptionAddOn, Self::Error> {
        let decoded_fee: SubscriptionFee = self
            .legacy_fee
            .ok_or_else(|| {
                StoreError::InvalidArgument(
                    "subscription_add_on has no legacy_fee (v2 rows are resolved by repository)"
                        .to_string(),
                )
            })?
            .try_into()?;

        Ok(SubscriptionAddOn {
            id: self.id,
            subscription_id: self.subscription_id,
            add_on_id: self.add_on_id,
            name: self.name,
            period: self.period.into(),
            fee: decoded_fee,
            created_at: self.created_at,
            product_id: self.product_id,
            price_id: self.price_id,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SubscriptionAddOnNewInternal {
    pub add_on_id: AddOnId,
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
}

#[derive(Clone, Debug)]
pub struct SubscriptionAddOnNew {
    pub subscription_id: SubscriptionId,
    pub internal: SubscriptionAddOnNewInternal,
}

impl TryInto<SubscriptionAddOnRowNew> for SubscriptionAddOnNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<SubscriptionAddOnRowNew, Self::Error> {
        let legacy_fee: serde_json::Value = self.internal.fee.try_into()?;

        Ok(SubscriptionAddOnRowNew {
            id: SubscriptionAddOnId::new(),
            subscription_id: self.subscription_id,
            add_on_id: self.internal.add_on_id,
            name: self.internal.name,
            period: self.internal.period.into(),
            legacy_fee: Some(legacy_fee),
            product_id: self.internal.product_id,
            price_id: self.internal.price_id,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionAddOnOverride {
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionAddOnParameterization {
    pub initial_slot_count: Option<u32>,
    pub billing_period: Option<BillingPeriodEnum>,
    pub committed_capacity: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubscriptionAddOnCustomization {
    Override(SubscriptionAddOnOverride),
    Parameterization(SubscriptionAddOnParameterization),
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubscriptionAddOn {
    pub add_on_id: AddOnId,
    pub customization: SubscriptionAddOnCustomization,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubscriptionAddOns {
    pub add_ons: Vec<CreateSubscriptionAddOn>,
}
