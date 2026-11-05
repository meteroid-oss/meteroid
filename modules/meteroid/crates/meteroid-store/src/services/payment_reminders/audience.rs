//! Audience JSON → bound-only SQL over the `audience_facts.sql` CTE, plus the save-time
//! validation of an audience against the tenant's catalog.

use crate::constants::{Countries, Currencies};
use crate::domain::custom_property_definitions::CustomPropertyDefinition;
use crate::domain::enums::{CustomPropertyEntityType, CustomPropertyType};
use crate::domain::payment_reminders::{
    Audience, AudienceAttribute, AudiencePredicate, CmpOp, PropertyOp,
};
use crate::errors::StoreError;
use crate::store::PgConn;
use crate::{StoreResult, domain};
use common_domain::ids::{BaseId, CustomerId, InvoiceId, InvoicingEntityId, PlanId, TenantId};
use diesel::pg::Pg;
use diesel::query_builder::{BoxedSqlQuery, SqlQuery};
use diesel::sql_types;
use diesel_async::RunQueryDsl;
use diesel_models::custom_property_definitions::CustomPropertyDefinitionRow;
use diesel_models::customers::CustomerRow;
use diesel_models::errors::DatabaseErrorContainer;
use diesel_models::plans::PlanRow;
use diesel_models::query::payment_reminders::{
    AUDIENCE_FACTS_SQL, AudienceCustomerMatchRow, AudienceMatchRow, CUSTOMER_AUDIENCE_FACTS_SQL,
};
use error_stack::Report;
use std::collections::HashSet;
use uuid::Uuid;

pub const MAX_PLAN_IDS: usize = 100;
pub const MAX_CUSTOMER_IDS: usize = 500;
pub const MAX_LEAVES: usize = 50;
pub const MAX_DEPTH: usize = 2;

const PLAN_TYPES: [&str; 3] = ["STANDARD", "FREE", "CUSTOM"];
const INVOICE_KINDS: [&str; 3] = ["subscription", "one_off", "consolidated"];
const PAYMENT_METHOD_TYPES: [&str; 4] = ["CARD", "DIRECT_DEBIT", "TRANSFER", "NONE"];

/// A positional parameter of the compiled predicate. `Numeric` carries the decimal's text and
/// is cast in SQL, so no arbitrary-precision type crosses the driver boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum Bind {
    Text(String),
    TextArray(Vec<String>),
    UuidArray(Vec<Uuid>),
    Bool(bool),
    Numeric(String),
    BigInt(i64),
}

/// `sql` references `f.*` columns and `$n` placeholders only; `binds[i]` is `$(i + 3)`.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledPredicate {
    pub sql: String,
    pub binds: Vec<Bind>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AudienceError {
    #[error("Segment audiences are not supported yet")]
    Unsupported,
    #[error("{0}")]
    Invalid(String),
}

impl From<AudienceError> for Report<StoreError> {
    fn from(e: AudienceError) -> Self {
        Report::new(StoreError::InvalidArgument(e.to_string()))
    }
}

struct Compiler {
    binds: Vec<Bind>,
}

impl Compiler {
    fn push(&mut self, bind: Bind) -> String {
        self.binds.push(bind);
        format!("${}", self.binds.len() + 2)
    }

