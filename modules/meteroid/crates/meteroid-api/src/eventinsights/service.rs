use std::collections::HashMap;
use std::sync::Arc;

use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{BaseId, BillableMetricId, CustomerId};
use common_grpc::middleware::server::auth::RequestExt;
use error_stack::Report;
use futures::StreamExt;
use meteroid_grpc::meteroid::api::eventinsights::v1 as grpc;
use meteroid_grpc::meteroid::api::eventinsights::v1::event_insights_service_server::EventInsightsService;
use meteroid_grpc::meteroid::api::eventinsights::v1::query_metric_usage_request::{
    Breakdown, Grain, Metric,
};
use meteroid_store::clients::usage::{
    EventPropertiesProfile, MetricUsageBreakdown, MetricUsagePoint, MetricUsageQuery,
    TenantUsageGrain,
};
use meteroid_store::domain::enums::BillingMetricAggregateEnum;
use meteroid_store::domain::{MetricDefinition, UsagePeriod};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use tonic::{Request, Response, Status};

use super::{EventInsightsServiceComponents, mapping};
use meteroid_grpc_mapping::shared::mapping::datetime::chrono_from_timestamp;

const MAX_USAGE_RANGE_DAYS: i64 = 366;
const MAX_BREAKDOWN_SERIES: u32 = 50;
const MAX_USAGE_CUSTOMERS: usize = 100;
const CUSTOMER_DIMENSION: &str = "customer";
const OTHER_LABEL: &str = "Other";

/// Backstop on the metrics one customer-usage summary reads, well above any real tenant's meter
/// count; `total_count` keeps "N of M" honest if it ever bites.
const CUSTOMER_USAGE_METRIC_CAP: usize = 100;
const CUSTOMER_USAGE_CONCURRENCY: usize = 8;
const CUSTOMER_USAGE_WINDOW_DAYS: i64 = 30;

fn store_status(context: &str, error: &Report<StoreError>) -> Status {
    match error.current_context() {
        StoreError::InvalidArgument(message) => Status::invalid_argument(message.clone()),
        StoreError::ValueNotFound(_) => Status::not_found(format!("{context}: not found")),
        StoreError::MeteringNotReady | StoreError::UsageUnavailable => {
            Status::unavailable(format!("{context}: {}", error.current_context()))
        }
        _ => {
            log::error!("{context}: {error:?}");
            Status::internal(context.to_string())
        }
    }
}

/// While metering does not serve discovery yet (a rollout), the editor gets no suggestions rather
/// than an error; the miss is not cached.
fn not_yet_discoverable(context: &str, error: &Report<StoreError>) -> Result<(), Status> {
    if matches!(error.current_context(), StoreError::MeteringNotReady) {
        log::warn!("{context}: {error:?}");
        Ok(())
    } else {
        Err(store_status(context, error))
    }
}

/// Parse a yyyy-mm-dd date into UTC-midnight epoch seconds for a ClickHouse bound.
fn date_to_epoch(value: &str, field: &str) -> Result<i64, Status> {
    parse_date(value, field).map(|d| d.and_time(chrono::NaiveTime::MIN).and_utc().timestamp())
}

fn parse_date(value: &str, field: &str) -> Result<NaiveDate, Status> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|e| Status::invalid_argument(format!("invalid `{field}` date: {e}")))
}

fn usage_period(start: NaiveDateTime, end: NaiveDateTime) -> Result<UsagePeriod, Status> {
    if end <= start {
        return Err(Status::invalid_argument("`to` must be after `from`"));
    }
    if end - start > chrono::Duration::days(MAX_USAGE_RANGE_DAYS) {
        return Err(Status::invalid_argument(format!(
            "the range spans more than {MAX_USAGE_RANGE_DAYS} days"
        )));
    }
    Ok(UsagePeriod { start, end })
}

fn decimal_to_f64(value: Decimal) -> f64 {
    value.to_f64().unwrap_or_default()
}

