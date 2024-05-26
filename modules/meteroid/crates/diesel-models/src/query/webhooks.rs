use crate::errors::IntoDbResult;
use crate::extend::order::OrderByRequest;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::webhooks::{
    WebhookInEvent, WebhookInEventNew, WebhookOutEndpoint, WebhookOutEndpointNew, WebhookOutEvent,
    WebhookOutEventNew,
};
use crate::{DbResult, PgConn};
use diesel::{debug_query, ExpressionMethods, JoinOnDsl, QueryDsl, SelectableHelper};
use error_stack::ResultExt;

impl WebhookOutEndpointNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<WebhookOutEndpoint> {
        use crate::schema::webhook_out_endpoint::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(webhook_out_endpoint).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting webhook_out_endpoint")
            .into_db_result()
    }
}

impl WebhookOutEventNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<WebhookOutEvent> {
        use crate::schema::webhook_out_event::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(webhook_out_event).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting webhook_out_event")
            .into_db_result()
    }
}

impl WebhookOutEndpoint {
    pub async fn list_by_tenant_id(
        conn: &mut PgConn,
        tenant_id: uuid::Uuid,
    ) -> DbResult<Vec<WebhookOutEndpoint>> {
        use crate::schema::webhook_out_endpoint::dsl;
        use diesel_async::RunQueryDsl;

        let query = dsl::webhook_out_endpoint.filter(dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing webhook_out_endpoints by tenant_id")
            .into_db_result()
    }

    pub async fn find_by_id_and_tenant_id(
        conn: &mut PgConn,
        id: uuid::Uuid,
        tenant_id: uuid::Uuid,
    ) -> DbResult<WebhookOutEndpoint> {
        use crate::schema::webhook_out_endpoint::dsl;
        use diesel_async::RunQueryDsl;

        let query = dsl::webhook_out_endpoint
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::id.eq(id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while fetching webhook_out_endpoint by id and tenant_id")
            .into_db_result()
    }
}

impl WebhookOutEvent {
    pub async fn list_events(
        conn: &mut PgConn,
        tenant_id: uuid::Uuid,
        endpoint_id: uuid::Uuid,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> DbResult<PaginatedVec<WebhookOutEvent>> {
        use crate::schema::webhook_out_endpoint::dsl as end_dsl;
        use crate::schema::webhook_out_event::dsl as ev_dsl;

        let mut query = ev_dsl::webhook_out_event
            .inner_join(end_dsl::webhook_out_endpoint.on(ev_dsl::endpoint_id.eq(end_dsl::id)))
            .filter(ev_dsl::endpoint_id.eq(endpoint_id))
            .filter(end_dsl::tenant_id.eq(tenant_id))
            .select(WebhookOutEvent::as_select())
            .into_boxed();

        match order_by {
            OrderByRequest::IdAsc => query = query.order(ev_dsl::id.asc()),
            OrderByRequest::IdDesc => query = query.order(ev_dsl::id.desc()),
            OrderByRequest::DateAsc => query = query.order(ev_dsl::created_at.asc()),
            OrderByRequest::DateDesc => query = query.order(ev_dsl::created_at.desc()),
            _ => query = query.order(ev_dsl::id.asc()),
        }

        let paginated_query = query.paginate(pagination);

        log::debug!(
            "{}",
            debug_query::<diesel::pg::Pg, _>(&paginated_query).to_string()
        );

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while fetching webhook_out events")
            .into_db_result()
    }
}

impl WebhookInEventNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<WebhookInEvent> {
        use crate::schema::webhook_in_event::dsl as wi_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(wi_dsl::webhook_in_event).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting webhook_in_event")
            .into_db_result()
    }
}
