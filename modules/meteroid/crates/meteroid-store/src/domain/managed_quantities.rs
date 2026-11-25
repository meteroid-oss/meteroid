//! A quantity a subscription holds over time (seats, licences), shared by the fees that bill it.
use chrono::{DateTime, Utc};
use common_domain::ids::{FeatureId, ManagedQuantityId, TenantId};
use diesel_models::enums::{QuantityDecreaseEnum, QuantityIncreaseEnum};
use diesel_models::managed_quantities::{ManagedQuantityRow, ManagedQuantityRowPatch};
use o2o::o2o;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, o2o)]
#[map_owned(QuantityIncreaseEnum)]
pub enum QuantityIncrease {
    /// In force at once, billed as the change's collection mode says.
    #[default]
    Immediately,
    /// In force from the next renewal, nothing billed now.
    NextPeriod,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, o2o)]
#[map_owned(QuantityDecreaseEnum)]
pub enum QuantityDecrease {
    /// Billed until the fee's next renewal (R-108).
    #[default]
    AtPeriodEnd,
    /// Out of force at once, its unused time given back.
    Immediately,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(ManagedQuantityRow)]
pub struct ManagedQuantity {
    pub id: ManagedQuantityId,
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
    #[from(~.into())]
    pub on_increase: QuantityIncrease,
    #[from(~.into())]
    pub on_decrease: QuantityDecrease,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    /// Metered features whose limit counts per unit held.
    #[ghost({Vec::new()})]
    pub per_unit_features: Vec<FeatureId>,
}

#[derive(Clone, Debug)]
pub struct ManagedQuantityPatch {
    pub tenant_id: TenantId,
    pub id: ManagedQuantityId,
    pub name: Option<String>,
    pub on_increase: Option<QuantityIncrease>,
    pub on_decrease: Option<QuantityDecrease>,
}

impl From<&ManagedQuantityPatch> for ManagedQuantityRowPatch {
    fn from(patch: &ManagedQuantityPatch) -> Self {
        ManagedQuantityRowPatch {
            name: patch.name.clone(),
            on_increase: patch.on_increase.map(Into::into),
            on_decrease: patch.on_decrease.map(Into::into),
        }
    }
}

/// A change of a subscription's seats: by a number of them, or to a count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatChange {
    By(i32),
    To(u32),
}

impl From<i32> for SeatChange {
    fn from(delta: i32) -> Self {
        SeatChange::By(delta)
    }
}

impl SeatChange {
    /// The change from `held` seats, refused when it changes nothing.
    pub fn delta(self, held: i32) -> Option<i32> {
        let delta = match self {
            SeatChange::By(delta) => delta,
            SeatChange::To(count) => i32::try_from(count).ok()?.checked_sub(held)?,
        };
        (delta != 0).then_some(delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_to_a_count_is_its_difference_with_the_seats_held() {
        assert_eq!(SeatChange::To(12).delta(10), Some(2));
        assert_eq!(SeatChange::To(7).delta(10), Some(-3));
        assert_eq!(SeatChange::To(10).delta(10), None);
        assert_eq!(SeatChange::By(-2).delta(10), Some(-2));
        assert_eq!(SeatChange::By(0).delta(10), None);
    }
}
