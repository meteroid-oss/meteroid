//! What a metered entitlement's limit is measured on, as both the entitlements read path and the
//! live usage controls derive it: its monitor's identity and the metrics it reads.
pub(crate) mod rules;

use crate::StoreResult;
use crate::domain::BillableMetric;
use crate::domain::entitlement_fold::EvalLevel;
use crate::errors::StoreError;
use crate::store::PgConn;
use common_domain::ids::{BaseId, BillableMetricId, CustomerId, FeatureId, TenantId};
use diesel_models::billable_metrics::BillableMetricRow;
use error_stack::Report;
use std::collections::HashMap;
use uuid::Uuid;

/// The monitor of a feature's level for a customer, whichever subscription decides it. The
/// customer level's id is v5(feature, customer), the id customer limits had before v2; phase 2's
/// migration rekeys `usage_alert_state` to it with the same UUID v5.
pub fn level_monitor_id(feature_id: FeatureId, customer_id: CustomerId, level: EvalLevel) -> Uuid {
    let mut name = customer_id.as_uuid().as_bytes().to_vec();
    if let EvalLevel::Type(type_id) = level {
        name.extend_from_slice(type_id.as_uuid().as_bytes());
    }
    Uuid::new_v5(&feature_id.as_uuid(), &name)
}

pub async fn load_metrics(
    conn: &mut PgConn,
    metric_ids: &[BillableMetricId],
    tenant_id: TenantId,
) -> StoreResult<HashMap<BillableMetricId, BillableMetric>> {
    if metric_ids.is_empty() {
        return Ok(HashMap::new());
    }
    BillableMetricRow::get_by_ids(conn, metric_ids, &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(|row| -> Result<_, Report<StoreError>> {
            let metric: BillableMetric = row.try_into()?;
            Ok((metric.id, metric))
        })
        .collect()
}
