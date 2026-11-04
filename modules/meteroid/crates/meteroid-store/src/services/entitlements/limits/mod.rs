//! What a metered entitlement's limit is measured on, as both the entitlements read path and the
//! live usage controls derive it: its monitor's identity, the subscription its window follows, the
//! metrics it reads and how grouped usage counts against it.
pub(crate) mod rules;

use crate::StoreResult;
use crate::domain::BillableMetric;
use crate::errors::StoreError;
use crate::services::clients::usage::GroupedUsageData;
use crate::store::PgConn;
use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, BillableMetricId, CustomerId, SubscriptionId, TenantId};
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use rust_decimal::Decimal;
use std::collections::HashMap;
use uuid::Uuid;

/// The entitlement monitor of a feature on a subscription; a feature is entitled at most once
/// per subscription, whichever level of the resolution chain set it.
pub fn entitlement_monitor_id(feature_id: impl BaseId, subscription_id: SubscriptionId) -> Uuid {
    Uuid::new_v5(&feature_id.as_uuid(), subscription_id.as_uuid().as_bytes())
}

/// The entitlement monitor of a feature for a customer without an active subscription.
pub fn customer_entitlement_monitor_id(feature_id: impl BaseId, customer_id: CustomerId) -> Uuid {
    Uuid::new_v5(&feature_id.as_uuid(), customer_id.as_uuid().as_bytes())
}

/// The subscription a customer-level window follows: the earliest started one, the lowest id
/// among those started together. Unlike a renewal date, it does not move when another
/// subscription renews, so a window never restarts mid-period.
pub fn anchor_subscription<'a>(
    subscriptions: impl IntoIterator<Item = &'a SubscriptionRow>,
) -> Option<&'a SubscriptionRow> {
    subscriptions
        .into_iter()
        .min_by_key(|subscription| (subscription.start_date, subscription.id.as_uuid()))
}

/// The earliest activation among the customer's active subscriptions.
pub fn earliest_activation<'a>(
    subscriptions: impl IntoIterator<Item = &'a SubscriptionRow>,
) -> Option<NaiveDateTime> {
    subscriptions
        .into_iter()
        .filter_map(|subscription| subscription.activated_at)
        .min()
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

fn add_up_by_group<'a>(
    usage: &'a [GroupedUsageData],
    group_by: &str,
) -> StoreResult<HashMap<&'a str, Decimal>> {
    let mut groups: HashMap<&str, Decimal> = HashMap::new();
    for row in usage {
        let Some(value) = row.dimensions.get(group_by) else {
            continue;
        };
        let total = groups.entry(value).or_insert(Decimal::ZERO);
        *total = total.checked_add(row.value).ok_or_else(|| {
            Report::new(StoreError::InvalidDecimal).attach("usage quantity out of range")
        })?;
    }
    Ok(groups)
}

/// The usage of the largest group of `group_by`, as the engine keeps it: rows sharing the
/// property value add up, rows without the property belong to no group.
pub(crate) fn largest_group(usage: &[GroupedUsageData], group_by: &str) -> StoreResult<Decimal> {
    Ok(add_up_by_group(usage, group_by)?
        .into_values()
        .max()
        .unwrap_or(Decimal::ZERO))
}

/// The usage of one group of `group_by`, added up as `largest_group` does.
pub fn group_usage(
    usage: &[GroupedUsageData],
    group_by: &str,
    value: &str,
) -> StoreResult<Decimal> {
    Ok(add_up_by_group(usage, group_by)?
        .get(value)
        .copied()
        .unwrap_or(Decimal::ZERO))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn usage(value: Decimal, dimensions: &[(&str, &str)]) -> GroupedUsageData {
        GroupedUsageData {
            value,
            dimensions: dimensions
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn the_largest_group_adds_up_rows_of_a_value_and_ignores_rows_without_the_property() {
        let rows = [
            usage(dec!(20), &[("project", "a"), ("region", "eu")]),
            usage(dec!(15), &[("project", "a"), ("region", "us")]),
            usage(dec!(30), &[("project", "b")]),
            usage(dec!(999), &[("region", "eu")]),
        ];
        assert_eq!(largest_group(&rows, "project").unwrap(), dec!(35));
        assert_eq!(largest_group(&rows, "missing").unwrap(), Decimal::ZERO);
        assert_eq!(largest_group(&[], "project").unwrap(), Decimal::ZERO);
        assert_eq!(group_usage(&rows, "project", "a").unwrap(), dec!(35));
        assert_eq!(group_usage(&rows, "project", "c").unwrap(), Decimal::ZERO);
    }
}
