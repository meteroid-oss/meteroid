use crate::StoreResult;
use crate::domain::Product;
use crate::domain::add_ons::AddOn;
use crate::domain::prices::Price;
use crate::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnNew, SubscriptionAddOnNewInternal,
};
use crate::errors::StoreError;
use crate::store::PgConn;
use common_domain::ids::{PriceId, ProductId, SubscriptionId};
use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use error_stack::Report;
use std::collections::HashMap;

/// Resolves checkout add-ons against their catalog definitions and persists them.
/// Used by both the synchronous checkout completion and the async payment settlement webhook.
pub(crate) async fn resolve_and_insert_checkout_addons(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    addons: &[AddOn],
    create_add_ons: &[CreateSubscriptionAddOn],
    products_by_id: &HashMap<ProductId, Product>,
    prices_by_id: &HashMap<PriceId, Price>,
    units: &crate::domain::credit_units::CreditUnitsById,
    effective_from: chrono::NaiveDate,
) -> StoreResult<()> {
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

    let addon_refs: Vec<_> = addon_rows_new.iter().collect();
    SubscriptionAddOnRow::insert_batch(conn, addon_refs)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    Ok(())
}
