use crate::connectors::clickhouse::sql::meter_value::{self, property};
use crate::connectors::clickhouse::sql::{BindValue, PropertyColumn, SafeQuery};
use crate::domain::{
    EventAttribution, EventSortOrder, MeterAggregation, OriginFilter, PropertyFilterOp,
    QueryMeterParams, QueryRawEventsParams, SegmentationFilter, WindowSize,
};
use chrono::Utc;
use common_domain::ids::BaseId;
use metric_core::sql::ast::{Expr, Ident, Item, Order, Query, Select};
use metric_core::sql::{EventDedup, EventsTable, VoidScope, render_scoped};

fn bind_str(value: &str) -> Expr {
    Expr::Bind(BindValue::String(value.to_string()))
}

/// `toDateTime(?)` over epoch seconds.
fn at(secs: i64) -> Expr {
    Expr::datetime(secs)
}

fn timestamp() -> Expr {
    Expr::col("timestamp")
}

/// Pre-aggregation property filters, ANDed. `In` keeps matching events; `NotIn` is
/// presence-required (mapContains guards an absent key from passing `!=`). Shared by the meter
/// and raw-event queries so the events explorer filters as billing meters.
fn property_filters(filters: &[crate::domain::PropertyFilter]) -> Vec<Expr> {
    filters
        .iter()
        .filter(|f| !f.values.is_empty())
        .map(|f| match f.op {
            PropertyFilterOp::In => Expr::Or(
                f.values
                    .iter()
                    .map(|v| property(&f.property).eq(bind_str(v)))
                    .collect(),
            ),
            PropertyFilterOp::NotIn => {
                let mut parts = vec![Expr::call(
                    "mapContains",
                    [Expr::col("properties"), bind_str(&f.property)],
                )];
                parts.extend(
                    f.values
                        .iter()
                        .map(|v| property(&f.property).ne(bind_str(v))),
                );
                Expr::And(parts)
            }
        })
        .collect()
}

