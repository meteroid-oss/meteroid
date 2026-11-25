use common_domain::ids::{BaseId, PriceId, ProductId, TenantId};
use diesel_models::plans::PlanRow;
use diesel_models::prices::{PriceRow, PriceSuccessorRowNew};
use diesel_models::products::ProductRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use uuid::Uuid;

use crate::StoreResult;
use crate::domain::price_components::{
    DimensionRate, DimensionRates, MatrixDimension, UsagePricingModel,
};
use crate::domain::prices::{
    AffectedPlan, MatrixDimensionKey, MatrixPriceUpdate, MatrixUpdatePreview, Price, Pricing,
};
use crate::errors::StoreError;

use super::Services;

/// The key of `rates` holding the values of `key`, in the order of its dimensions; none when
/// `key` names other dimensions.
fn key_in(rates: &DimensionRates, key: &MatrixDimensionKey) -> Option<Vec<String>> {
    let cells: Vec<&MatrixDimension> = std::iter::once(&key.dimension1)
        .chain(key.dimension2.as_ref())
        .collect();
    if cells.len() != rates.dimensions.len() {
        return None;
    }
    rates
        .dimensions
        .iter()
        .map(|dimension| {
            cells
                .iter()
                .find(|cell| &cell.key == dimension)
                .map(|cell| cell.value.clone())
        })
        .collect()
}

fn has_key(rates: &DimensionRates, key: &[String]) -> bool {
    rates.rates.iter().any(|rate| rate.key == key)
}

/// Refuses a product none of whose prices rates by dimension: a product has no structure, its
/// charges do, and only the prices rated by dimension are edited.
fn validate_matrix_prices(prices: &[PriceRow]) -> StoreResult<()> {
    let rated = prices.iter().any(|row| {
        matches!(
            serde_json::from_value::<Pricing>(row.pricing.clone()),
            Ok(Pricing::Usage(UsagePricingModel::DimensionRates(_)))
        )
    });
    if rated {
        Ok(())
    } else {
        Err(Report::new(StoreError::InvalidArgument(
            "Product is not rated by dimension".to_string(),
        )))
    }
}

