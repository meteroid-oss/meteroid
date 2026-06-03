use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use crate::domain::price_components::PriceEntry;
use crate::domain::{SubscriptionFee, SubscriptionFeeInterface};
use crate::errors::{StoreError, StoreErrorReport};
use chrono::{NaiveDate, NaiveDateTime};
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
    pub quantity: i32,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    /// Lineage root this add-on descends from across overrides. `None` means the
    /// row is its own root.
    pub lineage_id: Option<SubscriptionAddOnId>,
    /// True when this add-on was added by a manual amendment. A one-time fee on such
    /// an add-on is billed on the invoice for the period it becomes effective.
    pub added_by_amendment: bool,
}

impl SubscriptionAddOn {
    /// The lineage root id: the original add-on this one descends from across
    /// overrides, or its own id when it is a root.
    #[inline]
    pub fn lineage(&self) -> SubscriptionAddOnId {
        self.lineage_id.unwrap_or(self.id)
    }
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

    #[inline]
    fn instance_quantity(&self) -> rust_decimal::Decimal {
        rust_decimal::Decimal::from(self.quantity.max(0))
    }

    #[inline]
    fn effective_from(&self) -> Option<NaiveDate> {
        Some(self.effective_from)
    }

    #[inline]
    fn effective_to(&self) -> Option<NaiveDate> {
        self.effective_to
    }

    #[inline]
    fn added_by_amendment(&self) -> bool {
        self.added_by_amendment
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
            quantity: self.quantity,
            effective_from: self.effective_from,
            effective_to: self.effective_to,
            lineage_id: self.lineage_id,
            added_by_amendment: self.added_by_amendment,
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
    pub quantity: i32,
    pub effective_from: NaiveDate,
}

#[derive(Clone, Debug)]
pub struct SubscriptionAddOnNew {
    pub subscription_id: SubscriptionId,
    pub internal: SubscriptionAddOnNewInternal,
}

impl TryInto<SubscriptionAddOnRowNew> for SubscriptionAddOnNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<SubscriptionAddOnRowNew, Self::Error> {
        // Always snapshot the resolved fee into legacy_fee (even for v2 price-backed
        // add-ons), mirroring subscription_component. The active loader still resolves
        // v2 add-ons from their price_id; the snapshot is what lets historical/closed
        // add-on rows be reconstructed for arrears billing after a mid-cycle removal.
        let legacy_fee = Some(self.internal.fee.try_into()?);

        Ok(SubscriptionAddOnRowNew {
            id: SubscriptionAddOnId::new(),
            subscription_id: self.subscription_id,
            add_on_id: self.internal.add_on_id,
            name: self.internal.name,
            period: self.internal.period.into(),
            legacy_fee,
            product_id: self.internal.product_id,
            price_id: self.internal.price_id,
            quantity: self.internal.quantity,
            effective_from: self.internal.effective_from,
            // Default to a root; the amendment override path sets the predecessor's
            // lineage on the row after conversion.
            lineage_id: None,
            // Defaults to false; amendment insert paths flip it on the row after
            // conversion so a one-time fee bills on its effective period.
            added_by_amendment: false,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionAddOnParameterization {
    pub initial_slot_count: Option<u32>,
    pub billing_period: Option<BillingPeriodEnum>,
    pub committed_capacity: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubscriptionAddOnCustomization {
    PriceOverride {
        name: Option<String>,
        price_entry: PriceEntry,
    },
    Parameterization(SubscriptionAddOnParameterization),
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubscriptionAddOn {
    pub add_on_id: AddOnId,
    pub customization: SubscriptionAddOnCustomization,
    pub quantity: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubscriptionAddOns {
    pub add_ons: Vec<CreateSubscriptionAddOn>,
}
