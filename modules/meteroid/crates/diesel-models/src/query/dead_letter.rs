use crate::dead_letter::{DeadLetterMessageRow, DeadLetterMessageRowNew, DeadLetterWithTenantRow};
use crate::enums::DeadLetterStatusEnum;
use crate::errors::IntoDbResult;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::schema::{dead_letter_message, organization, tenant};
use crate::{DbResult, PgConn};
use common_domain::ids::{OrganizationId, TenantId};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

impl DeadLetterMessageRowNew {
    pub async fn insert_batch(conn: &mut PgConn, entries: &[Self]) -> DbResult<()> {
        diesel::insert_into(dead_letter_message::table)
            .values(entries)
            .execute(conn)
            .await
            .map(drop)
            .attach("Failed to insert dead letter messages")
            .into_db_result()
    }
}

impl DeadLetterMessageRow {
    pub async fn find_by_id(conn: &mut PgConn, id: Uuid) -> DbResult<DeadLetterMessageRow> {
        dead_letter_message::table
            .find(id)
            .first(conn)
            .await
            .attach("Failed to get dead letter message")
            .into_db_result()
    }

    pub async fn find_by_id_with_tenant(
        conn: &mut PgConn,
        id: Uuid,
    ) -> DbResult<DeadLetterWithTenantRow> {
        dead_letter_message::table
            .left_join(tenant::table.inner_join(organization::table))
            .filter(dead_letter_message::id.eq(id))
            .select(DeadLetterWithTenantRow::as_select())
            .first(conn)
            .await
            .attach("Failed to get dead letter message")
            .into_db_result()
    }

