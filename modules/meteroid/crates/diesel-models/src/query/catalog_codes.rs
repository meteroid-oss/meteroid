//! The codes of the catalog (plans, products, add-ons, product families) and the external ids of
//! subscriptions: unique per tenant, the handles API v2 paths take beside ids (ADR-0013). A row
//! inserted without a code gets its name's from a trigger; an explicit one is set in the
//! inserting transaction with `set_*_code`. A metric's (its `catalog_code`) is inserted with it.
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::{
    AddOnId, BillableMetricId, CouponId, CreditNoteId, CreditUnitId, CustomerId, FeatureId,
    InvoiceId, PlanId, ProductFamilyId, ProductId, SubscriptionId, TenantId,
};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

macro_rules! coded_table {
    ($table:ident, $id:ty, $find:ident, $set:ident, $what:literal) => {
        /// The id of the tenant's row of `code`, archived ones included.
        pub async fn $find(
            conn: &mut PgConn,
            tenant_id: TenantId,
            code: &str,
        ) -> DbResult<Option<$id>> {
            use crate::schema::$table::dsl as t;
            let query = t::$table
                .filter(t::tenant_id.eq(tenant_id))
                .filter(t::code.eq(code))
                .select(t::id);
            crate::debug_sql(&query);
            query
                .first::<$id>(conn)
                .await
                .optional()
                .attach(concat!("Error while finding a ", $what, " by code"))
                .into_db_result()
        }

        /// Sets the code of a row just inserted, in its transaction: a code the tenant already
        /// has is a unique violation.
        pub async fn $set(
            conn: &mut PgConn,
            tenant_id: TenantId,
            id: $id,
            code: &str,
        ) -> DbResult<usize> {
            use crate::schema::$table::dsl as t;
            let query = diesel::update(t::$table)
                .filter(t::tenant_id.eq(tenant_id))
                .filter(t::id.eq(id))
                .set(t::code.eq(code));
            crate::debug_sql(&query);
            query
                .execute(conn)
                .await
                .attach(concat!("Error while setting the code of a ", $what))
                .into_db_result()
        }
    };
}

coded_table!(plan, PlanId, plan_id_by_code, set_plan_code, "plan");
coded_table!(
    product,
    ProductId,
    product_id_by_code,
    set_product_code,
    "product"
);
coded_table!(
    add_on,
    AddOnId,
    add_on_id_by_code,
    set_add_on_code,
    "add-on"
);
coded_table!(
    product_family,
    ProductFamilyId,
    product_family_id_by_code,
    set_product_family_code,
    "product family"
);

/// The id of the tenant's subscription of `external_id`.
pub async fn subscription_id_by_external_id(
    conn: &mut PgConn,
    tenant_id: TenantId,
    external_id: &str,
) -> DbResult<Option<SubscriptionId>> {
    use crate::schema::subscription::dsl as s;
    let query = s::subscription
        .filter(s::tenant_id.eq(tenant_id))
        .filter(s::external_id.eq(external_id))
        .select(s::id);
    crate::debug_sql(&query);
    query
        .first::<SubscriptionId>(conn)
        .await
        .optional()
        .attach("Error while finding a subscription by external id")
        .into_db_result()
}

/// Sets or clears a subscription's external id: one another subscription of the tenant has is a
/// unique violation.
pub async fn set_subscription_external_id(
    conn: &mut PgConn,
    tenant_id: TenantId,
    id: SubscriptionId,
    external_id: Option<&str>,
) -> DbResult<usize> {
    use crate::schema::subscription::dsl as s;
    let query = diesel::update(s::subscription)
        .filter(s::tenant_id.eq(tenant_id))
        .filter(s::id.eq(id))
        .set(s::external_id.eq(external_id));
    crate::debug_sql(&query);
    query
        .execute(conn)
        .await
        .attach("Error while setting a subscription's external id")
        .into_db_result()
}

