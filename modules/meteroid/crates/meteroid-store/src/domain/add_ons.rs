use crate::domain::Price;
use crate::domain::credit_units::CreditUnitsById;
use crate::domain::entitlements::Entitlement;
use crate::domain::enums::{BillingPeriodEnum, FeeTypeEnum, SubscriptionFeeBillingPeriod};
use crate::domain::price_cells::{cadence_free, cell, single_cadence};
use crate::domain::price_components::{ComponentParameters, PriceEntry, PriceInput, ResolvedFee};
use crate::domain::prices;
use crate::domain::prices::FeeStructure;
use crate::domain::subscription_add_ons::{
    SubscriptionAddOnCustomization, SubscriptionAddOnParameterization,
};
use crate::domain::subscription_components::SubscriptionFee;
use crate::errors::StoreError;
use chrono::NaiveDateTime;
use common_domain::ids::{AddOnId, BaseId, PriceId, ProductId, TenantId};
use diesel_models::add_ons::{AddOnRow, AddOnRowNew, AddOnRowPatch};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct AddOn {
    pub id: AddOnId,
    /// Unique per tenant and immutable (ADR-0013).
    pub code: String,
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
    /// How the add-on bills its product, its own; `fee_type` is its kind. None only when the
    /// stored structure does not read.
    pub fee_type: Option<FeeTypeEnum>,
    pub fee_structure: Option<FeeStructure>,
    // Eagerly loaded
    /// Its product's icon and color.
    pub product_icon: Option<String>,
    pub product_color: Option<String>,
    pub price: Option<Price>,
    /// Every price of the add-on, one per currency and billing period; `price_id` is the first.
    pub prices: Vec<Price>,
    pub entitlements: Vec<Entitlement>,
}

impl From<AddOnRow> for AddOn {
    fn from(row: AddOnRow) -> Self {
        let fee_structure: Option<FeeStructure> = serde_json::from_value(row.fee_structure)
            .inspect_err(|error| {
                tracing::error!(add_on = %row.id, %error, "unreadable add-on fee structure");
            })
            .ok();
        AddOn {
            id: row.id,
            code: row.code,
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
            fee_type: fee_structure.as_ref().map(FeeStructure::fee_type),
            fee_structure,
            product_icon: None,
            product_color: None,
            price: None,
            prices: vec![],
            entitlements: vec![],
        }
    }
}

impl AddOn {
    /// Every price of the add-on; one not loaded with its prices has its first only.
    pub fn catalog_prices(&self) -> Vec<Price> {
        if self.prices.is_empty() {
            self.price.iter().cloned().collect()
        } else {
            self.prices.clone()
        }
    }

    /// The ids of every price of the add-on, its first included.
    pub fn price_ids(&self) -> Vec<PriceId> {
        let mut ids: Vec<PriceId> = self.prices.iter().map(|p| p.id).collect();
        if !ids.contains(&self.price_id) {
            ids.insert(0, self.price_id);
        }
        ids
    }

    /// The add-on as sold in `currency` on `term`: at its price there on the term, or on its one
    /// cadence when it has a single one across currencies (as a plan charge). Refused when it has
    /// none: "<add-on> has no price in <currency>".
    pub fn in_currency(
        &self,
        currency: &str,
        term: Option<BillingPeriodEnum>,
    ) -> Result<AddOn, StoreError> {
        let prices = self.catalog_prices();
        if prices.is_empty() {
            return Ok(self.clone());
        }
        let currency = currency.trim().to_ascii_uppercase();
        let every = cell(&prices, &currency, None);
        if every.is_empty() {
            return Err(StoreError::InvalidArgument(format!(
                "{} has no price in {currency}",
                self.name
            )));
        }
        let candidates = match term {
            None => every,
            Some(term) => {
                let on_term = cell(&prices, &currency, Some(term));
                if !on_term.is_empty() {
                    on_term
                } else if let Some(own) = single_cadence(&prices, &currency) {
                    cell(&prices, &currency, Some(own))
                } else {
                    return Err(StoreError::InvalidArgument(format!(
                        "{} has no price in {currency} for {term:?}",
                        self.name
                    )));
                }
            }
        };
        // Its first price when it is of the cell, so a legacy add-on bills as it always did.
        let chosen = candidates
            .iter()
            .find(|p| p.id == self.price_id)
            .or_else(|| candidates.iter().min_by_key(|p| p.cadence))
            .map(|p| (*p).clone());
        let mut priced = self.clone();
        if let Some(price) = chosen {
            priced.price_id = price.id;
            priced.price = Some(price);
        }
        Ok(priced)
    }

    /// The add-on's structure, refused when it does not read.
    pub fn structure(&self) -> Result<&FeeStructure, StoreError> {
        self.fee_structure.as_ref().ok_or_else(|| {
            StoreError::InvalidArgument(format!("Add-on {} has no fee structure", self.name))
        })
    }

