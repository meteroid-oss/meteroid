use crate::StoreResult;
use crate::domain::enums::FeeTypeEnum;
use crate::domain::price_components::{FeeType, structure_json};
use crate::domain::prices::{FeeStructure, extract_fee_structure, extract_pricing};
use crate::errors::StoreError;
use crate::repositories::plan_options::version_options;
use crate::repositories::price_components::charge_structure_tx;
use crate::store::{PgConn, Store};
use common_domain::ids::{BaseId, PlanVersionId, PriceId, ProductFamilyId, ProductId, TenantId};
use diesel_models::plan_component_prices::{PlanComponentPriceRow, PlanComponentPriceRowNew};
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::plans::PlanRow;
use diesel_models::price_components::PriceComponentRow;
use diesel_models::prices::PriceRowNew;
use diesel_models::products::ProductRowNew;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use std::collections::HashSet;

const BATCH_SIZE: i64 = 100;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LegacyPlanUpgradeReport {
    pub upgraded: usize,
    /// Left on v1: a fixed fee priced on several cadences would need an option per cadence,
    /// moving the subscriptions that bought the one option they have.
    pub several_cadences: usize,
    pub failed: usize,
}

enum Outcome {
    Upgraded,
    SeveralCadences,
    AlreadyUpgraded,
}

/// A v1 component of a version: its `legacy_fee`, no linked price.
pub(crate) type LegacyComponent = (PriceComponentRow, FeeType);

/// A product a charge of the plan is sold as, latest version first.
pub(crate) struct SoldAs {
    name: String,
    product_id: ProductId,
    structure: FeeStructure,
}

impl SoldAs {
    /// The same charge: its name, and what it bills (kind, metric, counted quantity).
    fn sells(&self, name: &str, structure: &FeeStructure) -> bool {
        self.name == name
            && match (&self.structure, structure) {
                (
                    FeeStructure::Usage { metric_id: a, .. },
                    FeeStructure::Usage { metric_id: b, .. },
                )
                | (
                    FeeStructure::Capacity { metric_id: a },
                    FeeStructure::Capacity { metric_id: b },
                ) => a == b,
                (
                    FeeStructure::Slot { quantity_id: a, .. },
                    FeeStructure::Slot { quantity_id: b, .. },
                ) => a == b,
                (a, b) => a.fee_type() == b.fee_type(),
            }
    }
}

impl Store {
    /// Gives every v1 plan version real prices, as editing it does for its draft. Subscriptions
    /// are left alone: their rows keep billing the fee stored when they were made.
    pub async fn upgrade_legacy_plan_versions(&self) -> StoreResult<LegacyPlanUpgradeReport> {
        let mut report = LegacyPlanUpgradeReport::default();
        let mut after = None;
        loop {
            let batch = {
                let mut conn = self.get_conn().await?;
                PlanVersionRow::list_legacy_pricing(&mut conn, after, BATCH_SIZE)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
            };
            let Some(&(last, _)) = batch.last() else {
                break;
            };
            after = Some(last);

            for (version_id, tenant_id) in batch {
                let outcome = self
                    .transaction(|conn| {
                        async move { upgrade_version(conn, tenant_id, version_id).await }
                            .scope_boxed()
                    })
                    .await;
                match outcome {
                    Ok(Outcome::Upgraded) => report.upgraded += 1,
                    Ok(Outcome::SeveralCadences) => {
                        log::info!(
                            "Plan version {version_id} of tenant {tenant_id} left on v1: a fee priced on several terms"
                        );
                        report.several_cadences += 1;
                    }
                    Ok(Outcome::AlreadyUpgraded) => {}
                    Err(e) => {
                        log::warn!(
                            "Plan version {version_id} of tenant {tenant_id} not upgraded from v1: {e:?}"
                        );
                        report.failed += 1;
                    }
                }
            }
        }
        Ok(report)
    }
}

