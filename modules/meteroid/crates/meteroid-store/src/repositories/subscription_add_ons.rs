use crate::domain::Product;
use crate::domain::add_ons::AddOn;
use crate::domain::prices::Price;
use crate::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnNew, SubscriptionAddOnNewInternal,
};
use crate::errors::StoreError;
use crate::store::PgConn;
use crate::{Store, StoreResult};
use common_domain::ids::{PriceId, ProductId, SubscriptionAddOnId, SubscriptionId, TenantId};
use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use error_stack::Report;
use std::collections::HashMap;

#[async_trait::async_trait]
pub trait SubscriptionAddOnInterface {
    async fn insert_subscription_add_on(
        &self,
        tenant_id: TenantId,
        new: SubscriptionAddOnNew,
    ) -> StoreResult<()>;

    async fn delete_subscription_add_on(
        &self,
        id: SubscriptionAddOnId,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl SubscriptionAddOnInterface for Store {
    async fn insert_subscription_add_on(
        &self,
        tenant_id: TenantId,
        new: SubscriptionAddOnNew,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        // Verify the subscription belongs to the tenant
        diesel_models::subscriptions::SubscriptionRow::get_subscription_by_id(
            &mut conn,
            &tenant_id,
            new.subscription_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let row_new: SubscriptionAddOnRowNew = new.try_into().map_err(|e: Report<StoreError>| e)?;

        SubscriptionAddOnRow::insert_batch(&mut conn, vec![&row_new])
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(())
    }

    async fn delete_subscription_add_on(
        &self,
        id: SubscriptionAddOnId,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        SubscriptionAddOnRow::delete_by_id(&mut conn, id, &subscription_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }
}

/// Resolves checkout add-ons against their catalog definitions and persists them.
/// Used by both the synchronous checkout completion and the async payment settlement webhook.
pub(crate) async fn resolve_and_insert_checkout_addons(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    addons: &[AddOn],
    create_add_ons: &[CreateSubscriptionAddOn],
    products_by_id: &HashMap<ProductId, Product>,
    prices_by_id: &HashMap<PriceId, Price>,
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
            .resolve_customized(products_by_id, prices_by_id, &cs_ao.customization)
            .map_err(Report::new)?;

        let new_internal = SubscriptionAddOnNewInternal {
            add_on_id: addon.id,
            name: resolved.name,
            period: resolved.period,
            fee: resolved.fee,
            product_id: resolved.product_id,
            price_id: resolved.price_id,
            quantity: cs_ao.quantity,
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
