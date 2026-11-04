//! Shared resolution of API [`Filter`]s into typed query predicates: look up custom properties
//! against the tenant's definitions, validate operator/type, and coerce values to match the stored
//! JSONB shape; resolve built-in ("native") fields against a static per-list registry
//! ([`native_fields`]) that names each field's SQL column and type.

use crate::StoreResult;
use crate::domain::custom_property_definitions::{CustomPropertyDefinition, coerce_filter_value};
use crate::domain::enums::CustomPropertyType;
use crate::domain::filters::{FieldRef, Filter, FilterOperator};
use crate::errors::StoreError;
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime};
use common_domain::ids::{BaseId, CustomerId, PlanId, UserId};
use diesel_models::extend::filtering::{
    CmpOp, CpFilter, CpKind, ListFilter, NativeCast, NativeFilter, NativeKind,
};
use error_stack::Report;
use std::str::FromStr;
use std::sync::LazyLock;

pub(crate) fn invalid_arg(msg: impl Into<String>) -> Report<StoreError> {
    StoreError::InvalidArgument(msg.into()).into()
}

fn cp_object(key: &str, value: serde_json::Value) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert(key.to_string(), value);
    serde_json::Value::Object(map)
}

/// Escape a raw string for use inside a `LIKE`/`ILIKE` pattern (so user `%`/`_` are literal).
fn escape_like(raw: &str) -> String {
    raw.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn coerced_f64(def: &CustomPropertyDefinition, raw: &str) -> StoreResult<f64> {
    coerce_filter_value(def, raw)?
        .as_f64()
        .ok_or_else(|| invalid_arg(format!("custom property '{}' expects a number", def.key)))
}

fn coerced_string(def: &CustomPropertyDefinition, raw: &str) -> StoreResult<String> {
    coerce_filter_value(def, raw)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| invalid_arg(format!("custom property '{}' expects a string", def.key)))
}

pub(crate) fn nth<'a>(values: &'a [String], i: usize, key: &str) -> StoreResult<&'a String> {
    values
        .get(i)
        .ok_or_else(|| invalid_arg(format!("filter on '{key}' is missing a value")))
}

fn cmp_of(op: FilterOperator) -> CmpOp {
    match op {
        FilterOperator::Gt => CmpOp::Gt,
        FilterOperator::Gte => CmpOp::Gte,
        FilterOperator::Lt | FilterOperator::Before => CmpOp::Lt,
        FilterOperator::After => CmpOp::Gt,
        _ => CmpOp::Lte,
    }
}

/// Build the JSONB predicate kind for a custom-property filter. Operator/type validity is checked
/// by [`build_cp_filter`] against [`FilterOperator::allowed_for`].
fn build_cp_kind(
    def: &CustomPropertyDefinition,
    op: FilterOperator,
    values: &[String],
) -> StoreResult<CpKind> {
    use CustomPropertyType as T;
    use FilterOperator as Op;
    let key = def.key.as_str();

    match op {
        Op::IsSet => Ok(CpKind::KeyExists),
        Op::IsEmpty => Ok(CpKind::KeyEmpty),
        Op::IsTrue => Ok(CpKind::Contains(cp_object(key, serde_json::json!(true)))),
        Op::IsFalse => Ok(CpKind::Contains(cp_object(key, serde_json::json!(false)))),
        Op::Equals => {
            let coerced = coerce_filter_value(def, nth(values, 0, key)?)?;
            match def.property_type {
                T::Number => Ok(CpKind::NumCmp(
                    CmpOp::Eq,
                    coerced_f64(def, nth(values, 0, key)?)?,
                )),
                T::Date | T::Datetime => Ok(CpKind::LexCmp(
                    CmpOp::Eq,
                    coerced_string(def, nth(values, 0, key)?)?,
                )),
                _ => Ok(CpKind::Contains(cp_object(key, coerced))),
            }
        }
        Op::NotEquals => {
            let coerced = coerce_filter_value(def, nth(values, 0, key)?)?;
            match def.property_type {
                T::Number => Ok(CpKind::NumCmp(
                    CmpOp::Ne,
                    coerced_f64(def, nth(values, 0, key)?)?,
                )),
                _ => Ok(CpKind::NotContains(cp_object(key, coerced))),
            }
        }
        Op::In => {
            if values.is_empty() {
                return Err(invalid_arg(format!("filter on '{key}' is missing values")));
            }
            match def.property_type {
                T::Number => {
                    let ns = values
                        .iter()
                        .map(|v| coerced_f64(def, v))
                        .collect::<StoreResult<Vec<f64>>>()?;
                    Ok(CpKind::NumIn(ns))
                }
                T::MultiSelect => {
                    let objs = values
                        .iter()
                        .map(|v| {
                            coerced_string(def, v).map(|s| cp_object(key, serde_json::json!([s])))
                        })
                        .collect::<StoreResult<Vec<_>>>()?;
                    Ok(CpKind::ContainsAny(objs))
                }
                _ => {
                    let objs = values
                        .iter()
                        .map(|v| {
                            coerce_filter_value(def, v)
                                .map(|c| cp_object(key, c))
                                .map_err(Into::into)
                        })
                        .collect::<StoreResult<Vec<_>>>()?;
                    Ok(CpKind::ContainsAny(objs))
                }
            }
        }
        Op::Contains => match def.property_type {
            T::MultiSelect => {
                let s = coerced_string(def, nth(values, 0, key)?)?;
                Ok(CpKind::Contains(cp_object(key, serde_json::json!([s]))))
            }
            _ => Ok(CpKind::Ilike(format!(
                "%{}%",
                escape_like(nth(values, 0, key)?)
            ))),
        },
        Op::Gt | Op::Gte | Op::Lt | Op::Lte | Op::Before | Op::After => {
            let raw = nth(values, 0, key)?;
            match def.property_type {
                T::Number => Ok(CpKind::NumCmp(cmp_of(op), coerced_f64(def, raw)?)),
                _ => Ok(CpKind::LexCmp(cmp_of(op), coerced_string(def, raw)?)),
            }
        }
        Op::Between => {
            let lo = nth(values, 0, key)?;
            let hi = nth(values, 1, key)?;
            match def.property_type {
                T::Number => Ok(CpKind::NumBetween(
                    coerced_f64(def, lo)?,
                    coerced_f64(def, hi)?,
                )),
                _ => Ok(CpKind::LexBetween(
                    coerced_string(def, lo)?,
                    coerced_string(def, hi)?,
                )),
            }
        }
    }
}

