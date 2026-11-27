//! Exact meter values. Numeric aggregates leave ClickHouse as decimal text and are parsed
//! into `rust_decimal`; Float64 only guards values and orders them for Min and Max.

use crate::domain::{MeterAggregation, MeterValue};
use metric_core::sql::ast::{Expr, Ident, Item};
use metric_core::sql::latest_event_order;
use rust_decimal::Decimal;
use std::str::FromStr;

/// `properties[?]`: the property name is bound.
pub(super) fn property(name: &str) -> Expr {
    Expr::Index(
        Box::new(Expr::col("properties")),
        Box::new(Expr::Bind(metric_core::sql::BindValue::String(
            name.to_string(),
        ))),
    )
}

/// The value property parses, is finite and below 1e18. Anything else is skipped like an
/// unparsable value.
pub(super) fn numeric_guard(name: &str) -> Expr {
    let float = Expr::call("toFloat64OrNull", [property(name)]);
    Expr::call("abs", [float]).lt(Expr::Literal("1e18"))
}

fn decimal(name: &str) -> Expr {
    Expr::call(
        "ifNull",
        [
            Expr::call("toDecimal128OrNull", [property(name), Expr::Int(9)]),
            Expr::call(
                "accurateCastOrNull",
                [
                    Expr::call("toFloat64OrNull", [property(name)]),
                    Expr::str("Decimal(38, 9)"),
                ],
            ),
        ],
    )
}

fn as_text(expr: Expr) -> Expr {
    Expr::call("toString", [expr])
}

fn fallbacks(name: &str) -> Item {
    let not_decimal = Expr::call(
        "isNull",
        [Expr::call(
            "toDecimal128OrNull",
            [property(name), Expr::Int(9)],
        )],
    );
    Item::as_(
        as_text(Expr::call("countIf", [not_decimal])),
        "decimal_fallbacks",
    )
}

/// The winning event's original string and its decimal, ranked by `key`.
fn winner(function: &'static str, name: &str, key: Expr) -> Vec<Item> {
    let pair = Expr::call("tuple", [property(name), as_text(decimal(name))]);
    let element =
        |n: &'static str| Expr::Column(Some(Ident::trusted("_winner")), Ident::trusted(n));
    vec![
        Item::as_(Expr::call(function, [pair, key]), "_winner"),
        Item::as_(element("1"), "value_raw"),
        Item::as_(element("2"), "value"),
    ]
}

/// SELECT items of a meter aggregation: `value` (decimal text), `value_raw` (Min, Max,
/// Latest: the winner's original string), `value_count` (Avg) and `decimal_fallbacks`.
pub(super) fn aggregation_items(
    aggregation: &MeterAggregation,
    value_property: Option<&str>,
) -> Result<Vec<Item>, String> {
    if let MeterAggregation::Count = aggregation {
        return Ok(vec![Item::as_(as_text(Expr::call("count", [])), "value")]);
    }
    let Some(name) = value_property else {
        return Err(match aggregation {
            MeterAggregation::CountDistinct => {
                "value_property is required for CountDistinct aggregation"
            }
            _ => "value_property is required for non-Count aggregations",
        }
        .to_string());
    };
    let float = || Expr::call("toFloat64OrNull", [property(name)]);
    let mut items = match aggregation {
        MeterAggregation::Count => unreachable!(),
        MeterAggregation::CountDistinct => {
            return Ok(vec![Item::as_(
                as_text(Expr::call("uniqExact", [property(name)])),
                "value",
            )]);
        }
        MeterAggregation::Sum => vec![Item::as_(
            as_text(Expr::call("sum", [decimal(name)])),
            "value",
        )],
        MeterAggregation::Avg => vec![
            Item::as_(as_text(Expr::call("sum", [decimal(name)])), "value"),
            Item::as_(as_text(Expr::call("count", [])), "value_count"),
        ],
        MeterAggregation::Min => winner("argMin", name, float()),
        MeterAggregation::Max => winner("argMax", name, float()),
        MeterAggregation::Latest => winner("argMax", name, latest_event_order()),
    };
    items.push(fallbacks(name));
    Ok(items)
}

/// Reads the columns of [`aggregation_items`] back. A value beyond `rust_decimal`'s 28
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