fn columns(names: &[&'static str]) -> Vec<Ident> {
    names.iter().map(|c| Ident::trusted(c)).collect()
}

const RAW_EVENT_COLUMNS: &[&str] = &[
    "id",
    "code",
    "customer_id",
    "tenant_id",
    "timestamp",
    "ingested_at",
    "properties",
    "is_tombstone",
];

const METER_EVENT_COLUMNS: &[&str] = &[
    "id",
    "customer_id",
    "timestamp",
    "ingested_at",
    "properties",
];

pub fn query_raw_events_sql(
    params: QueryRawEventsParams,
    events: &EventsTable,
    database: &str,
) -> Result<SafeQuery, String> {
    let tenant_id = params.tenant_id.as_uuid();
    render_scoped(&raw_events_read(params, events, database)?, &[], tenant_id)
}

fn raw_events_read(
    params: QueryRawEventsParams,
    events: &EventsTable,
    database: &str,
) -> Result<Query, String> {
    let tenant_id = params.tenant_id.as_uuid();
    let source = events.source(
        database,
        tenant_id,
        EventDedup::new(params.keep_last_events),
    );
    let to = params.to.unwrap_or_else(Utc::now).timestamp();
    let mut filter = vec![
        Expr::col("tenant_id").eq(Expr::Bind(BindValue::Uuid(tenant_id))),
        timestamp().ge(at(params.from.timestamp())),
        timestamp().lt(at(to)),
    ];

    let customers: Vec<_> = params.customer_ids.iter().map(|id| id.as_uuid()).collect();
    let scope = VoidScope::default()
        .codes(&params.event_codes)
        .customers(customers.clone())
        .since(format!("toDateTime({})", params.from.timestamp()))
        .until(format!("toDateTime({to})"));
    if !customers.is_empty() {
        filter.push(Expr::col("customer_id").in_set(Expr::Bind(BindValue::Uuids(customers))));
    }
    if !params.event_codes.is_empty() {
        filter.push(Expr::col("code").in_set(Expr::Bind(BindValue::Strings(params.event_codes))));
    }
    if let Some(search) = params.search {
        let pattern = || bind_str(&format!("%{search}%"));
        let values = Expr::call(
            "arrayStringConcat",
            [
                Expr::call("mapValues", [Expr::col("properties")]),
                Expr::str(" "),
            ],
        );
        filter.push(Expr::Or(vec![
            Expr::col("id").ilike(pattern()),
            Expr::col("code").ilike(pattern()),
            values.ilike(pattern()),
        ]));
    }
    filter.extend(property_filters(&params.filters));

    let (order_by, keyset_cmp) = match params.sort_order {
        EventSortOrder::TimestampDesc => (
            vec![Order::desc(timestamp()), Order::desc(Expr::col("id"))],
            Some(true),
        ),
        EventSortOrder::TimestampAsc => (
            vec![Order::asc(timestamp()), Order::asc(Expr::col("id"))],
            Some(false),
        ),
        EventSortOrder::IngestedDesc => (vec![Order::desc(Expr::col("ingested_at"))], None),
        EventSortOrder::IngestedAsc => (vec![Order::asc(Expr::col("ingested_at"))], None),
    };
    let keyset = match params.after {
        Some(after) => {
            let descending = keyset_cmp
                .ok_or_else(|| "keyset pagination requires a timestamp sort order".to_string())?;
            let past = |lhs: Expr, rhs: Expr| if descending { lhs.lt(rhs) } else { lhs.gt(rhs) };
            let nanos = || Expr::call("toUnixTimestamp64Nano", [timestamp()]);
            let at_nanos = || Expr::Bind(BindValue::I64(after.timestamp_nanos));
            Some(Expr::Or(vec![
                past(nanos(), at_nanos()),
                Expr::And(vec![
                    nanos().eq(at_nanos()),
                    past(Expr::col("id"), bind_str(&after.id)),
                ]),
            ]))
        }
        None => None,
    };

    let items: Vec<Item> = RAW_EVENT_COLUMNS
        .iter()
        .map(|c| Item::new(Expr::col(c)))
        .collect();
    // Without a dedup the read stays flat: ClickHouse 25.6 lazy materialization segfaults on
    // `ORDER BY .. LIMIT n` over a bare subquery of a Distributed table.
    let select = match source.dedup() {
        EventDedup::Off => {
            filter.extend(keyset);
            let flat = source
                .read_flat(filter, scope)
                .ok_or("a tenant without dedup reads flat")?;
            Select::new(items, flat)
        }
        EventDedup::LastIngested => {
            let read = source.read(columns(RAW_EVENT_COLUMNS), filter, scope);
            Select::new(items, read).filter(keyset.into_iter().collect())
        }
    };
    let select = select.order_by(order_by).page(
        Expr::Bind(BindValue::U32(params.limit)),
        Expr::Bind(BindValue::U32(params.offset)),
    );
    Ok(select.query())
}

/// The value of the first of `properties` the event has non-empty, `''` without any.
fn first_present(properties: &[String]) -> Expr {
    let Some((last, earlier)) = properties.split_last() else {
        return Expr::str("");
    };
    if earlier.is_empty() {
        return property(last);
    }
    let mut branches: Vec<Expr> = earlier
        .iter()
        .flat_map(|p| [property(p).ne(Expr::str("")), property(p)])
        .collect();
    branches.push(property(last));
    Expr::call("multiIf", branches)
}

/// The deduplicated events a subscription reads, so the latest version of an event decides which
/// subscription it belongs to and no two subscriptions read different versions of it.
fn attribution_filter(attribution: &EventAttribution) -> Option<Expr> {
    if attribution.segments.is_empty() {
        return None;
    }
    let segments = attribution
        .segments
        .iter()
        .map(|segment| {
            let mut parts = Vec::new();
            if let Some(from) = segment.from {
                parts.push(timestamp().ge(at(from.timestamp())));
            }
            if let Some(to) = segment.to {
                parts.push(timestamp().lt(at(to.timestamp())));
            }
            if let Some(matching) = &segment.matching {
                parts.push(first_present(&matching.properties).eq(bind_str(&matching.value)));
            }
            for claim in &segment.excluded {
                let mut claimed = vec![
                    first_present(&claim.properties).eq(bind_str(&claim.value)),
                    timestamp().ge(at(claim.from.timestamp())),
                ];
                if let Some(to) = claim.to {
                    claimed.push(timestamp().lt(at(to.timestamp())));
                }
                parts.push(Expr::Not(Box::new(Expr::And(claimed))));
            }
            if parts.is_empty() {
                Expr::Int(1)
            } else {
                Expr::And(parts)
            }
        })
        .collect();
    Some(Expr::Or(segments))
}

fn origin_filter(origin: OriginFilter) -> Option<Expr> {
    match origin {
        OriginFilter::All => None,
    }
}

/// `properties[?] AS _prop_<name>`.
fn property_item(name: &str) -> Item {
    Item::aliased(
        property(name),
        Ident::sanitized(&PropertyColumn(name).as_alias()),
    )
}

/// The property columns a segmentation breaks the result down by.
fn segment_properties(segmentation: &SegmentationFilter) -> Vec<&str> {
    match segmentation {
        SegmentationFilter::Independent(filters) => {
            filters.iter().map(|(column, _)| column.as_str()).collect()
        }
        SegmentationFilter::Linked {
            dimension1_key,
            dimension2_key,
            ..
        } => vec![dimension1_key, dimension2_key],
    }
}

fn segmentation_filter(segmentation: &SegmentationFilter) -> Result<Vec<Expr>, String> {
    match segmentation {
        SegmentationFilter::Independent(filters) => filters
            .iter()
            .map(|(column, values)| {
                if values.is_empty() {
                    return Err(format!("Empty filter for dimension: {column}"));
                }
                Ok(Expr::Or(
                    values
                        .iter()
                        .map(|v| property(column).eq(bind_str(v)))
                        .collect(),
                ))
            })
            .collect(),
        SegmentationFilter::Linked {
            dimension1_key,
            dimension2_key,
            values,
        } => {
            let linked: Vec<Expr> = values
                .iter()
                .map(|(dim1, dim2_values)| {
                    let first = property(dimension1_key).eq(bind_str(dim1));
                    if dim2_values.is_empty() {
                        first
                    } else {
                        let second = property(dimension2_key)
                            .in_set(Expr::Bind(BindValue::Strings(dim2_values.clone())));
                        Expr::And(vec![first, second])
                    }
                })
                .collect();
            Ok(if linked.is_empty() {
                vec![]
            } else {
                vec![Expr::Or(linked)]
            })
        }
    }
}

fn tumble(function: &'static str, window: &WindowSize, tz: &str) -> Expr {
    let interval = match window {
        WindowSize::Minute => "toIntervalMinute",
        WindowSize::Hour => "toIntervalHour",
        WindowSize::Day => "toIntervalDay",
        WindowSize::Week => "toIntervalWeek",
    };
    Expr::call(
        function,
        [
            Expr::call("toDateTime", [timestamp()]),
            Expr::call(interval, [Expr::Int(1)]),
            bind_str(tz),
        ],
    )
}

/// Window bounds are truncated to whole seconds (`toDateTime`): callers that split one window
/// into ranges must truncate their bounds the same way, or events fall in two ranges.
pub fn query_meter_sql(
    params: QueryMeterParams,
    events: &EventsTable,
    database: &str,
) -> Result<SafeQuery, String> {
    let tenant_id = params.tenant_id.as_uuid();
    render_scoped(&meter_read(params, events, database)?, &[], tenant_id)
}

fn meter_read(
    params: QueryMeterParams,
    events: &EventsTable,
    database: &str,
) -> Result<Query, String> {
    let tenant_id = params.tenant_id.as_uuid();
    let source = events.source(
        database,
        tenant_id,
        EventDedup::new(params.keep_last_events),
    );

    let mut filter = vec![
        Expr::col("tenant_id").eq(Expr::Bind(BindValue::Uuid(tenant_id))),
        Expr::col("code").eq(bind_str(&params.code)),
        timestamp().ge(at(params.from.timestamp())),
    ];
    // Half-open [from, to): billing periods are half-open and consecutive periods share their
    // boundary instant, so an inclusive upper bound bills the boundary event twice.
    if let Some(to) = params.to {
        filter.push(timestamp().lt(at(to.timestamp())));
    }
    let customers: Vec<_> = params.customer_ids.iter().map(|id| id.as_uuid()).collect();
    let mut scope = VoidScope::default()
        .code(&params.code)
        .customers(customers.clone())
        .since(format!("toDateTime({})", params.from.timestamp()));
    if let Some(to) = params.to {
        scope = scope.until(format!("toDateTime({})", to.timestamp()));
    }
    if !customers.is_empty() {
        filter.push(Expr::col("customer_id").in_set(Expr::Bind(BindValue::Uuids(customers))));
    }
    if let Some(segmentation) = &params.segmentation_filter {
        filter.extend(segmentation_filter(segmentation)?);
    }
    // WHERE-only: unlike segmentation, these add no SELECT/GROUP BY column, so the rated result
    // shape is unchanged.
    filter.extend(property_filters(&params.filters));
    filter.extend(origin_filter(params.origin));
    if let Some(value_prop) = &params.value_property
        && !matches!(params.aggregation, MeterAggregation::Count)
    {
        filter.push(property(value_prop).ne(Expr::str("")));
        // CountDistinct counts distinct raw values (commonly strings), so it skips the guard.
        if !matches!(params.aggregation, MeterAggregation::CountDistinct) {
            filter.push(meter_value::numeric_guard(value_prop));
        }
    }

    let tz = params
        .window_time_zone
        .unwrap_or(chrono_tz::UTC)
        .name()
        .to_string();
    let mut items = match &params.window_size {
        Some(window) => vec![
            Item::as_(tumble("tumbleStart", window, &tz), "window_start"),
            Item::as_(tumble("tumbleEnd", window, &tz), "window_end"),
        ],
        None => {
            let seconds = || Expr::call("toDateTime", [timestamp()]);
            vec![
                Item::as_(Expr::call("min", [seconds()]), "window_start"),
                Item::as_(Expr::call("max", [seconds()]), "window_end"),
            ]
        }
    };
    items.extend(meter_value::aggregation_items(
        &params.aggregation,
        params.value_property.as_deref(),
    )?);
    // A `customer_id` group-by property is `properties['customer_id']`, not this column.
    let group_by_customer = params.groups_by_customer();
    let segments: Vec<&str> = params
        .segmentation_filter
        .as_ref()
        .filter(|_| !params.merge_segments)
        .map(segment_properties)
        .unwrap_or_default();
    let breakdown: Vec<&str> = params
        .group_by
        .iter()
        .map(String::as_str)
        .chain(segments)
        .collect();
    if group_by_customer {
        items.push(Item::new(Expr::col("customer_id")));
    }
    items.extend(breakdown.iter().map(|p| property_item(p)));

    let mut group_by = Vec::new();
    if let Some(window) = &params.window_size {
        group_by.push(tumble("tumbleStart", window, &tz));
        group_by.push(tumble("tumbleEnd", window, &tz));
    }
    if group_by_customer {
        group_by.push(Expr::col("customer_id"));
    }
    group_by.extend(breakdown.iter().map(|p| property(p)));

    let read = source.read(columns(METER_EVENT_COLUMNS), filter, scope);
    let mut select = Select::new(items, read)
        .filter(
            params
                .attribution
                .as_ref()
                .and_then(attribution_filter)
                .into_iter()
                .collect(),
        )
        .group_by(group_by);
    if params.window_size.is_some() {
        select = select.order_by_window_start();
    }
    Ok(select.query())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        MeterAggregation, PropertyFilter, PropertyFilterOp, QueryMeterParams, SegmentationFilter,
        WindowSize,
    };
    use chrono::{TimeZone, Utc};
    use common_domain::ids::{CustomerId, TenantId};
    use metric_core::sql::render::pretty;
    use std::collections::HashMap;
    use uuid::Uuid;

    /// The rendered query, and its review form for the snapshot.
    fn meter(params: QueryMeterParams) -> (SafeQuery, String) {
        let tree = meter_read(params.clone(), &"raw_events_v2".into(), "meteroid").unwrap();
        let rendered = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        (rendered, pretty(&tree, &[]))
    }

    /// As [`meter`]; an open-ended read ends now, which the snapshot shows as `NOW`.
    fn raw_events(params: QueryRawEventsParams) -> (SafeQuery, String) {
        let before = Utc::now().timestamp();
        let tree = raw_events_read(params.clone(), &"raw_events_v2".into(), "meteroid").unwrap();
        let rendered = query_raw_events_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let mut snapshot = pretty(&tree, &[]);
        for now in before..=Utc::now().timestamp() {
            snapshot = snapshot.replace(&now.to_string(), "NOW");
        }
        (rendered, snapshot)
    }

    /// Whitespace-insensitive, with the void filter (tested in metric-core) reduced to `LIVE`.
    fn normalize_sql(sql: &str) -> String {
        const LIVE: &str = "AND is_tombstone = 0 AND (";
        let sql = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        let sql = sql.replace("( ", "(").replace(" )", ")");
        let Some(start) = sql.find(LIVE) else {
            return sql;
        };
        let mut depth = 0;
        let mut end = sql.len();
        for (i, c) in sql[start + LIVE.len() - 1..].char_indices() {
            depth += match c {
                '(' => 1,
                ')' => -1,
                _ => 0,
            };
            if depth == 0 {
                end = start + LIVE.len() + i;
                break;
            }
        }
        format!("{}AND LIVE{}", &sql[..start], &sql[end..])
    }

    fn assert_bind_parity(result: &SafeQuery) {
        let placeholders = result.placeholders();
        assert_eq!(
            placeholders,
            result.binds.len(),
            "placeholder/bind count mismatch ({} placeholders vs {} binds) for SQL:\n{}",
            placeholders,
            result.binds.len(),
            result.sql
        );
    }

    fn bind_strings(binds: &[BindValue]) -> Vec<String> {
        binds
            .iter()
            .map(|b| match b {
                BindValue::String(s) => format!("S:{s}"),
                BindValue::Strings(v) => format!("A:{}", v.join(",")),
                BindValue::I64(v) => format!("I:{v}"),
                BindValue::U32(v) => format!("U:{v}"),
                BindValue::Uuid(v) => format!("Uuid:{v}"),
                BindValue::Uuids(v) => {
                    format!(
                        "AUuid:{}",
                        v.iter()
                            .map(|u| u.to_string())
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                }
            })
            .collect()
    }

    fn value_params(aggregation: MeterAggregation) -> QueryMeterParams {
        QueryMeterParams {
            aggregation,
            tenant_id: TenantId::default(),
            code: "c".to_string(),
            value_property: Some("v".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            merge_segments: false,
            attribution: None,
            origin: OriginFilter::All,
            group_by_customer: false,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            keep_last_events: false,
        }
    }

    #[test]
    fn test_query_meter_min_max_return_the_winning_string() {
        for aggregation in [MeterAggregation::Min, MeterAggregation::Max] {
            let (result, pretty) = meter(value_params(aggregation));
            insta::assert_snapshot!(pretty);
            assert_bind_parity(&result);
        }
    }

    #[test]
    fn test_query_meter_window_bounds_truncate_to_whole_seconds() {
        let params = QueryMeterParams {
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
                + chrono::Duration::milliseconds(999),
            to: Some(
                Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()
                    + chrono::Duration::nanoseconds(1),
            ),
            ..value_params(MeterAggregation::Sum)
        };
        let bs = bind_strings(
            &query_meter_sql(params, &"raw_events_v2".into(), "meteroid")
                .unwrap()
                .binds,
        );
        assert!(bs.contains(&"I:1704067200".to_string()), "{bs:?}");
        assert!(bs.contains(&"I:1704153600".to_string()), "{bs:?}");
    }

    #[test]
    fn test_query_meter_with_minute_window() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "test_event".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: Some(WindowSize::Minute),
            window_time_zone: Some(chrono_tz::UTC),
            filters: vec![],
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:test_event".to_string()));
        assert!(bs.contains(&"S:amount".to_string()));
        assert!(bs.contains(&"I:1704067200".to_string()));
        assert!(bs.contains(&"I:1704153600".to_string()));
        assert!(bs.contains(&"S:UTC".to_string()));
    }

    #[test]
    fn test_query_meter_with_count() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "api_call".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:api_call".to_string()));
        assert!(bs.contains(&"I:1704067200".to_string()));
        assert!(bs.contains(&"I:1704153600".to_string()));
    }

    #[test]
    fn test_query_meter_with_customer_filter() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "usage".to_string(),
            value_property: Some("bytes".to_string()),
            customer_ids: vec![
                CustomerId::from(Uuid::from_u128(1)),
                CustomerId::from(Uuid::from_u128(2)),
            ],
            segmentation_filter: None,
            group_by: vec![],
            window_size: Some(WindowSize::Hour),
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:usage".to_string()));
        assert!(bs.contains(&"S:bytes".to_string()));
        assert!(
            bs.contains(
                &"AUuid:00000000-0000-0000-0000-000000000001,00000000-0000-0000-0000-000000000002"
                    .to_string()
            )
        );
    }

    #[test]
    fn test_query_meter_group_by_customer_without_customer_filter() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: true,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "api_call".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec!["region".to_string()],
            window_size: Some(WindowSize::Day),
            window_time_zone: None,
            filters: vec![],
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap()),
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        assert_bind_parity(&result);
        let tenant = format!("Uuid:{}", Uuid::max());
        assert_eq!(
            bind_strings(&result.binds),
            vec![
                "S:UTC",
                "S:UTC",
                "S:region",
                tenant.as_str(),
                "S:api_call",
                "I:1704067200",
                "I:1706745600",
                "S:UTC",
                "S:UTC",
                "S:region",
            ]
        );
    }

    #[test]
    fn test_query_meter_customer_id_property_is_grouped_alongside_the_column() {
        // A `customer_id` group-by is the event property, so the column must still be grouped.
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "api_call".to_string(),
            value_property: None,
            customer_ids: vec![CustomerId::from(Uuid::from_u128(1))],
            segmentation_filter: None,
            group_by: vec!["customer_id".to_string()],
            window_size: None,
            window_time_zone: None,
            filters: vec![],
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let sql = normalize_sql(&result.sql);
        assert!(sql.contains(
            "toString(count()) AS value, customer_id, properties[?] AS _prop_customer_id"
        ));
        assert!(sql.ends_with("GROUP BY customer_id, properties[?]"));
        assert_bind_parity(&result);
    }

    #[test]
    fn test_query_meter_with_group_by() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Avg,
            tenant_id: TenantId::default(),
            code: "transaction".to_string(),
            value_property: Some("duration".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec!["region".to_string(), "endpoint".to_string()],
            window_size: Some(WindowSize::Day),
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:transaction".to_string()));
        assert!(bs.contains(&"S:duration".to_string()));
        assert!(bs.contains(&"S:region".to_string()));
        assert!(bs.contains(&"S:endpoint".to_string()));
    }

    #[test]
    fn test_query_meter_with_independent_segmentation() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "sale".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: Some(SegmentationFilter::Independent(vec![
                (
                    "region".to_string(),
                    vec!["us-east".to_string(), "us-west".to_string()],
                ),
                ("tier".to_string(), vec!["premium".to_string()]),
            ])),
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:sale".to_string()));
        assert!(bs.contains(&"S:amount".to_string()));
        assert!(bs.contains(&"S:region".to_string()));
        assert!(bs.contains(&"S:tier".to_string()));
        assert!(bs.contains(&"S:us-east".to_string()));
        assert!(bs.contains(&"S:us-west".to_string()));
        assert!(bs.contains(&"S:premium".to_string()));
    }

    #[test]
    fn test_query_meter_property_filter_in_is_where_only() {
        // An `In` filter adds a WHERE condition but NO SELECT/GROUP BY column (unlike
        // segmentation), so the rated result shape is unchanged.
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "usage".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            filters: vec![PropertyFilter {
                property: "operation".to_string(),
                op: PropertyFilterOp::In,
                values: vec!["write".to_string(), "delete".to_string()],
            }],
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);
        insta::assert_snapshot!(pretty);
        assert_bind_parity(&result);
        // No grouping/select column for the filtered property.
        assert!(!result.sql.contains("_prop_operation"));
        assert!(!result.sql.contains("GROUP BY"));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:operation".to_string()));
        assert!(bs.contains(&"S:write".to_string()));
        assert!(bs.contains(&"S:delete".to_string()));
    }

    #[test]
    fn test_query_meter_property_filter_not_in_requires_presence() {
        // `NotIn` is presence-required: mapContains guards against an absent key passing `!=`.
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "usage".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            filters: vec![PropertyFilter {
                property: "tier".to_string(),
                op: PropertyFilterOp::NotIn,
                values: vec!["free".to_string()],
            }],
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);
        insta::assert_snapshot!(pretty);
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:tier".to_string()));
        assert!(bs.contains(&"S:free".to_string()));
    }

    fn attributed(attribution: Option<EventAttribution>) -> QueryMeterParams {
        QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "tokens".to_string(),
            value_property: Some("tokens".to_string()),
            customer_ids: vec![CustomerId::default()],
            segmentation_filter: None,
            filters: vec![],
            group_by: vec!["model".to_string()],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()),
            keep_last_events: false,
        }
    }

    #[test]
    fn test_query_meter_attribution_filters_the_deduplicated_events() {
        use crate::domain::{AttributionSegment, EventClaim, EventMatch};
        let change = Utc.with_ymd_and_hms(2026, 9, 10, 0, 0, 0).unwrap();
        let claim_to = Utc.with_ymd_and_hms(2026, 9, 20, 14, 30, 0).unwrap();
        let (result, pretty) = meter(attributed(Some(EventAttribution {
            segments: vec![
                AttributionSegment {
                    from: None,
                    to: Some(change),
                    matching: Some(EventMatch {
                        properties: vec!["project".to_string()],
                        value: "p1".to_string(),
                    }),
                    excluded: vec![],
                },
                AttributionSegment {
                    from: Some(change),
                    to: None,
                    matching: None,
                    excluded: vec![EventClaim {
                        properties: vec!["api_key".to_string()],
                        value: "key-A".to_string(),
                        from: change,
                        to: Some(claim_to),
                    }],
                },
            ],
        })));
        insta::assert_snapshot!(pretty);
        assert_bind_parity(&result);
        let binds = bind_strings(&result.binds);
        let attribution_binds = &binds[binds.len() - 9..binds.len() - 1];
        assert_eq!(
            attribution_binds,
            [
                format!("I:{}", change.timestamp()),
                "S:project".to_string(),
                "S:p1".to_string(),
                format!("I:{}", change.timestamp()),
                "S:api_key".to_string(),
                "S:key-A".to_string(),
                format!("I:{}", change.timestamp()),
                format!("I:{}", claim_to.timestamp()),
            ]
        );
        assert_eq!(binds.last(), Some(&"S:model".to_string()));
    }

    #[test]
    fn test_attribution_reads_the_first_key_present() {
        use crate::domain::{AttributionSegment, EventMatch};
        let result = query_meter_sql(
            attributed(Some(EventAttribution {
                segments: vec![AttributionSegment {
                    from: None,
                    to: None,
                    matching: Some(EventMatch {
                        properties: vec!["site_id".to_string(), "site".to_string()],
                        value: "s1".to_string(),
                    }),
                    excluded: vec![],
                }],
            })),
            &"raw_events_v2".into(),
            "meteroid",
        )
        .unwrap();
        assert!(
            normalize_sql(&result.sql)
                .contains("WHERE multiIf(properties[?] != '', properties[?], properties[?]) = ?"),
            "{}",
            result.sql
        );
        assert_bind_parity(&result);
    }

    #[test]
    fn test_keep_last_tenant_attributes_the_last_ingested_copy() {
        use crate::domain::{AttributionSegment, EventMatch};
        let mut params = attributed(Some(EventAttribution {
            segments: vec![AttributionSegment {
                from: None,
                to: None,
                matching: Some(EventMatch {
                    properties: vec!["project".to_string()],
                    value: "p1".to_string(),
                }),
                excluded: vec![],
            }],
        }));
        params.keep_last_events = true;
        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        assert!(
            normalize_sql(&result.sql).contains(
                "ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id) WHERE properties[?] = ?"
            ),
            "{}",
            result.sql
        );
        assert_bind_parity(&result);
    }

    #[test]
    fn test_query_meter_without_attribution_is_unchanged() {
        let none = query_meter_sql(attributed(None), &"raw_events_v2".into(), "meteroid").unwrap();
        let empty = query_meter_sql(
            attributed(Some(EventAttribution::default())),
            &"raw_events_v2".into(),
            "meteroid",
        )
        .unwrap();
        assert_eq!(none.sql, empty.sql);
        assert_eq!(bind_strings(&none.binds), bind_strings(&empty.binds));
        assert!(!none.sql.contains(") WHERE"));
    }

    #[test]
    fn test_query_meter_empty_property_filters_is_noop() {
        // The safety invariant: no filters ⇒ identical SQL to the pre-feature baseline.
        let base = |filters: Vec<PropertyFilter>| QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "event".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            filters,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            keep_last_events: false,
        };
        let empty = query_meter_sql(base(vec![]), &"raw_events_v2".into(), "meteroid").unwrap();
        // A filter with no values is skipped, so it must produce the same SQL as no filter.
        let empty_values = query_meter_sql(
            base(vec![PropertyFilter {
                property: "x".to_string(),
                op: PropertyFilterOp::In,
                values: vec![],
            }]),
            &"raw_events_v2".into(),
            "meteroid",
        )
        .unwrap();
        assert_eq!(normalize_sql(&empty.sql), normalize_sql(&empty_values.sql));
    }

    #[test]
    fn test_query_meter_with_linked_segmentation() {
        let mut values = HashMap::new();
        values.insert(
            "prod1".to_string(),
            vec!["v1".to_string(), "v2".to_string()],
        );
        values.insert("prod2".to_string(), vec!["v3".to_string()]);

        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "usage".to_string(),
            value_property: Some("count".to_string()),
            customer_ids: vec![],
            segmentation_filter: Some(SegmentationFilter::Linked {
                dimension1_key: "product".to_string(),
                dimension2_key: "version".to_string(),
                values,
            }),
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();

        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:usage".to_string()));
        assert!(bs.contains(&"S:count".to_string()));
        assert!(bs.contains(&"S:product".to_string()));
        assert!(bs.contains(&"S:version".to_string()));
        assert!(bs.contains(&"S:prod1".to_string()));
        assert!(bs.contains(&"S:prod2".to_string()));
    }

    #[test]
    fn test_query_meter_merged_segments_filter_without_breakdown() {
        let customer = CustomerId::from(Uuid::from_u128(1));
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: true,
            aggregation: MeterAggregation::Max,
            tenant_id: TenantId::default(),
            code: "usage".to_string(),
            value_property: Some("count".to_string()),
            customer_ids: vec![customer],
            segmentation_filter: Some(SegmentationFilter::Linked {
                dimension1_key: "product".to_string(),
                dimension2_key: "version".to_string(),
                values: HashMap::from([("prod1".to_string(), vec!["v1".to_string()])]),
            }),
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:prod1".to_string()));
        assert!(bs.contains(&"S:product".to_string()));
    }

    #[test]
    fn test_query_meter_with_count_distinct() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::CountDistinct,
            tenant_id: TenantId::default(),
            code: "login".to_string(),
            value_property: Some("user_id".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: Some(WindowSize::Day),
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:login".to_string()));
        assert!(bs.contains(&"S:user_id".to_string()));
    }

    #[test]
    fn test_query_meter_with_latest_aggregation() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Latest,
            tenant_id: TenantId::default(),
            code: "status".to_string(),
            value_property: Some("value".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:status".to_string()));
        assert!(bs.contains(&"S:value".to_string()));
    }

    #[test]
    fn test_query_meter_validates_empty_segmentation() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "test".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: Some(SegmentationFilter::Independent(vec![(
                "region".to_string(),
                vec![],
            )])),
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Empty filter for dimension: region");
    }

    #[test]
    fn test_query_meter_validates_value_property_required() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "test".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "value_property is required for non-Count aggregations"
        );
    }

    #[test]
    fn test_query_meter_includes_value_property_filter() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "purchase".to_string(),
            value_property: Some("price".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:purchase".to_string()));
        assert!(bs.contains(&"S:price".to_string()));
    }

    #[test]
    fn test_query_meter_time_filters_in_cte() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "event".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:event".to_string()));
        assert!(bs.contains(&"I:1704067200".to_string()));
        assert!(bs.contains(&"I:1704153600".to_string()));
    }

    #[test]
    fn test_query_meter_upper_bound_is_exclusive() {
        // Consecutive billing periods share their boundary instant (arrear end == next advance
        // start), so an inclusive upper bound would rate the boundary event on both invoices.
        let boundary = Utc.with_ymd_and_hms(2026, 7, 31, 0, 0, 0).unwrap();
        let params = |from, to| QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "memory".to_string(),
            value_property: Some("gib".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],
            from,
            to: Some(to),
            keep_last_events: false,
        };

        let first = query_meter_sql(
            params(
                Utc.with_ymd_and_hms(2026, 7, 16, 0, 0, 0).unwrap(),
                boundary,
            ),
            &"raw_events_v2".into(),
            "meteroid",
        )
        .unwrap();
        let second = query_meter_sql(
            params(
                boundary,
                Utc.with_ymd_and_hms(2026, 8, 31, 0, 0, 0).unwrap(),
            ),
            &"raw_events_v2".into(),
            "meteroid",
        )
        .unwrap();

        assert!(first.sql.contains("timestamp < toDateTime(?)"));
        assert!(!first.sql.contains("timestamp <= toDateTime(?)"));
        assert!(second.sql.contains("timestamp >= toDateTime(?)"));
    }

    #[test]
    fn test_query_meter_with_reserved_column_in_group_by() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "purchase".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec!["code".to_string()],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:purchase".to_string()));
        assert!(bs.contains(&"S:amount".to_string()));
        assert!(bs.contains(&"S:code".to_string()));
    }

    #[test]
    fn test_query_meter_with_customer_id_in_segmentation() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "usage".to_string(),
            value_property: Some("bytes".to_string()),
            customer_ids: vec![],
            segmentation_filter: Some(SegmentationFilter::Independent(vec![(
                "customer_id".to_string(),
                vec!["cust1".to_string(), "cust2".to_string()],
            )])),
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:usage".to_string()));
        assert!(bs.contains(&"S:bytes".to_string()));
        assert!(bs.contains(&"S:customer_id".to_string()));
        assert!(bs.contains(&"S:cust1".to_string()));
        assert!(bs.contains(&"S:cust2".to_string()));
    }

    #[test]
    fn test_query_meter_mixed_reserved_and_custom_columns() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "event".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![
                "customer_id".to_string(),
                "region".to_string(),
                "code".to_string(),
            ],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let (result, pretty) = meter(params);

        insta::assert_snapshot!(pretty);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:event".to_string()));
        assert!(bs.contains(&"S:customer_id".to_string()));
        assert!(bs.contains(&"S:region".to_string()));
        assert!(bs.contains(&"S:code".to_string()));
    }

    #[test]
    fn test_query_raw_events_basic() {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            limit: 10,
            offset: 0,
            search: None,
            event_codes: vec![],
            customer_ids: vec![],
            sort_order: EventSortOrder::TimestampDesc,
            filters: vec![],
            after: None,
            keep_last_events: false,
        };

        let (result, pretty) = raw_events(params);

        insta::assert_snapshot!(pretty);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"Uuid:ffffffff-ffff-ffff-ffff-ffffffffffff".to_string()));
        assert!(bs.contains(&"I:1704067200".to_string()));
        assert!(bs.contains(&"I:1704153600".to_string()));
        assert!(bs.contains(&"U:10".to_string()));
        assert!(bs.contains(&"U:0".to_string()));
    }

    #[test]
    fn test_query_raw_events_with_customer_and_code_filters() {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            limit: 20,
            offset: 5,
            search: None,
            event_codes: vec!["api_call".to_string(), "storage".to_string()],
            customer_ids: vec![
                CustomerId::from(Uuid::from_u128(1)),
                CustomerId::from(Uuid::from_u128(2)),
            ],
            sort_order: EventSortOrder::IngestedDesc,
            filters: vec![],
            after: None,
            keep_last_events: false,
        };

        let (result, pretty) = raw_events(params);

        insta::assert_snapshot!(pretty);
        let voids = "FROM meteroid.event_voids_local WHERE tenant_id = \
            toUUID('ffffffff-ffff-ffff-ffff-ffffffffffff') AND code IN ('api_call', 'storage') AND \
            customer_id IN (toUUID('00000000-0000-0000-0000-000000000001'), \
            toUUID('00000000-0000-0000-0000-000000000002')) AND timestamp >= \
            toDateTime(1704067200) AND timestamp <= toDateTime(1704153600)";
        assert_eq!(result.sql.matches(voids).count(), 2, "{}", result.sql);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"A:api_call,storage".to_string()));
        assert!(bs.contains(&"U:20".to_string()));
        assert!(bs.contains(&"U:5".to_string()));
    }

    #[test]
    fn test_query_raw_events_with_property_filters() {
        // The events explorer applies the metric's filters (IN + presence-required NOT_IN)
        // to the events it lists, so it lists only the events the metric would count.
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            limit: 10,
            offset: 0,
            search: None,
            event_codes: vec![],
            customer_ids: vec![],
            filters: vec![
                PropertyFilter {
                    property: "region".to_string(),
                    op: PropertyFilterOp::In,
                    values: vec!["eu-west".to_string(), "us-east".to_string()],
                },
                PropertyFilter {
                    property: "tier".to_string(),
                    op: PropertyFilterOp::NotIn,
                    values: vec!["free".to_string()],
                },
            ],
            sort_order: EventSortOrder::TimestampDesc,
            after: None,
            keep_last_events: false,
        };

        let result = query_raw_events_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let n = normalize_sql(&result.sql);
        assert!(
            n.contains("(properties[?] = ? OR properties[?] = ?)"),
            "{n}"
        );
        assert!(
            n.contains("AND mapContains(properties, ?) AND properties[?] != ?"),
            "{n}"
        );
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:region".to_string()));
        assert!(bs.contains(&"S:eu-west".to_string()));
        assert!(bs.contains(&"S:tier".to_string()));
        assert!(bs.contains(&"S:free".to_string()));
    }

    #[test]
    fn test_query_raw_events_with_search() {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            limit: 10,
            offset: 0,
            search: Some("foo".to_string()),
            event_codes: vec![],
            customer_ids: vec![],
            sort_order: EventSortOrder::TimestampAsc,
            filters: vec![],
            after: None,
            keep_last_events: false,
        };

        let (result, pretty) = raw_events(params);

        insta::assert_snapshot!(pretty);
        let bs = bind_strings(&result.binds);
        assert_eq!(bs.iter().filter(|b| *b == "S:%foo%").count(), 3);
    }

    #[test]
    fn question_marks_in_codes_and_search_stay_literal() {
        for keep_last_events in [false, true] {
            let params = QueryRawEventsParams {
                tenant_id: TenantId::default(),
                from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                to: None,
                limit: 10,
                offset: 0,
                search: Some("x?y".to_string()),
                event_codes: vec!["a?b".to_string()],
                customer_ids: vec![],
                sort_order: EventSortOrder::TimestampAsc,
                filters: vec![],
                after: None,
                keep_last_events,
            };
            let result = query_raw_events_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
            assert!(result.sql.contains("'a??b'"), "{}", result.sql);
            assert_bind_parity(&result);
        }
    }

    #[test]
    fn test_query_raw_events_keyset() {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            limit: 10,
            offset: 0,
            search: None,
            event_codes: vec![],
            customer_ids: vec![],
            sort_order: EventSortOrder::TimestampAsc,
            filters: vec![],
            after: Some(crate::domain::EventCursor {
                timestamp_nanos: 1_704_067_200_000_000_001,
                id: "evt_1".to_string(),
            }),
            keep_last_events: false,
        };

        let (result, pretty) = raw_events(params);
        insta::assert_snapshot!(pretty);
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert_eq!(
            bs.iter().filter(|b| *b == "I:1704067200000000001").count(),
            2
        );
        assert!(bs.contains(&"S:evt_1".to_string()));
    }

    #[test]
    fn test_query_raw_events_keyset_rejects_ingested_order() {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            limit: 10,
            offset: 0,
            search: None,
            event_codes: vec![],
            customer_ids: vec![],
            sort_order: EventSortOrder::IngestedAsc,
            filters: vec![],
            after: Some(crate::domain::EventCursor {
                timestamp_nanos: 0,
                id: String::new(),
            }),
            keep_last_events: false,
        };
        assert!(query_raw_events_sql(params, &"raw_events_v2".into(), "meteroid").is_err());
    }

    fn count_meter_sql(keep_last_events: bool) -> (SafeQuery, String) {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "api_call".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            keep_last_events,
        };
        let (result, pretty) = meter(params);
        assert_bind_parity(&result);
        (result, pretty)
    }

    #[test]
    fn test_query_meter_keep_last_tenant_keeps_last_ingested() {
        insta::assert_snapshot!(count_meter_sql(true).1);
    }

    #[test]
    fn test_query_meter_default_tenant_has_no_sort() {
        let (with, _) = count_meter_sql(true);
        let (without, pretty) = count_meter_sql(false);
        insta::assert_snapshot!(pretty);
        assert_eq!(bind_strings(&with.binds), bind_strings(&without.binds));
    }

    fn raw_events_sql(keep_last_events: bool) -> String {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            limit: 10,
            offset: 0,
            search: None,
            event_codes: vec![],
            customer_ids: vec![],
            sort_order: EventSortOrder::IngestedDesc,
            filters: vec![],
            after: None,
            keep_last_events,
        };
        let (result, pretty) = raw_events(params);
        assert_bind_parity(&result);
        pretty
    }

    #[test]
    fn test_query_raw_events_keep_last_tenant_keeps_last_ingested() {
        insta::assert_snapshot!(raw_events_sql(true));
    }

    #[test]
    fn test_query_raw_events_default_tenant_reads_the_table_flat() {
        insta::assert_snapshot!(raw_events_sql(false));
    }

    #[test]
    fn test_query_raw_events_keep_last_keyset_filters_the_deduplicated_events() {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            limit: 1,
            offset: 0,
            search: None,
            event_codes: vec![],
            customer_ids: vec![],
            sort_order: EventSortOrder::TimestampAsc,
            filters: vec![],
            after: Some(crate::domain::EventCursor {
                timestamp_nanos: 7,
                id: "evt_1".to_string(),
            }),
            keep_last_events: true,
        };
        let (result, pretty) = raw_events(params);
        insta::assert_snapshot!(pretty);
        assert_bind_parity(&result);
        assert_eq!(
            bind_strings(&result.binds)[3..],
            ["I:7", "I:7", "S:evt_1", "U:1", "U:0"]
        );
    }
}