    pub async fn find_by_pgmq_msg_id(
        conn: &mut PgConn,
        queue: &str,
        pgmq_msg_id: i64,
    ) -> DbResult<Option<DeadLetterMessageRow>> {
        dead_letter_message::table
            .filter(dead_letter_message::queue.eq(queue))
            .filter(dead_letter_message::pgmq_msg_id.eq(pgmq_msg_id))
            .order(dead_letter_message::dead_lettered_at.desc())
            .first(conn)
            .await
            .optional()
            .attach("Failed to find dead letter by pgmq_msg_id")
            .into_db_result()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list(
        conn: &mut PgConn,
        queue_filter: Option<&str>,
        status_filter: Option<DeadLetterStatusEnum>,
        tenant_id_filter: Option<TenantId>,
        organization_id_filter: Option<OrganizationId>,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<DeadLetterWithTenantRow>> {
        let mut query = dead_letter_message::table
            .left_join(tenant::table.inner_join(organization::table))
            .select(DeadLetterWithTenantRow::as_select())
            .order(dead_letter_message::dead_lettered_at.desc())
            .into_boxed();

        if let Some(q) = queue_filter {
            query = query.filter(dead_letter_message::queue.eq(q));
        }
        if let Some(s) = status_filter {
            query = query.filter(dead_letter_message::status.eq(s));
        }
        if let Some(tid) = tenant_id_filter {
            query = query.filter(dead_letter_message::tenant_id.eq(tid));
        }
        if let Some(oid) = organization_id_filter {
            query = query.filter(tenant::organization_id.eq(oid));
        }

        query
            .paginate(pagination)
            .load_and_count_pages(conn)
            .await
            .attach("Failed to list dead letter messages")
            .into_db_result()
    }

    pub async fn update_status(
        conn: &mut PgConn,
        id: Uuid,
        status: DeadLetterStatusEnum,
        resolved_by: Uuid,
        requeued_pgmq_msg_id: Option<i64>,
    ) -> DbResult<DeadLetterMessageRow> {
        diesel::update(dead_letter_message::table.find(id))
            .set((
                dead_letter_message::status.eq(status),
                dead_letter_message::resolved_at.eq(diesel::dsl::now),
                dead_letter_message::resolved_by.eq(Some(resolved_by)),
                dead_letter_message::requeued_pgmq_msg_id.eq(requeued_pgmq_msg_id),
            ))
            .get_result(conn)
            .await
            .attach("Failed to update dead letter message status")
            .into_db_result()
    }

    pub async fn batch_update_status(
        conn: &mut PgConn,
        ids: &[Uuid],
        status: DeadLetterStatusEnum,
        resolved_by: Uuid,
    ) -> DbResult<Vec<DeadLetterMessageRow>> {
        diesel::update(
            dead_letter_message::table
                .filter(dead_letter_message::id.eq_any(ids))
                .filter(dead_letter_message::status.eq(DeadLetterStatusEnum::Pending)),
        )
        .set((
            dead_letter_message::status.eq(status),
            dead_letter_message::resolved_at.eq(diesel::dsl::now),
            dead_letter_message::resolved_by.eq(Some(resolved_by)),
        ))
        .get_results(conn)
        .await
        .attach("Failed to batch update dead letter messages")
        .into_db_result()
    }

    pub async fn queue_stats(conn: &mut PgConn) -> DbResult<Vec<QueueStatsRow>> {
        use diesel::sql_types;

        #[derive(QueryableByName)]
        pub struct Row {
            #[diesel(sql_type = sql_types::Text)]
            pub queue: String,
            #[diesel(sql_type = sql_types::BigInt)]
            pub pending_count: i64,
            #[diesel(sql_type = sql_types::BigInt)]
            pub requeued_count: i64,
            #[diesel(sql_type = sql_types::BigInt)]
            pub discarded_count: i64,
        }

        // FILTER (WHERE ...) isn't directly supported by diesel DSL, raw SQL is simpler here
        let rows: Vec<Row> = diesel::sql_query(
            "SELECT queue, \
             COUNT(*) FILTER (WHERE status = 'PENDING') AS pending_count, \
             COUNT(*) FILTER (WHERE status = 'REQUEUED') AS requeued_count, \
             COUNT(*) FILTER (WHERE status = 'DISCARDED') AS discarded_count \
             FROM dead_letter_message GROUP BY queue ORDER BY queue",
        )
        .get_results(conn)
        .await
        .attach("Failed to get dead letter queue stats")
        .into_db_result()?;

        Ok(rows
            .into_iter()
            .map(|r| QueueStatsRow {
                queue: r.queue,
                pending_count: r.pending_count,
                requeued_count: r.requeued_count,
                discarded_count: r.discarded_count,
            })
            .collect())
    }
}

pub struct QueueStatsRow {
    pub queue: String,
    pub pending_count: i64,
    pub requeued_count: i64,
    pub discarded_count: i64,
}

use crate::organizations::OrganizationRow;
use crate::tenants::TenantRow;

pub async fn search_organizations(
    conn: &mut PgConn,
    query: &str,
    limit: i64,
) -> DbResult<Vec<(OrganizationRow, Vec<TenantRow>)>> {
    use crate::schema::organization;
    use crate::schema::tenant;
    use diesel_async::RunQueryDsl;

    let pattern = format!("%{query}%");

    let orgs: Vec<OrganizationRow> = organization::table
        .filter(
            organization::trade_name
                .ilike(&pattern)
                .or(organization::slug.ilike(&pattern)),
        )
        .order(organization::trade_name.asc())
        .limit(limit)
        .get_results(conn)
        .await
        .attach("Failed to search organizations")
        .into_db_result()?;

    if orgs.is_empty() {
        return Ok(vec![]);
    }

    let tenants: Vec<TenantRow> = tenant::table
        .filter(tenant::organization_id.eq_any(orgs.iter().map(|o| o.id)))
        .filter(tenant::archived_at.is_null())
        .order(tenant::name.asc())
        .get_results(conn)
        .await
        .attach("Failed to fetch tenants for organizations")
        .into_db_result()?;

    let result = orgs
        .into_iter()
        .map(|org| {
            let org_tenants: Vec<TenantRow> = tenants
                .iter()
                .filter(|t| t.organization_id == org.id)
                .cloned()
                .collect();
            (org, org_tenants)
        })
        .collect();

    Ok(result)
}
