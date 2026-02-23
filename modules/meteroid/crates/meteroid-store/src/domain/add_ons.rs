use crate::domain::enums::{FeeTypeEnum, SubscriptionFeeBillingPeriod};
use crate::domain::price_components::{ComponentParameters, PriceEntry, ResolvedFee};
use crate::domain::prices;
use crate::domain::prices::FeeStructure;
use crate::domain::subscription_add_ons::{
    SubscriptionAddOnCustomization, SubscriptionAddOnParameterization,
};
use crate::domain::subscription_components::SubscriptionFee;
use crate::domain::{Price, Product};
use crate::errors::StoreError;
use chrono::NaiveDateTime;
use common_domain::ids::{AddOnId, BaseId, PriceId, ProductId, TenantId};
use diesel_models::add_ons::{AddOnRow, AddOnRowNew, AddOnRowPatch};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct AddOn {
    pub id: AddOnId,
    pub name: String,
    pub tenant_id: TenantId,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub product_id: ProductId,
    pub price_id: PriceId,
    pub description: Option<String>,
    pub self_serviceable: bool,
    pub max_instances_per_subscription: Option<i32>,
    pub archived_at: Option<NaiveDateTime>,
    // Eagerly loaded
    pub fee_type: Option<FeeTypeEnum>,
    pub fee_structure: Option<FeeStructure>,
    pub price: Option<Price>,
}

impl From<AddOnRow> for AddOn {
    fn from(row: AddOnRow) -> Self {
        AddOn {
            id: row.id,
            name: row.name,
            tenant_id: row.tenant_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
            product_id: row.product_id,
            price_id: row.price_id,
            description: row.description,
            self_serviceable: row.self_serviceable,
            max_instances_per_subscription: row.max_instances_per_subscription,
            archived_at: row.archived_at,
            fee_type: None,
            fee_structure: None,
            price: None,
        }
    }
}

impl AddOn {
    /// Centralized resolution — uses the linked product + price to resolve fee.
    pub fn resolve_subscription_fee(
        &self,
        products: &HashMap<ProductId, Product>,
        prices: &HashMap<PriceId, Price>,
        params: Option<&ComponentParameters>,
    ) -> Result<ResolvedFee, StoreError> {
        let product = products.get(&self.product_id).ok_or_else(|| {
            StoreError::InvalidArgument(format!(
                "Product {} not found for add-on {}",
                self.product_id, self.id
            ))
        })?;
        let price = prices.get(&self.price_id).ok_or_else(|| {
            StoreError::InvalidArgument(format!(
                "Price {} not found for add-on {}",
                self.price_id, self.id
            ))
        })?;

        let fee_structure = &product.fee_structure;

        let fee = prices::resolve_subscription_fee(fee_structure, &price.pricing, params)?;
        let period = if let Some(bp) = params.and_then(|p| p.billing_period) {
            bp.as_subscription_billing_period()
        } else {
            prices::fee_type_billing_period(fee_structure)
                .unwrap_or_else(|| price.cadence.as_subscription_billing_period())
        };

        Ok(ResolvedFee {
            period,
            fee,
            price_id: Some(price.id),
        })
    }

