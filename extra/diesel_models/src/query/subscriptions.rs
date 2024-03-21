use crate::errors::IntoDbResult;
use crate::schema::subscription;
use crate::subscriptions::{Subscription, SubscriptionNew};
use crate::{errors, DbResult, PgConn};
use diesel::associations::HasTable;
use diesel::debug_query;
use error_stack::ResultExt;

impl SubscriptionNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<Subscription> {
        use crate::schema::subscription::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting subscription")
            .into_db_result()
    }
}

impl Subscription {
    pub async fn insert_subscription_batch(
        conn: &mut PgConn,
        batch: Vec<SubscriptionNew>,
    ) -> DbResult<Vec<Subscription>> {
        use crate::schema::subscription::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription).values(&batch);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting subscription batch")
            .into_db_result()
    }
}