/// Resolve one custom-property filter (field already known to be `cp:<key>`): look up the
/// definition, validate the operator against its type, and build the predicate kind.
pub(crate) fn build_cp_filter(
    definitions: &[CustomPropertyDefinition],
    key: &str,
    op: FilterOperator,
    values: &[String],
) -> StoreResult<CpFilter> {
    let def = definitions
        .iter()
        .find(|d| d.key == key)
        .ok_or_else(|| invalid_arg(format!("unknown custom property '{key}'")))?;
    if !FilterOperator::allowed_for(def.property_type).contains(&op) {
        return Err(invalid_arg(format!(
            "operator not allowed for custom property '{key}'"
        )));
    }
    let kind = build_cp_kind(def, op, values)?;
    Ok(CpFilter {
        key: key.to_string(),
        kind,
    })
}

/// The entity lists that accept filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListEntity {
    Customer,
    Subscription,
    Invoice,
    CreditNote,
    Quote,
}

/// How a built-in field is typed, which fixes its operators and value validation.
#[derive(Debug, Clone, Copy)]
pub(crate) enum NativeType {
    /// ISO 4217 code.
    Currency,
    /// ISO 3166-1 alpha-2 code.
    Country,
    /// A Postgres enum compared on its labels (the column expression casts it to text).
    Enum(&'static [&'static str]),
    Date,
    Timestamp,
    Timestamptz,
    /// Minor-unit amount; filter values are major units, scaled by the row's currency (named by
    /// the given column).
    Money {
        currency_column: &'static str,
    },
    Customer,
    /// A user id column (the account owner).
    User,
    /// A `plan_version_id` column, matched by plan.
    Plan,
    /// Nullable text only tested for presence.
    Presence,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NativeField {
    pub id: &'static str,
    /// Trusted SQL expression over the list query's source.
    pub column: &'static str,
    pub ty: NativeType,
    pub nullable: bool,
}

const fn field(id: &'static str, column: &'static str, ty: NativeType) -> NativeField {
    NativeField {
        id,
        column,
        ty,
        nullable: false,
    }
}

const fn nullable(id: &'static str, column: &'static str, ty: NativeType) -> NativeField {
    NativeField {
        id,
        column,
        ty,
        nullable: true,
    }
}

const CREDIT_TYPES: &[&str] = &["CREDIT_TO_BALANCE", "REFUND", "DEBT_CANCELLATION"];

const CUSTOMER_FIELDS: &[NativeField] = &[
    nullable(
        "country",
        "(customer.billing_address ->> 'country')",
        NativeType::Country,
    ),
    field("currency", "customer.currency", NativeType::Currency),
    field("created_at", "customer.created_at", NativeType::Timestamp),
    // Invoice credit spendable now in the customer's currency, from the credit ledger.
    field(
        "balance",
        "(SELECT coalesce(sum(g.remaining), 0) FROM customer_credit_grant g \
          JOIN customer_credit_account a ON a.id = g.account_id \
          WHERE g.tenant_id = customer.tenant_id AND g.customer_id = customer.id \
            AND a.currency = upper(customer.currency) AND g.application = 'INVOICE' \
            AND g.remaining > 0 AND g.effective_at <= now() \
            AND (g.lapses_at IS NULL OR g.lapses_at > now()))",
        NativeType::Money {
            currency_column: "customer.currency",
        },
    ),
    nullable(
        "billing_email",
        "customer.billing_email",
        NativeType::Presence,
    ),
    nullable("owner", "customer.owner_user_id", NativeType::User),
];