    /// Whether the add-on charges per seat: it then has one instance at most, billing the
    /// count of its quantity.
    pub fn charges_seats(&self) -> bool {
        matches!(self.fee_structure, Some(FeeStructure::Slot { .. }))
    }

    /// Centralized resolution — the add-on's structure and its price.
    pub fn resolve_subscription_fee(
        &self,
        prices: &HashMap<PriceId, Price>,
        units: &CreditUnitsById,
        params: Option<&ComponentParameters>,
    ) -> Result<ResolvedFee, StoreError> {
        let price = prices
            .get(&self.price_id)
            .or_else(|| self.prices.iter().find(|p| p.id == self.price_id))
            .ok_or_else(|| {
                StoreError::InvalidArgument(format!(
                    "Price {} not found for add-on {}",
                    self.price_id, self.id
                ))
            })?;

        let fee_structure = self.structure()?;

        let fee = prices::resolve_subscription_fee(
            fee_structure,
            &price.pricing,
            price.credit_unit.as_ref(),
            units,
            params,
        )?;
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
        prices: &HashMap<PriceId, Price>,
        units: &CreditUnitsById,
        customization: &SubscriptionAddOnCustomization,
    ) -> Result<ResolvedAddOn, StoreError> {
        match customization {
            SubscriptionAddOnCustomization::None => {
                let resolved = self.resolve_subscription_fee(prices, units, None)?;
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
                let fee_structure = self.structure()?;

                match price_entry {
                    PriceEntry::Existing(price_id) => {
                        let price = prices.get(price_id).ok_or_else(|| {
                            StoreError::InvalidArgument(format!(
                                "Override price {} not found for add-on {}",
                                price_id, self.id
                            ))
                        })?;
                        let fee = prices::resolve_subscription_fee(
                            fee_structure,
                            &price.pricing,
                            price.credit_unit.as_ref(),
                            units,
                            None,
                        )?;
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
                            price_input.credit_unit.as_ref(),
                            units,
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
                let resolved = self.resolve_subscription_fee(prices, units, Some(&params))?;
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

/// The instances an add-on of `structure` may have on a subscription: one at most when it
/// charges per seat (it bills its quantity's count, not instances), refused above.
pub fn instances_allowed(
    structure: &FeeStructure,
    max_instances_per_subscription: Option<i32>,
) -> Result<Option<i32>, StoreError> {
    match structure {
        FeeStructure::Slot { .. } => match max_instances_per_subscription {
            None | Some(1) => Ok(Some(1)),
            Some(_) => Err(StoreError::InvalidArgument(
                "An add-on charging per seat has one instance at most: its seats are its quantity"
                    .to_string(),
            )),
        },
        _ => Ok(max_instances_per_subscription),
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
    /// How the add-on bills its product.
    pub fee_structure: FeeStructure,
    pub description: Option<String>,
    pub self_serviceable: bool,
    pub max_instances_per_subscription: Option<i32>,
    pub entitlements: Vec<crate::domain::entitlements::EntitlementSpec>,
}

impl TryFrom<AddOnNew> for AddOnRowNew {
    type Error = StoreError;

    fn try_from(new: AddOnNew) -> Result<Self, Self::Error> {
        let fee_structure = serde_json::to_value(&new.fee_structure).map_err(|e| {
            StoreError::SerdeError("Failed to serialize fee_structure".to_string(), e)
        })?;
        Ok(AddOnRowNew {
            id: AddOnId::new(),
            name: new.name,
            tenant_id: new.tenant_id,
            product_id: new.product_id,
            price_id: new.price_id,
            description: new.description,
            self_serviceable: new.self_serviceable,
            max_instances_per_subscription: new.max_instances_per_subscription,
            fee_structure,
        })
    }
}

/// How an edit reprices an add-on.
#[derive(Debug, Clone)]
pub enum AddOnPriceChange {
    /// Replaces the price of its cell, as an add-on of one price was edited.
    One(PriceEntry),
    /// Every price of the add-on, one per currency and period; those left out are unlinked.
    All(Vec<PriceInput>),
}

/// Whether two prices are of one cell: one currency, on one cadence or either of none.
pub fn same_cell(a: &Price, b: &Price) -> bool {
    a.currency.eq_ignore_ascii_case(&b.currency)
        && (a.cadence == b.cadence || cadence_free(&a.pricing) || cadence_free(&b.pricing))
}

/// An add-on has one price per currency and billing period.
pub fn ensure_one_price_per_cell(name: &str, prices: &[Price]) -> Result<(), StoreError> {
    for (idx, a) in prices.iter().enumerate() {
        if let Some(b) = prices[idx + 1..].iter().find(|b| same_cell(a, b)) {
            return Err(StoreError::InvalidArgument(format!(
                "{name} has two prices in {} for {:?}",
                a.currency.to_ascii_uppercase(),
                if cadence_free(&a.pricing) {
                    b.cadence
                } else {
                    a.cadence
                }
            )));
        }
    }
    Ok(())
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
            updated_at: common_clock::now().naive_utc(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn price(currency: &str, cadence: BillingPeriodEnum, rate: i64) -> Price {
        Price {
            id: PriceId::new(),
            product_id: ProductId::default(),
            cadence,
            currency: currency.to_owned(),
            pricing: prices::Pricing::Rate {
                rate: Decimal::from(rate),
            },
            tenant_id: TenantId::default(),
            created_at: NaiveDateTime::default(),
            archived_at: None,
            catalog: true,
            credit_unit: None,
        }
    }

    fn add_on(prices: Vec<Price>) -> AddOn {
        AddOn {
            id: AddOnId::new(),
            code: "support".to_owned(),
            name: "Support".to_owned(),
            tenant_id: TenantId::default(),
            created_at: NaiveDateTime::default(),
            updated_at: NaiveDateTime::default(),
            product_id: ProductId::default(),
            price_id: prices[0].id,
            description: None,
            self_serviceable: true,
            max_instances_per_subscription: None,
            archived_at: None,
            fee_type: Some(FeeTypeEnum::Rate),
            fee_structure: Some(FeeStructure::Rate {}),
            product_icon: None,
            product_color: None,
            price: Some(prices[0].clone()),
            prices,
            entitlements: vec![],
        }
    }

    #[test]
    fn an_add_on_is_sold_at_its_price_in_the_currency_on_the_term() {
        let support = add_on(vec![
            price("USD", BillingPeriodEnum::Monthly, 10),
            price("EUR", BillingPeriodEnum::Monthly, 9),
            price("EUR", BillingPeriodEnum::Annual, 90),
        ]);
        let yearly = support
            .in_currency("eur", Some(BillingPeriodEnum::Annual))
            .expect("EUR annual");
        assert_eq!(yearly.price_id, support.prices[2].id);
        assert_eq!(yearly.price.map(|p| p.id), Some(support.prices[2].id));
        let monthly = support
            .in_currency("EUR", None)
            .expect("EUR, the shortest cadence");
        assert_eq!(monthly.price_id, support.prices[1].id);
        assert_eq!(
            support.in_currency("USD", None).expect("USD").price_id,
            support.price_id,
            "its first price when of the cell"
        );
    }

    #[test]
    fn an_add_on_without_a_price_in_the_currency_is_refused() {
        let support = add_on(vec![
            price("USD", BillingPeriodEnum::Monthly, 10),
            price("USD", BillingPeriodEnum::Annual, 100),
        ]);
        let err = support
            .in_currency("EUR", Some(BillingPeriodEnum::Monthly))
            .expect_err("no EUR price");
        assert!(
            err.to_string().contains("Support has no price in EUR"),
            "{err}"
        );
        let err = support
            .in_currency("USD", Some(BillingPeriodEnum::Quarterly))
            .expect_err("priced per term, none quarterly");
        assert!(
            err.to_string()
                .contains("Support has no price in USD for Quarterly")
        );
    }

    #[test]
    fn an_add_on_of_one_cadence_bills_on_it_under_every_term() {
        let support = add_on(vec![
            price("USD", BillingPeriodEnum::Monthly, 10),
            price("EUR", BillingPeriodEnum::Monthly, 9),
        ]);
        let annual = support
            .in_currency("EUR", Some(BillingPeriodEnum::Annual))
            .expect("its monthly price");
        assert_eq!(annual.price_id, support.prices[1].id);
    }

    #[test]
    fn two_prices_in_one_cell_are_refused() {
        let prices = vec![
            price("EUR", BillingPeriodEnum::Monthly, 9),
            price("eur", BillingPeriodEnum::Monthly, 10),
        ];
        assert!(ensure_one_price_per_cell("Support", &prices).is_err());
        let prices = vec![
            price("EUR", BillingPeriodEnum::Monthly, 9),
            price("EUR", BillingPeriodEnum::Annual, 90),
        ];
        assert!(ensure_one_price_per_cell("Support", &prices).is_ok());
    }

    /// An add-on charging per seat has one instance at most; others keep their maximum.
    /// Rules: R-138
    #[test]
    fn a_seat_add_on_has_one_instance_at_most() {
        let seats = FeeStructure::Slot {
            unit_name: "Seats".to_owned(),
            quantity_id: None,
            upgrade_policy: crate::domain::price_components::UpgradePolicy::Prorated,
            downgrade_policy: crate::domain::price_components::DowngradePolicy::RemoveAtEndOfPeriod,
        };
        assert_eq!(instances_allowed(&seats, None).unwrap(), Some(1));
        assert_eq!(instances_allowed(&seats, Some(1)).unwrap(), Some(1));
        assert!(instances_allowed(&seats, Some(2)).is_err());
        assert_eq!(
            instances_allowed(&FeeStructure::Rate {}, Some(10)).unwrap(),
            Some(10)
        );
        assert_eq!(
            instances_allowed(&FeeStructure::Rate {}, None).unwrap(),
            None
        );
    }
}
