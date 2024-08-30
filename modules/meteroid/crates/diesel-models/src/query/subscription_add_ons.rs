use crate::errors::IntoDbResult;
use crate::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use crate::{DbResult, PgConn};
use diesel::{debug_query, QueryDsl};
use diesel::{ExpressionMethods, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl SubscriptionAddOnRow {
    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: Vec<&SubscriptionAddOnRowNew>,
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        let query = diesel::insert_into(sao_dsl::subscription_add_on).values(batch);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting SubscriptionAddOn batch")
            .into_db_result()
    }

    pub async fn list_by_subscription_id(
        conn: &mut PgConn,
        tenant_id: &uuid::Uuid,
        subscription_id: &uuid::Uuid,
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        let query = sao_dsl::subscription_add_on
            .inner_join(s_dsl::subscription)
            .filter(sao_dsl::subscription_id.eq(subscription_id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .select(SubscriptionAddOnRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing SubscriptionAddOn by subscription_id")
            .into_db_result()
    }
}
