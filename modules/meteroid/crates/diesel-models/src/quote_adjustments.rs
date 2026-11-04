use chrono::NaiveDateTime;
use common_domain::ids::{ProductId, QuoteId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

use crate::enums::AdjustmentKindEnum;

#[derive(Queryable, Debug, Identifiable, Selectable, Clone)]
#[diesel(table_name = crate::schema::quote_adjustment)]
#[diesel(primary_key(quote_id, kind))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteAdjustmentRow {
    pub quote_id: QuoteId,
    pub kind: AdjustmentKindEnum,
    pub amount_subunit: i64,
    pub includes_all_components: bool,
    /// Names of inline new-product extras to fold into the scope at conversion (see migration).
    pub pending_extra_product_names: Vec<Option<String>>,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::quote_adjustment)]
pub struct QuoteAdjustmentRowNew {
    pub quote_id: QuoteId,
    pub kind: AdjustmentKindEnum,
    pub amount_subunit: i64,
    pub includes_all_components: bool,
    pub pending_extra_product_names: Vec<Option<String>>,
}

#[derive(Queryable, Debug, Identifiable, Selectable, Clone)]
#[diesel(table_name = crate::schema::quote_adjustment_component)]
#[diesel(primary_key(quote_id, kind, product_id))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteAdjustmentComponentRow {
    pub quote_id: QuoteId,
    pub kind: AdjustmentKindEnum,
    pub product_id: ProductId,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = crate::schema::quote_adjustment_component)]
pub struct QuoteAdjustmentComponentRowNew {
    pub quote_id: QuoteId,
    pub kind: AdjustmentKindEnum,
    pub product_id: ProductId,
}
