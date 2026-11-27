use crate::errors::IntoDbResult;
use crate::product_family_journeys::ProductFamilyJourneyRow;
use crate::{DbResult, PgConn};
use common_domain::ids::{CustomerId, ProductFamilyId, TenantId};
use diesel::sql_types::{Array, BigInt, Bool, Text, Uuid};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, QueryableByName, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl ProductFamilyJourneyRow {
    pub async fn find(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
    ) -> DbResult<Option<ProductFamilyJourneyRow>> {
        use crate::schema::product_family_journey::dsl as j;

        let query = j::product_family_journey
            .filter(j::tenant_id.eq(tenant_id))
            .filter(j::product_family_id.eq(product_family_id))
            .select(ProductFamilyJourneyRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding a product family's journey")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<ProductFamilyJourneyRow>> {
        use crate::schema::product_family_journey::dsl as j;

        let query = j::product_family_journey
            .filter(j::tenant_id.eq(tenant_id))
            .select(ProductFamilyJourneyRow::as_select());
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while listing journeys")
            .into_db_result()
    }

    pub async fn upsert(&self, conn: &mut PgConn) -> DbResult<ProductFamilyJourneyRow> {
        use crate::schema::product_family_journey::dsl as j;

        let query = diesel::insert_into(j::product_family_journey)
            .values(self)
            .on_conflict(j::product_family_id)
            .do_update()
            .set((
                j::signup_plan_id.eq(self.signup_plan_id),
                j::signup_option.eq(self.signup_option.clone()),
                j::period_switch.eq(self.period_switch),
                j::self_serve_downgrade.eq(self.self_serve_downgrade),
                j::updated_at.eq(common_clock::now()),
            ))
            .returning(ProductFamilyJourneyRow::as_returning());
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .attach("Error while saving a product family's journey")
            .into_db_result()
    }
}

/// `c`, a customer of `$1` not archived, has no subscription still going to a plan of family `$2`.
const PLANLESS: &str = "c.tenant_id = $1 AND c.archived_at IS NULL AND NOT EXISTS (
    SELECT 1 FROM subscription s
    JOIN plan_version pv ON pv.id = s.plan_version_id
    JOIN plan p ON p.id = pv.plan_id
    WHERE s.tenant_id = $1 AND s.customer_id = c.id AND p.product_family_id = $2
      AND s.status NOT IN ('CANCELLED', 'ABORTED', 'COMPLETED'))";

/// `c` had a subscription to a plan of family `$2` that ended.
const CHURNED: &str = "EXISTS (
    SELECT 1 FROM subscription s
    JOIN plan_version pv ON pv.id = s.plan_version_id
    JOIN plan p ON p.id = pv.plan_id
    WHERE s.tenant_id = $1 AND s.customer_id = c.id AND p.product_family_id = $2
      AND s.status IN ('CANCELLED', 'COMPLETED'))";

/// How many customers have no plan in a product family, per currency and churn.
#[derive(QueryableByName, Debug, Clone, PartialEq, Eq)]
pub struct PlanlessCount {
    #[diesel(sql_type = Text)]
    pub currency: String,
    #[diesel(sql_type = Bool)]
    pub churned: bool,
    #[diesel(sql_type = BigInt)]
    pub customers: i64,
}

#[derive(QueryableByName)]
struct IdRow {
    #[diesel(sql_type = Uuid)]
    id: CustomerId,
}

#[derive(QueryableByName)]
struct BoolRow {
    #[diesel(sql_type = Bool)]
    value: bool,
}

pub async fn count_planless(
    conn: &mut PgConn,
    tenant_id: TenantId,
    product_family_id: ProductFamilyId,
) -> DbResult<Vec<PlanlessCount>> {
    diesel::sql_query(format!(
        "SELECT upper(c.currency) AS currency, {CHURNED} AS churned, count(*) AS customers
         FROM customer c WHERE {PLANLESS} GROUP BY 1, 2"
    ))
    .bind::<Uuid, _>(tenant_id)
    .bind::<Uuid, _>(product_family_id)
    .get_results(conn)
    .await
    .attach("Error while counting the customers without a plan in a product family")
    .into_db_result()
}

/// The customers not archived whose start in family `$2` is unfinished: a subscription pending
/// its activation or its trial's checkout, none active.
pub async fn count_unfinished(
    conn: &mut PgConn,
    tenant_id: TenantId,
    product_family_id: ProductFamilyId,
) -> DbResult<i64> {
    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        customers: i64,
    }
    let in_family = |statuses: &str| {
        format!(
            "EXISTS (SELECT 1 FROM subscription s
                JOIN plan_version pv ON pv.id = s.plan_version_id
                JOIN plan p ON p.id = pv.plan_id
                WHERE s.tenant_id = $1 AND s.customer_id = c.id AND p.product_family_id = $2
                  AND s.status IN ({statuses}))"
        )
    };
    diesel::sql_query(format!(
        "SELECT count(*) AS customers FROM customer c
         WHERE c.tenant_id = $1 AND c.archived_at IS NULL AND {} AND NOT {}",
        in_family("'PENDING_ACTIVATION', 'TRIAL_EXPIRED'"),
        in_family("'ACTIVE', 'TRIAL_ACTIVE'"),
    ))
    .bind::<Uuid, _>(tenant_id)
    .bind::<Uuid, _>(product_family_id)
    .get_result::<CountRow>(conn)
    .await
    .map(|row| row.customers)
    .attach("Error while counting the customers with an unfinished start in a product family")
    .into_db_result()
}

/// Those paying in one of `currencies` (upper-case), churned ones only with `include_churned`.
pub async fn planless_customer_ids(
    conn: &mut PgConn,
    tenant_id: TenantId,
    product_family_id: ProductFamilyId,
    currencies: &[String],
    include_churned: bool,
) -> DbResult<Vec<CustomerId>> {
    diesel::sql_query(format!(
        "SELECT c.id FROM customer c WHERE {PLANLESS}
           AND upper(c.currency) = ANY($3) AND ($4 OR NOT {CHURNED})
         ORDER BY c.id"
    ))
    .bind::<Uuid, _>(tenant_id)
    .bind::<Uuid, _>(product_family_id)
    .bind::<Array<Text>, _>(currencies)
    .bind::<Bool, _>(include_churned)
    .get_results::<IdRow>(conn)
    .await
    .map(|rows| rows.into_iter().map(|row| row.id).collect())
    .attach("Error while listing the customers without a plan in a product family")
    .into_db_result()
}

pub async fn is_planless(
    conn: &mut PgConn,
    tenant_id: TenantId,
    product_family_id: ProductFamilyId,
    customer_id: CustomerId,
) -> DbResult<bool> {
    diesel::sql_query(format!(
        "SELECT EXISTS (SELECT 1 FROM customer c WHERE {PLANLESS} AND c.id = $3) AS value"
    ))
    .bind::<Uuid, _>(tenant_id)
    .bind::<Uuid, _>(product_family_id)
    .bind::<Uuid, _>(customer_id)
    .get_result::<BoolRow>(conn)
    .await
    .map(|row| row.value)
    .attach("Error while checking a customer has no plan in a product family")
    .into_db_result()
}