    /// Resolves an add-on with its customization into a uniform result.
    /// Handles None, PriceOverride, and Parameterization variants.
    pub fn resolve_customized(
        &self,
        products: &HashMap<ProductId, Product>,
        prices: &HashMap<PriceId, Price>,
        customization: &SubscriptionAddOnCustomization,
    ) -> Result<ResolvedAddOn, StoreError> {
        match customization {
            SubscriptionAddOnCustomization::None => {
                let resolved = self.resolve_subscription_fee(products, prices, None)?;
                Ok(ResolvedAddOn {
                    name: self.name.clone(),
                    period: resolved.period,
                    fee: resolved.fee,
                    product_id: Some(self.product_id),
                    price_id: resolved.price_id,
                    price_entry: None,
                })
            }
            SubscriptionAddOnCustomization::PriceOverride { name, price_entry } => {
                let product = products.get(&self.product_id).ok_or_else(|| {
                    StoreError::InvalidArgument(format!(
                        "Product {} not found for add-on {}",
                        self.product_id, self.id
                    ))
                })?;
                let fee_structure = &product.fee_structure;

                match price_entry {
                    PriceEntry::Existing(price_id) => {
                        let price = prices.get(price_id).ok_or_else(|| {
                            StoreError::InvalidArgument(format!(
                                "Override price {} not found for add-on {}",
                                price_id, self.id
                            ))
                        })?;
                        let fee =
                            prices::resolve_subscription_fee(fee_structure, &price.pricing, None)?;
                        let period = prices::fee_type_billing_period(fee_structure)
                            .unwrap_or_else(|| price.cadence.as_subscription_billing_period());
                        Ok(ResolvedAddOn {
                            name: name.clone().unwrap_or_else(|| self.name.clone()),
                            period,
                            fee,
                            product_id: Some(self.product_id),
                            price_id: Some(price.id),
                            price_entry: Some(price_entry.clone()),
                        })
                    }
                    PriceEntry::New(price_input) => {
                        let fee = prices::resolve_subscription_fee(
                            fee_structure,
                            &price_input.pricing,
                            None,
                        )?;
                        let period =
                            prices::fee_type_billing_period(fee_structure).unwrap_or_else(|| {
                                price_input.cadence.as_subscription_billing_period()
                            });
                        Ok(ResolvedAddOn {
                            name: name.clone().unwrap_or_else(|| self.name.clone()),
                            period,
                            fee,
                            product_id: Some(self.product_id),
                            price_id: None,
                            price_entry: Some(price_entry.clone()),
                        })
                    }
                }
            }
            SubscriptionAddOnCustomization::Parameterization(param) => {
                let params = Self::params_from_addon_parameterization(param);
                let resolved = self.resolve_subscription_fee(products, prices, Some(&params))?;
                Ok(ResolvedAddOn {
                    name: self.name.clone(),
                    period: resolved.period,
                    fee: resolved.fee,
                    product_id: Some(self.product_id),
                    price_id: resolved.price_id,
                    price_entry: None,
                })
            }
        }
    }

    fn params_from_addon_parameterization(
        param: &SubscriptionAddOnParameterization,
    ) -> ComponentParameters {
        ComponentParameters {
            initial_slot_count: param.initial_slot_count,
            billing_period: param.billing_period,
            committed_capacity: param.committed_capacity,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedAddOn {
    pub name: String,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
    /// Set when the override uses a PriceEntry (for deferred materialization)
    pub price_entry: Option<PriceEntry>,
}

#[derive(Debug, Clone)]
pub struct AddOnNew {
    pub name: String,
    pub tenant_id: TenantId,
    pub product_id: ProductId,
    pub price_id: PriceId,
    pub description: Option<String>,
    pub self_serviceable: bool,
    pub max_instances_per_subscription: Option<i32>,
}

impl From<AddOnNew> for AddOnRowNew {
    fn from(new: AddOnNew) -> Self {
        AddOnRowNew {
            id: AddOnId::new(),
            name: new.name,
            tenant_id: new.tenant_id,
            product_id: new.product_id,
            price_id: new.price_id,
            description: new.description,
            self_serviceable: new.self_serviceable,
            max_instances_per_subscription: new.max_instances_per_subscription,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AddOnPatch {
    pub id: AddOnId,
    pub tenant_id: TenantId,
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub self_serviceable: Option<bool>,
    pub max_instances_per_subscription: Option<Option<i32>>,
}

impl AddOnPatch {
    pub fn into_row_patch(self, price_id: Option<PriceId>) -> AddOnRowPatch {
        AddOnRowPatch {
            id: self.id,
            tenant_id: self.tenant_id,
            name: self.name,
            price_id,
            description: self.description,
            self_serviceable: self.self_serviceable,
            max_instances_per_subscription: self.max_instances_per_subscription,
            updated_at: chrono::Utc::now().naive_utc(),
        }
    }
}
