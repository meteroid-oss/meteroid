//! How customers move through a product family's self-serve offers.
use common_domain::ids::{PlanId, ProductFamilyId};
use diesel_models::product_family_journeys::ProductFamilyJourneyRow;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductFamilyJourney {
    pub product_family_id: ProductFamilyId,
    /// The plan a sign-up asking for it starts on; none: sign-ups pick a plan.
    pub signup_plan_id: Option<PlanId>,
    /// The option a sign-up starts on, by alias (kept across versions); none: the plan's default
    /// in the customer's currency.
    pub signup_option: Option<String>,
    /// Customers move between the options of their plan.
    pub period_switch: bool,
    /// Customers schedule a downgrade themselves; else they ask the account team.
    pub self_serve_downgrade: bool,
}

impl ProductFamilyJourney {
    pub fn default_for(product_family_id: ProductFamilyId) -> Self {
        ProductFamilyJourney {
            product_family_id,
            signup_plan_id: None,
            signup_option: None,
            period_switch: true,
            self_serve_downgrade: true,
        }
    }
}

impl From<ProductFamilyJourneyRow> for ProductFamilyJourney {
    fn from(row: ProductFamilyJourneyRow) -> Self {
        ProductFamilyJourney {
            product_family_id: row.product_family_id,
            signup_plan_id: row.signup_plan_id,
            signup_option: row.signup_option,
            period_switch: row.period_switch,
            self_serve_downgrade: row.self_serve_downgrade,
        }
    }
}
