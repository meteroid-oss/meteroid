use crate::errors::IntoDbResult;

use crate::enums::SubscriptionEventType;
use crate::subscription_events::SubscriptionEventRow;
use crate::{DbResult, PgConn};
use chrono::NaiveDate;

use common_domain::ids::SubscriptionId;
use diesel::debug_query;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use error_stack::ResultExt;

impl SubscriptionEventRow {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SubscriptionEventRow> {
        use crate::schema::subscription_event::dsl::subscription_event;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription_event).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting slot transaction")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        events: Vec<&SubscriptionEventRow>,
    ) -> DbResult<Vec<SubscriptionEventRow>> {
        use crate::schema::subscription_event::dsl::subscription_event;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription_event).values(events);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while inserting slot transaction")
            .into_db_result()
    }

    pub async fn fetch_by_subscription_id_and_date(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        date: NaiveDate,
    ) -> DbResult<Vec<SubscriptionEventRow>> {
        use crate::schema::subscription_event::dsl::{
            applies_to, subscription_event, subscription_id,
        };
        use diesel_async::RunQueryDsl;

        let query = subscription_event
            .filter(subscription_id.eq(subscription_uid))
            .filter(applies_to.eq(date));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while fetching subscription events")
            .into_db_result()
    }

    pub async fn fetch_by_subscription_id_and_event_type(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        param_event_type: SubscriptionEventType,
        date: NaiveDate,
    ) -> DbResult<Option<SubscriptionEventRow>> {
        use crate::schema::subscription_event::dsl::{
            applies_to, event_type, subscription_event, subscription_id,
        };
        use diesel_async::RunQueryDsl;

        let query = subscription_event
            .filter(subscription_id.eq(subscription_uid))
            .filter(event_type.eq(param_event_type))
            .filter(applies_to.eq(date));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching subscription event by type")
            .into_db_result()
    }

    pub async fn update_mrr_movement_log_id(
        conn: &mut PgConn,
        event_id: uuid::Uuid,
        mrr_log_id: uuid::Uuid,
    ) -> DbResult<()> {
        use crate::schema::subscription_event::dsl::{
            bi_mrr_movement_log_id, id, subscription_event,
        };
        use diesel::ExpressionMethods;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(subscription_event)
            .filter(id.eq(event_id))
            .set(bi_mrr_movement_log_id.eq(Some(mrr_log_id)));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while updating subscription event mrr movement log id")
            .into_db_result()
    }
}
