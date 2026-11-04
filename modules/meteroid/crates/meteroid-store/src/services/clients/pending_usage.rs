use crate::StoreResult;
use crate::domain::BillableMetric;
use crate::services::clients::usage::GroupedUsageData;
use chrono::NaiveDateTime;
use common_domain::ids::{CustomerId, TenantId};

/// Which events a usage read counts; mirrors the metering-side filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OriginFilter {
    #[default]
    All,
}

#[derive(Debug, Clone)]
pub struct PendingUsageRequest<'a> {
    pub tenant_id: &'a TenantId,
    /// The tenant's setting, resolved by the caller: the source never reads Postgres.
    pub keep_last_events: bool,
    pub customer_ids: &'a [CustomerId],
    pub metric: &'a BillableMetric,
    /// A dimension to break the usage down by, on top of those the metric already groups by.
    pub group_by: Option<&'a str>,
    /// Whole-second instants, half-open `[since, until)`.
    pub since: NaiveDateTime,
    pub until: NaiveDateTime,
    pub origin: OriginFilter,
}

#[derive(Debug, Clone)]
pub struct CustomerUsage {
    pub customer_id: CustomerId,
    pub groups: Vec<GroupedUsageData>,
}

/// Usage accrued after a durable base, for a batch of customers of one tenant. Kept separate
/// from [`crate::clients::usage::UsageClient`] so the store behind it can be swapped.
#[async_trait::async_trait]
pub trait PendingUsageSource: Send + Sync {
    /// Customers without events in the window are omitted.
    async fn usage_since(
        &self,
        request: PendingUsageRequest<'_>,
    ) -> StoreResult<Vec<CustomerUsage>>;
}
