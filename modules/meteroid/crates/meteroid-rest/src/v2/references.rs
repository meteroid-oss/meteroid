//! What a charge names, resolved before it is written ([`resolve_refs`]) and named back by code
//! when it is read ([`CatalogNames`]).
use crate::AppState;
use crate::v2::Reader;
use crate::v2::charges::{Charge, ChargeRefs, Names, PriceModel, Quantity, StoredCharge};
use crate::v2::ids;
use crate::v2::problem::Problem;
use common_domain::ids::{
    BaseId, BillableMetricId, ManagedQuantityId, ProductId, RateCardId, TenantId,
};
use meteroid_store::domain::price_components::UsagePricingModel;
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::products::ProductInterface;
use meteroid_store::repositories::{ManagedQuantitiesInterface, RateCardsInterface};
use std::collections::HashMap;

/// A reference not found, read as the field naming it.
fn not_found(field: String, what: &'static str, raw: &str) -> impl FnOnce(Problem) -> Problem {
    let raw = raw.to_owned();
    move |problem: Problem| {
        if problem.status == 404 {
            Problem::field(field, "not_found", format!("{what} {raw} not found"))
        } else {
            problem
        }
    }
}

/// Resolves the product, metric, managed quantity and rate cards `charge` names; `path` is its
/// JSON pointer. An archived managed quantity or rate card is refused: a charge written now is a
/// new use of it.
pub async fn resolve_refs(
    state: &AppState,
    tenant_id: TenantId,
    charge: &Charge,
    path: &str,
) -> Result<ChargeRefs, Problem> {
    let product_id = ids::product_id(state, tenant_id, &charge.product)
        .await
        .map_err(not_found(
            format!("{path}/product"),
            "product",
            &charge.product,
        ))?;
    let product = state
        .store
        .find_product_by_id(product_id, tenant_id)
        .await?;
    let mut refs = ChargeRefs {
        product_id,
        product_name: product.name,
        metric_id: None,
        quantity: None,
        rate_cards: HashMap::new(),
    };
    match &charge.quantity {
        Some(Quantity::Metered(metered)) => {
            let metric = ids::metric(state, tenant_id, &metered.metric)
                .await
                .map_err(not_found(
                    format!("{path}/quantity/metric"),
                    "metric",
                    &metered.metric,
                ))?;
            refs.metric_id = Some(metric.id);
        }
        Some(Quantity::Managed(managed)) => {
            let quantity = ids::quantity(state, tenant_id, &managed.quantity)
                .await
                .map_err(not_found(
                    format!("{path}/quantity/quantity"),
                    "managed quantity",
                    &managed.quantity,
                ))?;
            if quantity.archived_at.is_some() {
                return Err(Problem::field(
                    format!("{path}/quantity/quantity"),
                    "archived",
                    format!("managed quantity {} is archived", quantity.code),
                ));
            }
            refs.quantity = Some((quantity.id, quantity.name));
        }
        _ => {}
    }
    for (index, price) in charge.prices.iter().enumerate() {
        if let PriceModel::RateCard(model) = &price.model
            && !refs.rate_cards.contains_key(&model.rate_card)
        {
            let id = ids::rate_card_id(state, tenant_id, &model.rate_card)
                .await
                .map_err(not_found(
                    format!("{path}/prices/{index}/model/rate_card"),
                    "rate card",
                    &model.rate_card,
                ))?;
            let card = state.store.find_rate_card(tenant_id, id).await?;
            if card.archived_at.is_some() {
                return Err(Problem::field(
                    format!("{path}/prices/{index}/model/rate_card"),
                    "archived",
                    format!("rate card {} is archived", card.code),
                ));
            }
            refs.rate_cards.insert(model.rate_card.clone(), id);
        }
    }
    Ok(refs)
}

/// The codes of what stored charges name, loaded once for all of them.
#[derive(Debug, Default)]
pub struct CatalogNames {
    products: HashMap<ProductId, String>,
    metrics: HashMap<BillableMetricId, String>,
    quantities: HashMap<ManagedQuantityId, String>,
    rate_cards: HashMap<RateCardId, String>,
}

impl CatalogNames {
    pub async fn load<'a>(
        state: &impl Reader,
        tenant_id: TenantId,
        charges: impl IntoIterator<Item = &'a StoredCharge>,
    ) -> Result<Self, Problem> {
        let mut product_ids = Vec::new();
        let mut metric_ids = Vec::new();
        let mut needs_quantities = false;
        let mut needs_rate_cards = false;
        for charge in charges {
            product_ids.push(charge.product_id);
            match &charge.structure {
                Some(FeeStructure::Usage { metric_id, .. })
                | Some(FeeStructure::Capacity { metric_id }) => metric_ids.push(*metric_id),
                Some(FeeStructure::Slot { .. }) => needs_quantities = true,
                _ => {}
            }
            needs_rate_cards |= charge.prices.iter().any(|price| {
                matches!(
                    price.pricing,
                    Pricing::Usage(UsagePricingModel::RateCard { .. })
                )
            });
        }
        product_ids.sort_by_key(|id| id.as_uuid());
        product_ids.dedup();
        metric_ids.sort_by_key(|id| id.as_uuid());
        metric_ids.dedup();

        let mut names = CatalogNames::default();
        if !product_ids.is_empty() {
            names.products = state
                .store()
                .find_products_by_ids(&product_ids, tenant_id)
                .await?
                .into_iter()
                .map(|product| (product.id, product.code))
                .collect();
        }
        if !metric_ids.is_empty() {
            names.metrics = state
                .store()
                .find_billable_metrics_by_ids(&metric_ids, tenant_id)
                .await?
                .into_iter()
                .map(|metric| (metric.id, metric.catalog_code))
                .collect();
        }
        if needs_quantities {
            names.quantities = state
                .store()
                .list_managed_quantities(tenant_id)
                .await?
                .into_iter()
                .map(|quantity| (quantity.id, quantity.code))
                .collect();
        }
        if needs_rate_cards {
            // Archived cards too: the prices they rate are read back by their code.
            names.rate_cards = state
                .store()
                .list_rate_cards_after(tenant_id, None, None, true, None, usize::MAX)
                .await?
                .into_iter()
                .map(|card| (card.id, card.code))
                .collect();
        }
        Ok(names)
    }
}

impl Names for CatalogNames {
    fn product(&self, id: ProductId) -> String {
        self.products
            .get(&id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }

    fn metric(&self, id: BillableMetricId) -> String {
        self.metrics
            .get(&id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }

    fn quantity(&self, id: ManagedQuantityId) -> String {
        self.quantities
            .get(&id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }

    fn rate_card(&self, id: RateCardId) -> String {
        self.rate_cards
            .get(&id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }
}
