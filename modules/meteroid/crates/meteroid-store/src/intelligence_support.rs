//! Store internals the intelligence module is built on: the billing internals its projection
//! derives revenue from, and what its alerts, signals and targets read and write. Internal to the
//! intelligence module; nothing else may depend on this module.

use std::collections::HashMap;

use common_domain::ids::{
    CreditUnitId, CustomerId, PriceId, ProductId, SubscriptionId, TenantId, UserId,
};
use error_stack::Report;
use scoped_futures::ScopedBoxFuture;

use crate::domain::credit_units::CreditUnitsById;
use crate::domain::filters::Filter;
use crate::domain::usage_attribution::SubscriptionUsageScopes;
use crate::domain::{LineItem, Price, Product, SubscriptionFee, SubscriptionFeeBillingPeriod};
use crate::errors::StoreError;
use crate::store::PgConn;
use crate::{Store, StoreResult};

pub use crate::repositories::credit::customer_credit::give_back_source;
pub use crate::services::credit::scope::{LineOwners, line_subscription};
pub use crate::services::intelligence_support::{
    DUNNING_RETRY_SCHEDULE_DAYS, LimitedUsage, MrrLine, PreparedPlanChange, SignalServices,
    SubscriptionCycle, USAGE_FETCH_CONCURRENCY, calculate_mrr, floor_subscription_mrr,
    load_period_true_up, monthly_share, stop_reason_of, upcoming_invoice_details,
};
pub use crate::services::invoice_lines::minimum::{
    UpFrontShortfall, is_up_front, load_up_front_shortfalls,
};

/// The lines a note crediting `credits` (line, subtotal) of `line_items` carries, with nothing
/// credited or returned before it.
#[doc(hidden)]
pub fn credited_lines(
    line_items: &[LineItem],
    credits: &[(String, i64)],
    precision: u8,
) -> Vec<LineItem> {
    crate::repositories::credit_notes::credited_lines(
        line_items,
        credits,
        &HashMap::new(),
        &HashMap::new(),
        precision,
    )
}

/// `store`'s transaction on a connection the caller already holds.
pub async fn transaction_with<'a, R, F>(
    store: &Store,
    conn: &mut PgConn,
    callback: F,
) -> StoreResult<R>
where
    F: for<'r> FnOnce(&'r mut PgConn) -> ScopedBoxFuture<'a, 'r, Result<R, Report<StoreError>>>
        + Send
        + 'a,
    R: Send + 'a,
{
    store.transaction_with(conn, callback).await
}

/// The customers the customers list's `filters` match; `None` when there is no filter.
pub async fn resolve_customer_filter_ids(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    filters: Vec<Filter>,
) -> StoreResult<Option<Vec<CustomerId>>> {
    crate::repositories::customers::resolve_customer_filter_ids(
        &store.internal,
        conn,
        tenant_id,
        filters,
    )
    .await
}

/// Inside the caller's write transaction: see `repositories::organizations::ensure_org_member`.
pub async fn ensure_org_member(
    conn: &mut PgConn,
    tenant_id: TenantId,
    user_id: UserId,
) -> StoreResult<()> {
    crate::repositories::organizations::ensure_org_member(conn, tenant_id, user_id).await
}

pub async fn usage_scopes_of(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<SubscriptionUsageScopes> {
    crate::repositories::subscriptions::usage_attribution::usage_scopes_of(
        conn,
        tenant_id,
        subscription_id,
    )
    .await
}

pub async fn load_credit_units(
    conn: &mut PgConn,
    tenant_id: TenantId,
    ids: impl IntoIterator<Item = CreditUnitId>,
) -> StoreResult<CreditUnitsById> {
    crate::repositories::credit_units::load_credit_units(conn, tenant_id, ids).await
}

pub async fn fetch_prices_and_products(
    conn: &mut PgConn,
    tenant_id: TenantId,
    price_ids: impl Iterator<Item = PriceId>,
    product_ids: impl Iterator<Item = ProductId>,
) -> StoreResult<(HashMap<PriceId, Price>, HashMap<ProductId, Product>)> {
    crate::repositories::subscriptions::fetch_prices_and_products(
        conn,
        tenant_id,
        price_ids,
        product_ids,
    )
    .await
}

pub async fn fetch_price_credit_units(
    conn: &mut PgConn,
    tenant_id: TenantId,
    prices_by_id: &HashMap<PriceId, Price>,
) -> StoreResult<CreditUnitsById> {
    crate::repositories::subscriptions::fetch_price_credit_units(conn, tenant_id, prices_by_id)
        .await
}

pub fn resolve_fee_from_maps(
    price_id: Option<PriceId>,
    product_id: Option<ProductId>,
    prices_by_id: &HashMap<PriceId, Price>,
    products_by_id: &HashMap<ProductId, Product>,
    units: &CreditUnitsById,
) -> Option<(SubscriptionFeeBillingPeriod, SubscriptionFee)> {
    crate::repositories::subscriptions::resolve_fee_from_maps(
        price_id,
        product_id,
        prices_by_id,
        products_by_id,
        units,
    )
}

pub async fn restore_stored_fee_credit_units(
    conn: &mut PgConn,
    tenant_id: TenantId,
    fees: Vec<(Option<PriceId>, &mut SubscriptionFee)>,
) -> StoreResult<()> {
    crate::repositories::subscriptions::restore_stored_fee_credit_units(conn, tenant_id, fees).await
}