/// `customer_names` is set under a customer breakdown, whose points are labelled by customer.
fn point_to_grpc(
    point: MetricUsagePoint,
    customer_names: Option<&HashMap<CustomerId, String>>,
) -> grpc::MetricUsagePoint {
    let mut dimensions = point.dimensions;
    if let Some(names) = customer_names {
        let label = match point.customer_id {
            Some(id) => names.get(&id).cloned().unwrap_or_else(|| id.as_proto()),
            None => OTHER_LABEL.to_string(),
        };
        dimensions.insert(CUSTOMER_DIMENSION.to_string(), label);
    }
    grpc::MetricUsagePoint {
        window_start: point.window_start.date().to_string(),
        window_end: point.window_end.date().to_string(),
        value: decimal_to_f64(point.value),
        dimensions,
        customer_id: point
            .customer_id
            .map(|id| id.as_proto())
            .unwrap_or_default(),
        other: point.other,
    }
}

/// The whole-range total from the daily points where they compose: sum/count add, min/max fold,
/// latest is the last day's. `None` for avg and count_distinct, which need their own aggregate.
fn total_from_trend(
    agg: BillingMetricAggregateEnum,
    daily: &[MetricUsagePoint],
) -> Option<Decimal> {
    use BillingMetricAggregateEnum as E;
    let values = daily.iter().map(|p| p.value);
    match agg {
        E::Sum | E::Count => Some(values.sum()),
        E::Max => Some(values.max().unwrap_or_default()),
        E::Min => Some(values.min().unwrap_or_default()),
        E::Latest => Some(
            daily
                .iter()
                .max_by_key(|p| p.window_start)
                .map(|p| p.value)
                .unwrap_or_default(),
        ),
        E::Mean | E::CountDistinct => None,
    }
}

fn aggregation_label(agg: BillingMetricAggregateEnum) -> &'static str {
    use BillingMetricAggregateEnum as E;
    match agg {
        E::Sum => "sum",
        E::Mean => "avg",
        E::Min => "min",
        E::Max => "max",
        E::Count => "count",
        E::CountDistinct => "count_distinct",
        E::Latest => "latest",
    }
}

