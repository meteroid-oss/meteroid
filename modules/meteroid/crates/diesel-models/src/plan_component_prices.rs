use common_domain::ids::{PlanOptionId, PriceComponentId, PriceId};
use diesel::{Insertable, Queryable, Selectable};

/// A price in a charge's cell of one option: the same price may be linked in several options.
#[derive(Queryable, Debug, Clone, Selectable)]
#[diesel(table_name = crate::schema::plan_component_price)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanComponentPriceRow {
    pub plan_component_id: PriceComponentId,
    pub price_id: PriceId,
    pub option_id: PlanOptionId,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::plan_component_price)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanComponentPriceRowNew {
    pub plan_component_id: PriceComponentId,
    pub price_id: PriceId,
    pub option_id: PlanOptionId,
}
