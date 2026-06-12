use crate::entity_activity::{EntityActivityRow, EntityActivityRowNew};
use crate::enums::ActorTypeEnum;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::actor::ActorType;
use common_domain::ids::TenantId;
use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl, SelectableHelper, debug_query};
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
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

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
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .load(conn)
            .await
            .attach("Error while listing entity activities (filtered)")
            .into_db_result()
    }
}
