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

/// A new quantity of the catalog, named by a seat charge.
#[derive(Clone, Debug)]
pub struct ManagedQuantityNew {
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
    pub on_increase: QuantityIncrease,
    pub on_decrease: QuantityDecrease,
}

/// The code the web app's "New quantity…" makes of a name (`quantityCode`): trimmed, lowercase,
/// accents dropped, every other run of characters a `-`, none at the ends; "Seats" and " seats "
/// are both `seats`, "Sièges réservés" is `sieges-reserves`.
pub fn quantity_code(name: &str) -> String {
    let mut code = String::new();
    let mut dash = false;
    for c in name.trim().to_lowercase().chars() {
        let c = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            other => other,
        };
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
            code.push(c);
            dash = false;
        } else if !dash {
            code.push('-');
            dash = true;
        }
    }
    code.trim_matches('-').chars().take(64).collect()
}

/// The quantity of `quantities` a seat unit names, for a charge or a snapshot naming no quantity
/// id: the one coded as the unit, else as the trimmed unit, else as it ignoring case, else as
/// the code "New quantity…" makes of it (`quantity_code`). One unit is one count whatever its
/// spelling; an existing code is never rewritten.
pub fn quantity_of_unit<'a>(
    quantities: &'a [ManagedQuantityRow],
    unit: &str,
) -> Option<&'a ManagedQuantityRow> {
    let trimmed = unit.trim();
    let lower = trimmed.to_lowercase();
    let derived = quantity_code(unit);
    quantities
        .iter()
        .find(|q| q.code == unit)
        .or_else(|| quantities.iter().find(|q| q.code == trimmed))
        .or_else(|| quantities.iter().find(|q| q.code.to_lowercase() == lower))
        .or_else(|| {
            quantities
                .iter()
                .find(|q| !derived.is_empty() && q.code == derived)
        })
}

/// A code stays a handle: lowercase letters, digits, `_` and `-`, 1 to 64 of them.
pub fn validate_quantity_code(code: &str) -> Result<(), crate::errors::StoreError> {
    let valid = !code.is_empty()
        && code.len() <= 64
        && code
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(crate::errors::StoreError::InvalidArgument(format!(
            "A quantity's code is lowercase letters, digits, '_' and '-': {code}"
        )))
    }
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

    #[test]
    fn a_unit_name_is_coded_as_new_quantity_codes_it() {
        assert_eq!(quantity_code(" Seats "), "seats");
        assert_eq!(quantity_code("Sièges réservés"), "sieges-reserves");
        assert_eq!(quantity_code("API  keys!"), "api-keys");
        assert_eq!(quantity_code("  "), "");
    }
}