/// The codes of the tenant's plans of `ids`.
pub async fn plan_codes(
    conn: &mut PgConn,
    tenant_id: TenantId,
    ids: &[PlanId],
) -> DbResult<Vec<(PlanId, String)>> {
    use crate::schema::plan::dsl as p;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let query = p::plan
        .filter(p::tenant_id.eq(tenant_id))
        .filter(p::id.eq_any(ids))
        .select((p::id, p::code));
    crate::debug_sql(&query);
    query
        .load::<(PlanId, String)>(conn)
        .await
        .attach("Error while reading plan codes")
        .into_db_result()
}

macro_rules! keys_of {
    ($find:ident, $table:ident, $id:ty, $key:ident, $value:ty, $what:literal) => {
        /// The human keys of the tenant's rows of `ids`, as API v2 references name them; a row
        /// not the tenant's is absent.
        pub async fn $find(
            conn: &mut PgConn,
            tenant_id: TenantId,
            ids: &[$id],
        ) -> DbResult<Vec<($id, $value)>> {
            use crate::schema::$table::dsl as t;
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let query = t::$table
                .filter(t::tenant_id.eq(tenant_id))
                .filter(t::id.eq_any(ids))
                .select((t::id, t::$key));
            crate::debug_sql(&query);
            query
                .load::<($id, $value)>(conn)
                .await
                .attach(concat!("Error while reading the keys of ", $what))
                .into_db_result()
        }
    };
}

keys_of!(product_codes, product, ProductId, code, String, "products");
keys_of!(add_on_codes, add_on, AddOnId, code, String, "add-ons");
keys_of!(
    product_family_codes,
    product_family,
    ProductFamilyId,
    code,
    String,
    "product families"
);
keys_of!(
    metric_codes,
    billable_metric,
    BillableMetricId,
    catalog_code,
    String,
    "metrics"
);
keys_of!(
    credit_unit_codes,
    credit_unit,
    CreditUnitId,
    code,
    String,
    "credit units"
);
keys_of!(coupon_codes, coupon, CouponId, code, String, "coupons");
keys_of!(feature_codes, feature, FeatureId, code, String, "features");
keys_of!(
    subscription_external_ids,
    subscription,
    SubscriptionId,
    external_id,
    Option<String>,
    "subscriptions"
);
keys_of!(
    invoice_numbers,
    invoice,
    InvoiceId,
    invoice_number,
    String,
    "invoices"
);
keys_of!(
    credit_note_numbers,
    credit_note,
    CreditNoteId,
    credit_note_number,
    String,
    "credit notes"
);

/// The alias and the name of the tenant's customers of `ids`.
pub async fn customer_keys(
    conn: &mut PgConn,
    tenant_id: TenantId,
    ids: &[CustomerId],
) -> DbResult<Vec<(CustomerId, Option<String>, String)>> {
    use crate::schema::customer::dsl as c;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let query = c::customer
        .filter(c::tenant_id.eq(tenant_id))
        .filter(c::id.eq_any(ids))
        .select((c::id, c::alias, c::name));
    crate::debug_sql(&query);
    query
        .load::<(CustomerId, Option<String>, String)>(conn)
        .await
        .attach("Error while reading the keys of customers")
        .into_db_result()
}

/// The ids of the tenant's credit units of `codes`, archived ones included.
pub async fn credit_unit_ids(
    conn: &mut PgConn,
    tenant_id: TenantId,
    codes: &[String],
) -> DbResult<Vec<(String, CreditUnitId)>> {
    use crate::schema::credit_unit::dsl as u;
    if codes.is_empty() {
        return Ok(Vec::new());
    }
    let query = u::credit_unit
        .filter(u::tenant_id.eq(tenant_id))
        .filter(u::code.eq_any(codes))
        .select((u::code, u::id));
    crate::debug_sql(&query);
    query
        .load::<(String, CreditUnitId)>(conn)
        .await
        .attach("Error while reading the ids of credit units")
        .into_db_result()
}
