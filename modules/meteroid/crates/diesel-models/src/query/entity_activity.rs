use crate::entity_activity::{EntityActivityRow, EntityActivityRowNew};
use crate::enums::ActorTypeEnum;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::actor::ActorType;
use common_domain::ids::{EntityActivityId, OAuthAppId, TenantId};
use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl, SelectableHelper};
use error_stack::ResultExt;
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct EntityActivityFilter {
    pub entity_types: Vec<String>,
    pub activity_types: Vec<String>,
    pub actor_type: Option<ActorType>,
    pub actor_uuid: Option<Uuid>,
    pub actor_alias: Option<String>,
    pub entity_id: Option<Uuid>,
    pub entity_type: Option<String>,
    pub occurred_after: Option<NaiveDateTime>,
    pub occurred_before: Option<NaiveDateTime>,
    pub rollup_customer_id: Option<Uuid>,
    pub rollup_subscription_id: Option<Uuid>,
}

/// Keyset cursor on `(occurred_at DESC, id DESC)`.
#[derive(Debug, Clone, Copy)]
pub struct ActivityCursor {
    pub occurred_at: NaiveDateTime,
    pub id: Uuid,
}

impl EntityActivityRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<EntityActivityRow> {
        use crate::schema::entity_activity::dsl::entity_activity;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(entity_activity).values(self);
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting entity activity")
            .into_db_result()
    }

    pub async fn insert_batch(rows: &[Self], conn: &mut PgConn) -> DbResult<usize> {
        use crate::schema::entity_activity::dsl::entity_activity;
        use diesel_async::RunQueryDsl;

        if rows.is_empty() {
            return Ok(0);
        }

        let query = diesel::insert_into(entity_activity).values(rows);
        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while batch-inserting entity activities")
            .into_db_result()
    }
}

impl EntityActivityRow {
    pub async fn list_by_entity(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_entity_type: &str,
        param_entity_id: Uuid,
        before: Option<ActivityCursor>,
        limit: i64,
    ) -> DbResult<Vec<EntityActivityRow>> {
        use crate::schema::entity_activity::dsl as ea;
        use diesel_async::RunQueryDsl;

        let mut query = ea::entity_activity
            .filter(ea::tenant_id.eq(param_tenant_id))
            .filter(ea::entity_type.eq(param_entity_type))
            .filter(ea::entity_id.eq(param_entity_id))
            .order((ea::occurred_at.desc(), ea::id.desc()))
            .select(EntityActivityRow::as_select())
            .limit(limit)
            .into_boxed();

        if let Some(cur) = before {
            query = query.filter(
                ea::occurred_at
                    .lt(cur.occurred_at)
                    .or(ea::occurred_at.eq(cur.occurred_at).and(ea::id.lt(cur.id))),
            );
        }

        query
            .load(conn)
            .await
            .attach("Error while listing entity activities")
            .into_db_result()
    }

    pub async fn list_filtered(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        filter: &EntityActivityFilter,
        before: Option<ActivityCursor>,
        limit: i64,
    ) -> DbResult<Vec<EntityActivityRow>> {
        use crate::schema::entity_activity::dsl as ea;
        use diesel_async::RunQueryDsl;

        let mut query = ea::entity_activity
            .filter(ea::tenant_id.eq(param_tenant_id))
            .order((ea::occurred_at.desc(), ea::id.desc()))
            .select(EntityActivityRow::as_select())
            .limit(limit)
            .into_boxed();

        if !filter.entity_types.is_empty() {
            query = query.filter(ea::entity_type.eq_any(filter.entity_types.clone()));
        }
        if !filter.activity_types.is_empty() {
            query = query.filter(ea::activity_type.eq_any(filter.activity_types.clone()));
        }
        if let Some(at) = filter.actor_type {
            query = query.filter(ea::actor_type.eq(ActorTypeEnum::from(at)));
        }
        if let Some(au) = filter.actor_uuid {
            query = query.filter(ea::actor_uuid.eq(au));
        }
        if let Some(ref alias) = filter.actor_alias {
            query = query.filter(ea::actor_alias.eq(alias));
        }
        if let Some(eid) = filter.entity_id {
            query = query.filter(ea::entity_id.eq(eid));
        }
        if let Some(ref et) = filter.entity_type {
            query = query.filter(ea::entity_type.eq(et));
        }
        if let Some(after) = filter.occurred_after {
            query = query.filter(ea::occurred_at.ge(after));
        }
        if let Some(before_ts) = filter.occurred_before {
            query = query.filter(ea::occurred_at.le(before_ts));
        }
        if let Some(cust) = filter.rollup_customer_id {
            query = query.filter(
                ea::agg_customer_id
                    .eq(cust)
                    .or(ea::entity_type.eq("customer").and(ea::entity_id.eq(cust))),
            );
        }
        if let Some(sub) = filter.rollup_subscription_id {
            query = query.filter(
                ea::agg_subscription_id.eq(sub).or(ea::entity_type
                    .eq("subscription")
                    .and(ea::entity_id.eq(sub))),
            );
        }
        if let Some(cur) = before {
            query = query.filter(
                ea::occurred_at
                    .lt(cur.occurred_at)
                    .or(ea::occurred_at.eq(cur.occurred_at).and(ea::id.lt(cur.id))),
            );
        }

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing entity activities (filtered)")
            .into_db_result()
    }

    /// Row-locked so concurrent operator actions on the same activity serialize.
    pub async fn find_by_id_for_update(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_id: EntityActivityId,
    ) -> DbResult<Option<EntityActivityRow>> {
        use crate::schema::entity_activity::dsl as ea;
        use diesel::OptionalExtension;
        use diesel_async::RunQueryDsl;

        ea::entity_activity
            .filter(ea::tenant_id.eq(param_tenant_id))
            .filter(ea::id.eq(param_id))
            .select(EntityActivityRow::as_select())
            .for_no_key_update()
            .first(conn)
            .await
            .optional()
            .attach("Error while locking entity activity")
            .into_db_result()
    }

    pub async fn list_by_entity_and_type(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_entity_type: &str,
        param_entity_id: Uuid,
        param_activity_type: &str,
    ) -> DbResult<Vec<EntityActivityRow>> {
        use crate::schema::entity_activity::dsl as ea;
        use diesel_async::RunQueryDsl;

        ea::entity_activity
            .filter(ea::tenant_id.eq(param_tenant_id))
            .filter(ea::entity_type.eq(param_entity_type))
            .filter(ea::entity_id.eq(param_entity_id))
            .filter(ea::activity_type.eq(param_activity_type))
            .select(EntityActivityRow::as_select())
            .load(conn)
            .await
            .attach("Error while listing entity activities by type")
            .into_db_result()
    }
}

pub async fn oauth_app_names(
    conn: &mut PgConn,
    ids: &[OAuthAppId],
) -> DbResult<Vec<(OAuthAppId, String)>> {
    use crate::schema::connect_oauth_app::dsl as coa;
    use diesel_async::RunQueryDsl;

    if ids.is_empty() {
        return Ok(vec![]);
    }
    coa::connect_oauth_app
        .filter(coa::id.eq_any(ids))
        .select((coa::id, coa::name))
        .get_results(conn)
        .await
        .attach("Error while fetching oauth app names by ids")
        .into_db_result()
}
