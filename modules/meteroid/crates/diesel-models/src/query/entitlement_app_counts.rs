use crate::entitlement_app_counts::{
    AppCountAddsRow, EntitlementAppCountOpRow, EntitlementAppCountRow,
};
use crate::enums::AppCountOpEnum;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, FeatureId, ResourceId, TenantId};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use rust_decimal::Decimal;

impl EntitlementAppCountRow {
    /// Creates the path's row at 0 unless it exists, then locks it until the transaction ends, so
    /// writes on one path serialise.
    pub async fn lock_or_create(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature_id: FeatureId,
        path: &str,
        resource_id: Option<ResourceId>,
    ) -> DbResult<EntitlementAppCountRow> {
        use crate::schema::entitlement_app_count::dsl;

        diesel::insert_into(dsl::entitlement_app_count)
            .values((
                dsl::tenant_id.eq(tenant_id),
                dsl::customer_id.eq(customer_id),
                dsl::feature_id.eq(feature_id),
                dsl::path.eq(path),
                dsl::resource_id.eq(resource_id),
            ))
            .on_conflict_do_nothing()
            .execute(conn)
            .await
            .attach("Error while creating an app count")
            .into_db_result()?;
        dsl::entitlement_app_count
            .find((tenant_id, customer_id, feature_id, path))
            .select(Self::as_select())
            .for_update()
            .first(conn)
            .await
            .attach("Error while locking an app count")
            .into_db_result()
    }

    /// Writes the count and, when set, the last set; `updated_at` is the caller's.
    pub async fn save(&self, conn: &mut PgConn) -> DbResult<()> {
        diesel::update(self)
            .set(self)
            .execute(conn)
            .await
            .attach("Error while saving an app count")
            .into_db_result()?;
        Ok(())
    }

    pub async fn find(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature_id: FeatureId,
        path: &str,
    ) -> DbResult<Option<EntitlementAppCountRow>> {
        use crate::schema::entitlement_app_count::dsl;

        dsl::entitlement_app_count
            .find((tenant_id, customer_id, feature_id, path))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading an app count")
            .into_db_result()
    }

    /// Every path of the customer's feature, the customer's own (`''`) included.
    pub async fn list_for_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature_id: FeatureId,
    ) -> DbResult<Vec<EntitlementAppCountRow>> {
        use crate::schema::entitlement_app_count::dsl;

        dsl::entitlement_app_count
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::customer_id.eq(customer_id))
            .filter(dsl::feature_id.eq(feature_id))
            .order(dsl::path.asc())
            .select(Self::as_select())
            .get_results(conn)
            .await
            .attach("Error while listing app counts")
            .into_db_result()
    }

    /// The customer-wide total: the sum over all its paths.
    pub async fn sum_for_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature_id: FeatureId,
    ) -> DbResult<Decimal> {
        use crate::schema::entitlement_app_count::dsl;

        dsl::entitlement_app_count
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::customer_id.eq(customer_id))
            .filter(dsl::feature_id.eq(feature_id))
            .select(diesel::dsl::sum(dsl::count))
            .get_result::<Option<Decimal>>(conn)
            .await
            .attach("Error while summing app counts")
            .into_db_result()
            .map(Option::unwrap_or_default)
    }

    pub async fn exists_for_feature(
        conn: &mut PgConn,
        tenant_id: TenantId,
        feature_id: FeatureId,
    ) -> DbResult<bool> {
        use crate::schema::entitlement_app_count::dsl;

        diesel::select(diesel::dsl::exists(
            dsl::entitlement_app_count
                .filter(dsl::tenant_id.eq(tenant_id))
                .filter(dsl::feature_id.eq(feature_id)),
        ))
        .get_result(conn)
        .await
        .attach("Error while looking for app counts")
        .into_db_result()
    }
}

impl EntitlementAppCountOpRow {
    /// Records the op; `false` when its idempotency key was already used.
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<bool> {
        use crate::schema::entitlement_app_count_op::dsl;

        diesel::insert_into(dsl::entitlement_app_count_op)
            .values(self)
            .on_conflict_do_nothing()
            .execute(conn)
            .await
            .attach("Error while recording an app count op")
            .into_db_result()
            .map(|inserted| inserted == 1)
    }

    pub async fn find_by_key(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature_id: FeatureId,
        idempotency_key: &str,
    ) -> DbResult<Option<EntitlementAppCountOpRow>> {
        use crate::schema::entitlement_app_count_op::dsl;

        dsl::entitlement_app_count_op
            .find((tenant_id, customer_id, feature_id, idempotency_key))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while finding an app count op")
            .into_db_result()
    }

    /// The adds of `path` at or after `at`: a set counts what happened strictly before its own.
    pub async fn adds_since(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature_id: FeatureId,
        path: &str,
        at: DateTime<Utc>,
    ) -> DbResult<AppCountAddsRow> {
        use crate::schema::entitlement_app_count_op::dsl;
        use diesel::dsl::{count_star, max, sum};

        dsl::entitlement_app_count_op
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::customer_id.eq(customer_id))
            .filter(dsl::feature_id.eq(feature_id))
            .filter(dsl::path.eq(path))
            .filter(dsl::op.eq(AppCountOpEnum::Add))
            .filter(dsl::at.ge(at))
            .select((count_star(), sum(dsl::amount), max(dsl::at)))
            .get_result(conn)
            .await
            .attach("Error while reading the adds of an app count")
            .into_db_result()
    }
}
