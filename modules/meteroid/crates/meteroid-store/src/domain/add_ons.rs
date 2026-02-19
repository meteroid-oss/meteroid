use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::prices;
use crate::domain::price_components::{ComponentParameters, ResolvedFee};
use crate::domain::subscription_add_ons::{SubscriptionAddOnCustomization, SubscriptionAddOnParameterization};
use crate::domain::subscription_components::SubscriptionFee;
use crate::domain::{Price, Product};
use crate::errors::StoreError;
use chrono::NaiveDateTime;
use common_domain::ids::{AddOnId, BaseId, PlanVersionId, PriceId, ProductId, TenantId};
use diesel_models::add_ons::{AddOnRow, AddOnRowNew, AddOnRowPatch};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct AddOn {
    pub id: AddOnId,
    pub name: String,
    pub tenant_id: TenantId,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub plan_version_id: Option<PlanVersionId>,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
}

impl From<AddOnRow> for AddOn {
    fn from(row: AddOnRow) -> Self {
        AddOn {
            id: row.id,
            name: row.name,
            tenant_id: row.tenant_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
            plan_version_id: row.plan_version_id,
            product_id: row.product_id,
            price_id: row.price_id,
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
        let product_id = self.product_id.ok_or_else(|| {
            StoreError::InvalidArgument(format!("AddOn {} has no product_id", self.id))
        })?;
        let price_id = self.price_id.ok_or_else(|| {
            StoreError::InvalidArgument(format!("AddOn {} has no price_id", self.id))
        })?;

        let product = products.get(&product_id).ok_or_else(|| {
            StoreError::InvalidArgument(format!("Product {} not found for add-on {}", product_id, self.id))
        })?;
        let price = prices.get(&price_id).ok_or_else(|| {
            StoreError::InvalidArgument(format!("Price {} not found for add-on {}", price_id, self.id))
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
    /// Handles None, Override, and Parameterization variants.
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
                    product_id: self.product_id,
                    price_id: resolved.price_id,
                })
            }
            SubscriptionAddOnCustomization::Override(ov) => {
                Ok(ResolvedAddOn {
                    name: ov.name.clone(),
                    period: ov.period,
                    fee: ov.fee.clone(),
                    product_id: self.product_id,
                    price_id: self.price_id,
                })
            }
            SubscriptionAddOnCustomization::Parameterization(param) => {
                let params = Self::params_from_addon_parameterization(param);
                let resolved = self.resolve_subscription_fee(products, prices, Some(&params))?;
                Ok(ResolvedAddOn {
                    name: self.name.clone(),
                    period: resolved.period,
                    fee: resolved.fee,
                    product_id: self.product_id,
                    price_id: resolved.price_id,
                })
            }
        }
    }

    fn params_from_addon_parameterization(param: &SubscriptionAddOnParameterization) -> ComponentParameters {
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
}

#[derive(Debug, Clone)]
pub struct AddOnNew {
    pub name: String,
    pub tenant_id: TenantId,
    pub plan_version_id: Option<PlanVersionId>,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
}

impl From<AddOnNew> for AddOnRowNew {
    fn from(new: AddOnNew) -> Self {
        AddOnRowNew {
            id: AddOnId::new(),
            name: new.name,
            tenant_id: new.tenant_id,
            plan_version_id: new.plan_version_id,
            product_id: new.product_id,
            price_id: new.price_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AddOnPatch {
    pub id: AddOnId,
    pub tenant_id: TenantId,
    pub name: Option<String>,
    pub plan_version_id: Option<Option<PlanVersionId>>,
    pub product_id: Option<Option<ProductId>>,
    pub price_id: Option<Option<PriceId>>,
}

impl From<AddOnPatch> for AddOnRowPatch {
    fn from(patch: AddOnPatch) -> Self {
        AddOnRowPatch {
            id: patch.id,
            tenant_id: patch.tenant_id,
            name: patch.name,
            plan_version_id: patch.plan_version_id,
            product_id: patch.product_id,
            price_id: patch.price_id,
            updated_at: chrono::Utc::now().naive_utc(),
        }
    }
}