const SUBSCRIPTION_FIELDS: &[NativeField] = &[
    field("plan", "subscription.plan_version_id", NativeType::Plan),
    field("currency", "subscription.currency", NativeType::Currency),
    field(
        "mrr",
        "subscription.mrr_cents",
        NativeType::Money {
            currency_column: "subscription.currency",
        },
    ),
    field("start_date", "subscription.start_date", NativeType::Date),
    nullable("end_date", "subscription.end_date", NativeType::Date),
    field("customer", "subscription.customer_id", NativeType::Customer),
];

const INVOICE_FIELDS: &[NativeField] = &[
    field("currency", "invoice.currency", NativeType::Currency),
    field(
        "total",
        "invoice.total",
        NativeType::Money {
            currency_column: "invoice.currency",
        },
    ),
    field(
        "amount_due",
        "invoice.amount_due",
        NativeType::Money {
            currency_column: "invoice.currency",
        },
    ),
    field("invoice_date", "invoice.invoice_date", NativeType::Date),
    nullable("due_at", "invoice.due_at", NativeType::Timestamp),
    field("customer", "invoice.customer_id", NativeType::Customer),
];

const CREDIT_NOTE_FIELDS: &[NativeField] = &[
    field("currency", "credit_note.currency", NativeType::Currency),
    // Stored negative (a credit); filtered as a positive amount.
    field(
        "total",
        "abs(credit_note.total)",
        NativeType::Money {
            currency_column: "credit_note.currency",
        },
    ),
    field(
        "created_at",
        "credit_note.created_at",
        NativeType::Timestamptz,
    ),
    field("customer", "credit_note.customer_id", NativeType::Customer),
    field(
        "credit_type",
        "credit_note.credit_type::text",
        NativeType::Enum(CREDIT_TYPES),
    ),
];

const QUOTE_FIELDS: &[NativeField] = &[
    field("currency", "quote.currency", NativeType::Currency),
    field("plan", "quote.plan_version_id", NativeType::Plan),
    field("customer", "quote.customer_id", NativeType::Customer),
    field("created_at", "quote.created_at", NativeType::Timestamptz),
    nullable("expires_at", "quote.expires_at", NativeType::Timestamptz),
];

/// Built-in filterable fields per list. Keep in sync with `NATIVE_FIELDS` in the web app.
pub(crate) fn native_fields(entity: ListEntity) -> &'static [NativeField] {
    match entity {
        ListEntity::Customer => CUSTOMER_FIELDS,
        ListEntity::Subscription => SUBSCRIPTION_FIELDS,
        ListEntity::Invoice => INVOICE_FIELDS,
        ListEntity::CreditNote => CREDIT_NOTE_FIELDS,
        ListEntity::Quote => QUOTE_FIELDS,
    }
}

/// Operators a built-in field accepts. Mirrors `NATIVE_FIELDS[*].ops` in the web app.
pub(crate) fn native_ops(f: &NativeField) -> Vec<FilterOperator> {
    use FilterOperator as Op;
    use NativeType as T;
    let mut ops = match f.ty {
        T::Currency | T::Country | T::Enum(_) | T::Customer | T::User => {
            vec![Op::Equals, Op::NotEquals, Op::In]
        }
        T::Plan => vec![Op::Equals, Op::In],
        T::Date | T::Timestamp | T::Timestamptz => vec![Op::Before, Op::After, Op::Between],
        T::Money { .. } => vec![Op::Equals, Op::Gt, Op::Gte, Op::Lt, Op::Lte, Op::Between],
        T::Presence => vec![],
    };
    if f.nullable {
        ops.extend([Op::IsSet, Op::IsEmpty]);
    }
    ops
}

/// `CASE <currency_column> WHEN 'JPY' THEN 0 … ELSE 2 END`, from the owned ISO table. Codes are
/// static `[A-Z]{3}` values, so interpolating them is safe.
fn currency_scale_sql(currency_column: &str) -> String {
    static NON_DEFAULT: LazyLock<Vec<(String, u8)>> = LazyLock::new(|| {
        crate::constants::Currencies::all()
            .filter(|c| c.precision != 2)
            .map(|c| (c.code.to_string(), c.precision))
            .filter(|(code, _)| code.len() == 3 && code.chars().all(|ch| ch.is_ascii_uppercase()))
            .collect()
    });
    let arms: String = NON_DEFAULT
        .iter()
        .map(|(code, p)| format!(" WHEN '{code}' THEN {p}"))
        .collect();
    format!("(CASE {currency_column}{arms} ELSE 2 END)")
}

fn parse_date(s: &str, id: &str) -> StoreResult<NaiveDate> {
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d);
    }
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.date_naive())
        .map_err(|_| invalid_arg(format!("invalid date '{s}' for '{id}'")))
}

/// A date-only value is a whole day; a full timestamp is an instant.
enum TimeValue {
    Day(NaiveDate),
    Instant(NaiveDateTime),
}

fn parse_time(s: &str, id: &str) -> StoreResult<TimeValue> {
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(TimeValue::Day(d));
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(TimeValue::Instant(dt.naive_utc()));
    }
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
        .map(TimeValue::Instant)
        .map_err(|_| invalid_arg(format!("invalid date/time '{s}' for '{id}'")))
}

