pub mod clickhouse;

pub use metric_core::{errors, json};

use crate::connectors::errors::ConnectorError;
use crate::domain::{
    CustomerActivity, CustomerFirstEvent, DiscoveredProperty, EventCodeCount,
    EventPropertiesProfile, PropertyValueCount, QueryMeterParams, QueryRawEventsParams,
    QueryRawEventsResult, TenantEventScan, TenantFirstEvent, Usage, UsageActivityParams,
};
use common_domain::ids::{CustomerId, TenantId};
use error_stack::Report;

use tonic::async_trait;

#[async_trait]
pub trait Connector {
    async fn query_meter(
        &self,
        params: QueryMeterParams,
    ) -> Result<Vec<Usage>, Report<ConnectorError>>;

    async fn query_raw_events(
        &self,
        params: QueryRawEventsParams,
    ) -> Result<QueryRawEventsResult, Report<ConnectorError>>;

    /// Hard-delete every raw event belonging to the given (deleted) tenants.
    async fn purge_tenants(&self, tenant_ids: &[TenantId]) -> Result<(), Report<ConnectorError>>;

    /// First ingested event per tenant, each no earlier than its own cutoff
    /// (empty filter = all tenants, unbounded).
    async fn list_tenant_first_events(
        &self,
        tenants: &[TenantEventScan],
    ) -> Result<Vec<TenantFirstEvent>, Report<ConnectorError>>;

    /// First ingested event per named customer. Callers pass only the customers
    /// they still need, so the scan is bounded by that set.
    async fn list_customer_first_events(
        &self,
        tenant_id: &TenantId,
        customer_ids: &[CustomerId],
    ) -> Result<Vec<CustomerFirstEvent>, Report<ConnectorError>>;

    /// Customers that received events since `params.since`, oldest activity first.
    async fn poll_usage_activity(
        &self,
        params: UsageActivityParams,
    ) -> Result<Vec<CustomerActivity>, Report<ConnectorError>>;

    /// Distinct event codes of the tenant's recent events, most frequent first.
    async fn discover_event_codes(
        &self,
        tenant_id: &TenantId,
        search: &str,
        limit: u32,
    ) -> Result<Vec<EventCodeCount>, Report<ConnectorError>>;

    /// Property keys carried by the events of `event_code` over the trailing `lookback_days`.
    async fn discover_properties(
        &self,
        tenant_id: &TenantId,
        event_code: &str,
        lookback_days: u32,
        limit: u32,
    ) -> Result<Vec<DiscoveredProperty>, Report<ConnectorError>>;

    /// Most frequent recent values of one property of `event_code`.
    async fn discover_property_values(
        &self,
        tenant_id: &TenantId,
        event_code: &str,
        property_key: &str,
        search: &str,
        limit: u32,
    ) -> Result<Vec<PropertyValueCount>, Report<ConnectorError>>;

    /// Per-property distribution of the events of `event_code` in `[from_ts, to_ts)` (epoch
    /// seconds, 0 = trailing 30 days ending now).
    async fn profile_event_properties(
        &self,
        tenant_id: &TenantId,
        event_code: &str,
        from_ts: i64,
        to_ts: i64,
        top_values: u32,
        limit: u32,
    ) -> Result<EventPropertiesProfile, Report<ConnectorError>>;
}

pub struct PrintConnector {}

#[async_trait]
impl Connector for PrintConnector {
    async fn query_meter(
        &self,
        params: QueryMeterParams,
    ) -> Result<Vec<Usage>, Report<ConnectorError>> {
        println!("Querying meter: {:?}", params);
        Ok(vec![])
    }

    async fn query_raw_events(
        &self,
        params: QueryRawEventsParams,
    ) -> Result<QueryRawEventsResult, Report<ConnectorError>> {
        println!("Querying raw events: {:?}", params);
        Ok(QueryRawEventsResult { events: vec![] })
    }

    async fn purge_tenants(&self, tenant_ids: &[TenantId]) -> Result<(), Report<ConnectorError>> {
        println!("Purging tenants: {:?}", tenant_ids);
        Ok(())
    }

    async fn list_tenant_first_events(
        &self,
        tenants: &[TenantEventScan],
    ) -> Result<Vec<TenantFirstEvent>, Report<ConnectorError>> {
        println!("Listing tenant first events: {:?}", tenants);
        Ok(vec![])
    }

    async fn list_customer_first_events(
        &self,
        tenant_id: &TenantId,
        customer_ids: &[CustomerId],
    ) -> Result<Vec<CustomerFirstEvent>, Report<ConnectorError>> {
        println!("Listing customer first events: {tenant_id:?} for {customer_ids:?}");
        Ok(vec![])
    }

    async fn poll_usage_activity(
        &self,
        _params: UsageActivityParams,
    ) -> Result<Vec<CustomerActivity>, Report<ConnectorError>> {
        Ok(vec![])
    }

    async fn discover_event_codes(
        &self,
        _tenant_id: &TenantId,
        _search: &str,
        _limit: u32,
    ) -> Result<Vec<EventCodeCount>, Report<ConnectorError>> {
        Ok(vec![])
    }

    async fn discover_properties(
        &self,
        _tenant_id: &TenantId,
        _event_code: &str,
        _lookback_days: u32,
        _limit: u32,
    ) -> Result<Vec<DiscoveredProperty>, Report<ConnectorError>> {
        Ok(vec![])
    }

    async fn discover_property_values(
        &self,
        _tenant_id: &TenantId,
        _event_code: &str,
        _property_key: &str,
        _search: &str,
        _limit: u32,
    ) -> Result<Vec<PropertyValueCount>, Report<ConnectorError>> {
        Ok(vec![])
    }

    async fn profile_event_properties(
        &self,
        _tenant_id: &TenantId,
        _event_code: &str,
        _from_ts: i64,
        _to_ts: i64,
        _top_values: u32,
        _limit: u32,
    ) -> Result<EventPropertiesProfile, Report<ConnectorError>> {
        Ok(EventPropertiesProfile {
            total_events: 0,
            properties: vec![],
        })
    }
}
