use common_domain::ids::{PriceComponentId, PriceId};
use diesel::{Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Selectable)]
#[diesel(table_name = crate::schema::plan_component_price)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanComponentPriceRow {
    pub plan_component_id: PriceComponentId,
    pub price_id: PriceId,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::plan_component_price)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanComponentPriceRowNew {
    pub plan_component_id: PriceComponentId,
    pub price_id: PriceId,
}
