use crate::StoreResult;
use crate::domain::Product;
use crate::domain::add_ons::AddOn;
use crate::domain::prices::Price;
use crate::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnNew, SubscriptionAddOnNewInternal,
};
use crate::errors::StoreError;
use crate::store::PgConn;
use common_domain::ids::{PriceId, ProductId, SubscriptionId, TenantId};
use diesel_models::subscription_add_ons::SubscriptionAddOnRowNew;
use error_stack::Report;
use std::collections::HashMap;

/// The rows checkout add-ons are stored as, resolved against their catalog definitions.
pub(crate) fn resolve_checkout_addons(
    subscription_id: SubscriptionId,
    addons: &[AddOn],
    create_add_ons: &[CreateSubscriptionAddOn],
    products_by_id: &HashMap<ProductId, Product>,
    prices_by_id: &HashMap<PriceId, Price>,
    units: &crate::domain::credit_units::CreditUnitsById,
    effective_from: chrono::NaiveDate,
) -> StoreResult<Vec<SubscriptionAddOnRowNew>> {
    let mut addon_rows_new: Vec<SubscriptionAddOnRowNew> = Vec::new();
    for cs_ao in create_add_ons {
        let addon = addons
            .iter()
            .find(|a| a.id == cs_ao.add_on_id)
            .ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(format!(
                    "Add-on {} not found",
                    cs_ao.add_on_id
                )))
            })?;

        let resolved = addon
            .resolve_customized(products_by_id, prices_by_id, units, &cs_ao.customization)
            .map_err(Report::new)?;

        let new_internal = SubscriptionAddOnNewInternal {
            add_on_id: addon.id,
            name: resolved.name,
            period: resolved.period,
            fee: resolved.fee,
            product_id: resolved.product_id,
            price_id: resolved.price_id,
            quantity: cs_ao.quantity,
            effective_from,
        };

        let new = SubscriptionAddOnNew {
            subscription_id,
            internal: new_internal,
        };
        let row_new: SubscriptionAddOnRowNew = new.try_into()?;
        addon_rows_new.push(row_new);
    }
    Ok(addon_rows_new)
}

/// The rows buying `create_add_ons` on `effective_from` stores, their catalog read on `conn`:
/// what a checkout completion inserts, never written.
pub(crate) async fn checkout_add_on_rows(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    create_add_ons: &[CreateSubscriptionAddOn],
    effective_from: chrono::NaiveDate,
) -> StoreResult<Vec<SubscriptionAddOnRowNew>> {
    let addon_ids: Vec<_> = create_add_ons.iter().map(|a| a.add_on_id).collect();
    let rows = diesel_models::add_ons::AddOnRow::list_by_ids(conn, &addon_ids, &tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let addons = crate::repositories::add_ons::enrich_add_ons(conn, rows, tenant_id).await?;
    let product_ids: Vec<_> = addons.iter().map(|a| a.product_id).collect();
    let price_ids: Vec<_> = addons.iter().map(|a| a.price_id).collect();
    let (prices_by_id, products_by_id) =
        crate::repositories::subscriptions::fetch_prices_and_products(
            conn,
            tenant_id,
            price_ids.into_iter(),
            product_ids.into_iter(),
        )
        .await?;
    let units = crate::repositories::credit_units::load_add_on_credit_units(
        conn,
        tenant_id,
        &prices_by_id,
        create_add_ons,
    )
    .await?;
    resolve_checkout_addons(
        subscription_id,
        &addons,
        create_add_ons,
        &products_by_id,
        &prices_by_id,
        &units,
        effective_from,
    )
}
