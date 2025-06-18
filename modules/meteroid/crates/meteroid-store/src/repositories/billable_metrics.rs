use crate::domain::outbox_event::OutboxEvent;
use crate::domain::{
    BillableMetric, BillableMetricMeta, BillableMetricNew, PaginatedVec, PaginationRequest,
};
use crate::errors::StoreError;
use crate::{Store, StoreResult, domain};
use common_domain::ids::{BaseId, BillableMetricId, ProductFamilyId, TenantId};
use common_eventbus::Event;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::billable_metrics::{BillableMetricRow, BillableMetricRowNew};
use diesel_models::product_families::ProductFamilyRow;
use error_stack::Report;

#[async_trait::async_trait]
pub trait BillableMetricInterface {
    async fn find_billable_metric_by_id(
        &self,
        id: BillableMetricId,
        tenant_id: TenantId,
    ) -> StoreResult<domain::BillableMetric>;

    async fn list_billable_metrics(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        product_family_id: Option<ProductFamilyId>,
    ) -> StoreResult<PaginatedVec<domain::BillableMetricMeta>>;

    async fn insert_billable_metric(
        &self,
        billable_metric: domain::BillableMetricNew,
    ) -> StoreResult<domain::BillableMetric>;

    async fn list_billable_metrics_by_code(
        &self,
        tenant_id: TenantId,
        code: String,
    ) -> StoreResult<Vec<BillableMetric>>;
}

#[async_trait::async_trait]
impl BillableMetricInterface for Store {
    async fn find_billable_metric_by_id(
        &self,
        id: BillableMetricId,
        tenant_id: TenantId,
    ) -> StoreResult<domain::BillableMetric> {
        let mut conn = self.get_conn().await?;

        BillableMetricRow::find_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn list_billable_metrics(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        product_family_id: Option<ProductFamilyId>,
    ) -> StoreResult<PaginatedVec<BillableMetricMeta>> {
        let mut conn = self.get_conn().await?;

        let rows =
            BillableMetricRow::list(&mut conn, tenant_id, pagination.into(), product_family_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<BillableMetricMeta> = PaginatedVec {
            items: rows.items.into_iter().map(|s| s.into()).collect(),
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        };

        Ok(res)
    }

    async fn insert_billable_metric(
        &self,
        billable_metric: BillableMetricNew,
    ) -> StoreResult<BillableMetric> {
        let mut conn = self.get_conn().await?;

        let family = ProductFamilyRow::find_by_id(
            &mut conn,
            billable_metric.product_family_id,
            billable_metric.tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        // TODO create product if None ?

        let insertable_entity = BillableMetricRowNew {
            id: BillableMetricId::new(),
            name: billable_metric.name,
            description: billable_metric.description,
            code: billable_metric.code,
            aggregation_type: billable_metric.aggregation_type.into(),
            aggregation_key: billable_metric.aggregation_key,
            unit_conversion_factor: billable_metric.unit_conversion_factor,
            unit_conversion_rounding: billable_metric.unit_conversion_rounding.map(Into::into),
            segmentation_matrix: billable_metric
                .segmentation_matrix
                .map(|x| {
                    serde_json::to_value(&x).map_err(|e| {
                        StoreError::SerdeError(
                            "Failed to serialize segmentation_matrix".to_string(),
                            e,
                        )
                    })
                })
                .transpose()?,
            usage_group_key: billable_metric.usage_group_key,
            created_by: billable_metric.created_by,
            tenant_id: billable_metric.tenant_id,
            product_family_id: family.id,
            product_id: billable_metric.product_id,
        };

        let res: BillableMetric = self
            .transaction_with(&mut conn, |conn| {
                async move {
                    let res: BillableMetric = insertable_entity
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                        .and_then(TryInto::try_into)?;

                    self.internal
                        .insert_outbox_events_tx(
                            conn,
                            vec![OutboxEvent::billable_metric_created(res.clone().into())],
                        )
                        .await?;

                    Ok(res)
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::billable_metric_created(
                res.created_by,
                res.id.as_uuid(),
                res.tenant_id.as_uuid(),
            ))
            .await;

        Ok(res)
    }

    async fn list_billable_metrics_by_code(
        &self,
        tenant_id: TenantId,
        code: String,
    ) -> StoreResult<Vec<BillableMetric>> {
        let mut conn = self.get_conn().await?;

        BillableMetricRow::list_by_code(&mut conn, &tenant_id, code.as_str())
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }
}
