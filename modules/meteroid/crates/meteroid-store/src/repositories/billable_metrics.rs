use error_stack::Report;
use uuid::Uuid;

use common_eventbus::Event;

use crate::domain::{
    BillableMetric, BillableMetricMeta, BillableMetricNew, PaginatedVec, PaginationRequest,
};
use crate::errors::StoreError;
use crate::{domain, Store, StoreResult};

#[async_trait::async_trait]
pub trait BillableMetricInterface {
    async fn find_billable_metric_by_id(
        &self,
        id: Uuid,
        tenant_id: Uuid,
    ) -> StoreResult<domain::BillableMetric>;

    async fn list_billable_metrics(
        &self,
        tenant_id: Uuid,
        pagination: PaginationRequest,
        product_family_external_id: String,
    ) -> StoreResult<PaginatedVec<domain::BillableMetricMeta>>;

    async fn insert_billable_metric(
        &self,
        billable_metric: domain::BillableMetricNew,
    ) -> StoreResult<domain::BillableMetric>;
}

#[async_trait::async_trait]
impl BillableMetricInterface for Store {
    async fn find_billable_metric_by_id(
        &self,
        id: Uuid,
        tenant_id: Uuid,
    ) -> StoreResult<domain::BillableMetric> {
        let mut conn = self.get_conn().await?;

        diesel_models::billable_metrics::BillableMetric::find_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn list_billable_metrics(
        &self,
        tenant_id: Uuid,
        pagination: PaginationRequest,
        product_family_external_id: String,
    ) -> StoreResult<PaginatedVec<domain::BillableMetricMeta>> {
        let mut conn = self.get_conn().await?;

        let rows = diesel_models::billable_metrics::BillableMetric::list(
            &mut conn,
            tenant_id,
            pagination.into(),
            product_family_external_id,
        )
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

        let family =
            diesel_models::product_families::ProductFamily::find_by_external_id_and_tenant_id(
                &mut conn,
                &billable_metric.family_external_id,
                billable_metric.tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let insertable_entity = diesel_models::billable_metrics::BillableMetricNew {
            id: Uuid::now_v7(),
            name: billable_metric.name,
            description: billable_metric.description,
            code: billable_metric.code,
            aggregation_type: billable_metric.aggregation_type.into(),
            aggregation_key: billable_metric.aggregation_key,
            unit_conversion_factor: billable_metric.unit_conversion_factor,
            unit_conversion_rounding: billable_metric.unit_conversion_rounding.map(Into::into),
            segmentation_matrix: billable_metric.segmentation_matrix,
            usage_group_key: billable_metric.usage_group_key,
            created_by: billable_metric.created_by,
            tenant_id: billable_metric.tenant_id,
            product_family_id: family.id,
        };

        let res: Result<domain::BillableMetric, Report<StoreError>> = insertable_entity
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .map(Into::into);

        if let Ok(inserted_entity) = &res {
            let inserted_entity = inserted_entity.clone();
            let _ = self
                .eventbus
                .publish(Event::billable_metric_created(
                    inserted_entity.created_by,
                    inserted_entity.id,
                    inserted_entity.tenant_id,
                ))
                .await;
        };

        res
    }
}