async fn upgrade_version(
    conn: &mut PgConn,
    tenant_id: TenantId,
    version_id: PlanVersionId,
) -> StoreResult<Outcome> {
    let version = PlanVersionRow::select_for_update_by_id_and_tenant(conn, version_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    if version.uses_product_pricing {
        return Ok(Outcome::AlreadyUpgraded);
    }
    let legacy = legacy_components(conn, tenant_id, version_id).await?;
    if legacy.iter().any(|(_, fee)| several_cadences(fee)) {
        return Ok(Outcome::SeveralCadences);
    }
    let plan = PlanRow::get_with_version(conn, version_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .plan;
    let products =
        diesel_models::query::price_components::products_by_charge_name(conn, tenant_id, plan.id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .filter_map(|(name, product_id, structure)| {
                serde_json::from_value::<FeeStructure>(structure)
                    .ok()
                    .map(|structure| SoldAs {
                        name,
                        product_id,
                        structure,
                    })
            })
            .collect::<Vec<_>>();
    upgrade_legacy_components(
        conn,
        tenant_id,
        version_id,
        &version.currency,
        plan.product_family_id,
        legacy,
        &products,
    )
    .await?;
    PlanVersionRow::mark_product_pricing(conn, version_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(Outcome::Upgraded)
}

/// Only rates and seats are priced per cadence; a usage or one-time price has none.
fn several_cadences(fee: &FeeType) -> bool {
    let (kind, _) = extract_fee_structure(fee);
    if !matches!(kind, FeeTypeEnum::Rate | FeeTypeEnum::Slot) {
        return false;
    }
    let pricing = extract_pricing(fee);
    pricing.iter().any(|(cadence, _)| *cadence != pricing[0].0)
}

pub(crate) async fn legacy_components(
    conn: &mut PgConn,
    tenant_id: TenantId,
    version_id: PlanVersionId,
) -> StoreResult<Vec<LegacyComponent>> {
    let components = PriceComponentRow::list_by_plan_version_id(conn, tenant_id, version_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let ids: Vec<_> = components.iter().map(|c| c.id).collect();
    let linked: HashSet<_> = PlanComponentPriceRow::list_by_component_ids(conn, &ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(|link| link.plan_component_id)
        .collect();

    components
        .into_iter()
        .filter(|c| !linked.contains(&c.id))
        .filter_map(|c| {
            let fee = c.legacy_fee.clone()?;
            Some(
                serde_json::from_value::<FeeType>(fee)
                    .map(|fee| (c, fee))
                    .map_err(|e| {
                        Report::new(StoreError::SerdeError(
                            "Failed to parse legacy_fee".to_string(),
                            e,
                        ))
                    }),
            )
        })
        .collect()
}

/// Each v1 component gets its fee's structure, a product (its own, else the one the same charge
/// is sold as in `products`, else a new one) and a price per pricing entry in `currency`, linked
/// in every option.
pub(crate) async fn upgrade_legacy_components(
    conn: &mut PgConn,
    tenant_id: TenantId,
    version_id: PlanVersionId,
    currency: &str,
    product_family_id: ProductFamilyId,
    legacy: Vec<LegacyComponent>,
    products: &[SoldAs],
) -> StoreResult<()> {
    if legacy.is_empty() {
        return Ok(());
    }
    let options: Vec<_> = version_options(conn, tenant_id, version_id)
        .await?
        .into_iter()
        .map(|option| option.id)
        .collect();

    let mut links = Vec::new();
    for (component, fee) in legacy {
        let (_, fee_structure) = extract_fee_structure(&fee);
        let fee_structure = charge_structure_tx(conn, tenant_id, fee_structure).await?;
        let reused = products
            .iter()
            .find(|sold| sold.sells(&component.name, &fee_structure))
            .map(|sold| sold.product_id);
        if component.product_id.is_none()
            && let Some(product_id) = reused
        {
            log::info!(
                "v1 charge {} ({}) of version {version_id} sold as product {product_id}",
                component.id,
                component.name
            );
        }
        let product_id = match component.product_id.or(reused) {
            Some(product_id) => product_id,
            None => {
                ProductRowNew {
                    id: ProductId::new(),
                    name: component.name.clone(),
                    description: None,
                    tenant_id,
                    product_family_id,
                    catalog: true,
                    tax_category_id: None,
                    icon: None,
                    color: None,
                }
                .insert(conn)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .id
            }
        };
        let mut upgraded = component.clone();
        upgraded.product_id = Some(product_id);
        upgraded.fee_structure = structure_json(Some(&fee_structure))?;
        upgraded
            .update(conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        for (cadence, pricing) in extract_pricing(&fee) {
            let price = PriceRowNew {
                id: PriceId::new(),
                product_id,
                cadence: cadence.into(),
                currency: currency.to_owned(),
                pricing: serde_json::to_value(&pricing).map_err(|e| {
                    Report::new(StoreError::SerdeError(
                        "Failed to serialize pricing".to_string(),
                        e,
                    ))
                })?,
                tenant_id,
                catalog: true,
                credit_unit_id: None,
                credit_unit_rate: None,
            }
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
            links.extend(options.iter().map(|&option_id| PlanComponentPriceRowNew {
                plan_component_id: component.id,
                price_id: price.id,
                option_id,
            }));
        }
    }

    if !links.is_empty() {
        PlanComponentPriceRowNew::insert_batch(conn, &links)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }
    Ok(())
}
