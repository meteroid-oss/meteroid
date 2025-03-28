use crate::errors::IntoDbResult;

use crate::subscription_events::SubscriptionEventRow;
use crate::{DbResult, PgConn};
use chrono::NaiveDate;

use common_domain::ids::SubscriptionId;
use diesel::debug_query;
use diesel::{ExpressionMethods, QueryDsl};
use error_stack::ResultExt;

impl SubscriptionEventRow {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SubscriptionEventRow> {
        use crate::schema::subscription_event::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription_event).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting slot transaction")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        events: Vec<&SubscriptionEventRow>,
    ) -> DbResult<Vec<SubscriptionEventRow>> {
        use crate::schema::subscription_event::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription_event).values(events);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting slot transaction")
            .into_db_result()
    }

    pub async fn fetch_by_subscription_id_and_date(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        date: NaiveDate,
    ) -> DbResult<Vec<SubscriptionEventRow>> {
        use crate::schema::subscription_event::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = subscription_event
            .filter(subscription_id.eq(subscription_uid))
            .filter(applies_to.eq(date));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching subscription events")
            .into_db_result()
    }
}
