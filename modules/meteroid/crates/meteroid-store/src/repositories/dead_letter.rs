use crate::domain::dead_letter::{
    DeadLetterMessage, DeadLetterMessageNew, DeadLetterQueueStats, OrganizationWithTenants,
    TenantSummary,
};
use crate::domain::enums::DeadLetterStatus;
use crate::domain::pgmq::PgmqQueue;
use crate::domain::{PaginatedVec, PaginationRequest};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::{OrganizationId, TenantId};
use common_domain::pgmq::{Headers, Message};
use diesel_models::dead_letter::DeadLetterMessageRow;
use diesel_models::query::pgmq;
use error_stack::Report;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait DeadLetterInterface {
    async fn insert_dead_letter_batch(&self, entries: Vec<DeadLetterMessageNew>)
    -> StoreResult<()>;

    async fn list_dead_letters(
        &self,
        queue: Option<&str>,
        status: Option<DeadLetterStatus>,
        tenant_id: Option<TenantId>,
        organization_id: Option<OrganizationId>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<DeadLetterMessage>>;

    async fn get_dead_letter(&self, id: Uuid) -> StoreResult<DeadLetterMessage>;

    async fn find_dead_letter_by_pgmq_msg_id(
        &self,
        queue: &str,
        pgmq_msg_id: i64,
    ) -> StoreResult<Option<DeadLetterMessage>>;

    async fn requeue_dead_letter(
        &self,
        id: Uuid,
        resolved_by: Uuid,
    ) -> StoreResult<DeadLetterMessage>;

    async fn discard_dead_letter(
        &self,
        id: Uuid,
        resolved_by: Uuid,
    ) -> StoreResult<DeadLetterMessage>;

    async fn batch_requeue_dead_letters(
        &self,
        ids: Vec<Uuid>,
        resolved_by: Uuid,
    ) -> StoreResult<u32>;

    async fn batch_discard_dead_letters(
        &self,
        ids: Vec<Uuid>,
        resolved_by: Uuid,
    ) -> StoreResult<u32>;

    async fn dead_letter_queue_stats(&self) -> StoreResult<Vec<DeadLetterQueueStats>>;

    async fn search_organizations(
        &self,
        query: &str,
        limit: u32,
    ) -> StoreResult<Vec<OrganizationWithTenants>>;
}

#[async_trait::async_trait]
impl DeadLetterInterface for Store {
    async fn insert_dead_letter_batch(
        &self,
        entries: Vec<DeadLetterMessageNew>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let rows: Vec<_> = entries.into_iter().map(Into::into).collect();
        diesel_models::dead_letter::DeadLetterMessageRowNew::insert_batch(&mut conn, &rows)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn list_dead_letters(
        &self,
        queue: Option<&str>,
        status: Option<DeadLetterStatus>,
        tenant_id: Option<TenantId>,
        organization_id: Option<OrganizationId>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<DeadLetterMessage>> {
        let mut conn = self.get_conn().await?;

        let rows = DeadLetterMessageRow::list(
            &mut conn,
            queue,
            status.map(Into::into),
            tenant_id,
            organization_id,
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(PaginatedVec {
            items: rows.items.into_iter().map(Into::into).collect(),
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
    }

    async fn get_dead_letter(&self, id: Uuid) -> StoreResult<DeadLetterMessage> {
        let mut conn = self.get_conn().await?;
        DeadLetterMessageRow::find_by_id_with_tenant(&mut conn, id)
            .await
            .map(Into::into)
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn find_dead_letter_by_pgmq_msg_id(
        &self,
        queue: &str,
        pgmq_msg_id: i64,
    ) -> StoreResult<Option<DeadLetterMessage>> {
        let mut conn = self.get_conn().await?;
        DeadLetterMessageRow::find_by_pgmq_msg_id(&mut conn, queue, pgmq_msg_id)
            .await
            .map(|opt| opt.map(Into::into))
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn requeue_dead_letter(
        &self,
        id: Uuid,
        resolved_by: Uuid,
    ) -> StoreResult<DeadLetterMessage> {
        let mut conn = self.get_conn().await?;

        let entry: DeadLetterMessage = DeadLetterMessageRow::find_by_id(&mut conn, id)
            .await
            .map(Into::into)
            .map_err(Into::<Report<StoreError>>::into)?;

        if entry.status != DeadLetterStatus::Pending {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "Dead letter {} is not in PENDING status",
                id
            ))));
        }

        let queue: PgmqQueue = entry
            .queue
            .parse()
            .map_err(|e: String| Report::new(StoreError::InvalidArgument(e)))?;

        let msg = diesel_models::pgmq::PgmqMessageRowNew {
            message: entry.message.map(Message),
            headers: entry.headers.map(Headers),
        };

        let new_ids = pgmq::send_batch_returning_ids(&mut conn, queue.as_str(), &[msg])
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let requeued_pgmq_msg_id = new_ids.into_iter().next();

        DeadLetterMessageRow::update_status(
            &mut conn,
            id,
            diesel_models::enums::DeadLetterStatusEnum::Requeued,
            resolved_by,
            requeued_pgmq_msg_id,
        )
        .await
        .map(Into::into)
        .map_err(Into::<Report<StoreError>>::into)
    }

    async fn discard_dead_letter(
        &self,
        id: Uuid,
        resolved_by: Uuid,
    ) -> StoreResult<DeadLetterMessage> {
        let mut conn = self.get_conn().await?;

        let entry: DeadLetterMessage = DeadLetterMessageRow::find_by_id(&mut conn, id)
            .await
            .map(Into::into)
            .map_err(Into::<Report<StoreError>>::into)?;

        if entry.status != DeadLetterStatus::Pending {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "Dead letter {} is not in PENDING status",
                id
            ))));
        }

        DeadLetterMessageRow::update_status(
            &mut conn,
            id,
            diesel_models::enums::DeadLetterStatusEnum::Discarded,
            resolved_by,
            None,
        )
        .await
        .map(Into::into)
        .map_err(Into::<Report<StoreError>>::into)
    }

    async fn batch_requeue_dead_letters(
        &self,
        ids: Vec<Uuid>,
        resolved_by: Uuid,
    ) -> StoreResult<u32> {
        let mut conn = self.get_conn().await?;

        let pending = DeadLetterMessageRow::find_pending_by_ids(&mut conn, &ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Enqueue first, collect IDs of successfully enqueued entries
        let mut enqueued_ids = Vec::new();
        for row in &pending {
            let queue: PgmqQueue = match row.queue.parse() {
                Ok(q) => q,
                Err(e) => {
                    log::error!("Unknown queue for dead letter {}: {e}", row.id);
                    continue;
                }
            };
            let msg = diesel_models::pgmq::PgmqMessageRowNew {
                message: row.message.clone().map(Message),
                headers: row.headers.clone().map(Headers),
            };
            match pgmq::send_batch(&mut conn, queue.as_str(), &[msg]).await {
                Ok(()) => enqueued_ids.push(row.id),
                Err(e) => log::error!("Failed to requeue dead letter {}: {:?}", row.id, e),
            }
        }

        // Only mark successfully enqueued entries as requeued
        if !enqueued_ids.is_empty() {
            DeadLetterMessageRow::batch_update_status(
                &mut conn,
                &enqueued_ids,
                diesel_models::enums::DeadLetterStatusEnum::Requeued,
                resolved_by,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        }

        Ok(enqueued_ids.len() as u32)
    }

    async fn batch_discard_dead_letters(
        &self,
        ids: Vec<Uuid>,
        resolved_by: Uuid,
    ) -> StoreResult<u32> {
        let mut conn = self.get_conn().await?;

        let updated = DeadLetterMessageRow::batch_update_status(
            &mut conn,
            &ids,
            diesel_models::enums::DeadLetterStatusEnum::Discarded,
            resolved_by,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(updated.len() as u32)
    }

    async fn dead_letter_queue_stats(&self) -> StoreResult<Vec<DeadLetterQueueStats>> {
        let mut conn = self.get_conn().await?;
        DeadLetterMessageRow::queue_stats(&mut conn)
            .await
            .map(|rows| {
                rows.into_iter()
                    .map(|r| DeadLetterQueueStats {
                        queue: r.queue,
                        pending_count: r.pending_count,
                        requeued_count: r.requeued_count,
                        discarded_count: r.discarded_count,
                    })
                    .collect()
            })
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn search_organizations(
        &self,
        query: &str,
        limit: u32,
    ) -> StoreResult<Vec<OrganizationWithTenants>> {
        use diesel_models::query::dead_letter::search_organizations;

        let mut conn = self.get_conn().await?;
        let rows = search_organizations(&mut conn, query, limit as i64)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows
            .into_iter()
            .map(|(org, tenants)| OrganizationWithTenants {
                id: org.id,
                trade_name: org.trade_name,
                slug: org.slug,
                tenants: tenants
                    .into_iter()
                    .map(|t| TenantSummary {
                        id: t.id,
                        name: t.name,
                        slug: t.slug,
                    })
                    .collect(),
            })
            .collect())
    }
}
