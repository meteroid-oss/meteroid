use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use crate::domain::{SubscriptionFee, SubscriptionFeeInterface};
use crate::errors::StoreErrorReport;
use chrono::NaiveDateTime;
use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SubscriptionAddOn {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub add_on_id: Uuid,
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub created_at: NaiveDateTime,
}

impl SubscriptionFeeInterface for SubscriptionAddOn {
    #[inline]
    fn price_component_id(&self) -> Option<Uuid> {
        None
    }

    #[inline]
    fn product_id(&self) -> Option<Uuid> {
        None
    }

    #[inline]
    fn subscription_id(&self) -> Uuid {
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
}

impl TryInto<SubscriptionAddOn> for SubscriptionAddOnRow {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<SubscriptionAddOn, Self::Error> {
        let decoded_fee: SubscriptionFee = self.fee.try_into()?;

        Ok(SubscriptionAddOn {
            id: self.id,
            subscription_id: self.subscription_id,
            add_on_id: self.add_on_id,
            name: self.name,
            period: self.period.into(),
            fee: decoded_fee,
            created_at: self.created_at,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SubscriptionAddOnNewInternal {
    pub add_on_id: Uuid,
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
}

#[derive(Clone, Debug)]
pub struct SubscriptionAddOnNew {
    pub subscription_id: Uuid,
    pub internal: SubscriptionAddOnNewInternal,
}

impl TryInto<SubscriptionAddOnRowNew> for SubscriptionAddOnNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<SubscriptionAddOnRowNew, Self::Error> {
        let fee = self.internal.fee.try_into()?;

        Ok(SubscriptionAddOnRowNew {
            id: Uuid::now_v7(),
            subscription_id: self.subscription_id,
            add_on_id: self.internal.add_on_id,
            name: self.internal.name,
            period: self.internal.period.into(),
            fee,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SubscriptionAddOnOverride {
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
}

#[derive(Debug, Clone)]
pub struct SubscriptionAddOnParameterization {
    pub initial_slot_count: Option<u32>,
    pub billing_period: Option<BillingPeriodEnum>,
    pub committed_capacity: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum SubscriptionAddOnCustomization {
    Override(SubscriptionAddOnOverride),
    Parameterization(SubscriptionAddOnParameterization),
    None,
}

#[derive(Debug, Clone)]
pub struct CreateSubscriptionAddOn {
    pub add_on_id: Uuid,
    pub customization: SubscriptionAddOnCustomization,
}

#[derive(Debug, Clone)]
pub struct CreateSubscriptionAddOns {
    pub add_ons: Vec<CreateSubscriptionAddOn>,
}