#[tonic::async_trait]
impl EventInsightsService for EventInsightsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn discover_event_codes(
        &self,
        request: Request<grpc::DiscoverEventCodesRequest>,
    ) -> Result<Response<grpc::DiscoverEventCodesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let rows = match self
            .usage_client
            .discover_event_codes(&tenant_id, &req.search, req.limit)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                not_yet_discoverable("Failed to discover event codes", &e)?;
                vec![]
            }
        };

        let codes = rows
            .into_iter()
            .map(|c| grpc::EventCodeCount {
                code: c.code,
                count: c.count,
            })
            .collect();

        Ok(Response::new(grpc::DiscoverEventCodesResponse { codes }))
    }

    #[tracing::instrument(skip_all)]
    async fn discover_properties(
        &self,
        request: Request<grpc::DiscoverPropertiesRequest>,
    ) -> Result<Response<grpc::DiscoverPropertiesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let usage_client = self.usage_client.clone();
        let event_code = req.event_code.clone();
        let rows = match self
            .property_schema_cache
            .try_get_with(
                (tenant_id, req.event_code, req.lookback_days, req.limit),
                async move {
                    usage_client
                        .discover_properties(&tenant_id, &event_code, req.lookback_days, req.limit)
                        .await
                        .map(Arc::new)
                },
            )
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                not_yet_discoverable("Failed to discover properties", &e)?;
                Arc::default()
            }
        };

        let properties = rows
            .iter()
            .map(|p| grpc::DiscoveredProperty {
                key: p.key.clone(),
                cardinality: p.cardinality,
                sample_values: p.sample_values.clone(),
                numeric_ratio: p.numeric_ratio,
            })
            .collect();

        Ok(Response::new(grpc::DiscoverPropertiesResponse {
            properties,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn discover_property_values(
        &self,
        request: Request<grpc::DiscoverPropertyValuesRequest>,
    ) -> Result<Response<grpc::DiscoverPropertyValuesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let rows = match self
            .usage_client
            .discover_property_values(
                &tenant_id,
                &req.event_code,
                &req.property_key,
                &req.search,
                req.limit,
            )
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                not_yet_discoverable("Failed to discover property values", &e)?;
                vec![]
            }
        };

        let values = rows
            .into_iter()
            .map(|v| grpc::PropertyValueCount {
                value: v.value,
                count: v.count,
            })
            .collect();

        Ok(Response::new(grpc::DiscoverPropertyValuesResponse {
            values,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn profile_event_properties(
        &self,
        request: Request<grpc::ProfileEventPropertiesRequest>,
    ) -> Result<Response<grpc::ProfileEventPropertiesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let to_ts = match req.to.as_str() {
            "" => 0,
            s => date_to_epoch(s, "to")?,
        };
        let from_ts = match req.from.as_str() {
            "" => 0,
            s => date_to_epoch(s, "from")?,
        };

        let profile = match self
            .usage_client
            .profile_event_properties(
                &tenant_id,
                &req.event_code,
                from_ts,
                to_ts,
                req.top_values,
                req.limit,
            )
            .await
        {
            Ok(profile) => profile,
            Err(e) => {
                not_yet_discoverable("Failed to profile event properties", &e)?;
                EventPropertiesProfile {
                    total_events: 0,
                    properties: vec![],
                }
            }
        };

        let properties = profile
            .properties
            .into_iter()
            .map(|p| grpc::PropertyProfile {
                key: p.key,
                event_count: p.event_count,
                coverage_ratio: p.coverage_ratio,
                cardinality: p.cardinality,
                numeric_ratio: p.numeric_ratio,
                numeric: p.numeric.map(|n| grpc::PropertyNumericStats {
                    min: n.min,
                    max: n.max,
                    avg: n.avg,
                    sum: n.sum,
                    p50: n.p50,
                    p95: n.p95,
                }),
                top_values: p
                    .top_values
                    .into_iter()
                    .map(|v| grpc::PropertyValueCount {
                        value: v.value,
                        count: v.count,
                    })
                    .collect(),
            })
            .collect();

        Ok(Response::new(grpc::ProfileEventPropertiesResponse {
            total_events: profile.total_events,
            properties,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn query_metric_usage(
        &self,
        request: Request<grpc::QueryMetricUsageRequest>,
    ) -> Result<Response<grpc::QueryMetricUsageResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let grain = match req.grain() {
            Grain::Total => None,
            Grain::Day => Some(TenantUsageGrain::Day),
        };
        let breakdown = match req.breakdown() {
            Breakdown::None => MetricUsageBreakdown::None,
            Breakdown::Customer => MetricUsageBreakdown::Customer,
            Breakdown::Property if req.breakdown_property.is_empty() => {
                return Err(Status::invalid_argument(
                    "a property breakdown needs breakdown_property",
                ));
            }
            Breakdown::Property => MetricUsageBreakdown::Property(req.breakdown_property.clone()),
        };
        let customer_breakdown = breakdown == MetricUsageBreakdown::Customer;
        // A breakdown is always bounded, so a high-cardinality split cannot return a series per value.
        let top_n = match (&breakdown, req.top_n) {
            (MetricUsageBreakdown::None, _) => None,
            (_, 0) => Some(MAX_BREAKDOWN_SERIES),
            (_, n) => Some(n.min(MAX_BREAKDOWN_SERIES)),
        };
        if req.customer_ids.len() > MAX_USAGE_CUSTOMERS {
            return Err(Status::invalid_argument(format!(
                "at most {MAX_USAGE_CUSTOMERS} customers"
            )));
        }
        let customer_ids = req
            .customer_ids
            .iter()
            .map(CustomerId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;
        let period = usage_period(
            chrono_from_timestamp(
                req.from
                    .ok_or_else(|| Status::invalid_argument("from is required"))?,
            )?,
            chrono_from_timestamp(
                req.to
                    .ok_or_else(|| Status::invalid_argument("to is required"))?,
            )?,
        )?;

        let metric = match req.metric {
            Some(Metric::MetricId(id)) => {
                let id = BillableMetricId::from_proto(&id)?;
                let metric = self
                    .store
                    .find_billable_metric_by_id(id, tenant_id)
                    .await
                    .map_err(|e| store_status("Failed to load the metric", &e))?;
                MetricDefinition::from(&metric)
            }
            Some(Metric::Draft(draft)) => mapping::draft_to_domain(draft)?,
            None => return Err(Status::invalid_argument("metric is required")),
        };

        let points = self
            .usage_client
            .query_metric_usage(
                &tenant_id,
                MetricUsageQuery {
                    metric: &metric,
                    customer_ids: &customer_ids,
                    breakdown,
                    grain,
                    top_n,
                    period,
                },
            )
            .await
            .map_err(|e| store_status("Failed to query metric usage", &e))?;

        let names = if customer_breakdown {
            let mut ids: Vec<CustomerId> = points.iter().filter_map(|p| p.customer_id).collect();
            ids.sort_by_key(|id| id.as_uuid());
            ids.dedup();
            let names: HashMap<CustomerId, String> = if ids.is_empty() {
                HashMap::new()
            } else {
                self.store
                    .list_customers_by_ids(tenant_id, ids)
                    .await
                    .map_err(|e| store_status("Failed to load customers", &e))?
                    .into_iter()
                    .map(|c| (c.id, c.name))
                    .collect()
            };
            Some(names)
        } else {
            None
        };

        Ok(Response::new(grpc::QueryMetricUsageResponse {
            points: points
                .into_iter()
                .map(|p| point_to_grpc(p, names.as_ref()))
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn get_customer_usage(
        &self,
        request: Request<grpc::GetCustomerUsageRequest>,
    ) -> Result<Response<grpc::GetCustomerUsageResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let customer_id = CustomerId::from_proto(&req.customer_id)?;

        let to = match req.to.as_str() {
            "" => chrono::Utc::now().naive_utc().date(),
            s => parse_date(s, "to")?,
        };
        let from = match req.from.as_str() {
            "" => to - chrono::Duration::days(CUSTOMER_USAGE_WINDOW_DAYS),
            s => parse_date(s, "from")?,
        };
        // Both dates are inclusive; metering ranges are half-open.
        let period = usage_period(
            from.and_time(chrono::NaiveTime::MIN),
            (to + chrono::Duration::days(1)).and_time(chrono::NaiveTime::MIN),
        )?;

        let mut billable = self
            .store
            .list_active_billable_metrics(tenant_id)
            .await
            .map_err(|e| store_status("Failed to list billable metrics", &e))?;
        billable.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then(a.id.as_uuid().cmp(&b.id.as_uuid()))
        });

        let total_count = billable.len() as u32;
        let usage_client = &self.usage_client;
        let customer_ids = std::slice::from_ref(&customer_id);
        let period = &period;

        // A metric whose daily read fails is left out rather than failing the page.
        let metrics: Vec<grpc::CustomerUsageMetric> =
            futures::stream::iter(billable.into_iter().take(CUSTOMER_USAGE_METRIC_CAP))
                .map(|m| async move {
                    let definition = MetricDefinition::from(&m);
                    let query = |grain| MetricUsageQuery {
                        metric: &definition,
                        customer_ids,
                        breakdown: MetricUsageBreakdown::None,
                        grain,
                        top_n: None,
                        period: period.clone(),
                    };
                    let daily = match usage_client
                        .query_metric_usage(&tenant_id, query(Some(TenantUsageGrain::Day)))
                        .await
                    {
                        Ok(daily) => daily,
                        Err(e) => {
                            log::warn!("customer usage of metric {} failed: {e:?}", m.id);
                            return None;
                        }
                    };
                    let total = match total_from_trend(m.aggregation_type, &daily) {
                        Some(total) => total,
                        None => match usage_client
                            .query_metric_usage(&tenant_id, query(None))
                            .await
                        {
                            Ok(total) => total.first().map(|p| p.value).unwrap_or_default(),
                            Err(e) => {
                                log::warn!("customer usage total of metric {} failed: {e:?}", m.id);
                                return None;
                            }
                        },
                    };
                    Some(grpc::CustomerUsageMetric {
                        metric_id: m.id.as_proto(),
                        label: m.name,
                        code: m.code,
                        aggregation: aggregation_label(m.aggregation_type).to_string(),
                        total: decimal_to_f64(total),
                        trend: daily.into_iter().map(|p| point_to_grpc(p, None)).collect(),
                    })
                })
                .buffer_unordered(CUSTOMER_USAGE_CONCURRENCY)
                .filter_map(|m| async move { m })
                .collect()
                .await;

        Ok(Response::new(grpc::GetCustomerUsageResponse {
            metrics,
            total_count,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn day(day: u32, value: Decimal) -> MetricUsagePoint {
        let start = NaiveDate::from_ymd_opt(2026, 1, day).unwrap();
        MetricUsagePoint {
            window_start: start.and_time(chrono::NaiveTime::MIN),
            window_end: start.succ_opt().unwrap().and_time(chrono::NaiveTime::MIN),
            value,
            dimensions: HashMap::new(),
            customer_id: None,
            other: false,
        }
    }

    #[test]
    fn total_from_trend_recombines_the_composable_ops() {
        use BillingMetricAggregateEnum as E;
        let daily = vec![day(1, dec!(10)), day(2, dec!(5)), day(3, dec!(8.5))];
        assert_eq!(total_from_trend(E::Sum, &daily), Some(dec!(23.5)));
        assert_eq!(total_from_trend(E::Count, &daily), Some(dec!(23.5)));
        assert_eq!(total_from_trend(E::Max, &daily), Some(dec!(10)));
        assert_eq!(total_from_trend(E::Min, &daily), Some(dec!(5)));
        assert_eq!(total_from_trend(E::Latest, &daily), Some(dec!(8.5)));
        assert_eq!(total_from_trend(E::Mean, &daily), None);
        assert_eq!(total_from_trend(E::CountDistinct, &daily), None);
    }

    #[test]
    fn total_from_trend_of_no_usage_is_zero() {
        use BillingMetricAggregateEnum as E;
        for agg in [E::Sum, E::Max, E::Min, E::Latest] {
            assert_eq!(total_from_trend(agg, &[]), Some(Decimal::ZERO));
        }
    }

    #[test]
    fn discovery_before_metering_serves_it_is_empty_not_an_error() {
        let not_ready = Report::new(StoreError::MeteringNotReady);
        assert!(not_yet_discoverable("discovery", &not_ready).is_ok());
        let failed = Report::new(StoreError::MeteringServiceError);
        assert_eq!(
            not_yet_discoverable("discovery", &failed)
                .unwrap_err()
                .code(),
            tonic::Code::Internal
        );
    }

    #[tokio::test]
    async fn a_failed_discovery_is_not_cached() {
        let cache: super::super::PropertySchemaCache = moka::future::Cache::new(10);
        let key = (
            common_domain::ids::TenantId::default(),
            "calls".to_string(),
            0,
            0,
        );
        let missed = cache
            .try_get_with(key.clone(), async {
                Err::<Arc<Vec<_>>, _>(Report::new(StoreError::MeteringNotReady))
            })
            .await;
        assert!(missed.is_err());
        assert!(cache.get(&key).await.is_none());
    }

    #[test]
    fn usage_ranges_are_bounded() {
        let at = |d: u32| {
            NaiveDate::from_ymd_opt(2026, 1, d)
                .unwrap()
                .and_time(chrono::NaiveTime::MIN)
        };
        assert!(usage_period(at(2), at(1)).is_err());
        assert!(usage_period(at(1), at(1)).is_err());
        assert!(usage_period(at(1), at(1) + chrono::Duration::days(367)).is_err());
        assert!(usage_period(at(1), at(1) + chrono::Duration::days(366)).is_ok());
    }

    #[test]
    fn the_other_customer_point_is_labelled() {
        let mut point = day(1, dec!(3));
        point.other = true;
        let out = point_to_grpc(point, Some(&HashMap::new()));
        assert_eq!(
            out.dimensions.get(CUSTOMER_DIMENSION).map(String::as_str),
            Some(OTHER_LABEL)
        );
        assert!(out.customer_id.is_empty());

        let id = CustomerId::from(uuid::Uuid::from_u128(7));
        let mut point = day(1, dec!(3));
        point.customer_id = Some(id);
        let out = point_to_grpc(point, Some(&HashMap::from([(id, "Acme".to_string())])));
        assert_eq!(
            out.dimensions.get(CUSTOMER_DIMENSION).map(String::as_str),
            Some("Acme")
        );
        assert_eq!(out.customer_id, id.as_proto());

        let out = point_to_grpc(day(1, dec!(3)), None);
        assert!(out.dimensions.is_empty());
    }
}