    fn predicate(&mut self, p: &AudiencePredicate) -> Result<String, AudienceError> {
        Ok(match p {
            AudiencePredicate::Attribute { key, values } => {
                let bind = self.push(Bind::TextArray(values.clone()));
                match key {
                    AudienceAttribute::PlanType => format!("f.plan_types && {bind}::text[]"),
                    AudienceAttribute::InvoiceKind => {
                        format!("f.invoice_kind = ANY({bind}::text[])")
                    }
                    AudienceAttribute::PaymentMethodType => {
                        format!("f.payment_method_type = ANY({bind}::text[])")
                    }
                    AudienceAttribute::Currency => format!("f.currency = ANY({bind}::text[])"),
                    AudienceAttribute::Country => format!("f.country = ANY({bind}::text[])"),
                }
            }
            AudiencePredicate::Plan { plan_ids } => {
                let bind = self.push(Bind::UuidArray(
                    plan_ids.iter().map(|id| id.as_uuid()).collect(),
                ));
                format!("f.plan_ids && {bind}::uuid[]")
            }
            AudiencePredicate::Customer { customer_ids } => {
                let bind = self.push(Bind::UuidArray(
                    customer_ids.iter().map(|id| id.as_uuid()).collect(),
                ));
                format!("f.customer_id = ANY({bind}::uuid[])")
            }
            AudiencePredicate::Amount {
                op,
                threshold,
                currency,
            } => {
                let currency = self.push(Bind::Text(currency.clone()));
                let threshold = self.push(Bind::BigInt(*threshold));
                format!(
                    "(f.currency = {currency} AND f.amount_due {} {threshold})",
                    cmp_sql(*op)
                )
            }
            AudiencePredicate::Property {
                key,
                op,
                values,
                value,
            } => self.property(key, *op, values, value.as_ref())?,
            AudiencePredicate::Segment { .. } => return Err(AudienceError::Unsupported),
            AudiencePredicate::And { children } => self.group(children, " AND ")?,
            AudiencePredicate::Or { children } => self.group(children, " OR ")?,
            AudiencePredicate::Not { child } => format!("(NOT {})", self.predicate(child)?),
        })
    }

    fn group(
        &mut self,
        children: &[AudiencePredicate],
        joiner: &str,
    ) -> Result<String, AudienceError> {
        let parts = children
            .iter()
            .map(|c| self.predicate(c))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(format!("({})", parts.join(joiner)))
    }

    fn property(
        &mut self,
        key: &str,
        op: PropertyOp,
        values: &[String],
        value: Option<&serde_json::Value>,
    ) -> Result<String, AudienceError> {
        let k = self.push(Bind::Text(key.to_string()));
        Ok(match op {
            PropertyOp::In => {
                let v = self.push(Bind::TextArray(values.to_vec()));
                format!("(f.props ->> {k}) = ANY({v}::text[])")
            }
            PropertyOp::Any => {
                let v = self.push(Bind::TextArray(values.to_vec()));
                format!("(f.props -> {k}) ?| {v}::text[]")
            }
            PropertyOp::Eq => match value {
                Some(serde_json::Value::Bool(b)) => {
                    let b = self.push(Bind::Bool(*b));
                    format!("(f.props -> {k}) = to_jsonb({b}::boolean)")
                }
                Some(serde_json::Value::Number(n)) => self.numeric(&k, op, n),
                _ => {
                    return Err(AudienceError::Invalid(format!(
                        "Property {key}: eq needs a boolean or a number"
                    )));
                }
            },
            PropertyOp::Gt | PropertyOp::Gte | PropertyOp::Lt | PropertyOp::Lte => {
                let n = value.and_then(|v| v.as_number()).ok_or_else(|| {
                    AudienceError::Invalid(format!(
                        "Property {key}: {} needs a number",
                        op_name(op)
                    ))
                })?;
                self.numeric(&k, op, n)
            }
            PropertyOp::Set => format!("(f.props ? {k})"),
            PropertyOp::NotSet => format!("NOT (f.props ? {k})"),
        })
    }

    /// CASE guards the cast whatever order the planner evaluates in; a missing or
    /// non-numeric value compares as FALSE (so `not` matches it).
    fn numeric(&mut self, k: &str, op: PropertyOp, n: &serde_json::Number) -> String {
        let n = self.push(Bind::Numeric(n.to_string()));
        format!(
            "COALESCE((CASE WHEN jsonb_typeof(f.props -> {k}) = 'number' THEN (f.props ->> {k})::numeric END) {} {n}::numeric, FALSE)",
            property_cmp_sql(op)
        )
    }
}

fn cmp_sql(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "=",
        CmpOp::Gt => ">",
        CmpOp::Gte => ">=",
        CmpOp::Lt => "<",
        CmpOp::Lte => "<=",
    }
}

fn property_cmp_sql(op: PropertyOp) -> &'static str {
    match op {
        PropertyOp::Gt => ">",
        PropertyOp::Gte => ">=",
        PropertyOp::Lt => "<",
        PropertyOp::Lte => "<=",
        _ => "=",
    }
}

