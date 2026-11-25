use super::enums::{BillingPeriodEnum, BillingType, SubscriptionFeeBillingPeriod};
use crate::domain::UsagePricingModel;
use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use common_domain::ids::{
    BaseId, BillableMetricId, PriceComponentId, PriceId, ProductId, SubscriptionAddOnId,
    SubscriptionId, SubscriptionPriceComponentId,
};
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use serde::{Deserialize, Serialize};

pub trait SubscriptionFeeInterface {
    fn price_component_id(&self) -> Option<PriceComponentId>;
    fn product_id(&self) -> Option<ProductId>;
    fn subscription_id(&self) -> SubscriptionId;
    fn name_ref(&self) -> &String;
    fn period_ref(&self) -> &SubscriptionFeeBillingPeriod;
    fn fee_ref(&self) -> &SubscriptionFee;
    fn sub_component_id(&self) -> Option<SubscriptionPriceComponentId>;
    fn sub_add_on_id(&self) -> Option<SubscriptionAddOnId>;
    /// Temporal bounds for usage-based billing split. Returns None for non-temporal types.
    fn effective_from(&self) -> Option<chrono::NaiveDate> {
        None
    }
    fn effective_to(&self) -> Option<chrono::NaiveDate> {
        None
    }
    /// Number of instances this record bills. Add-ons store a per-unit fee with
    /// the instance count separately; fixed fees bill for all instances by
    /// carrying this count on the line's quantity (so the line reads N × unit
    /// price, not 1 × N·price). Defaults to 1 for plan components.
    fn instance_quantity(&self) -> rust_decimal::Decimal {
        rust_decimal::Decimal::ONE
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SubscriptionComponent {
    pub id: SubscriptionPriceComponentId,
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub subscription_id: SubscriptionId,
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub price_id: Option<PriceId>,
    pub effective_from: chrono::NaiveDate,
    pub effective_to: Option<chrono::NaiveDate>,
    /// Lineage root this component descends from across overrides. `None` means the
    /// row is its own root.
    pub lineage_id: Option<SubscriptionPriceComponentId>,
    /// True when this component was added by a manual amendment. A one-time fee on
    /// such a component is billed on the invoice for the period it becomes effective.
    pub added_by_amendment: bool,
}

impl SubscriptionComponent {
    /// The lineage root id: the original component this one descends from across
    /// overrides, or its own id when it is a root.
    #[inline]
    pub fn lineage(&self) -> SubscriptionPriceComponentId {
        self.lineage_id.unwrap_or(self.id)
    }
}

impl SubscriptionFeeInterface for SubscriptionComponent {
    #[inline]
    fn price_component_id(&self) -> Option<PriceComponentId> {
        self.price_component_id
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
        Some(self.id)
    }

    #[inline]
    fn sub_add_on_id(&self) -> Option<SubscriptionAddOnId> {
        None
    }

    #[inline]
    fn effective_from(&self) -> Option<chrono::NaiveDate> {
        Some(self.effective_from)
    }

    #[inline]
    fn effective_to(&self) -> Option<chrono::NaiveDate> {
        self.effective_to
    }
}

impl TryInto<SubscriptionComponent> for SubscriptionComponentRow {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<SubscriptionComponent, Self::Error> {
        let decoded_fee: SubscriptionFee = self
            .legacy_fee
            .ok_or_else(|| {
                StoreError::InvalidArgument(
                    "subscription_component has no legacy_fee (v2 rows are resolved by repository)"
                        .to_string(),
                )
            })?
            .try_into()?;

        Ok(SubscriptionComponent {
            id: self.id,
            price_component_id: self.price_component_id,
            product_id: self.product_id,
            subscription_id: self.subscription_id,
            name: self.name,
            period: self.period.into(),
            fee: decoded_fee,
            price_id: self.price_id,
            effective_from: self.effective_from,
            effective_to: self.effective_to,
            lineage_id: self.lineage_id,
            added_by_amendment: self.added_by_amendment,
        })
    }
}

impl SubscriptionComponent {
    pub fn metric_id(&self) -> Option<BillableMetricId> {
        self.fee.metric_id()
    }

    pub fn is_standard(&self) -> bool {
        self.fee.is_standard()
    }
}

#[derive(Debug)]
pub struct SubscriptionComponentNew {
    pub subscription_id: SubscriptionId,
    pub internal: SubscriptionComponentNewInternal,
}

impl TryInto<SubscriptionComponentRowNew> for SubscriptionComponentNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<SubscriptionComponentRowNew, Self::Error> {
        let legacy_fee: serde_json::Value = self.internal.fee.try_into()?;

        Ok(SubscriptionComponentRowNew {
            id: SubscriptionPriceComponentId::new(),
            subscription_id: self.subscription_id,
            price_component_id: self.internal.price_component_id,
            product_id: self.internal.product_id,
            name: self.internal.name,
            period: self.internal.period.into(),
            legacy_fee: Some(legacy_fee),
            price_id: self.internal.price_id,
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
pub struct CreateSubscriptionComponents {
    pub parameterized_components: Vec<ComponentParameterization>,
    pub overridden_components: Vec<ComponentOverride>,
    pub extra_components: Vec<ExtraComponent>,
    pub remove_components: Vec<PriceComponentId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentParameterization {
    pub component_id: PriceComponentId,
    pub parameters: ComponentParameters,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentParameters {
    pub initial_slot_count: Option<u32>,
    pub billing_period: Option<BillingPeriodEnum>,
    pub committed_capacity: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentOverride {
    pub component_id: PriceComponentId,
    pub name: String,
    pub price_entry: crate::domain::price_components::PriceEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraComponent {
    pub name: String,
    pub product_ref: crate::domain::price_components::ProductRef,
    pub price_entry: crate::domain::price_components::PriceEntry,
    /// How the charge bills the product; None takes a new product's, else the structure of the
    /// charge an existing price is linked to (`charge_structure_of_extra`).
    #[serde(default)]
    pub fee_structure: Option<crate::domain::prices::FeeStructure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionComponentNewInternal {
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub is_override: bool,
    pub price_id: Option<PriceId>,
    pub effective_from: chrono::NaiveDate,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum SubscriptionFee {
    Rate {
        rate: rust_decimal::Decimal,
    },
    OneTime {
        rate: rust_decimal::Decimal,
        quantity: u32,
        /// Prices `quantity` instead of `rate`; see `fixed_quantity_amount`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<crate::domain::price_components::RatingModel>,
    },
    /// A credit pack: charged like an advance fee (`rate` × `quantity`), it funds
    /// `credit_amount` × `quantity` of invoice credit once paid. See `FeeType::Credit`.
    Credit {
        rate: rust_decimal::Decimal,
        quantity: u32,
        credit_amount: rust_decimal::Decimal,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expiry: Option<crate::domain::price_components::CreditExpiry>,
        /// The unit `credit_amount` is in; absent is the subscription currency.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credit_unit: Option<crate::domain::credit_units::CreditUnitRef>,
        /// Only on included credit (zero price).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rollover: Option<crate::domain::price_components::CreditRollover>,
        /// Granted once per subscription (R16).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        once: bool,
    },
    Recurring {
        rate: rust_decimal::Decimal,
        quantity: u32,
        billing_type: BillingType,
        /// Prices `quantity` instead of `rate`; see `fixed_quantity_amount`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<crate::domain::price_components::RatingModel>,
    },
    Capacity {
        rate: rust_decimal::Decimal,
        included: u64,
        overage_rate: rust_decimal::Decimal,
        metric_id: BillableMetricId,
        /// Set when `overage_rate` is in credits.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credit_unit: Option<crate::domain::credit_units::FeeCreditUnit>,
    },
    Slot {
        /// The seats' label.
        unit: String,
        /// The managed quantity the seats count. A fee recorded before quantities were named
        /// has none: its seats count the quantity of its tenant coded as `unit` (I32), which
        /// `name_seat_quantities` fills in when it is loaded.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quantity_id: Option<common_domain::ids::ManagedQuantityId>,
        #[serde(
            alias = "unit_rate",
            deserialize_with = "crate::domain::price_components::RatingModel::rate_or_model"
        )]
        model: crate::domain::price_components::RatingModel,
        min_slots: Option<u32>,
        max_slots: Option<u32>,
        initial_slots: u32,
        // upgrade downgrade policies TODO
    },
    Usage {
        metric_id: BillableMetricId,
        model: UsagePricingModel,
        /// Set when `model` amounts are in credits.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credit_unit: Option<crate::domain::credit_units::FeeCreditUnit>,
    },
}

json_value_serde!(SubscriptionFee);

// Stored on every subscription component: each shape ever written must still read.
golden::golden!(SubscriptionFee, {
    "rate" => SubscriptionFee::Rate {
        rate: rust_decimal::Decimal::new(100, 2),
    },
    "one_time" => SubscriptionFee::OneTime {
        rate: rust_decimal::Decimal::new(100, 2),
        quantity: 1,
        model: None,
    },
    "credit" => SubscriptionFee::Credit {
        rate: rust_decimal::Decimal::new(100, 2),
        quantity: 1,
        credit_amount: rust_decimal::Decimal::new(1000, 2),
        expiry: None,
        credit_unit: None,
        rollover: None,
        once: false,
    },
    "recurring" => SubscriptionFee::Recurring {
        rate: rust_decimal::Decimal::new(100, 2),
        quantity: 1,
        billing_type: BillingType::Advance,
        model: None,
    },
    "capacity" => SubscriptionFee::Capacity {
        rate: rust_decimal::Decimal::new(100, 2),
        included: 100,
        overage_rate: rust_decimal::Decimal::new(10, 2),
        metric_id: uuid::Uuid::nil().into(),
        credit_unit: None,
    },
    "slot" => SubscriptionFee::Slot {
        unit: "seat".to_string(),
        quantity_id: None,
        model: crate::domain::price_components::RatingModel::PerUnit {
            rate: rust_decimal::Decimal::new(1000, 2),
        },
        min_slots: None,
        max_slots: None,
        initial_slots: 3,
    },
    "usage" => SubscriptionFee::Usage {
        metric_id: uuid::Uuid::nil().into(),
        model: UsagePricingModel::PerUnit {
            rate: rust_decimal::Decimal::new(100, 2),
        },
        credit_unit: None,
    },
});

impl SubscriptionFee {
    /// Whether the fee can bill anything, whatever the usage or the seats bought.
    pub fn charges(&self) -> bool {
        use crate::domain::price_components::fixed_quantity_charges;
        let positive = |amount: &rust_decimal::Decimal| *amount > rust_decimal::Decimal::ZERO;
        match self {
            SubscriptionFee::Rate { rate } => positive(rate),
            SubscriptionFee::OneTime {
                rate,
                quantity,
                model,
            }
            | SubscriptionFee::Recurring {
                rate,
                quantity,
                model,
                ..
            } => fixed_quantity_charges(*rate, *quantity, model.as_ref()),
            SubscriptionFee::Credit { rate, quantity, .. } => positive(rate) && *quantity > 0,
            SubscriptionFee::Capacity {
                rate, overage_rate, ..
            } => positive(rate) || positive(overage_rate),
            SubscriptionFee::Slot { model, .. } => model.charges(),
            SubscriptionFee::Usage { model, .. } => model.charges(),
        }
    }

    /// Whether the fee sells credit for a price, as opposed to including it at no charge.
    pub fn sells_credit(&self) -> bool {
        matches!(self, SubscriptionFee::Credit { rate, .. } if *rate > rust_decimal::Decimal::ZERO)
    }

    /// Marks the fee's usage amounts as credits of `unit`: all of a usage fee, the overage of a
    /// capacity fee. Refused on any other fee.
    pub fn price_in_credit_unit(
        &mut self,
        unit: crate::domain::credit_units::FeeCreditUnit,
    ) -> Result<(), crate::errors::StoreError> {
        match self {
            SubscriptionFee::Usage { credit_unit, .. }
            | SubscriptionFee::Capacity { credit_unit, .. } => {
                *credit_unit = Some(unit);
                Ok(())
            }
            _ => Err(crate::errors::StoreError::InvalidArgument(
                "only usage and capacity overage can be priced in a credit unit".to_owned(),
            )),
        }
    }

    /// Apply subscription-level parameters to an already-resolved fee.
    /// Used when an override provides the pricing and a parameterization provides
    /// runtime values like initial_slot_count.
    pub fn apply_parameters(&mut self, params: &ComponentParameters) {
        if let SubscriptionFee::Slot { initial_slots, .. } = self
            && let Some(count) = params.initial_slot_count
        {
            *initial_slots = count;
        }
    }

    /// Re-deriving a fee from its price loses the seat count, which only the snapshot recorded.
    pub fn keep_slots_from(&mut self, snapshot: &SubscriptionFee) {
        if let (
            SubscriptionFee::Slot { initial_slots, .. },
            SubscriptionFee::Slot {
                initial_slots: recorded,
                ..
            },
        ) = (self, snapshot)
        {
            *initial_slots = *recorded;
        }
    }

    pub fn metric_id(&self) -> Option<BillableMetricId> {
        match self {
            SubscriptionFee::Usage { metric_id, .. } => Some(*metric_id),
            SubscriptionFee::Capacity { metric_id, .. } => Some(*metric_id),
            _ => None,
        }
    }

    /// The managed quantity a seat fee counts; None for another fee, or a seat fee recorded
    /// before quantities were named and not loaded through `name_seat_quantities`.
    pub fn seat_quantity(&self) -> Option<common_domain::ids::ManagedQuantityId> {
        match self {
            SubscriptionFee::Slot { quantity_id, .. } => *quantity_id,
            _ => None,
        }
    }

    /// Revenue taxonomy for analytics. Capacity bills a recurring base plus metered
    /// overage; it carries a metric so it classifies as usage, matching the
    /// projector's metric-driven usage recognition.
    pub fn charge_type(&self) -> crate::domain::ChargeType {
        use crate::domain::ChargeType;
        match self {
            SubscriptionFee::Usage { .. } | SubscriptionFee::Capacity { .. } => ChargeType::Usage,
            SubscriptionFee::OneTime { .. } | SubscriptionFee::Credit { .. } => ChargeType::OneTime,
            SubscriptionFee::Rate { .. }
            | SubscriptionFee::Recurring { .. }
            | SubscriptionFee::Slot { .. } => ChargeType::Recurring,
        }
    }

    /**
     * Returns true if the component is Rate/Slot/Capacity, false otherwise.
     */
    pub fn is_standard(&self) -> bool {
        match self {
            SubscriptionFee::Rate { .. }
            | SubscriptionFee::Slot { .. }
            | SubscriptionFee::Capacity { .. } => true,
            SubscriptionFee::OneTime { .. }
            | SubscriptionFee::Credit { .. }
            | SubscriptionFee::Recurring { .. }
            | SubscriptionFee::Usage { .. } => false,
        }
    }

    /// Whether attaching this fee bills an amount up front (an advance or
    /// one-time charge that must be collected now). Arrears/usage fees and
    /// zero-rate fees do not — they can be attached without a checkout.
    pub fn has_upfront_charge(&self) -> bool {
        use rust_decimal::Decimal;
        match self {
            SubscriptionFee::Rate { rate } => *rate > Decimal::ZERO,
            SubscriptionFee::OneTime {
                rate,
                quantity,
                model,
            } => crate::domain::price_components::fixed_quantity_charges(
                *rate,
                *quantity,
                model.as_ref(),
            ),
            SubscriptionFee::Credit { rate, quantity, .. } => {
                *rate > Decimal::ZERO && *quantity > 0
            }
            SubscriptionFee::Recurring {
                rate,
                quantity,
                billing_type,
                model,
            } => {
                matches!(billing_type, BillingType::Advance)
                    && crate::domain::price_components::fixed_quantity_charges(
                        *rate,
                        *quantity,
                        model.as_ref(),
                    )
            }
            SubscriptionFee::Capacity { rate, .. } => *rate > Decimal::ZERO,
            SubscriptionFee::Slot {
                model,
                initial_slots,
                ..
            } => model.charges() && *initial_slots > 0,
            SubscriptionFee::Usage { .. } => false,
        }
    }

    /// Returns true if this fee type produces ONLY arrears-billed line items.
    /// Used to filter historical (closed) components for usage temporal split.
    /// Excludes Capacity because it also produces an advance-billed base rate line.
    pub fn is_pure_arrears(&self) -> bool {
        matches!(
            self,
            SubscriptionFee::Usage { .. }
                | SubscriptionFee::Recurring {
                    billing_type: BillingType::Arrears,
                    ..
                }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::price_components::RatingModel;
    use rust_decimal_macros::dec;

    /// A seat fee as stored before rating models, a bare `unit_rate`, reads as per unit.
    #[test]
    fn a_seat_fee_stored_with_a_unit_rate_reads_as_per_unit() {
        let fee: SubscriptionFee = serde_json::from_str(
            r#"{"Slot":{"unit":"seat","unit_rate":"10.00","min_slots":null,"max_slots":null,"initial_slots":3}}"#,
        )
        .unwrap();
        let SubscriptionFee::Slot {
            unit,
            quantity_id,
            model,
            min_slots,
            max_slots,
            initial_slots,
        } = fee
        else {
            panic!("a seat fee");
        };
        assert_eq!(unit, "seat");
        assert_eq!(quantity_id, None);
        assert!(matches!(model, RatingModel::PerUnit { rate } if rate == dec!(10.00)));
        assert_eq!((min_slots, max_slots, initial_slots), (None, None, 3));
    }

    /// A seat fee naming its quantity keeps it through storage; one stored before reads none.
    /// Rules: R-137
    #[test]
    fn a_seat_fee_keeps_the_quantity_it_counts() {
        let quantity = common_domain::ids::ManagedQuantityId::from(uuid::Uuid::from_u128(7));
        let fee = SubscriptionFee::Slot {
            unit: "editor".to_string(),
            quantity_id: Some(quantity),
            model: RatingModel::PerUnit { rate: dec!(4) },
            min_slots: None,
            max_slots: None,
            initial_slots: 2,
        };
        let stored = serde_json::to_value(&fee).unwrap();
        let read: SubscriptionFee = serde_json::from_value(stored).unwrap();
        assert_eq!(read.seat_quantity(), Some(quantity));

        let old: SubscriptionFee = serde_json::from_str(
            r#"{"Slot":{"unit":"seat","unit_rate":"4.00","min_slots":null,"max_slots":null,"initial_slots":2}}"#,
        )
        .unwrap();
        assert_eq!(old.seat_quantity(), None);
    }
}
