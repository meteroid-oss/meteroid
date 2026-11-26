//! Limits counted by the app (ADR 0011 §3.8): the count is Postgres state, written under its row
//! lock, with an op log that answers retries and late sets.
use crate::StoreResult;
use crate::domain::resources::{CUSTOMER_PATH, ResourceInput};
use crate::errors::{EntitlementRefusal, StoreError};
use crate::repositories::resources::load_for_check;
use crate::store::{PgConn, Store};
use chrono::{DateTime, Duration, Utc};
use common_domain::ids::{AliasOr, CustomerId, FeatureId, TenantId};
use diesel_async::RunQueryDsl;
use diesel_models::customers::CustomerRow;
use diesel_models::entitlement_app_counts::{EntitlementAppCountOpRow, EntitlementAppCountRow};
use diesel_models::entitlements::FeatureRow;
use diesel_models::enums::{AppCountOpEnum, FeatureKindEnum, FeatureLimitCountedByEnum};
use diesel_models::errors::DatabaseErrorContainer;
use diesel_models::tenants::TenantRow;
use error_stack::{Report, bail};
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppCountOp {
    Set(Decimal),
    Add(Decimal),
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppCountWrite {
    /// At most one key, of any resource type (`{"site_id": "s1"}`); none for the customer.
    pub resources: BTreeMap<String, String>,
    pub op: AppCountOp,
    /// When the count was true; the receive time when absent.
    pub at: Option<DateTime<Utc>>,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppCountResult {
    pub path: String,
    pub count: Decimal,
    /// The op moved the count: a late one is recorded but changes nothing.
    pub applied: bool,
    /// A retry: its stored result, nothing applied.
    pub replayed: bool,
}

/// How far ahead of the server's clock a reported `at` may be.
const MAX_SKEW_MINUTES: i64 = 5;

impl Store {
    /// Applies a set or an add to the count of the path the resources name, then lets the usage
    /// controls detect thresholds. A retry returns its stored result and applies nothing.
    pub async fn report_app_count(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature_id: FeatureId,
        write: AppCountWrite,
    ) -> StoreResult<AppCountResult> {
        if write.idempotency_key.trim().is_empty() {
            bail!(StoreError::InvalidArgument(
                "idempotency_key is required".to_owned()
            ));
        }
        if matches!(write.op, AppCountOp::Set(n) if n < Decimal::ZERO) {
            bail!(StoreError::InvalidArgument(
                "a set count cannot be negative".to_owned()
            ));
        }
        let now = common_clock::now();
        let result = self
            .transaction(|conn| {
                async move {
                    apply_app_count(conn, tenant_id, customer_id, feature_id, write, now).await
                }
                .scope_boxed()
            })
            .await?;
        // A replay runs it too, so a retry repairs a detection that failed after the commit.
        let (hooks, paths) = (self.hooks.credit.clone(), [result.path.clone()]);
        self.transaction(|conn| {
            async move {
                hooks
                    .app_count_changed(conn, tenant_id, customer_id, feature_id, &paths, now)
                    .await
            }
            .scope_boxed()
        })
        .await?;
        Ok(result)
    }

    /// The customer's total (the sum of all its paths) and, with `path`, that path's own. Raw:
    /// a negative total is the reader's to show as 0 and flag.
    pub async fn app_count_totals(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature_id: FeatureId,
        path: Option<&str>,
    ) -> StoreResult<(Decimal, Option<Decimal>)> {
        let customer =
            EntitlementAppCountRow::sum_for_customer(conn, tenant_id, customer_id, feature_id)
                .await?;
        let path = match path {
            Some(path) => Some(
                EntitlementAppCountRow::find(conn, tenant_id, customer_id, feature_id, path)
                    .await?
                    .map_or(Decimal::ZERO, |row| row.count),
            ),
            None => None,
        };
        Ok((customer, path))
    }

    /// Drops the ops a day past their tenant's retention, which no retry may name any more, and
    /// moves the tenant's `entitlement_op_log_complete_from` up to the horizon purged: a set older
    /// than it would miss adds, even once the retention is raised. The day spares the adds of a
    /// set accepted just inside the retention while this runs. For a periodic job.
    pub async fn purge_app_count_ops(&self, now: DateTime<Utc>) -> StoreResult<usize> {
        #[derive(diesel::QueryableByName)]
        struct Purged {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            purged: i64,
        }
        let mut conn = self.get_conn().await?;
        let purged = diesel::sql_query(
            "WITH horizon AS (
                 SELECT id, $1 - make_interval(days => entitlement_op_retention_days + 1) AS until
                 FROM tenant
             ), purged AS (
                 DELETE FROM entitlement_app_count_op o USING horizon h
                 WHERE h.id = o.tenant_id AND o.created_at < h.until AND o.at < h.until
                 RETURNING o.tenant_id
             ), advanced AS (
                 UPDATE tenant t
                 SET entitlement_op_log_complete_from =
                     GREATEST(t.entitlement_op_log_complete_from, h.until)
                 FROM horizon h
                 WHERE h.id = t.id AND t.id IN (SELECT tenant_id FROM purged)
             )
             SELECT count(*) AS purged FROM purged",
        )
        .bind::<diesel::sql_types::Timestamptz, _>(now)
        .get_result::<Purged>(&mut conn)
        .await
        .map_err(|e| Into::<Report<StoreError>>::into(DatabaseErrorContainer::from(e)))?
        .purged;
        Ok(usize::try_from(purged).unwrap_or_default())
    }
}

async fn apply_app_count(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    feature_id: FeatureId,
    write: AppCountWrite,
    now: DateTime<Utc>,
) -> StoreResult<AppCountResult> {
    CustomerRow::find_by_id_or_alias(conn, tenant_id, AliasOr::Id(customer_id)).await?;
    let feature = FeatureRow::find_by_id(conn, feature_id, tenant_id)
        .await?
        .feature;
    if feature.kind != FeatureKindEnum::Limit
        || feature.limit_counted_by != Some(FeatureLimitCountedByEnum::App)
    {
        bail!(StoreError::InvalidArgument(format!(
            "{} is not a limit counted by your app",
            feature.code
        )));
    }
    let input = ResourceInput {
        resources: write.resources,
        attributes: BTreeMap::new(),
    };
    let resource = load_for_check(conn, tenant_id, customer_id, &input).await?;
    match (feature.level_resource_type_id, &resource) {
        (Some(level), Some(r)) if r.type_id == level => {}
        (None, None) => {}
        (Some(_), _) => {
            return Err(EntitlementRefusal::ResourceRequired.report(format!(
                "{} is counted per resource of its type: name one",
                feature.code
            )));
        }
        (None, Some(_)) => bail!(StoreError::InvalidArgument(format!(
            "{} is counted for the customer: name no resource",
            feature.code
        ))),
    }
    let path = resource.as_ref().map_or_else(
        || CUSTOMER_PATH.to_owned(),
        |resource| resource.path.clone(),
    );
    let resource_id = resource.and_then(|resource| resource.reference.stored_id());
    let at = write.at.unwrap_or(now);
    if at > now + Duration::minutes(MAX_SKEW_MINUTES) {
        bail!(StoreError::InvalidArgument(format!(
            "`at` is more than {MAX_SKEW_MINUTES} minutes ahead of the server's clock"
        )));
    }
    let (op, amount) = match write.op {
        AppCountOp::Set(n) => (AppCountOpEnum::Set, n),
        AppCountOp::Add(n) => (AppCountOpEnum::Add, n),
    };
    // A set ahead of the server's clock counts from now, so an add received meanwhile isn't lost.
    let at = if op == AppCountOpEnum::Set {
        at.min(now)
    } else {
        at
    };
    let request_hash = request_hash(&path, op, amount, write.at);
    let key = write.idempotency_key;

    let mut row = EntitlementAppCountRow::lock_or_create(
        conn,
        tenant_id,
        customer_id,
        feature_id,
        &path,
        resource_id,
    )
    .await?;
    if let Some(done) =
        EntitlementAppCountOpRow::find_by_key(conn, tenant_id, customer_id, feature_id, &key)
            .await?
    {
        return replay(done, &request_hash);
    }
    if op == AppCountOpEnum::Set {
        let tenant = TenantRow::find_by_id(conn, tenant_id).await?;
        let retained_from = now - Duration::days(i64::from(tenant.entitlement_op_retention_days));
        let complete_from = tenant
            .entitlement_op_log_complete_from
            .map_or(retained_from, |purged| purged.max(retained_from));
        if at < complete_from {
            return Err(
                EntitlementRefusal::StaleSet.report("the set is older than the op log's retention")
            );
        }
    }

    // A set counts what happened strictly before its `at`: an op older than the last set is
    // recorded but changes nothing, and an add at the set's very instant still counts.
    let applied = row.last_set_at.is_none_or(|set| at >= set);
    let count = match op {
        _ if !applied => row.count,
        AppCountOpEnum::Add => row
            .count
            .checked_add(amount)
            .ok_or(StoreError::InvalidDecimal)?,
        AppCountOpEnum::Set => {
            let adds = EntitlementAppCountOpRow::adds_since(
                conn,
                tenant_id,
                customer_id,
                feature_id,
                &path,
                at,
            )
            .await?;
            row.last_set_value = Some(amount);
            row.last_set_at = Some(at);
            row.last_set_key = Some(key.clone());
            amount
                .checked_add(adds.sum.unwrap_or_default())
                .ok_or(StoreError::InvalidDecimal)?
        }
    };
    let recorded = EntitlementAppCountOpRow {
        tenant_id,
        customer_id,
        feature_id,
        idempotency_key: key.clone(),
        path: path.clone(),
        op,
        amount,
        at,
        result_count: count,
        request_hash: request_hash.clone(),
        created_at: now,
    }
    .insert(conn)
    .await?;
    if !recorded {
        // The same key, written at the same time on another path.
        let done =
            EntitlementAppCountOpRow::find_by_key(conn, tenant_id, customer_id, feature_id, &key)
                .await?
                .ok_or_else(|| Report::new(StoreError::ValueNotFound("app count op".into())))?;
        return replay(done, &request_hash);
    }
    row.count = count;
    row.updated_at = now;
    row.save(conn).await?;
    Ok(AppCountResult {
        path,
        count,
        applied,
        replayed: false,
    })
}

fn replay(done: EntitlementAppCountOpRow, request_hash: &str) -> StoreResult<AppCountResult> {
    if done.request_hash != request_hash {
        return Err(EntitlementRefusal::IdempotencyKeyReused
            .report("the idempotency_key was used for another request"));
    }
    Ok(AppCountResult {
        path: done.path,
        count: done.result_count,
        applied: false,
        replayed: true,
    })
}

/// The request as sent: `at` only when given, so a retry relying on the receive time matches.
fn request_hash(
    path: &str,
    op: AppCountOpEnum,
    amount: Decimal,
    at: Option<DateTime<Utc>>,
) -> String {
    let request = serde_json::json!([
        path,
        format!("{op:?}"),
        amount.normalize().to_string(),
        at.map(|at| at.timestamp_micros()),
    ]);
    hex::encode(Sha256::digest(request.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn the_hash_covers_the_request_as_sent() {
        use AppCountOpEnum::{Add, Set};
        let at = Some(common_clock::now());
        let set = request_hash("site=s1", Set, dec!(10), at);
        assert_eq!(set, request_hash("site=s1", Set, dec!(10.00), at));
        assert_ne!(set, request_hash("site=s10", Set, dec!(10), at));
        assert_ne!(set, request_hash("site=s1", Add, dec!(10), at));
        assert_ne!(set, request_hash("site=s1", Set, dec!(11), at));
        assert_ne!(set, request_hash("site=s1", Set, dec!(10), None));
    }
}