impl Services {
    pub(crate) async fn update_matrix_prices(
        &self,
        tenant_id: TenantId,
        product_id: ProductId,
        update: MatrixPriceUpdate,
        _actor: Uuid,
    ) -> StoreResult<Vec<Price>> {
        let mut conn = self.store.get_conn().await?;

        ProductRow::find_by_id_and_tenant_id(&mut conn, product_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        validate_matrix_prices(
            &PriceRow::list_by_product_id(&mut conn, product_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?,
        )?;

        drop(conn);

        let today = common_clock::now().date_naive();
        let effective_on = update.effective_on.unwrap_or(today);
        if effective_on < today {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "a price changes from today or later, not {effective_on}"
            ))));
        }

        // Each lineage gains a successor from `effective_on`; rows already billed or referenced
        // keep their rates. Read under the lock so two edits never start from the same rates.
        let update = &update;
        self.store
            .transaction_bumping_tenant(tenant_id, |conn| {
                async move {
                    let tips = lineage_tips(
                        PriceRow::lock_by_product_id(conn, product_id, tenant_id).await?,
                    );
                    if let Some(later) = tips
                        .iter()
                        .filter_map(|tip| tip.effective_from)
                        .find(|from| *from > effective_on)
                    {
                        return Err(Report::new(StoreError::InvalidArgument(format!(
                            "a change of these prices takes over on {later}: date this one on or after it"
                        ))));
                    }
                    let planned = plan_matrix_update(tips, update)?;
                    let mut updated_prices = Vec::with_capacity(planned.len());
                    for (tip, pricing_json) in planned {
                        let row = match pricing_json {
                            Some(pricing) => {
                                PriceSuccessorRowNew {
                                    id: PriceId::new(),
                                    product_id: tip.product_id,
                                    cadence: tip.cadence.clone(),
                                    currency: tip.currency.clone(),
                                    pricing,
                                    tenant_id,
                                    catalog: tip.catalog,
                                    credit_unit_id: tip.credit_unit_id,
                                    credit_unit_rate: tip.credit_unit_rate,
                                    effective_from: effective_on,
                                    supersedes: tip.supersedes.unwrap_or(tip.id),
                                }
                                .insert(conn)
                                .await?
                            }
                            None => tip,
                        };
                        updated_prices.push(Price::try_from(row)?);
                    }
                    Ok(updated_prices)
                }
                .scope_boxed()
            })
            .await
    }

    pub(crate) async fn preview_matrix_update(
        &self,
        tenant_id: TenantId,
        product_id: ProductId,
        update: &MatrixPriceUpdate,
    ) -> StoreResult<MatrixUpdatePreview> {
        let mut conn = self.store.get_conn().await?;

        ProductRow::find_by_id_and_tenant_id(&mut conn, product_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let roots = PriceRow::list_by_product_id(&mut conn, product_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        validate_matrix_prices(&roots)?;
        let root_ids: Vec<PriceId> = roots.iter().map(|root| root.id).collect();
        let successors = PriceRow::successors_of(&mut conn, &root_ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let prices = lineage_tips(roots.into_iter().chain(successors).collect());

        let mut affected_prices_count = 0;
        let mut affected_price_ids: Vec<PriceId> = Vec::new();
        let mut rows_to_add = 0;
        let mut rows_to_remove = 0;

        for price_row in &prices {
            let pricing: Pricing =
                serde_json::from_value(price_row.pricing.clone()).map_err(|e| {
                    Report::new(StoreError::SerdeError(
                        "Failed to deserialize Pricing".to_string(),
                        e,
                    ))
                })?;

            if let Pricing::Usage(UsagePricingModel::DimensionRates(rates)) = pricing {
                let mut price_affected = false;

                for key in &update.remove_rows {
                    if key_in(&rates, key).is_some_and(|key| has_key(&rates, &key)) {
                        rows_to_remove += 1;
                        price_affected = true;
                    }
                }

                for row_add in &update.add_rows {
                    let key = MatrixDimensionKey {
                        dimension1: row_add.dimension1.clone(),
                        dimension2: row_add.dimension2.clone(),
                    };
                    if key_in(&rates, &key).is_some_and(|key| !has_key(&rates, &key)) {
                        rows_to_add += 1;
                        price_affected = true;
                    }
                }

                if price_affected {
                    affected_prices_count += 1;
                    affected_price_ids.push(price_row.supersedes.unwrap_or(price_row.id));
                }
            }
        }

        let affected_subscriptions_count =
            SubscriptionComponentRow::count_active_subscriptions_by_product_id(
                &mut conn,
                &product_id,
                &tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let plan_info = PlanRow::list_plan_info_by_price_ids(&mut conn, &affected_price_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut plan_map: std::collections::BTreeMap<String, Vec<i32>> =
            std::collections::BTreeMap::new();
        for (name, version) in plan_info {
            plan_map.entry(name).or_default().push(version);
        }

        let affected_plans: Vec<AffectedPlan> = plan_map
            .into_iter()
            .map(|(plan_name, versions)| AffectedPlan {
                plan_name,
                versions,
            })
            .collect();

        Ok(MatrixUpdatePreview {
            affected_prices_count,
            affected_subscriptions_count: affected_subscriptions_count as usize,
            rows_to_add,
            rows_to_remove,
            affected_plans,
        })
    }
}

/// The latest member of each price lineage among `rows` (roots and their successors): the rates
/// an edit starts from.
fn lineage_tips(rows: Vec<PriceRow>) -> Vec<PriceRow> {
    let mut tips: std::collections::BTreeMap<PriceId, PriceRow> = std::collections::BTreeMap::new();
    for row in rows {
        let root = row.supersedes.unwrap_or(row.id);
        let later = tips.get(&root).is_none_or(|tip| {
            (tip.effective_from, tip.created_at, tip.id)
                < (row.effective_from, row.created_at, row.id)
        });
        if later {
            tips.insert(root, row);
        }
    }
    tips.into_values().collect()
}

/// The new pricing of each price rated by dimension, `None` where the update changes nothing.
fn plan_matrix_update(
    prices: Vec<PriceRow>,
    update: &MatrixPriceUpdate,
) -> StoreResult<Vec<(PriceRow, Option<serde_json::Value>)>> {
    let mut planned: Vec<(PriceRow, Option<serde_json::Value>)> = Vec::new();

    for price_row in prices {
        let pricing: Pricing = serde_json::from_value(price_row.pricing.clone()).map_err(|e| {
            Report::new(StoreError::SerdeError(
                "Failed to deserialize Pricing".to_string(),
                e,
            ))
        })?;

        if let Pricing::Usage(UsagePricingModel::DimensionRates(mut rates)) = pricing {
            let mut modified = false;

            // Remove rows matching remove_rows
            for key in &update.remove_rows {
                let Some(key) = key_in(&rates, key) else {
                    continue;
                };
                let before = rates.rates.len();
                rates.rates.retain(|rate| rate.key != key);
                if rates.rates.len() != before {
                    modified = true;
                }
            }

            // Add rows for add_rows (skip if already present)
            for row_add in &update.add_rows {
                let key = MatrixDimensionKey {
                    dimension1: row_add.dimension1.clone(),
                    dimension2: row_add.dimension2.clone(),
                };
                let Some(key) = key_in(&rates, &key) else {
                    continue;
                };
                if !has_key(&rates, &key) {
                    let rate = row_add
                        .per_unit_prices
                        .get(&price_row.currency)
                        .copied()
                        .unwrap_or(rust_decimal::Decimal::ZERO);
                    rates.rates.push(DimensionRate {
                        key,
                        rate,
                        label: None,
                    });
                    modified = true;
                }
            }

            if modified {
                let new_pricing = Pricing::Usage(UsagePricingModel::DimensionRates(rates));
                let pricing_json = serde_json::to_value(&new_pricing).map_err(|e| {
                    Report::new(StoreError::SerdeError(
                        "Failed to serialize Pricing".to_string(),
                        e,
                    ))
                })?;

                planned.push((price_row, Some(pricing_json)));
            } else {
                planned.push((price_row, None));
            }
        }
    }
    Ok(planned)
}
