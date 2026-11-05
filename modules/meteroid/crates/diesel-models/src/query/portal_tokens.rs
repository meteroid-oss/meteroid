use crate::enums::PortalResourceKindEnum;
use crate::errors::IntoDbResult;
use crate::portal_tokens::{PortalTokenRow, PortalTokenRowNew};
use crate::{DbResult, PgConn};

use chrono::{DateTime, Utc};
use common_domain::ids::{PortalTokenId, TenantId};
use diesel::prelude::{BoolExpressionMethods, ExpressionMethods, QueryDsl};
use diesel::{OptionalExtension, PgSortExpressionMethods, debug_query};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

impl PortalTokenRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PortalTokenRow> {
        use crate::schema::portal_token::dsl as d;

        let query = diesel::insert_into(d::portal_token).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting portal token")
            .into_db_result()
    }
}

impl PortalTokenRow {
    pub async fn find_by_hash(conn: &mut PgConn, token_hash: &str) -> DbResult<Option<Self>> {
        use crate::schema::portal_token::dsl as d;

        let query = d::portal_token.filter(d::token_hash.eq(token_hash));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding portal token by hash")
            .into_db_result()
    }

    /// Unrevoked tokens for the resource, scopes and delegation still valid at `valid_until`, and ending
    /// before `expires_before` when set. Longest-lived first.
    #[allow(clippy::too_many_arguments)]
    pub async fn list_reusable(
        conn: &mut PgConn,
        tenant_id: TenantId,
        resource_kind: PortalResourceKindEnum,
        resource_id: Uuid,
        recipient_email: Option<&str>,
        scopes: &[Option<String>],
        delegated: bool,
        valid_until: DateTime<Utc>,
        expires_before: Option<DateTime<Utc>>,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::portal_token::dsl as d;

        let mut query = d::portal_token
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::resource_kind.eq(resource_kind))
            .filter(d::resource_id.eq(resource_id))
            .filter(d::scopes.eq(scopes))
            .filter(d::delegated.eq(delegated))
            .filter(d::revoked_at.is_null())
            .filter(d::expires_at.is_null().or(d::expires_at.ge(valid_until)))
            .order(d::expires_at.desc().nulls_first())
            .limit(5)
            .into_boxed();
        if let Some(expires_before) = expires_before {
            query = query.filter(d::expires_at.le(expires_before));
        }
        query = match recipient_email {
            Some(email) => query.filter(d::recipient_email.eq(email)),
            None => query.filter(d::recipient_email.is_null()),
        };
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing reusable portal tokens")
            .into_db_result()
    }

    /// Records a use at most once per `granularity`, so the auth path rarely writes.
    pub async fn touch_last_used(
        conn: &mut PgConn,
        id: PortalTokenId,
        now: DateTime<Utc>,
        granularity: chrono::Duration,
    ) -> DbResult<()> {
        use crate::schema::portal_token::dsl as d;

        let query = diesel::update(
            d::portal_token.filter(d::id.eq(id)).filter(
                d::last_used_at
                    .is_null()
                    .or(d::last_used_at.lt(now - granularity)),
            ),
        )
        .set(d::last_used_at.eq(now));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while touching portal token")
            .into_db_result()
    }

    /// Wins the right to send a recovery email, at most once per `cooldown`.
    pub async fn claim_recovery(
        conn: &mut PgConn,
        id: PortalTokenId,
        now: DateTime<Utc>,
        cooldown: chrono::Duration,
    ) -> DbResult<bool> {
        use crate::schema::portal_token::dsl as d;

        let query = diesel::update(
            d::portal_token.filter(d::id.eq(id)).filter(
                d::last_recovery_at
                    .is_null()
                    .or(d::last_recovery_at.lt(now - cooldown)),
            ),
        )
        .set(d::last_recovery_at.eq(now));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .map(|n| n > 0)
            .attach("Error while claiming portal token recovery")
            .into_db_result()
    }

    pub async fn revoke_for_resource(
        conn: &mut PgConn,
        tenant_id: TenantId,
        resource_kind: PortalResourceKindEnum,
        resource_id: Uuid,
    ) -> DbResult<usize> {
        use crate::schema::portal_token::dsl as d;

        let query = diesel::update(
            d::portal_token
                .filter(d::tenant_id.eq(tenant_id))
                .filter(d::resource_kind.eq(resource_kind))
                .filter(d::resource_id.eq(resource_id))
                .filter(d::revoked_at.is_null()),
        )
        .set(d::revoked_at.eq(Utc::now()));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while revoking portal tokens")
            .into_db_result()
    }

    /// Expired customer tokens can't be recovered, so they go sooner than links.
    pub async fn delete_stale(
        conn: &mut PgConn,
        revoked_before: DateTime<Utc>,
        expired_before: DateTime<Utc>,
        customer_expired_before: DateTime<Utc>,
    ) -> DbResult<usize> {
        use crate::schema::portal_token::dsl as d;

        let query = diesel::delete(
            d::portal_token.filter(
                d::revoked_at
                    .lt(revoked_before)
                    .or(d::expires_at.lt(expired_before))
                    .or(d::resource_kind
                        .eq(PortalResourceKindEnum::Customer)
                        .and(d::expires_at.lt(customer_expired_before))),
            ),
        );
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while deleting stale portal tokens")
            .into_db_result()
    }
}
