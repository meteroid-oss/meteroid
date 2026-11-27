//! A corpus of metric queries covering every metric kind, source, dimension, filter, transform
//! and cohort predicate, and the snapshot form of a compiled tree. Shared by the golden snapshots
//! and the analytics test that runs each compiled query on ClickHouse.

use crate::domain::WindowSize;
use crate::metric::*;
use crate::sql::ast::{Query, Setting};
use crate::sql::{render, render_scoped};
use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, TenantId};

/// The review form of a tree, once its tenant scope and its binds are checked.
pub fn snapshot(tree: &Query, settings: &[Setting], tenant: uuid::Uuid) -> String {
    let rendered = render_scoped(tree, settings, tenant).expect("tenant-scoped");
    assert_eq!(
        rendered.placeholders(),
        rendered.binds.len(),
        "placeholders != binds:\n{}",
        rendered.sql
    );
    render::pretty(tree, settings)
}

/// An insta snapshot name for a case: letters, digits and `_` only.
pub fn snapshot_name(case: &str) -> String {
    case.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn ts(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

pub fn tenant() -> TenantId {
    uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111")
        .unwrap()
        .into()
}

fn customer(n: u8) -> CustomerId {
    uuid::Uuid::from_bytes([n; 16]).into()
}

fn fact(table: &str, value: &str) -> FactSource {
    FactSource {
        table: table.to_string(),
        value_column: value.to_string(),
        timestamp_column: "at".to_string(),
        customer_column: "customer_id".to_string(),
        tenant_column: "tenant_id".to_string(),
        snapshot: false,
    }
}

fn gauge(key: &str, op: GaugeOp, carry_over: bool) -> MetricDef {
    MetricDef {
        key: key.to_string(),
        event_code: "storage".to_string(),
        kind: MetricKind::Gauge(GaugeSpec {
            op,
            value_property: Some("gb".to_string()),
            carry_over,
        }),
        source: MetricSource::RawEvents,
        filters: vec![],
    }
}

fn metric(key: &str) -> Expr {
    Expr::Metric(key.to_string())
}

fn bin(l: Expr, op: BinOp, r: Expr) -> Expr {
    Expr::Binary(Box::new(l), op, Box::new(r))
}

pub fn registry() -> MetricRegistry {
    let mut r = MetricRegistry::new();
    for (key, op) in [
        ("raw_sum", AggregateOp::Sum),
        ("raw_avg", AggregateOp::Avg),
        ("raw_min", AggregateOp::Min),
        ("raw_max", AggregateOp::Max),
        ("raw_latest", AggregateOp::Latest),
        ("raw_distinct", AggregateOp::CountDistinct),
    ] {
        r.insert(MetricDef::aggregate(key, "api_call", op, Some("tokens")));
    }
    r.insert(MetricDef::aggregate(
        "raw_count",
        "api_call",
        AggregateOp::Count,
        None,
    ));
    r.insert(
        MetricDef::aggregate("raw_converted", "api_call", AggregateOp::Sum, Some("bytes"))
            .with_unit_conversion_factor(Some(1024.0)),
    );
    r.insert(
        MetricDef::aggregate("raw_filtered", "api_call", AggregateOp::Sum, Some("tokens"))
            .with_filter("region", &["eu", "us"]),
    );
    r.insert(MetricDef::aggregate(
        "raw_question_mark",
        "api?call'??",
        AggregateOp::Count,
        None,
    ));

    r.insert(gauge("gauge_latest", GaugeOp::Latest, false));
    r.insert(gauge("gauge_max", GaugeOp::Max, false));
    r.insert(gauge("gauge_carry", GaugeOp::Latest, true));
    r.insert(gauge(
        "tw_seconds",
        GaugeOp::TimeWeighted(TimeWeightedSpec {
            identity_property: "instance_id".to_string(),
            lifecycle_codes: vec![],
            weight: TimeWeight::Seconds,
        }),
        false,
    ));
    r.insert(gauge(
        "tw_value",
        GaugeOp::TimeWeighted(TimeWeightedSpec {
            identity_property: "instance_id".to_string(),
            lifecycle_codes: vec!["vm_start".to_string(), "vm_stop".to_string()],
            weight: TimeWeight::ValueSeconds("vcpu".to_string()),
        }),
        false,
    ));

    for (key, op) in [
        ("fact_sum", AggregateOp::Sum),
        ("fact_avg", AggregateOp::Avg),
        ("fact_min", AggregateOp::Min),
        ("fact_max", AggregateOp::Max),
        ("fact_count", AggregateOp::Count),
        ("fact_distinct", AggregateOp::CountDistinct),
        ("fact_latest", AggregateOp::Latest),
    ] {
        r.insert(MetricDef::fact_aggregate(
            key,
            op,
            fact("fact_mrr_movement", "delta_cents_usd"),
        ));
    }
    let mut snapshot = fact("fact_mrr_snapshot", "mrr_cents_usd");
    snapshot.timestamp_column = "snapshot_date".to_string();
    snapshot.snapshot = true;
    r.insert(MetricDef::fact_aggregate(
        "snapshot_mrr",
        AggregateOp::Sum,
        snapshot,
    ));
    r.insert(
        MetricDef::fact_aggregate(
            "mrr_level",
            AggregateOp::Sum,
            fact("fact_mrr_movement", "delta_cents_usd"),
        )
        .as_of(),
    );
    for (key, movement) in [
        ("churn", "churn"),
        ("expansion", "expansion"),
        ("contraction", "contraction"),
    ] {
        r.insert(
            MetricDef::fact_aggregate(
                key,
                AggregateOp::Sum,
                fact("fact_mrr_movement", "delta_cents_usd"),
            )
            .with_filter("movement_type", &[movement]),
        );
    }
    r.insert(
        MetricDef::fact_aggregate(
            "churn_or_contraction_by_plan",
            AggregateOp::Sum,
            fact("fact_mrr_movement", "delta_cents_usd"),
        )
        .with_filter("movement_type", &["churn", "contraction"])
        .with_filter("plan_id", &["99999999-9999-9999-9999-999999999999"]),
    );

    let opening = Expr::MetricTransformed {
        metric: "fact_sum".to_string(),
        transform: CrossPeriod::RunningTotalLagged,
    };
    r.insert(MetricDef::derived(
        "d_arith",
        bin(
            bin(metric("raw_sum"), BinOp::Add, metric("raw_count")),
            BinOp::Mul,
            bin(Expr::Const(2.5), BinOp::Sub, metric("raw_avg")),
        ),
    ));
    r.insert(MetricDef::derived(
        "d_div",
        bin(metric("raw_sum"), BinOp::Div, metric("raw_count")),
    ));
    r.insert(MetricDef::derived(
        "d_scalar",
        Expr::Clamp {
            expr: Box::new(Expr::Unary(
                UnaryOp::Neg,
                Box::new(Expr::Unary(UnaryOp::Abs, Box::new(metric("fact_sum")))),
            )),
            min: Some(Box::new(Expr::Const(0.0))),
            max: Some(Box::new(Expr::Least(vec![
                metric("fact_max"),
                Expr::Const(100.0),
            ]))),
        },
    ));
    r.insert(MetricDef::derived(
        "d_lists",
        Expr::Coalesce(vec![
            Expr::Greatest(vec![metric("raw_sum"), metric("fact_sum")]),
            Expr::Const(0.0),
        ]),
    ));
    r.insert(MetricDef::derived(
        "d_retention",
        bin(
            bin(metric("churn"), BinOp::Add, metric("contraction")),
            BinOp::Div,
            opening.clone(),
        ),
    ));
    r.insert(MetricDef::derived(
        "d_fused_with_plain",
        bin(
            bin(metric("churn"), BinOp::Add, metric("expansion")),
            BinOp::Sub,
            metric("fact_sum"),
        ),
    ));
    r.insert(MetricDef::derived(
        "d_fused_multi_filter",
        bin(
            metric("churn_or_contraction_by_plan"),
            BinOp::Add,
            metric("expansion"),
        ),
    ));
    r.insert(MetricDef::derived(
        "d_transforms",
        bin(
            Expr::MetricTransformed {
                metric: "raw_sum".to_string(),
                transform: CrossPeriod::MovingAverage { periods: 3 },
            },
            BinOp::Add,
            bin(
                Expr::MetricTransformed {
                    metric: "raw_sum".to_string(),
                    transform: CrossPeriod::PercentChange,
                },
                BinOp::Sub,
                Expr::MetricTransformed {
                    metric: "fact_sum".to_string(),
                    transform: CrossPeriod::PeriodOverPeriodDelta,
                },
            ),
        ),
    ));
    r.insert(MetricDef::derived(
        "d_running",
        bin(
            Expr::MetricTransformed {
                metric: "fact_sum".to_string(),
                transform: CrossPeriod::RunningTotal,
            },
            BinOp::Div,
            Expr::Const(100.0),
        ),
    ));
    r.insert(MetricDef::derived("d_single", metric("raw_sum")));
    r
}

pub fn query(metric: &str) -> MetricQuery {
    MetricQuery {
        tenant_id: tenant(),
        metric: metric.to_string(),
        from: ts("2026-01-05T00:00:00Z"),
        to: Some(ts("2026-02-01T00:00:00Z")),
        window: Some(WindowSize::Day),
        window_time_zone: None,
        group_by: vec![],
        customer_ids: vec![],
        filters: vec![],
        transform: None,
        cohort: None,
        reporting_currency: None,
        keep_last_events: false,
    }
}

fn filter(property: &str, values: &[&str], op: FilterOp) -> DimensionFilter {
    DimensionFilter {
        property: property.to_string(),
        values: values.iter().map(|v| v.to_string()).collect(),
        op,
    }
}

fn dimension_cohort() -> CohortPredicate {
    CohortPredicate::Dimension {
        table: "dim_customer".to_string(),
        id_column: "id".to_string(),
        column: "country".to_string(),
        values: vec!["FR".to_string(), "DE".to_string()],
    }
}

fn cohort_predicates() -> Vec<(&'static str, CohortPredicate)> {
    let custom = |op: PropertyOp, value_type: PropertyValueType, values: &[&str]| {
        CohortPredicate::CustomProperty {
            table: "dim_customer".to_string(),
            id_column: "id".to_string(),
            json_column: "custom_properties".to_string(),
            key: "seats".to_string(),
            op,
            value_type,
            values: values.iter().map(|v| v.to_string()).collect(),
        }
    };
    let mut out = vec![
        ("dimension", dimension_cohort()),
        (
            "dimension_map",
            CohortPredicate::DimensionMap {
                table: "dim_customer".to_string(),
                id_column: "id".to_string(),
                map_column: "attributes".to_string(),
                key: "segment".to_string(),
                values: vec!["smb".to_string()],
            },
        ),
        (
            "active_plan",
            CohortPredicate::ActivePlan {
                plan_ids: vec!["p1".to_string(), "p2".to_string()],
            },
        ),
        (
            "threshold_raw",
            CohortPredicate::MetricThreshold {
                metric: "raw_sum".to_string(),
                op: ThresholdOp::Gte,
                threshold: 100.0,
            },
        ),
        (
            "threshold_raw_count",
            CohortPredicate::MetricThreshold {
                metric: "raw_count".to_string(),
                op: ThresholdOp::Gt,
                threshold: 0.5,
            },
        ),
        (
            "threshold_raw_converted",
            CohortPredicate::MetricThreshold {
                metric: "raw_converted".to_string(),
                op: ThresholdOp::Lt,
                threshold: 3.0,
            },
        ),
        (
            "threshold_fact_latest",
            CohortPredicate::MetricThreshold {
                metric: "fact_latest".to_string(),
                op: ThresholdOp::Lte,
                threshold: 10.0,
            },
        ),
        (
            "threshold_fact_as_of",
            CohortPredicate::MetricThreshold {
                metric: "mrr_level".to_string(),
                op: ThresholdOp::Gt,
                threshold: 0.0,
            },
        ),
        (
            "inactive",
            CohortPredicate::MetricInactive {
                metric: "raw_count".to_string(),
                max_active: None,
            },
        ),
        (
            "inactive_fact",
            CohortPredicate::MetricInactive {
                metric: "fact_distinct".to_string(),
                max_active: Some(2.0),
            },
        ),
        (
            "delta_raw",
            CohortPredicate::MetricDelta {
                metric: "raw_latest".to_string(),
                op: ThresholdOp::Lte,
                pct: -30.0,
                prior_window_days: None,
            },
        ),
        (
            "delta_raw_count",
            CohortPredicate::MetricDelta {
                metric: "raw_count".to_string(),
                op: ThresholdOp::Gte,
                pct: 10.0,
                prior_window_days: Some(7),
            },
        ),
        (
            "delta_fact",
            CohortPredicate::MetricDelta {
                metric: "fact_distinct".to_string(),
                op: ThresholdOp::Gt,
                pct: 12.5,
                prior_window_days: None,
            },
        ),
        (
            "and_or_not",
            CohortPredicate::And {
                children: vec![
                    CohortPredicate::Or {
                        children: vec![
                            dimension_cohort(),
                            CohortPredicate::ActivePlan {
                                plan_ids: vec!["p3".to_string()],
                            },
                        ],
                    },
                    CohortPredicate::Not {
                        child: Box::new(dimension_cohort()),
                    },
                ],
            },
        ),
    ];
    for (name, op, value_type, values) in [
        (
            "prop_is_set",
            PropertyOp::IsSet,
            PropertyValueType::Text,
            &[][..],
        ),
        (
            "prop_is_empty",
            PropertyOp::IsEmpty,
            PropertyValueType::Text,
            &[],
        ),
        (
            "prop_is_true",
            PropertyOp::IsTrue,
            PropertyValueType::Boolean,
            &[],
        ),
        (
            "prop_is_false",
            PropertyOp::IsFalse,
            PropertyValueType::Boolean,
            &[],
        ),
        (
            "prop_equals",
            PropertyOp::Equals,
            PropertyValueType::Text,
            &["a"],
        ),
        (
            "prop_not_equals_num",
            PropertyOp::NotEquals,
            PropertyValueType::Number,
            &["3"],
        ),
        ("prop_gt", PropertyOp::Gt, PropertyValueType::Number, &["3"]),
        (
            "prop_gte",
            PropertyOp::Gte,
            PropertyValueType::Number,
            &["3"],
        ),
        ("prop_lt", PropertyOp::Lt, PropertyValueType::Number, &["3"]),
        (
            "prop_lte",
            PropertyOp::Lte,
            PropertyValueType::Number,
            &["3"],
        ),
        (
            "prop_before",
            PropertyOp::Before,
            PropertyValueType::Date,
            &["2026-01-01"],
        ),
        (
            "prop_after",
            PropertyOp::After,
            PropertyValueType::Date,
            &["2026-01-01"],
        ),
        (
            "prop_between",
            PropertyOp::Between,
            PropertyValueType::Number,
            &["1", "9"],
        ),
        (
            "prop_contains",
            PropertyOp::Contains,
            PropertyValueType::Text,
            &["ac"],
        ),
        (
            "prop_in_text",
            PropertyOp::In,
            PropertyValueType::Text,
            &["a", "b"],
        ),
        (
            "prop_in_number",
            PropertyOp::In,
            PropertyValueType::Number,
            &["1", "2"],
        ),
    ] {
        out.push((name, custom(op, value_type, values)));
    }
    out
}

fn all_dimensions() -> Vec<(&'static str, Dimension)> {
    vec![
        ("auto", Dimension::Auto("region".to_string())),
        ("property", Dimension::Property("region".to_string())),
        ("column", Dimension::Column("movement_type".to_string())),
        (
            "customer_attr",
            Dimension::CustomerAttr("country".to_string()),
        ),
        ("plan_attr", Dimension::PlanAttr("plan".to_string())),
        (
            "product_attr",
            Dimension::ProductAttr("product".to_string()),
        ),
        (
            "customer_name",
            Dimension::CustomerName("customer".to_string()),
        ),
        (
            "customer_attr_map",
            Dimension::CustomerAttrMap {
                map_key: "segment".to_string(),
                alias: "segment".to_string(),
            },
        ),
        (
            "customer_property",
            Dimension::CustomerProperty {
                key: "industry".to_string(),
                alias: "industry".to_string(),
            },
        ),
    ]
}

pub fn metric_cases() -> Vec<(String, MetricQuery)> {
    let mut cases: Vec<(String, MetricQuery)> = Vec::new();
    let mut push = |name: String, q: MetricQuery| cases.push((name, q));

    let simple = [
        "raw_sum",
        "raw_avg",
        "raw_min",
        "raw_max",
        "raw_latest",
        "raw_distinct",
        "raw_count",
        "raw_converted",
        "raw_filtered",
        "raw_question_mark",
        "gauge_latest",
        "gauge_max",
        "gauge_carry",
        "tw_seconds",
        "tw_value",
        "fact_sum",
        "fact_avg",
        "fact_min",
        "fact_max",
        "fact_count",
        "fact_distinct",
        "fact_latest",
        "snapshot_mrr",
        "churn",
        "d_arith",
        "d_div",
        "d_scalar",
        "d_lists",
        "d_retention",
        "d_fused_with_plain",
        "d_fused_multi_filter",
        "d_transforms",
        "d_running",
        "d_single",
    ];
    for m in simple {
        push(format!("{m}/day"), query(m));
        let mut q = query(m);
        q.window = None;
        push(format!("{m}/no_window"), q);
    }

    for (grain, w) in [
        ("minute", WindowSize::Minute),
        ("hour", WindowSize::Hour),
        ("week", WindowSize::Week),
    ] {
        for m in ["raw_sum", "fact_sum", "tw_seconds", "gauge_carry"] {
            let mut q = query(m);
            q.window = Some(w.clone());
            if grain == "minute" {
                q.to = Some(ts("2026-01-06T00:00:00Z"));
            }
            push(format!("{m}/{grain}"), q);
        }
    }

    for m in [
        "raw_sum",
        "fact_sum",
        "snapshot_mrr",
        "gauge_carry",
        "d_fused_with_plain",
    ] {
        let mut q = query(m);
        q.window = Some(WindowSize::Week);
        q.window_time_zone = Some(chrono_tz::Europe::Paris);
        push(format!("{m}/week_paris"), q);
    }

    for (dname, dim) in all_dimensions() {
        for m in [
            "raw_sum",
            "fact_sum",
            "snapshot_mrr",
            "d_fused_with_plain",
            "gauge_carry",
        ] {
            let mut q = query(m);
            q.group_by = vec![dim.clone()];
            push(format!("{m}/group_{dname}"), q);
        }
    }
    for m in ["tw_seconds", "tw_value"] {
        let mut q = query(m);
        q.group_by = vec![
            Dimension::Auto("zone".to_string()),
            Dimension::Property("az".to_string()),
        ];
        push(format!("{m}/group_props"), q);
    }
    let mut q = query("fact_sum");
    q.group_by = vec![
        Dimension::PlanAttr("plan".to_string()),
        Dimension::CustomerAttr("country".to_string()),
    ];
    push("fact_sum/group_two".to_string(), q);

    for m in [
        "raw_sum",
        "raw_count",
        "raw_distinct",
        "fact_sum",
        "tw_value",
        "gauge_latest",
    ] {
        let mut q = query(m);
        q.customer_ids = vec![customer(1), customer(2)];
        q.filters = vec![
            filter("region", &["eu", "us"], FilterOp::In),
            filter("tier", &["free"], FilterOp::NotIn),
            filter("empty", &[], FilterOp::In),
        ];
        push(format!("{m}/scoped_filtered"), q);
    }

    for m in [
        "fact_sum",
        "fact_avg",
        "fact_latest",
        "fact_count",
        "snapshot_mrr",
        "d_fused_with_plain",
        "d_retention",
    ] {
        let mut q = query(m);
        q.reporting_currency = Some("EUR".to_string());
        push(format!("{m}/fx_eur"), q);
        let mut q = query(m);
        q.reporting_currency = Some("EUR".to_string());
        q.group_by = vec![
            Dimension::CustomerAttr("country".to_string()),
            Dimension::PlanAttr("plan".to_string()),
            Dimension::Auto("movement_type".to_string()),
        ];
        q.customer_ids = vec![customer(3)];
        push(format!("{m}/fx_eur_grouped"), q);
    }
    let mut q = query("fact_sum");
    q.reporting_currency = Some("USD".to_string());
    push("fact_sum/fx_usd".to_string(), q);

    for (tname, t) in [
        ("running_total", CrossPeriod::RunningTotal),
        ("running_total_lagged", CrossPeriod::RunningTotalLagged),
        ("delta", CrossPeriod::PeriodOverPeriodDelta),
        ("pct_change", CrossPeriod::PercentChange),
        ("moving_avg", CrossPeriod::MovingAverage { periods: 4 }),
    ] {
        for m in ["raw_sum", "fact_sum", "d_div", "gauge_carry"] {
            let mut q = query(m);
            q.transform = Some(t.clone());
            push(format!("{m}/transform_{tname}"), q);
            let mut q = query(m);
            q.transform = Some(t.clone());
            q.group_by = vec![Dimension::Auto("region".to_string())];
            push(format!("{m}/transform_{tname}_grouped"), q);
        }
    }
    let mut q = query("fact_sum");
    q.transform = Some(CrossPeriod::RunningTotal);
    q.window = Some(WindowSize::Week);
    q.window_time_zone = Some(chrono_tz::America::New_York);
    push("fact_sum/running_total_week_ny".to_string(), q);

    for m in [
        "raw_sum",
        "fact_sum",
        "tw_seconds",
        "gauge_latest",
        "d_retention",
    ] {
        let mut q = query(m);
        q.keep_last_events = true;
        push(format!("{m}/keep_last"), q);
    }

    for (pname, p) in cohort_predicates() {
        for m in ["raw_sum", "fact_sum"] {
            let mut q = query(m);
            q.cohort = Some(CohortDef {
                predicates: vec![p.clone()],
            });
            push(format!("{m}/cohort_{pname}"), q);
        }
    }
    for m in ["tw_seconds", "d_fused_with_plain", "gauge_carry"] {
        let mut q = query(m);
        q.cohort = Some(CohortDef {
            predicates: vec![dimension_cohort(), cohort_predicates()[3].1.clone()],
        });
        q.customer_ids = vec![customer(4)];
        push(format!("{m}/cohort_two"), q);
    }
    cases
}

/// Cohort definitions compiled on their own, as segment members.
pub fn member_cases() -> Vec<(String, CohortDef)> {
    let mut cases: Vec<(String, CohortDef)> = cohort_predicates()
        .into_iter()
        .map(|(name, p)| {
            (
                name.to_string(),
                CohortDef {
                    predicates: vec![p],
                },
            )
        })
        .collect();
    cases.push((
        "two_predicates".to_string(),
        CohortDef {
            predicates: vec![dimension_cohort(), cohort_predicates()[2].1.clone()],
        },
    ));
    cases.push(("empty".to_string(), CohortDef { predicates: vec![] }));
    cases
}
