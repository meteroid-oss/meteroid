use uuid::Uuid;

use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, AsChangeset, Selectable)]
#[diesel(table_name = crate::schema::price_component)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PriceComponent {
    pub id: Uuid,
    pub name: String,
    pub fee: serde_json::Value,
    pub plan_version_id: Uuid,
    pub product_item_id: Option<Uuid>,
    pub billable_metric_id: Option<Uuid>,
}

#[derive(Debug, Default, Insertable)]
#[diesel(table_name = crate::schema::price_component)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PriceComponentNew {
    pub id: Uuid,
    pub name: String,
    pub fee: serde_json::Value,
    pub plan_version_id: Uuid,
    pub product_item_id: Option<Uuid>,
    pub billable_metric_id: Option<Uuid>,
}

// the changeset one