fn op_name(op: PropertyOp) -> &'static str {
    match op {
        PropertyOp::In => "in",
        PropertyOp::Any => "any",
        PropertyOp::Eq => "eq",
        PropertyOp::Gt => "gt",
        PropertyOp::Gte => "gte",
        PropertyOp::Lt => "lt",
        PropertyOp::Lte => "lte",
        PropertyOp::Set => "set",
        PropertyOp::NotSet => "not_set",
    }
}

/// Root predicates are an implicit AND; an empty audience matches everything.
pub fn compile(audience: &Audience) -> Result<CompiledPredicate, AudienceError> {
    let mut compiler = Compiler { binds: Vec::new() };
    let sql = if audience.predicates.is_empty() {
        "TRUE".to_string()
    } else {
        compiler.group(&audience.predicates, " AND ")?
    };
    Ok(CompiledPredicate {
        sql,
        binds: compiler.binds,
    })
}

fn bound_query<'a>(
    facts_sql: &str,
    projection: &str,
    tenant_id: TenantId,
    ids: Vec<Uuid>,
    compiled: &CompiledPredicate,
) -> BoxedSqlQuery<'a, Pg, SqlQuery> {
    let sql = format!(
        "{facts_sql} SELECT {projection} FROM f WHERE {}",
        compiled.sql
    );
    let mut query = diesel::sql_query(sql)
        .into_boxed()
        .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
        .bind::<sql_types::Array<sql_types::Uuid>, _>(ids);
    for bind in &compiled.binds {
        query = match bind.clone() {
            Bind::Text(v) => query.bind::<sql_types::Text, _>(v),
            Bind::TextArray(v) => query.bind::<sql_types::Array<sql_types::Text>, _>(v),
            Bind::UuidArray(v) => query.bind::<sql_types::Array<sql_types::Uuid>, _>(v),
            Bind::Bool(v) => query.bind::<sql_types::Bool, _>(v),
            Bind::Numeric(v) => query.bind::<sql_types::Text, _>(v),
            Bind::BigInt(v) => query.bind::<sql_types::BigInt, _>(v),
        };
    }
    query
}

/// The invoices of `ids` (all of `tenant_id`) the predicate matches, over live customer data.
pub async fn evaluate_audience(
    conn: &mut PgConn,
    tenant_id: TenantId,
    ids: &[InvoiceId],
    compiled: &CompiledPredicate,
) -> StoreResult<Vec<InvoiceId>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let rows: Vec<AudienceMatchRow> = bound_query(
        AUDIENCE_FACTS_SQL,
        "f.invoice_id",
        tenant_id,
        ids.iter().map(|id| id.as_uuid()).collect(),
        compiled,
    )
    .get_results(conn)
    .await
    .map_err(|e| Into::<Report<StoreError>>::into(DatabaseErrorContainer::from(e)))?;
    Ok(rows
        .into_iter()
        .map(|r| InvoiceId::from(r.invoice_id))
        .collect())
}

/// Same over customers, for an audience that passed [`check_customer_level`].
pub async fn evaluate_customer_audience(
    conn: &mut PgConn,
    tenant_id: TenantId,
    ids: &[CustomerId],
    compiled: &CompiledPredicate,
) -> StoreResult<Vec<CustomerId>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let rows: Vec<AudienceCustomerMatchRow> = bound_query(
        CUSTOMER_AUDIENCE_FACTS_SQL,
        "f.customer_id",
        tenant_id,
        ids.iter().map(|id| id.as_uuid()).collect(),
        compiled,
    )
    .get_results(conn)
    .await
    .map_err(|e| Into::<Report<StoreError>>::into(DatabaseErrorContainer::from(e)))?;
    Ok(rows
        .into_iter()
        .map(|r| CustomerId::from(r.customer_id))
        .collect())
}

// ---- save-time validation ---------------------------------------------------------------

fn invalid(msg: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(msg.into()))
}

