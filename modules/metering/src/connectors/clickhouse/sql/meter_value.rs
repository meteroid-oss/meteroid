//! Exact meter values. Numeric aggregates leave ClickHouse as decimal text and are parsed
//! into `rust_decimal`; Float64 only guards values and orders them for Min and Max.

use crate::connectors::clickhouse::sql::{BindValue, PropertyColumn};
use crate::domain::{MeterAggregation, MeterValue};
use metric_core::sql::LATEST_EVENT_ORDER;
use rust_decimal::Decimal;
use std::str::FromStr;

/// WHERE condition on the value property: it parses, is finite and below 1e18. Anything
/// else is skipped like an unparsable value.
pub(super) fn numeric_guard(col: &PropertyColumn, binds: &mut Vec<BindValue>) -> String {
    format!("abs(toFloat64OrNull({})) < 1e18", col.path_sql(binds))
}

fn decimal_expr(col: &PropertyColumn, binds: &mut Vec<BindValue>) -> String {
    let exact = col.path_sql(binds);
    let float = col.path_sql(binds);
    format!(
        "ifNull(toDecimal128OrNull({exact}, 9), accurateCastOrNull(toFloat64OrNull({float}), 'Decimal(38, 9)'))"
    )
}

fn fallbacks_column(col: &PropertyColumn, binds: &mut Vec<BindValue>) -> String {
    format!(
        "toString(countIf(isNull(toDecimal128OrNull({}, 9)))) AS decimal_fallbacks",
        col.path_sql(binds)
    )
}

/// The winning event's original string and its decimal, ordered by `key` (which may bind).
fn winner_columns(
    function: &str,
    col: &PropertyColumn,
    binds: &mut Vec<BindValue>,
    key: impl FnOnce(&mut Vec<BindValue>) -> String,
) -> Vec<String> {
    let raw = col.path_sql(binds);
    let decimal = decimal_expr(col, binds);
    let key = key(binds);
    vec![
        format!("{function}(tuple({raw}, toString({decimal})), {key}) AS _winner"),
        "_winner.1 AS value_raw".to_string(),
        "_winner.2 AS value".to_string(),
    ]
}

/// SELECT columns of a meter aggregation: `value` (decimal text), `value_raw` (Min, Max,
/// Latest: the winner's original string), `value_count` (Avg) and `decimal_fallbacks`.
pub(super) fn aggregation_columns(
    aggregation: &MeterAggregation,
    value_property: Option<&str>,
    binds: &mut Vec<BindValue>,
) -> Result<Vec<String>, String> {
    if let MeterAggregation::Count = aggregation {
        return Ok(vec!["toString(count()) AS value".to_string()]);
    }
    let Some(value_property) = value_property else {
        return Err(match aggregation {
            MeterAggregation::CountDistinct => {
                "value_property is required for CountDistinct aggregation"
            }
            _ => "value_property is required for non-Count aggregations",
        }
        .to_string());
    };
    let col = PropertyColumn(value_property);

    let mut columns = match aggregation {
        MeterAggregation::Count => unreachable!(),
        MeterAggregation::CountDistinct => {
            return Ok(vec![format!(
                "toString(uniqExact({})) AS value",
                col.path_sql(binds)
            )]);
        }
        MeterAggregation::Sum => {
            vec![format!(
                "toString(sum({})) AS value",
                decimal_expr(&col, binds)
            )]
        }
        MeterAggregation::Avg => vec![
            format!("toString(sum({})) AS value", decimal_expr(&col, binds)),
            "toString(count()) AS value_count".to_string(),
        ],
        MeterAggregation::Min => winner_columns("argMin", &col, binds, |binds| {
            format!("toFloat64OrNull({})", col.path_sql(binds))
        }),
        MeterAggregation::Max => winner_columns("argMax", &col, binds, |binds| {
            format!("toFloat64OrNull({})", col.path_sql(binds))
        }),
        MeterAggregation::Latest => {
            winner_columns("argMax", &col, binds, |_| LATEST_EVENT_ORDER.to_string())
        }
    };
    columns.push(fallbacks_column(&col, binds));
    Ok(columns)
}

/// Reads the columns of [`aggregation_columns`] back. A value beyond `rust_decimal`'s 28
/// digits is an error, never a rounded or dropped value.
pub fn parse_meter_value(row: &serde_json::Value) -> Result<(MeterValue, u64), String> {
    let decimal = match row.get("value") {
        // Aggregates over no events (no GROUP BY) are NULL; they read 0 as before.
        None | Some(serde_json::Value::Null) => Decimal::ZERO,
        Some(value) => parse_decimal(value)?,
    };
    let decimal = match row.get("value_raw").and_then(|raw| raw.as_str()) {
        Some(raw) => exact_original(raw, decimal).unwrap_or(decimal),
        None => decimal,
    };
    let value = match row.get("value_count") {
        Some(count) => MeterValue::Mean {
            sum: decimal,
            count: parse_u64(count)?,
        },
        None => MeterValue::Exact(decimal),
    };
    let fallbacks = row
        .get("decimal_fallbacks")
        .map(parse_u64)
        .transpose()?
        .unwrap_or(0);
    Ok((value, fallbacks))
}

