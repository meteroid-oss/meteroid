use common_domain::ids::{ProductId, TenantId};
use diesel_models::plans::PlanRow;
use diesel_models::prices::PriceRow;
use diesel_models::products::ProductRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use error_stack::Report;
use uuid::Uuid;

use crate::domain::price_components::{MatrixRow, UsagePricingModel};
use crate::domain::prices::{
    AffectedPlan, FeeStructure, MatrixDimensionKey, MatrixPriceUpdate, MatrixUpdatePreview, Price,
    Pricing, UsageModel,
};
use crate::errors::StoreError;
use crate::StoreResult;

use super::Services;

fn dimension_key_matches(row: &MatrixRow, key: &MatrixDimensionKey) -> bool {
    let d1_match = row.dimension1.key == key.dimension1.key
        && row.dimension1.value == key.dimension1.value;
    let d2_match = match (&row.dimension2, &key.dimension2) {
        (None, None) => true,
        (Some(a), Some(b)) => a.key == b.key && a.value == b.value,
        _ => false,
    };
    d1_match && d2_match
}

fn validate_matrix_product(fee_structure: &serde_json::Value) -> StoreResult<()> {
    let fs: FeeStructure = serde_json::from_value(fee_structure.clone()).map_err(|e| {
        Report::new(StoreError::SerdeError(
            "Failed to deserialize FeeStructure".to_string(),
            e,
        ))
    })?;

    match fs {
        FeeStructure::Usage {
            model: UsageModel::Matrix,
            ..
        } => Ok(()),
        _ => Err(Report::new(StoreError::InvalidArgument(
            "Product is not a matrix usage product".to_string(),
        ))),
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

        let product_row =
            ProductRow::find_by_id_and_tenant_id(&mut conn, product_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        validate_matrix_product(&product_row.fee_structure)?;

        let prices = PriceRow::list_by_product_id(&mut conn, product_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut updated_prices = Vec::new();

        for price_row in prices {
            let pricing: Pricing =
                serde_json::from_value(price_row.pricing.clone()).map_err(|e| {
                    Report::new(StoreError::SerdeError(
                        "Failed to deserialize Pricing".to_string(),
                        e,
                    ))
                })?;

            if let Pricing::Usage(UsagePricingModel::Matrix { mut rates }) = pricing {
                let mut modified = false;

                // Remove rows matching remove_rows
                for key in &update.remove_rows {
                    let before = rates.len();
                    rates.retain(|r| !dimension_key_matches(r, key));
                    if rates.len() != before {
                        modified = true;
                    }
                }

                // Add rows for add_rows (skip if already present)
                for row_add in &update.add_rows {
                    let key = MatrixDimensionKey {
                        dimension1: row_add.dimension1.clone(),
                        dimension2: row_add.dimension2.clone(),
                    };
                    if !rates.iter().any(|r| dimension_key_matches(r, &key)) {
                        let per_unit_price = row_add
                            .per_unit_prices
                            .get(&price_row.currency)
                            .copied()
                            .unwrap_or(rust_decimal::Decimal::ZERO);
                        rates.push(MatrixRow {
                            dimension1: row_add.dimension1.clone(),
                            dimension2: row_add.dimension2.clone(),
                            per_unit_price,
                        });
                        modified = true;
                    }
                }

                if modified {
                    let new_pricing = Pricing::Usage(UsagePricingModel::Matrix { rates });
                    let pricing_json = serde_json::to_value(&new_pricing).map_err(|e| {
                        Report::new(StoreError::SerdeError(
                            "Failed to serialize Pricing".to_string(),
                            e,
                        ))
                    })?;

                    let updated_row =
                        PriceRow::update_pricing(&mut conn, price_row.id, tenant_id, pricing_json)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                    updated_prices.push(Price::try_from(updated_row)?);
                } else {
                    updated_prices.push(Price::try_from(price_row)?);
                }
            }
        }

        Ok(updated_prices)
    }

    pub(crate) async fn preview_matrix_update(
        &self,
        tenant_id: TenantId,
        product_id: ProductId,
        update: &MatrixPriceUpdate,
    ) -> StoreResult<MatrixUpdatePreview> {
        let mut conn = self.store.get_conn().await?;

        let product_row =
            ProductRow::find_by_id_and_tenant_id(&mut conn, product_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        validate_matrix_product(&product_row.fee_structure)?;

        let prices = PriceRow::list_by_product_id(&mut conn, product_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut affected_prices_count = 0;
        let mut rows_to_add = 0;
        let mut rows_to_remove = 0;

        for price_row in &prices {
            let pricing: Pricing = serde_json::from_value(price_row.pricing.clone()).map_err(
                |e| {
                    Report::new(StoreError::SerdeError(
                        "Failed to deserialize Pricing".to_string(),
                        e,
                    ))
                },
            )?;

            if let Pricing::Usage(UsagePricingModel::Matrix { rates }) = pricing {
                let mut price_affected = false;

                for key in &update.remove_rows {
                    if rates.iter().any(|r| dimension_key_matches(r, key)) {
                        rows_to_remove += 1;
                        price_affected = true;
                    }
                }

                for row_add in &update.add_rows {
                    let key = MatrixDimensionKey {
                        dimension1: row_add.dimension1.clone(),
                        dimension2: row_add.dimension2.clone(),
                    };
                    if !rates.iter().any(|r| dimension_key_matches(r, &key)) {
                        rows_to_add += 1;
                        price_affected = true;
                    }
                }

                if price_affected {
                    affected_prices_count += 1;
                }
            }
        }

        let affected_price_ids: Vec<_> = prices.iter().map(|p| p.id).collect();

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