#[derive(Default)]
struct Refs<'a> {
    plan_ids: HashSet<PlanId>,
    customer_ids: HashSet<CustomerId>,
    property_keys: Vec<(
        &'a str,
        PropertyOp,
        &'a [String],
        Option<&'a serde_json::Value>,
    )>,
    leaves: usize,
}

fn walk<'a>(p: &'a AudiencePredicate, depth: usize, refs: &mut Refs<'a>) -> StoreResult<()> {
    match p {
        AudiencePredicate::And { children } | AudiencePredicate::Or { children } => {
            if depth >= MAX_DEPTH {
                return Err(invalid(format!(
                    "Audience groups nest at most {MAX_DEPTH} deep"
                )));
            }
            if children.is_empty() {
                return Err(invalid("Audience groups need at least one predicate"));
            }
            for c in children {
                walk(c, depth + 1, refs)?;
            }
            return Ok(());
        }
        AudiencePredicate::Not { child } => {
            if depth >= MAX_DEPTH {
                return Err(invalid(format!(
                    "Audience groups nest at most {MAX_DEPTH} deep"
                )));
            }
            return walk(child, depth + 1, refs);
        }
        _ => {}
    }
    refs.leaves += 1;
    match p {
        AudiencePredicate::Attribute { key, values } => {
            if values.is_empty() {
                return Err(invalid("Attribute predicates need at least one value"));
            }
            for v in values {
                let ok = match key {
                    AudienceAttribute::PlanType => PLAN_TYPES.contains(&v.as_str()),
                    AudienceAttribute::InvoiceKind => INVOICE_KINDS.contains(&v.as_str()),
                    AudienceAttribute::PaymentMethodType => {
                        PAYMENT_METHOD_TYPES.contains(&v.as_str())
                    }
                    AudienceAttribute::Currency => Currencies::resolve_currency(v).is_some(),
                    AudienceAttribute::Country => Countries::resolve_country(v).is_some(),
                };
                if !ok {
                    return Err(invalid(format!(
                        "Unknown value {v} for {}",
                        attribute_name(*key)
                    )));
                }
            }
        }
        AudiencePredicate::Plan { plan_ids } => {
            if plan_ids.is_empty() || plan_ids.len() > MAX_PLAN_IDS {
                return Err(invalid(format!(
                    "Plan predicates take 1 to {MAX_PLAN_IDS} plans"
                )));
            }
            refs.plan_ids.extend(plan_ids.iter().copied());
        }
        AudiencePredicate::Customer { customer_ids } => {
            if customer_ids.is_empty() || customer_ids.len() > MAX_CUSTOMER_IDS {
                return Err(invalid(format!(
                    "Customer predicates take 1 to {MAX_CUSTOMER_IDS} customers"
                )));
            }
            refs.customer_ids.extend(customer_ids.iter().copied());
        }
        AudiencePredicate::Amount {
            threshold,
            currency,
            ..
        } => {
            if *threshold < 0 {
                return Err(invalid("Amount threshold must be positive"));
            }
            if Currencies::resolve_currency(currency).is_none() {
                return Err(invalid(format!("Unknown currency {currency}")));
            }
        }
        AudiencePredicate::Property {
            key,
            op,
            values,
            value,
        } => {
            match op {
                PropertyOp::In | PropertyOp::Any if values.is_empty() => {
                    return Err(invalid(format!(
                        "Property {key}: {} needs values",
                        op_name(*op)
                    )));
                }
                PropertyOp::Eq
                    if !matches!(
                        value,
                        Some(serde_json::Value::Bool(_)) | Some(serde_json::Value::Number(_))
                    ) =>
                {
                    return Err(invalid(format!(
                        "Property {key}: eq needs a boolean or a number"
                    )));
                }
                PropertyOp::Gt | PropertyOp::Gte | PropertyOp::Lt | PropertyOp::Lte
                    if value.as_ref().and_then(|v| v.as_number()).is_none() =>
                {
                    return Err(invalid(format!(
                        "Property {key}: {} needs a number",
                        op_name(*op)
                    )));
                }
                _ => {}
            }
            refs.property_keys
                .push((key.as_str(), *op, values.as_slice(), value.as_ref()));
        }
        AudiencePredicate::Segment { .. } => return Err(AudienceError::Unsupported.into()),
        AudiencePredicate::And { .. }
        | AudiencePredicate::Or { .. }
        | AudiencePredicate::Not { .. } => {}
    }
    Ok(())
}

