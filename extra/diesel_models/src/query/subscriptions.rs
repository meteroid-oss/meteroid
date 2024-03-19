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
            .into_db_result()
            .attach_printable("Error while inserting subscription")
    }
}