fn midnight(d: NaiveDate) -> NaiveDateTime {
    d.and_hms_opt(0, 0, 0).expect("valid midnight")
}

/// Formats a UTC instant for a `CAST(... AS timestamp[tz])`.
fn ts_text(dt: NaiveDateTime, tz: bool) -> String {
    let s = dt.format("%Y-%m-%d %H:%M:%S%.6f").to_string();
    if tz { format!("{s}+00") } else { s }
}

fn resolve_native(
    f: &NativeField,
    op: FilterOperator,
    values: &[String],
) -> StoreResult<NativeFilter> {
    use FilterOperator as Op;
    use NativeType as T;
    let id = f.id;
    if !native_ops(f).contains(&op) {
        return Err(invalid_arg(format!("operator not allowed for '{id}'")));
    }
    let native = |cast: NativeCast, kind: NativeKind| NativeFilter {
        column: f.column,
        cast,
        kind,
    };
    // Presence: text-like columns also treat '' as empty; any other cast renders a plain NULL test.
    let presence_cast = match f.ty {
        T::Currency | T::Country | T::Enum(_) | T::Presence => NativeCast::Text,
        _ => NativeCast::Date,
    };
    match op {
        Op::IsSet => return Ok(native(presence_cast, NativeKind::IsSet)),
        Op::IsEmpty => return Ok(native(presence_cast, NativeKind::IsEmpty)),
        _ => {}
    }

    // Multi-value operators take the whole list; single-value ones take the first.
    let set = |vals: Vec<String>, negated: bool| NativeKind::In {
        values: vals,
        negated,
    };
    let pick = |validate: &dyn Fn(&str) -> StoreResult<String>| -> StoreResult<NativeKind> {
        match op {
            Op::Equals => Ok(set(vec![validate(nth(values, 0, id)?)?], false)),
            Op::NotEquals => Ok(set(vec![validate(nth(values, 0, id)?)?], true)),
            Op::In => {
                if values.is_empty() {
                    return Err(invalid_arg(format!("filter on '{id}' is missing values")));
                }
                Ok(set(
                    values
                        .iter()
                        .map(|v| validate(v))
                        .collect::<StoreResult<_>>()?,
                    false,
                ))
            }
            _ => Err(invalid_arg(format!("operator not allowed for '{id}'"))),
        }
    };

    match f.ty {
        T::Currency => {
            let kind = pick(&|v| {
                let code = v.trim().to_ascii_uppercase();
                crate::constants::Currencies::resolve_currency(&code)
                    .map(|_| code)
                    .ok_or_else(|| invalid_arg(format!("unknown currency '{v}'")))
            })?;
            Ok(native(NativeCast::Text, kind))
        }
        T::Country => {
            let kind = pick(&|v| {
                let code = v.trim().to_ascii_uppercase();
                crate::constants::Countries::resolve_country(&code)
                    .map(|_| code)
                    .ok_or_else(|| invalid_arg(format!("unknown country '{v}'")))
            })?;
            Ok(native(NativeCast::Text, kind))
        }
        T::Enum(labels) => {
            let kind = pick(&|v| {
                labels
                    .iter()
                    .find(|l| l.eq_ignore_ascii_case(v))
                    .map(|l| l.to_string())
                    .ok_or_else(|| invalid_arg(format!("invalid value '{v}' for '{id}'")))
            })?;
            Ok(native(NativeCast::Text, kind))
        }
        T::Customer => {
            let kind = pick(&|v| {
                CustomerId::parse_base62(v)
                    .map(|c| c.as_uuid().to_string())
                    .map_err(|_| invalid_arg(format!("invalid customer id '{v}'")))
            })?;
            Ok(native(NativeCast::Uuid, kind))
        }
        T::User => {
            let kind = pick(&|v| {
                v.parse::<UserId>()
                    .map(|u| u.as_uuid().to_string())
                    .map_err(|_| invalid_arg(format!("invalid user id '{v}'")))
            })?;
            Ok(native(NativeCast::Uuid, kind))
        }
        T::Plan => {
            let kind = pick(&|v| {
                PlanId::parse_base62(v)
                    .map(|p| p.as_uuid().to_string())
                    .map_err(|_| invalid_arg(format!("invalid plan id '{v}'")))
            })?;
            let NativeKind::In { values, .. } = kind else {
                unreachable!("plan operators are set operators")
            };
            Ok(native(
                NativeCast::Uuid,
                NativeKind::InSubquery {
                    subquery_sql: "SELECT pv.id FROM plan_version pv WHERE pv.plan_id =",
                    values,
                },
            ))
        }
        T::Date => {
            let d = |i| parse_date(nth(values, i, id)?, id).map(|d| d.to_string());
            let kind = match op {
                Op::Before => NativeKind::Cmp(CmpOp::Lt, d(0)?),
                Op::After => NativeKind::Cmp(CmpOp::Gt, d(0)?),
                _ => NativeKind::Between(d(0)?, d(1)?),
            };
            Ok(native(NativeCast::Date, kind))
        }
        T::Timestamp | T::Timestamptz => {
            let tz = matches!(f.ty, T::Timestamptz);
            let t = |i| parse_time(nth(values, i, id)?, id);
            // A whole day: "before" excludes it, "after" starts the next day, a range includes both
            // ends in full.
            let kind = match op {
                Op::Before => {
                    let at = match t(0)? {
                        TimeValue::Day(d) => midnight(d),
                        TimeValue::Instant(i) => i,
                    };
                    NativeKind::Cmp(CmpOp::Lt, ts_text(at, tz))
                }
                Op::After => match t(0)? {
                    TimeValue::Day(d) => {
                        NativeKind::Cmp(CmpOp::Gte, ts_text(midnight(d) + Duration::days(1), tz))
                    }
                    TimeValue::Instant(i) => NativeKind::Cmp(CmpOp::Gt, ts_text(i, tz)),
                },
                _ => {
                    let lo = match t(0)? {
                        TimeValue::Day(d) => midnight(d),
                        TimeValue::Instant(i) => i,
                    };
                    let hi = match t(1)? {
                        TimeValue::Day(d) => {
                            midnight(d) + Duration::days(1) - Duration::microseconds(1)
                        }
                        TimeValue::Instant(i) => i,
                    };
                    NativeKind::Between(ts_text(lo, tz), ts_text(hi, tz))
                }
            };
            let cast = if tz {
                NativeCast::Timestamptz
            } else {
                NativeCast::Timestamp
            };
            Ok(native(cast, kind))
        }
        T::Money { currency_column } => {
            let amount = |i| {
                let raw = nth(values, i, id)?;
                rust_decimal::Decimal::from_str(raw.trim())
                    .map(|d| d.to_string())
                    .map_err(|_| invalid_arg(format!("invalid amount '{raw}' for '{id}'")))
            };
            let scale_sql = currency_scale_sql(currency_column);
            let kind = match op {
                Op::Between => NativeKind::MoneyBetween {
                    lo: amount(0)?,
                    hi: amount(1)?,
                    scale_sql,
                },
                _ => NativeKind::MoneyCmp {
                    op: match op {
                        Op::Equals => CmpOp::Eq,
                        other => cmp_of(other),
                    },
                    value: amount(0)?,
                    scale_sql,
                },
            };
            Ok(native(NativeCast::Text, kind))
        }
        T::Presence => Err(invalid_arg(format!("operator not allowed for '{id}'"))),
    }
}