fn attribute_name(key: AudienceAttribute) -> &'static str {
    match key {
        AudienceAttribute::PlanType => "plan_type",
        AudienceAttribute::InvoiceKind => "invoice_kind",
        AudienceAttribute::PaymentMethodType => "payment_method_type",
        AudienceAttribute::Currency => "currency",
        AudienceAttribute::Country => "country",
    }
}

fn check_property(
    definitions: &[CustomPropertyDefinition],
    key: &str,
    op: PropertyOp,
    values: &[String],
    value: Option<&serde_json::Value>,
) -> StoreResult<()> {
    let def = definitions
        .iter()
        .find(|d| d.key == key)
        .ok_or_else(|| invalid(format!("Unknown customer property {key}")))?;
    let t = def.property_type;
    let ok = match op {
        PropertyOp::In => matches!(
            t,
            CustomPropertyType::Text
                | CustomPropertyType::SingleSelect
                | CustomPropertyType::Url
                | CustomPropertyType::Email
        ),
        PropertyOp::Any => t == CustomPropertyType::MultiSelect,
        PropertyOp::Eq => match value {
            Some(serde_json::Value::Bool(_)) => t == CustomPropertyType::Boolean,
            Some(serde_json::Value::Number(_)) => t == CustomPropertyType::Number,
            _ => false,
        },
        PropertyOp::Gt | PropertyOp::Gte | PropertyOp::Lt | PropertyOp::Lte => {
            t == CustomPropertyType::Number
        }
        PropertyOp::Set | PropertyOp::NotSet => !matches!(
            t,
            CustomPropertyType::Date | CustomPropertyType::Datetime | CustomPropertyType::Json
        ),
    };
    if !ok {
        return Err(invalid(format!(
            "Property {key}: operator {} does not apply to a {:?} property",
            op_name(op),
            t
        )));
    }
    if matches!(
        t,
        CustomPropertyType::SingleSelect | CustomPropertyType::MultiSelect
    ) && let Some(options) = def.config.options.as_deref()
        && !options.is_empty()
    {
        for v in values {
            if !options.iter().any(|o| o.value == *v) {
                return Err(invalid(format!(
                    "Property {key}: {v} is not one of its options"
                )));
            }
        }
    }
    Ok(())
}

/// The first leaf that needs an invoice (plan, amount, invoice kind, payment method, currency).
fn invoice_only_leaf(p: &AudiencePredicate) -> Option<&'static str> {
    match p {
        AudiencePredicate::And { children } | AudiencePredicate::Or { children } => {
            children.iter().find_map(invoice_only_leaf)
        }
        AudiencePredicate::Not { child } => invoice_only_leaf(child),
        AudiencePredicate::Attribute { key, .. } => match key {
            AudienceAttribute::Country => None,
            other => Some(attribute_name(*other)),
        },
        AudiencePredicate::Plan { .. } => Some("plan"),
        AudiencePredicate::Amount { .. } => Some("amount"),
        AudiencePredicate::Customer { .. }
        | AudiencePredicate::Property { .. }
        | AudiencePredicate::Segment { .. } => None,
    }
}

/// For kinds without an invoice (quotes): only customers, country and custom properties.
pub fn check_customer_level(audience: &Audience) -> StoreResult<()> {
    match audience.predicates.iter().find_map(invoice_only_leaf) {
        Some(leaf) => Err(invalid(format!(
            "This email is not tied to an invoice: {leaf} rules cannot apply, use customers, country or customer properties"
        ))),
        None => Ok(()),
    }
}

