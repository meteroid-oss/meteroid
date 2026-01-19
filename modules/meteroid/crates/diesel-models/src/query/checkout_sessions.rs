use crate::checkout_sessions::{CheckoutSessionRow, CheckoutSessionRowNew};
use crate::enums::CheckoutSessionStatusEnum;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use chrono::{DateTime, Utc};
use common_domain::ids::{CheckoutSessionId, CustomerId, SubscriptionId, TenantId};
use diesel::{BoolExpressionMethods, ExpressionMethods, OptionalExtension, QueryDsl, debug_query};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl CheckoutSessionRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<CheckoutSessionRow> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = diesel::insert_into(cs_dsl::checkout_session).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting checkout session")
            .into_db_result()
    }
}

impl CheckoutSessionRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> DbResult<CheckoutSessionRow> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = cs_dsl::checkout_session
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while getting checkout session")
            .into_db_result()
    }

    pub async fn get_by_id_for_update(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> DbResult<CheckoutSessionRow> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = cs_dsl::checkout_session
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .for_update();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while getting checkout session for update")
            .into_db_result()
    }

    /// Returns the most recent session that is still in Created or AwaitingPayment status.
    pub async fn get_by_subscription(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> DbResult<Option<CheckoutSessionRow>> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = cs_dsl::checkout_session
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .filter(cs_dsl::subscription_id.eq(subscription_id))
            .filter(
                cs_dsl::status
                    .eq(CheckoutSessionStatusEnum::Created)
                    .or(cs_dsl::status.eq(CheckoutSessionStatusEnum::AwaitingPayment)),
            )
            .order_by(cs_dsl::created_at.desc());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while getting checkout session by subscription")
            .into_db_result()
    }

    pub async fn mark_completed(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
        subscription_id: SubscriptionId,
        completed_at: DateTime<Utc>,
    ) -> DbResult<CheckoutSessionRow> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = diesel::update(cs_dsl::checkout_session)
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .set((
                cs_dsl::status.eq(CheckoutSessionStatusEnum::Completed),
                cs_dsl::subscription_id.eq(subscription_id),
                cs_dsl::completed_at.eq(completed_at),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while marking checkout session as completed")
            .into_db_result()
    }

    /// Mark a checkout session as awaiting payment (for async payment methods).
    /// Optionally links to a subscription if it was created (SalesLed flow).
    pub async fn mark_awaiting_payment(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
        subscription_id: Option<SubscriptionId>,
    ) -> DbResult<CheckoutSessionRow> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = diesel::update(cs_dsl::checkout_session)
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .set((
                cs_dsl::status.eq(CheckoutSessionStatusEnum::AwaitingPayment),
                cs_dsl::subscription_id.eq(subscription_id),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while marking checkout session as awaiting payment")
            .into_db_result()
    }

    pub async fn mark_expired_batch(conn: &mut PgConn, now: DateTime<Utc>) -> DbResult<usize> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = diesel::update(cs_dsl::checkout_session)
            .filter(cs_dsl::status.eq(CheckoutSessionStatusEnum::Created))
            .filter(cs_dsl::expires_at.is_not_null())
            .filter(cs_dsl::expires_at.lt(now))
            .set(cs_dsl::status.eq(CheckoutSessionStatusEnum::Expired));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while marking expired checkout sessions")
            .into_db_result()
    }

    /// Delete old expired checkout sessions after retention period.
    pub async fn delete_old(conn: &mut PgConn, older_than: DateTime<Utc>) -> DbResult<usize> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = diesel::delete(cs_dsl::checkout_session)
            .filter(cs_dsl::status.eq(CheckoutSessionStatusEnum::Expired))
            .filter(cs_dsl::created_at.lt(older_than));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while deleting old checkout sessions")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        status: Option<CheckoutSessionStatusEnum>,
    ) -> DbResult<Vec<CheckoutSessionRow>> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let mut query = cs_dsl::checkout_session
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .order_by(cs_dsl::created_at.desc())
            .into_boxed();

        if let Some(cid) = customer_id {
            query = query.filter(cs_dsl::customer_id.eq(cid));
        }

        if let Some(s) = status {
            query = query.filter(cs_dsl::status.eq(s));
        }

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing checkout sessions")
            .into_db_result()
    }

    pub async fn mark_cancelled(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> DbResult<Option<CheckoutSessionRow>> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        // Allow cancellation of Created or AwaitingPayment sessions
        let query = diesel::update(cs_dsl::checkout_session)
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .filter(
                cs_dsl::status
                    .eq(CheckoutSessionStatusEnum::Created)
                    .or(cs_dsl::status.eq(CheckoutSessionStatusEnum::AwaitingPayment)),
            )
            .set(cs_dsl::status.eq(CheckoutSessionStatusEnum::Cancelled));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while cancelling checkout session")
            .into_db_result()
    }
}