/// Resolve API filters for one list: custom properties against the tenant's definitions, built-in
/// fields against [`native_fields`]. Unknown fields and disallowed operators are rejected.
pub(crate) fn resolve_list_filters(
    entity: ListEntity,
    definitions: &[CustomPropertyDefinition],
    filters: Vec<Filter>,
) -> StoreResult<Vec<ListFilter>> {
    filters
        .into_iter()
        .map(|f| match &f.field {
            FieldRef::CustomProperty(key) => {
                build_cp_filter(definitions, key, f.op, &f.values).map(ListFilter::CustomProperty)
            }
            FieldRef::Native(col) => native_fields(entity)
                .iter()
                .find(|n| n.id == col)
                .ok_or_else(|| invalid_arg(format!("filtering on '{col}' is not supported")))
                .and_then(|n| resolve_native(n, f.op, &f.values))
                .map(ListFilter::Native),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::custom_property_definitions::PropertyConfig;
    use crate::domain::enums::CustomPropertyEntityType;
    use common_domain::ids::{BaseId, CustomPropertyDefinitionId, TenantId};
    use serde_json::json;

    use CustomPropertyType as T;
    use FilterOperator as Op;

    const ALL_OPS: [FilterOperator; 15] = [
        Op::Equals,
        Op::NotEquals,
        Op::In,
        Op::Contains,
        Op::Gt,
        Op::Gte,
        Op::Lt,
        Op::Lte,
        Op::Between,
        Op::IsSet,
        Op::IsEmpty,
        Op::IsTrue,
        Op::IsFalse,
        Op::Before,
        Op::After,
    ];

    const ALL_TYPES: [CustomPropertyType; 10] = [
        T::Text,
        T::Number,
        T::Boolean,
        T::Date,
        T::Datetime,
        T::SingleSelect,
        T::MultiSelect,
        T::Json,
        T::Url,
        T::Email,
    ];

    fn def(key: &str, property_type: CustomPropertyType) -> CustomPropertyDefinition {
        CustomPropertyDefinition {
            id: CustomPropertyDefinitionId::new(),
            tenant_id: TenantId::new(),
            entity_type: CustomPropertyEntityType::Customer,
            key: key.to_string(),
            name: key.to_string(),
            description: None,
            property_type,
            config: PropertyConfig::default(),
            required: false,
            default_value: None,
            display_order: 0,
            archived_at: None,
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap().naive_utc(),
            updated_at: None,
        }
    }

    fn vals(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    /// Two well-formed values for a property type (enough for any operator's arity).
    fn sample_values(t: CustomPropertyType) -> Vec<String> {
        match t {
            T::Number => vals(&["5", "10"]),
            T::Boolean => vals(&["true", "false"]),
            T::Date => vals(&["2026-01-01", "2026-02-01"]),
            T::Datetime => vals(&["2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z"]),
            _ => vals(&["a", "b"]),
        }
    }

    fn build(t: CustomPropertyType, op: FilterOperator, values: &[&str]) -> StoreResult<CpFilter> {
        build_cp_filter(&[def("k", t)], "k", op, &vals(values))
    }

    fn kind(t: CustomPropertyType, op: FilterOperator, values: &[&str]) -> CpKind {
        build(t, op, values)
            .unwrap_or_else(|e| panic!("{t:?} {op:?} {values:?} should resolve: {e:?}"))
            .kind
    }

    fn assert_invalid<V: std::fmt::Debug>(res: StoreResult<V>) {
        match res {
            Ok(f) => panic!("expected InvalidArgument, got {f:?}"),
            Err(e) => assert!(
                matches!(e.current_context(), StoreError::InvalidArgument(_)),
                "expected InvalidArgument, got {e:?}"
            ),
        }
    }

    #[test]
    fn operator_matrix_matches_allowed_for() {
        for t in ALL_TYPES {
            let allowed = FilterOperator::allowed_for(t);
            for op in ALL_OPS {
                let values = if op.takes_values() {
                    sample_values(t)
                } else {
                    vec![]
                };
                let res = build_cp_filter(&[def("k", t)], "k", op, &values);
                if allowed.contains(&op) {
                    assert!(res.is_ok(), "{t:?} {op:?} should be allowed: {res:?}");
                } else {
                    assert_invalid(res);
                }
            }
        }
    }

    #[test]
    fn explicit_disallowed_operators() {
        assert_invalid(build(T::Text, Op::Gt, &["a"]));
        assert_invalid(build(T::Text, Op::Between, &["a", "b"]));
        assert_invalid(build(T::Number, Op::Contains, &["1"]));
        assert_invalid(build(T::Number, Op::Before, &["1"]));
        assert_invalid(build(T::Boolean, Op::Equals, &["true"]));
        assert_invalid(build(T::Boolean, Op::IsEmpty, &[]));
        assert_invalid(build(T::Date, Op::Gt, &["2026-01-01"]));
        assert_invalid(build(T::Date, Op::In, &["2026-01-01"]));
        assert_invalid(build(T::Date, Op::NotEquals, &["2026-01-01"]));
        assert_invalid(build(T::SingleSelect, Op::Contains, &["a"]));
        assert_invalid(build(T::MultiSelect, Op::Equals, &["a"]));
        assert_invalid(build(T::Json, Op::Equals, &["a"]));
    }

    #[test]
    fn text_predicates() {
        assert!(matches!(kind(T::Text, Op::Equals, &["acme"]),
            CpKind::Contains(v) if v == json!({"k": "acme"})));
        assert!(matches!(kind(T::Text, Op::NotEquals, &["acme"]),
            CpKind::NotContains(v) if v == json!({"k": "acme"})));
        assert!(matches!(kind(T::Text, Op::In, &["a", "b"]),
            CpKind::ContainsAny(v) if v == vec![json!({"k": "a"}), json!({"k": "b"})]));
        assert!(matches!(kind(T::Text, Op::IsSet, &[]), CpKind::KeyExists));
        assert!(matches!(kind(T::Text, Op::IsEmpty, &[]), CpKind::KeyEmpty));
    }

    #[test]
    fn text_contains_escapes_like_metacharacters() {
        assert!(matches!(kind(T::Text, Op::Contains, &["ac"]),
            CpKind::Ilike(p) if p == "%ac%"));
        assert!(matches!(kind(T::Email, Op::Contains, &[r"50%_off\x"]),
            CpKind::Ilike(p) if p == r"%50\%\_off\\x%"));
    }

    #[test]
    fn number_predicates_and_coercion() {
        assert!(matches!(kind(T::Number, Op::Equals, &["1e3"]),
            CpKind::NumCmp(CmpOp::Eq, n) if n == 1000.0));
        assert!(matches!(kind(T::Number, Op::NotEquals, &["2"]),
            CpKind::NumCmp(CmpOp::Ne, n) if n == 2.0));
        assert!(matches!(kind(T::Number, Op::Gt, &["2.5"]),
            CpKind::NumCmp(CmpOp::Gt, n) if n == 2.5));
        assert!(matches!(kind(T::Number, Op::Gte, &["-1"]),
            CpKind::NumCmp(CmpOp::Gte, n) if n == -1.0));
        assert!(matches!(
            kind(T::Number, Op::Lt, &["3"]),
            CpKind::NumCmp(CmpOp::Lt, _)
        ));
        assert!(matches!(
            kind(T::Number, Op::Lte, &["3"]),
            CpKind::NumCmp(CmpOp::Lte, _)
        ));
        assert!(matches!(kind(T::Number, Op::Between, &["1", "9"]),
            CpKind::NumBetween(lo, hi) if lo == 1.0 && hi == 9.0));
        assert!(matches!(kind(T::Number, Op::In, &["1", "5"]),
            CpKind::NumIn(ns) if ns == vec![1.0, 5.0]));

        assert_invalid(build(T::Number, Op::Equals, &["abc"]));
        assert_invalid(build(T::Number, Op::Gt, &["12abc"]));
        assert_invalid(build(T::Number, Op::In, &["1", "x"]));
        assert_invalid(build(T::Number, Op::Between, &["1", "x"]));
        // NaN parses as f64 but is not a JSON number.
        assert_invalid(build(T::Number, Op::Equals, &["NaN"]));
    }

    #[test]
    fn date_predicates_and_coercion() {
        assert!(matches!(kind(T::Date, Op::Equals, &["2026-01-02"]),
            CpKind::LexCmp(CmpOp::Eq, s) if s == "2026-01-02"));
        assert!(matches!(kind(T::Date, Op::Before, &["2026-01-02"]),
            CpKind::LexCmp(CmpOp::Lt, s) if s == "2026-01-02"));
        assert!(matches!(kind(T::Date, Op::After, &["2026-01-02"]),
            CpKind::LexCmp(CmpOp::Gt, s) if s == "2026-01-02"));
        assert!(
            matches!(kind(T::Date, Op::Between, &["2026-01-01", "2026-12-31"]),
            CpKind::LexBetween(lo, hi) if lo == "2026-01-01" && hi == "2026-12-31")
        );

        assert_invalid(build(T::Date, Op::Equals, &["01/02/2026"]));
        assert_invalid(build(T::Date, Op::Before, &["2026-13-01"]));
        assert_invalid(build(T::Date, Op::Between, &["2026-01-01", "nope"]));
    }

    #[test]
    fn datetime_values_normalize_to_stored_rfc3339() {
        // Offset-less `datetime-local` input is interpreted as UTC and normalized.
        assert!(
            matches!(kind(T::Datetime, Op::After, &["2026-01-02T10:30"]),
            CpKind::LexCmp(CmpOp::Gt, s) if s == "2026-01-02T10:30:00Z")
        );
        // Offsets are converted to UTC.
        assert!(
            matches!(kind(T::Datetime, Op::Before, &["2026-01-02T10:30:00+02:00"]),
            CpKind::LexCmp(CmpOp::Lt, s) if s == "2026-01-02T08:30:00Z")
        );
        assert_invalid(build(T::Datetime, Op::Equals, &["yesterday"]));
    }

    #[test]
    fn boolean_predicates() {
        assert!(matches!(kind(T::Boolean, Op::IsTrue, &[]),
            CpKind::Contains(v) if v == json!({"k": true})));
        assert!(matches!(kind(T::Boolean, Op::IsFalse, &[]),
            CpKind::Contains(v) if v == json!({"k": false})));
        assert!(matches!(
            kind(T::Boolean, Op::IsSet, &[]),
            CpKind::KeyExists
        ));
    }

    #[test]
    fn select_predicates() {
        assert!(matches!(kind(T::SingleSelect, Op::Equals, &["EU"]),
            CpKind::Contains(v) if v == json!({"k": "EU"})));
        assert!(matches!(kind(T::SingleSelect, Op::In, &["EU", "US"]),
            CpKind::ContainsAny(v) if v == vec![json!({"k": "EU"}), json!({"k": "US"})]));
        // Multi-select wraps each value in an array for containment.
        assert!(matches!(kind(T::MultiSelect, Op::Contains, &["x"]),
            CpKind::Contains(v) if v == json!({"k": ["x"]})));
        assert!(matches!(kind(T::MultiSelect, Op::In, &["x", "y"]),
            CpKind::ContainsAny(v) if v == vec![json!({"k": ["x"]}), json!({"k": ["y"]})]));
    }

    #[test]
    fn arity_checks() {
        // BETWEEN needs two values.
        assert_invalid(build(T::Number, Op::Between, &[]));
        assert_invalid(build(T::Number, Op::Between, &["1"]));
        assert_invalid(build(T::Date, Op::Between, &["2026-01-01"]));
        // IN needs at least one.
        assert_invalid(build(T::Text, Op::In, &[]));
        assert_invalid(build(T::Number, Op::In, &[]));
        assert_invalid(build(T::MultiSelect, Op::In, &[]));
        // Single-value operators need one.
        for (t, op) in [
            (T::Text, Op::Equals),
            (T::Text, Op::NotEquals),
            (T::Text, Op::Contains),
            (T::Number, Op::Gt),
            (T::Date, Op::Before),
            (T::Date, Op::After),
        ] {
            assert_invalid(build(t, op, &[]));
        }
        // Valueless operators take none.
        assert!(matches!(kind(T::Number, Op::IsSet, &[]), CpKind::KeyExists));
        assert!(matches!(
            kind(T::Number, Op::IsEmpty, &[]),
            CpKind::KeyEmpty
        ));
        // Extra values on a valueless operator are ignored rather than rejected (current
        // behavior; `FilterOperator::takes_values` is not enforced at resolution).
        assert!(matches!(
            kind(T::Text, Op::IsSet, &["stray"]),
            CpKind::KeyExists
        ));
    }

    #[test]
    fn unknown_key_is_rejected() {
        assert_invalid(build_cp_filter(
            &[def("known", T::Text)],
            "unknown",
            Op::IsSet,
            &[],
        ));
    }

    fn filter(field: &str, op: FilterOperator, values: &[&str]) -> Filter {
        Filter {
            field: FieldRef::parse(field),
            op,
            values: vals(values),
        }
    }

    fn native(entity: ListEntity, f: Filter) -> StoreResult<NativeFilter> {
        match resolve_list_filters(entity, &[], vec![f])?.remove(0) {
            ListFilter::Native(n) => Ok(n),
            other => panic!("expected a native filter, got {other:?}"),
        }
    }

    #[test]
    fn resolve_list_filters_mixes_cp_and_native() {
        let defs = vec![def("tier", T::Text), def("seats", T::Number)];
        let resolved = resolve_list_filters(
            ListEntity::Invoice,
            &defs,
            vec![
                filter("cp:tier", Op::Equals, &["gold"]),
                filter("cp:seats", Op::Gt, &["3"]),
                filter("currency", Op::In, &["eur"]),
            ],
        )
        .unwrap();
        assert_eq!(resolved.len(), 3);
        assert!(matches!(&resolved[1], ListFilter::CustomProperty(cp)
            if cp.key == "seats" && matches!(cp.kind, CpKind::NumCmp(CmpOp::Gt, n) if n == 3.0)));
        // Codes are normalized to upper case.
        assert!(matches!(&resolved[2], ListFilter::Native(NativeFilter {
            column: "invoice.currency",
            kind: NativeKind::In { values, negated: false },
            ..
        }) if values == &vec!["EUR".to_string()]));

        // Unknown native fields (or ones from another list), and one bad filter, fail the set.
        for bad in [
            vec![filter("status", Op::Equals, &["DRAFT"])],
            vec![filter("mrr", Op::Gt, &["1"])],
            vec![
                filter("cp:tier", Op::IsSet, &[]),
                filter("cp:nope", Op::IsSet, &[]),
            ],
        ] {
            let err = resolve_list_filters(ListEntity::Invoice, &defs, bad).unwrap_err();
            assert!(matches!(
                err.current_context(),
                StoreError::InvalidArgument(_)
            ));
        }
    }

    #[test]
    fn native_values_are_validated() {
        let c = ListEntity::Customer;
        assert_invalid(native(c, filter("country", Op::Equals, &["ZZ"])));
        assert_invalid(native(c, filter("currency", Op::In, &["EUR", "nope"])));
        assert_invalid(native(c, filter("balance", Op::Gt, &["1,5"])));
        assert_invalid(native(c, filter("created_at", Op::Before, &["01/02/2026"])));
        assert_invalid(native(c, filter("billing_email", Op::Equals, &["x"])));
        assert_invalid(native(
            ListEntity::Subscription,
            filter("plan", Op::In, &["cus_abc"]),
        ));
    }

    #[test]
    fn money_is_scaled_per_row_currency() {
        let n = native(ListEntity::Invoice, filter("total", Op::Gt, &["100.5"])).unwrap();
        match n.kind {
            NativeKind::MoneyCmp {
                op: CmpOp::Gt,
                value,
                scale_sql,
            } => {
                assert_eq!(value, "100.5");
                assert!(scale_sql.starts_with("(CASE invoice.currency"));
                assert!(scale_sql.contains("WHEN 'JPY' THEN 0"));
                assert!(scale_sql.ends_with("ELSE 2 END)"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn whole_days_on_timestamp_columns() {
        let at = |op, v: &[&str]| {
            native(ListEntity::Invoice, filter("due_at", op, v))
                .unwrap()
                .kind
        };
        assert!(matches!(at(Op::Before, &["2026-02-10"]),
            NativeKind::Cmp(CmpOp::Lt, s) if s == "2026-02-10 00:00:00.000000"));
        // "After a day" starts the next day.
        assert!(matches!(at(Op::After, &["2026-02-10"]),
            NativeKind::Cmp(CmpOp::Gte, s) if s == "2026-02-11 00:00:00.000000"));
        assert!(matches!(at(Op::Between, &["2026-02-10", "2026-02-10"]),
            NativeKind::Between(lo, hi)
                if lo == "2026-02-10 00:00:00.000000" && hi == "2026-02-10 23:59:59.999999"));
        // Instants are kept as given (in UTC); timestamptz columns carry the offset.
        let q = native(
            ListEntity::Quote,
            filter("created_at", Op::After, &["2026-02-10T10:00:00+02:00"]),
        )
        .unwrap();
        assert!(
            matches!(q.kind, NativeKind::Cmp(CmpOp::Gt, s) if s == "2026-02-10 08:00:00.000000+00")
        );
        assert_eq!(q.cast, NativeCast::Timestamptz);
    }
}