fn parse_decimal(value: &serde_json::Value) -> Result<Decimal, String> {
    let text = match value {
        serde_json::Value::String(s) => s.clone(),
        // Bespoke extensions return Float64 or Int64 columns.
        serde_json::Value::Number(n) => n.to_string(),
        other => return Err(format!("meter value is not a number: {other}")),
    };
    Decimal::from_str_exact(&text)
        .or_else(|_| Decimal::from_scientific(&text))
        .map_err(|e| format!("meter value {text} does not fit a decimal: {e}"))
}

fn parse_u64(value: &serde_json::Value) -> Result<u64, String> {
    match value {
        serde_json::Value::String(s) => s.parse().ok(),
        serde_json::Value::Number(n) => n.as_u64(),
        _ => None,
    }
    .ok_or_else(|| format!("expected an unsigned integer, got {value}"))
}

/// The original string when it is plain decimal text that ClickHouse read the same way
/// (ClickHouse truncates to 9 decimals); otherwise ClickHouse's decimal stands.
fn exact_original(raw: &str, clickhouse: Decimal) -> Option<Decimal> {
    if !raw
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+'))
    {
        return None;
    }
    let exact = Decimal::from_str(raw).ok()?;
    (exact.trunc_with_scale(MeterValue::SCALE) == clickhouse).then_some(exact)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use serde_json::json;

    #[test]
    fn sum_and_count_parse_exactly() {
        let (value, fallbacks) =
            parse_meter_value(&json!({"value": "1", "decimal_fallbacks": "0"})).unwrap();
        assert_eq!(value, MeterValue::Exact(dec!(1)));
        assert_eq!(fallbacks, 0);
        // 25.8 no longer quotes 64-bit integers in JSON.
        let (_, fallbacks) =
            parse_meter_value(&json!({"value": "1", "decimal_fallbacks": 3})).unwrap();
        assert_eq!(fallbacks, 3);
    }

    #[test]
    fn mean_is_divided_once_when_rounded() {
        let (value, _) = parse_meter_value(&json!({"value": "1", "value_count": "3"})).unwrap();
        assert_eq!(
            value,
            MeterValue::Mean {
                sum: dec!(1),
                count: 3
            }
        );
        assert_eq!(value.rounded(), Some(dec!(0.333333333)));
        let (value, _) = parse_meter_value(&json!({"value": "2", "value_count": "3"})).unwrap();
        assert_eq!(value.rounded(), Some(dec!(0.666666667)));
        let empty = MeterValue::Mean {
            sum: Decimal::ZERO,
            count: 0,
        };
        assert_eq!(empty.rounded(), None);
    }

    #[test]
    fn rounding_is_midpoint_away_from_zero_at_nine_decimals() {
        assert_eq!(
            MeterValue::Exact(dec!(0.0000000005)).rounded(),
            Some(dec!(0.000000001))
        );
        assert_eq!(
            MeterValue::Exact(dec!(-0.0000000005)).rounded(),
            Some(dec!(-0.000000001))
        );
        assert_eq!(
            MeterValue::Exact(dec!(1)).rounded().unwrap().to_string(),
            "1"
        );
    }

    #[test]
    fn winner_keeps_its_original_string() {
        let row = json!({"value_raw": "0.30000000000000004", "value": "0.3"});
        let (value, _) = parse_meter_value(&row).unwrap();
        assert_eq!(value, MeterValue::Exact(dec!(0.30000000000000004)));

        let row = json!({"value_raw": "0.9999999999", "value": "0.999999999"});
        let (value, _) = parse_meter_value(&row).unwrap();
        assert_eq!(value.rounded(), Some(dec!(1.000000000)));
    }

    #[test]
    fn winner_falls_back_to_clickhouse_decimal_for_other_syntax() {
        for (raw, decimal, expected) in [
            ("1e3", "1000", dec!(1000)),
            ("e3", "0", dec!(0)),
            (".", "0", dec!(0)),
            ("+.5", "0.5", dec!(0.5)),
            ("00012", "12", dec!(12)),
        ] {
            let (value, _) =
                parse_meter_value(&json!({"value_raw": raw, "value": decimal})).unwrap();
            assert_eq!(value, MeterValue::Exact(expected), "{raw}");
        }
    }

    #[test]
    fn empty_aggregate_reads_zero() {
        let (value, _) =
            parse_meter_value(&json!({"value": null, "value_raw": "", "decimal_fallbacks": "0"}))
                .unwrap();
        assert_eq!(value, MeterValue::Exact(Decimal::ZERO));
    }

    #[test]
    fn overflow_is_an_error() {
        let err =
            parse_meter_value(&json!({"value": "100999999999999999899.000000101"})).unwrap_err();
        assert!(err.contains("does not fit a decimal"), "{err}");
    }

    #[test]
    fn extension_numbers_parse() {
        let (value, _) = parse_meter_value(&json!({"value": 3600})).unwrap();
        assert_eq!(value, MeterValue::Exact(dec!(3600)));
        let (value, _) = parse_meter_value(&json!({"value": 1.5})).unwrap();
        assert_eq!(value, MeterValue::Exact(dec!(1.5)));
    }
}