/// Save-time checks: literal sets, catalog references in the tenant (customers in the
/// entity), property keys and operators, structural limits, `segment` rejected.
pub async fn validate_audience(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoicing_entity_id: InvoicingEntityId,
    audience: &Audience,
) -> StoreResult<()> {
    if audience.predicates.is_empty() {
        return Err(invalid("An audience needs at least one predicate"));
    }
    let mut refs = Refs::default();
    for p in &audience.predicates {
        walk(p, 0, &mut refs)?;
    }
    if refs.leaves > MAX_LEAVES {
        return Err(invalid(format!(
            "An audience has at most {MAX_LEAVES} predicates"
        )));
    }

    if !refs.plan_ids.is_empty() {
        let ids: Vec<PlanId> = refs.plan_ids.iter().copied().collect();
        let found = PlanRow::list_by_ids(conn, &ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if found.len() != ids.len() {
            return Err(invalid("Audience references a plan that does not exist"));
        }
    }
    if !refs.customer_ids.is_empty() {
        let ids: Vec<CustomerId> = refs.customer_ids.iter().copied().collect();
        let found = CustomerRow::list_by_ids(conn, &tenant_id, ids.clone())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if found.len() != ids.len() {
            return Err(invalid(
                "Audience references a customer that does not exist",
            ));
        }
        if found
            .iter()
            .any(|c| c.invoicing_entity_id != invoicing_entity_id)
        {
            return Err(invalid(
                "Audience references a customer of another invoicing entity",
            ));
        }
    }
    if !refs.property_keys.is_empty() {
        let definitions = CustomPropertyDefinitionRow::list_active_for_entity(
            conn,
            tenant_id,
            CustomPropertyEntityType::Customer.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<CustomPropertyDefinition>, _>>()?;
        for (key, op, values, value) in &refs.property_keys {
            check_property(&definitions, key, *op, values, *value)?;
        }
    }
    Ok(())
}

/// Parses the JSON envelope; unknown leaves are a deserialization error and surface as invalid.
pub fn parse_audience(json: &str) -> StoreResult<domain::payment_reminders::Audience> {
    serde_json::from_str(json).map_err(|e| invalid(format!("Invalid audience: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn audience(predicates: serde_json::Value) -> Audience {
        serde_json::from_value(json!({ "predicates": predicates })).unwrap()
    }

    fn compile_one(predicate: serde_json::Value) -> CompiledPredicate {
        compile(&audience(json!([predicate]))).unwrap()
    }

    fn assert_bound_only(sql: &str) {
        // Every literal the operator typed must travel as a parameter, never inline.
        assert!(!sql.contains('\''), "literal in {sql}");
        assert!(
            !sql.contains("--") && !sql.contains(';'),
            "suspicious {sql}"
        );
    }

    #[test]
    fn compile_emits_bound_parameters_only_per_leaf() {
        let plan_id = PlanId::new();
        let customer_id = CustomerId::new();
        let cases: Vec<(serde_json::Value, &str, Vec<Bind>)> = vec![
            (
                json!({ "attribute": { "key": "plan_type", "values": ["STANDARD", "CUSTOM"] } }),
                "(f.plan_types && $3::text[])",
                vec![Bind::TextArray(vec!["STANDARD".into(), "CUSTOM".into()])],
            ),
            (
                json!({ "attribute": { "key": "invoice_kind", "values": ["one_off"] } }),
                "(f.invoice_kind = ANY($3::text[]))",
                vec![Bind::TextArray(vec!["one_off".into()])],
            ),
            (
                json!({ "attribute": { "key": "payment_method_type", "values": ["CARD"] } }),
                "(f.payment_method_type = ANY($3::text[]))",
                vec![Bind::TextArray(vec!["CARD".into()])],
            ),
            (
                json!({ "attribute": { "key": "currency", "values": ["EUR"] } }),
                "(f.currency = ANY($3::text[]))",
                vec![Bind::TextArray(vec!["EUR".into()])],
            ),
            (
                json!({ "attribute": { "key": "country", "values": ["FR", "BE"] } }),
                "(f.country = ANY($3::text[]))",
                vec![Bind::TextArray(vec!["FR".into(), "BE".into()])],
            ),
            (
                json!({ "plan": { "plan_ids": [plan_id.as_base62()] } }),
                "(f.plan_ids && $3::uuid[])",
                vec![Bind::UuidArray(vec![plan_id.as_uuid()])],
            ),
            (
                json!({ "customer": { "customer_ids": [customer_id.as_base62()] } }),
                "(f.customer_id = ANY($3::uuid[]))",
                vec![Bind::UuidArray(vec![customer_id.as_uuid()])],
            ),
            (
                json!({ "amount": { "op": "gte", "threshold": 50000, "currency": "EUR" } }),
                "((f.currency = $3 AND f.amount_due >= $4))",
                vec![Bind::Text("EUR".into()), Bind::BigInt(50000)],
            ),
            (
                json!({ "property": { "key": "tier", "op": "in", "values": ["enterprise"] } }),
                "((f.props ->> $3) = ANY($4::text[]))",
                vec![
                    Bind::Text("tier".into()),
                    Bind::TextArray(vec!["enterprise".into()]),
                ],
            ),
            (
                json!({ "property": { "key": "tags", "op": "any", "values": ["vip"] } }),
                "((f.props -> $3) ?| $4::text[])",
                vec![
                    Bind::Text("tags".into()),
                    Bind::TextArray(vec!["vip".into()]),
                ],
            ),
            (
                json!({ "property": { "key": "vip", "op": "eq", "value": true } }),
                "((f.props -> $3) = to_jsonb($4::boolean))",
                vec![Bind::Text("vip".into()), Bind::Bool(true)],
            ),
            (
                json!({ "property": { "key": "seats", "op": "gte", "value": 10 } }),
                "(COALESCE((CASE WHEN jsonb_typeof(f.props -> $3) = 'number' THEN (f.props ->> $3)::numeric END) >= $4::numeric, FALSE))",
                vec![Bind::Text("seats".into()), Bind::Numeric("10".into())],
            ),
            (
                json!({ "property": { "key": "seats", "op": "eq", "value": 10.5 } }),
                "(COALESCE((CASE WHEN jsonb_typeof(f.props -> $3) = 'number' THEN (f.props ->> $3)::numeric END) = $4::numeric, FALSE))",
                vec![Bind::Text("seats".into()), Bind::Numeric("10.5".into())],
            ),
            (
                json!({ "property": { "key": "seats", "op": "set" } }),
                "((f.props ? $3))",
                vec![Bind::Text("seats".into())],
            ),
            (
                json!({ "property": { "key": "seats", "op": "not_set" } }),
                "(NOT (f.props ? $3))",
                vec![Bind::Text("seats".into())],
            ),
        ];
        for (predicate, sql, binds) in cases {
            let compiled = compile_one(predicate.clone());
            assert_eq!(compiled.sql, sql, "{predicate}");
            assert_eq!(compiled.binds, binds, "{predicate}");
            // The only quote in any emitted SQL is the fixed jsonb_typeof literal.
            assert_bound_only(&compiled.sql.replace("'number'", ""));
        }
    }

    #[test]
    fn compile_nested_groups_parenthesise() {
        let compiled = compile(&audience(json!([
            { "attribute": { "key": "currency", "values": ["EUR"] } },
            { "or": { "children": [
                { "attribute": { "key": "country", "values": ["FR"] } },
                { "not": { "child": { "property": { "key": "vip", "op": "set" } } } }
            ] } }
        ])))
        .unwrap();
        assert_eq!(
            compiled.sql,
            "(f.currency = ANY($3::text[]) AND (f.country = ANY($4::text[]) OR (NOT (f.props ? $5))))"
        );
        assert_eq!(compiled.binds.len(), 3);
        assert_eq!(compile(&Audience::default()).unwrap().sql, "TRUE");
    }

    #[test]
    fn compile_never_inlines_operator_input() {
        let hostile = "x'; DROP TABLE invoice; -- \"";
        let compiled = compile(&audience(json!([
            { "property": { "key": hostile, "op": "in", "values": [hostile] } },
            { "property": { "key": hostile, "op": "eq", "value": 1 } },
            { "attribute": { "key": "country", "values": [hostile] } },
            { "amount": { "op": "eq", "threshold": 1, "currency": hostile } }
        ])))
        .unwrap();
        assert!(!compiled.sql.contains("DROP"), "{}", compiled.sql);
        assert_bound_only(&compiled.sql.replace("'number'", ""));
        assert_eq!(compiled.binds.len(), 7);
        assert_eq!(compiled.binds[0], Bind::Text(hostile.into()));
        assert_eq!(compiled.binds[2], Bind::Text(hostile.into()));
    }

    #[test]
    fn compile_rejects_segment() {
        let err = compile(&audience(json!([
            { "segment": { "segment_id": common_domain::ids::SegmentId::new().as_base62() } }
        ])))
        .unwrap_err();
        assert_eq!(err, AudienceError::Unsupported);
    }

    fn walk_one(predicate: serde_json::Value) -> Result<usize, String> {
        let a = audience(json!([predicate]));
        let mut refs = Refs::default();
        walk(&a.predicates[0], 0, &mut refs)
            .map(|_| refs.leaves)
            .map_err(|e| e.to_string())
    }

    #[test]
    fn customer_level_rejects_invoice_only_leaves_at_any_depth() {
        let customer_id = CustomerId::new();
        for ok in [
            json!([{ "attribute": { "key": "country", "values": ["FR"] } }]),
            json!([{ "customer": { "customer_ids": [customer_id.as_base62()] } }]),
            json!([{ "or": { "children": [
                { "property": { "key": "tier", "op": "in", "values": ["gold"] } },
                { "not": { "child": { "property": { "key": "vip", "op": "set" } } } }
            ] } }]),
        ] {
            assert!(check_customer_level(&audience(ok.clone())).is_ok(), "{ok}");
        }
        for (bad, leaf) in [
            (
                json!([{ "attribute": { "key": "plan_type", "values": ["FREE"] } }]),
                "plan_type",
            ),
            (
                json!([{ "attribute": { "key": "invoice_kind", "values": ["one_off"] } }]),
                "invoice_kind",
            ),
            (
                json!([{ "attribute": { "key": "payment_method_type", "values": ["CARD"] } }]),
                "payment_method_type",
            ),
            (
                json!([{ "attribute": { "key": "currency", "values": ["EUR"] } }]),
                "currency",
            ),
            (
                json!([{ "plan": { "plan_ids": [PlanId::new().as_base62()] } }]),
                "plan",
            ),
            (
                json!([{ "attribute": { "key": "country", "values": ["FR"] } }, { "or": { "children": [
                    { "not": { "child": { "amount": { "op": "gte", "threshold": 1, "currency": "EUR" } } } }
                ] } }]),
                "amount",
            ),
        ] {
            let err = check_customer_level(&audience(bad.clone())).unwrap_err();
            assert!(
                matches!(err.current_context(), StoreError::InvalidArgument(m) if m.contains(leaf)),
                "{bad}: {err:?}"
            );
        }
    }

    #[test]
    fn walk_enforces_structure_and_literals() {
        assert!(walk_one(json!({ "and": { "children": [] } })).is_err());
        assert!(
            walk_one(json!({ "and": { "children": [ { "or": { "children": [
            { "not": { "child": { "property": { "key": "vip", "op": "set" } } } } ] } } ] } }))
            .is_err()
        );
        assert!(
            walk_one(json!({ "attribute": { "key": "plan_type", "values": ["GOLD"] } })).is_err()
        );
        assert!(
            walk_one(json!({ "amount": { "op": "gt", "threshold": 1, "currency": "XXX" } }))
                .is_err()
        );
        assert!(
            walk_one(json!({ "property": { "key": "vip", "op": "eq", "value": "yes" } })).is_err()
        );
        assert!(walk_one(json!({ "property": { "key": "vip", "op": "eq" } })).is_err());
        assert_eq!(
            walk_one(json!({ "property": { "key": "seats", "op": "eq", "value": 10 } })),
            Ok(1)
        );
        assert!(
            walk_one(json!({ "property": { "key": "seats", "op": "gt", "value": true } })).is_err()
        );
        assert_eq!(
            walk_one(json!({ "or": { "children": [
                { "attribute": { "key": "country", "values": ["FR"] } },
                { "not": { "child": { "attribute": { "key": "currency", "values": ["EUR"] } } } }
            ] } })),
            Ok(2)
        );
    }
}
