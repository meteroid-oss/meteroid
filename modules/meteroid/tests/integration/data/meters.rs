use super::ids;
use diesel_async::AsyncConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::billable_metrics::BillableMetricRowNew;
use diesel_models::enums::BillingMetricAggregateEnum;
use diesel_models::errors::DatabaseErrorContainer;
use diesel_models::product_families::ProductFamilyRowNew;
use meteroid_store::store::PgPool;

pub async fn run_meters_seed(pool: &PgPool) {
    let mut conn = pool
        .get()
        .await
        .expect("couldn't get db connection from pool");

    conn.transaction(|tx| {
        async move {
            ProductFamilyRowNew {
                id: ids::PRODUCT_FAMILY_ID,
                name: "Default".to_string(),
                tenant_id: ids::TENANT_ID,
            }
            .insert(tx)
            .await?;

            BillableMetricRowNew {
                id: ids::METRIC_DATABASE_SIZE,
                name: "Database size (GB)".to_string(),
                description: None,
                code: "db_size".to_string(),
                aggregation_type: BillingMetricAggregateEnum::Latest,
                aggregation_key: Some("size_gb".to_string()),
                unit_conversion_factor: Some(1),
                unit_conversion_rounding: None,
                segmentation_matrix: None,
                usage_group_key: None,
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                product_id: None,
            }
            .insert(tx)
            .await?;

            BillableMetricRowNew {
                id: ids::METRIC_BANDWIDTH,
                name: "Bandwidth (GB)".to_string(),
                description: None,
                code: "bandwidth".to_string(),
                aggregation_type: BillingMetricAggregateEnum::Sum,
                aggregation_key: Some("value".to_string()),
                unit_conversion_factor: Some(1),
                unit_conversion_rounding: None,
                segmentation_matrix: None,
                usage_group_key: None,
                created_by: ids::USER_ID,
                tenant_id: ids::TENANT_ID,
                product_family_id: ids::PRODUCT_FAMILY_ID,
                product_id: None,
            }
            .insert(tx)
            .await?;

            Ok::<(), DatabaseErrorContainer>(())
        }
        .scope_boxed()
    })
    .await
    .unwrap();
}
