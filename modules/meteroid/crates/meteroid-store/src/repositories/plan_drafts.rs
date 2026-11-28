//! A plan's draft written whole (API v2, ADR-0013): its options by name and alias, each charge's
//! prices linked into the options they name, its minimum, add-ons and entitlements, in one
//! transaction under a lock on the plan. A plan created with its first draft is one transaction
//! too.
use crate::StoreResult;
use crate::domain::catalog_codes::validate_code;
use crate::domain::entity_activity::Actor;
use crate::domain::enums::PlanStatusEnum;
use crate::domain::plan_drafts::{DraftPriceOptions, PlanDraft, PlanDraftOption};
use crate::domain::plan_minimums::{MinimumScope, PlanMinimumNew};
use crate::domain::plan_options::valid_alias;
use crate::domain::price_components::{
    PriceComponentNew, PriceComponentNewInternal, PriceEntry, PriceInput, ProductRef,
};
use crate::domain::{FullPlan, Plan, PlanNew, PlanVersion, PlanVersionFilter, PlanVersionNew};
use crate::errors::StoreError;
use crate::repositories::PlansInterface;
use crate::repositories::entitlements::insert_entitlement_specs;
use crate::repositories::plans::set_sold_for;
use crate::repositories::price_components::{check_cell_prices, resolve_component_internal};
use crate::store::{PgConn, Store};
use common_domain::ids::{
    BaseId, EntitlementEntityId, PlanId, PlanOptionId, PlanVersionId, ProductFamilyId, ProductId,
    TenantId,
};
use common_eventbus::Event;
use diesel_models::entitlements::EntitlementRow;
use diesel_models::plan_component_prices::PlanComponentPriceRowNew;
use diesel_models::plan_version_options::PlanVersionOptionRowNew;
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::plans::{PlanRow, PlanRowPatch};
use diesel_models::price_components::PriceComponentRow;
use diesel_models::product_families::ProductFamilyRow;
use diesel_models::query::catalog_codes::set_plan_code;
use diesel_models::tenants::TenantRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use std::collections::{BTreeMap, HashMap, HashSet};

fn invalid(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

#[async_trait::async_trait]
pub trait PlanDraftsInterface {
    /// A new plan with `draft` as its first version, a draft. `code` none: its name's slug.
    async fn create_plan_with_draft(
        &self,
        actor: Actor,
        plan: PlanNew,
        code: Option<String>,
        draft: PlanDraft,
    ) -> StoreResult<FullPlan>;

    /// Writes the plan's draft as `draft` says: the draft there was is replaced (its version
    /// number kept), else a draft is made with the next number. Refused for an archived plan.
    async fn write_plan_draft(
        &self,
        actor: Actor,
        plan_id: PlanId,
        tenant_id: TenantId,
        draft: PlanDraft,
    ) -> StoreResult<FullPlan>;
}

#[async_trait::async_trait]
impl PlanDraftsInterface for Store {
    async fn create_plan_with_draft(
        &self,
        actor: Actor,
        plan: PlanNew,
        code: Option<String>,
        draft: PlanDraft,
    ) -> StoreResult<FullPlan> {
        if let Some(code) = code.as_deref() {
            validate_code(code, PlanId::PREFIX).map_err(Report::new)?;
        }
        check_options(&draft.options)?;
        let tenant_id = plan.tenant_id;
        let mut conn = self.get_conn().await?;
        let family = ProductFamilyRow::find_by_id(&mut conn, plan.product_family_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        crate::repositories::product_families::ensure_family_live(&family)?;
        let tenant = TenantRow::find_by_id(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let (plan_id, version_id) = self
            .transaction_with(&mut conn, |conn| {
                let actor = &actor;
                async move {
                    let mut plan = plan;
                    plan.status = PlanStatusEnum::Draft;
                    let sold_for = plan.sold_for_resource_type_id;
                    let inserted: Plan = plan
                        .into_raw(family.id)
                        .insert(conn)
                        .await
                        .map(Into::into)
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if let Some(code) = code.as_deref() {
                        set_plan_code(conn, tenant_id, inserted.id, code)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }
                    // Before the draft's rows, which are checked against it.
                    if sold_for.is_some() {
                        set_sold_for(conn, tenant_id, inserted.id, sold_for).await?;
                    }
                    let version = insert_draft_tx(
                        conn,
                        tenant_id,
                        inserted.id,
                        family.id,
                        1,
                        tenant.reporting_currency.clone(),
                        draft,
                    )
                    .await?;
                    let updated: Plan = draft_pointer(inserted.id, tenant_id, Some(version.id))
                        .update(conn)
                        .await
                        .map(Into::into)
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let event = crate::domain::outbox_event::PlanEvent::new(
                        updated.id,
                        version.id,
                        tenant_id,
                        updated.name.clone(),
                        updated.description.clone(),
                        updated.plan_type.clone(),
                        updated.status.clone(),
                        version.currency.clone(),
                        version.version,
                        updated.created_at,
                    );
                    self.internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            actor,
                            vec![crate::domain::outbox_event::OutboxEvent::plan_created(
                                event,
                            )],
                        )
                        .await?;
                    Ok((updated.id, version.id))
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::plan_created_draft(actor, version_id, tenant_id))
            .await;

        self.get_full_plan(plan_id, tenant_id, PlanVersionFilter::Draft)
            .await
    }

    async fn write_plan_draft(
        &self,
        actor: Actor,
        plan_id: PlanId,
        tenant_id: TenantId,
        draft: PlanDraft,
    ) -> StoreResult<FullPlan> {
        check_options(&draft.options)?;
        let mut conn = self.get_conn().await?;
        let tenant = TenantRow::find_by_id(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let version_id = self
            .transaction_with(&mut conn, |conn| {
                async move {
                    let plan = PlanRow::lock_by_id(conn, plan_id, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if plan.archived_at.is_some() {
                        return Err(Report::new(StoreError::FailedPrecondition(
                            "The plan is archived: unarchive it to draft a version".to_owned(),
                        )));
                    }
                    if let Some(previous) = plan.draft_version_id {
                        // The pointer goes first: it references the draft deleted.
                        draft_pointer(plan_id, tenant_id, None)
                            .update(conn)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        // Rows name their owner without a key: the draft's go with it.
                        EntitlementRow::delete_all_of_owner(
                            conn,
                            tenant_id,
                            EntitlementEntityId::PlanVersion(previous),
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                        PlanVersionRow::delete_draft(conn, previous, tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }
                    let number = PlanVersionRow::next_version_number(conn, plan_id, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    let version = insert_draft_tx(
                        conn,
                        tenant_id,
                        plan_id,
                        plan.product_family_id,
                        number,
                        tenant.reporting_currency.clone(),
                        draft,
                    )
                    .await?;
                    draft_pointer(plan_id, tenant_id, Some(version.id))
                        .update(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    Ok(version.id)
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::plan_created_draft(actor, version_id, tenant_id))
            .await;

        self.get_full_plan(plan_id, tenant_id, PlanVersionFilter::Draft)
            .await
    }
}

fn draft_pointer(
    plan_id: PlanId,
    tenant_id: TenantId,
    draft: Option<PlanVersionId>,
) -> PlanRowPatch {
    PlanRowPatch {
        id: plan_id,
        tenant_id,
        name: None,
        description: None,
        active_version_id: None,
        draft_version_id: Some(draft),
        self_service_rank: None,
        issues_license: None,
    }
}

/// At least one option, each named, the aliases valid and distinct, the default (first) shown.
fn check_options(options: &[PlanDraftOption]) -> StoreResult<()> {
    let Some(first) = options.first() else {
        return Err(invalid("A version has one option at least"));
    };
    if first.hidden {
        return Err(invalid(format!(
            "The default option is always shown: {} is first",
            first.alias
        )));
    }
    let mut seen = HashSet::new();
    for option in options {
        if option.name.trim().is_empty() {
            return Err(invalid("An option needs a name"));
        }
        if !valid_alias(&option.alias) {
            return Err(invalid(format!(
                "'{}' is not an alias: lowercase letters, digits, - and _, up to 40 characters",
                option.alias
            )));
        }
        if !seen.insert(option.alias.as_str()) {
            return Err(invalid(format!(
                "Two options of the version are '{}'",
                option.alias
            )));
        }
    }
    Ok(())
}

/// Refuses a charge naming a product, metric, rate card or credit unit of another tenant: each
/// reads as not found. Managed quantities are checked where seat structures are resolved
/// (`charge_structure_tx`).
async fn check_charge_refs_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    charges: &[crate::domain::plan_drafts::DraftCharge],
) -> StoreResult<()> {
    use crate::domain::price_components::UsagePricingModel;
    use crate::domain::prices::{FeeStructure, Pricing};
    use common_domain::ids::{BillableMetricId, CreditUnitId, RateCardId};

    let mut products: HashSet<ProductId> = HashSet::new();
    let mut metrics: HashSet<BillableMetricId> = HashSet::new();
    let mut rate_cards: HashSet<RateCardId> = HashSet::new();
    let mut credit_units: HashSet<CreditUnitId> = HashSet::new();
    for charge in charges {
        products.insert(charge.product_id);
        if let Some(FeeStructure::Usage { metric_id, .. } | FeeStructure::Capacity { metric_id }) =
            &charge.fee_structure
        {
            metrics.insert(*metric_id);
        }
        for price in &charge.prices {
            if let Some(unit) = &price.price.credit_unit {
                credit_units.insert(unit.unit_id);
            }
            match &price.price.pricing {
                Pricing::Usage(UsagePricingModel::RateCard { rate_card_id, .. }) => {
                    rate_cards.insert(*rate_card_id);
                }
                Pricing::Credit {
                    credit_unit_id: Some(unit),
                    ..
                } => {
                    credit_units.insert(*unit);
                }
                _ => {}
            }
        }
    }
    let missing =
        |what: &str, id: String| Report::new(StoreError::ValueNotFound(format!("{what} {id}")));
    for id in products {
        diesel_models::products::ProductRow::find_by_id_and_tenant_id(conn, id, tenant_id)
            .await
            .map_err(|_| missing("product", id.to_string()))?;
    }
    for id in metrics {
        diesel_models::billable_metrics::BillableMetricRow::find_by_id(conn, id, tenant_id)
            .await
            .map_err(|_| missing("metric", id.to_string()))?;
    }
    for id in rate_cards {
        let card = diesel_models::rate_cards::RateCardRow::find(conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| missing("rate card", id.to_string()))?;
        if card.purpose == diesel_models::enums::RateCardPurposeEnum::Cost {
            return Err(Report::new(
                crate::domain::rate_cards::cost_card_never_prices(&card.code),
            ));
        }
    }
    for id in credit_units {
        diesel_models::credit_units::CreditUnitRow::find_by_id(conn, tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| missing("credit unit", id.to_string()))?;
    }
    Ok(())
}

/// Inserts the draft version `number` of the plan with its options, charges, prices, minimum,
/// add-ons and entitlements.
async fn insert_draft_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_id: PlanId,
    product_family_id: ProductFamilyId,
    number: i32,
    tenant_currency: String,
    draft: PlanDraft,
) -> StoreResult<PlanVersion> {
    let PlanDraft {
        version: mut terms,
        options,
        charges,
        add_ons,
        minimum,
    } = draft;
    terms.is_draft_version = true;
    let entitlements = std::mem::take(&mut terms.entitlements);
    // Every id the draft names is the tenant's: a reference to another tenant's row reads as
    // not found, and nothing is written.
    for add_on in &add_ons {
        diesel_models::add_ons::AddOnRow::get_by_id(conn, tenant_id, add_on.add_on_id)
            .await
            .map_err(|_| {
                Report::new(StoreError::ValueNotFound(format!(
                    "add-on {}",
                    add_on.add_on_id
                )))
            })?;
    }
    check_charge_refs_tx(conn, tenant_id, &charges).await?;
    let version: PlanVersion = PlanVersionNew {
        plan_id,
        version: number,
        tenant_id,
        internal: terms,
    }
    .into_raw(tenant_currency)
    .insert(conn)
    .await
    .map(Into::into)
    .map_err(Into::<Report<StoreError>>::into)?;

    let option_rows: Vec<PlanVersionOptionRowNew> = options
        .iter()
        .enumerate()
        .map(|(position, option)| PlanVersionOptionRowNew {
            id: PlanOptionId::new(),
            tenant_id,
            plan_version_id: version.id,
            name: option.name.trim().to_owned(),
            alias: option.alias.clone(),
            position: position as i32,
            hidden: option.hidden,
        })
        .collect();
    PlanVersionOptionRowNew::insert_batch(conn, &option_rows)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let every: Vec<PlanOptionId> = option_rows.iter().map(|row| row.id).collect();
    let by_alias: HashMap<&str, PlanOptionId> = option_rows
        .iter()
        .map(|row| (row.alias.as_str(), row.id))
        .collect();
    let targets = |options: &DraftPriceOptions| -> StoreResult<Vec<PlanOptionId>> {
        match options {
            DraftPriceOptions::Every => Ok(every.clone()),
            DraftPriceOptions::Only(aliases) if aliases.is_empty() => {
                Err(invalid("A price is in one option at least"))
            }
            DraftPriceOptions::Only(aliases) => aliases
                .iter()
                .map(|alias| {
                    by_alias.get(alias.as_str()).copied().ok_or_else(|| {
                        invalid(format!("'{alias}' is not an option of the version"))
                    })
                })
                .collect(),
        }
    };

    let mut names = HashSet::new();
    let mut charged_products: Vec<ProductId> = Vec::new();
    let mut links: Vec<PlanComponentPriceRowNew> = Vec::new();
    for charge in &charges {
        if !names.insert(charge.name.as_str()) {
            return Err(invalid(format!(
                "Two charges of the version are named '{}': name them apart",
                charge.name
            )));
        }
        let charge_targets: Vec<Vec<PlanOptionId>> = charge
            .prices
            .iter()
            .map(|price| targets(&price.options))
            .collect::<StoreResult<_>>()?;

        // Each cell (an option, a currency) holds one price, or a capacity charge's levels.
        let mut cells: BTreeMap<(uuid::Uuid, String), Vec<PriceInput>> = BTreeMap::new();
        for (price, options) in charge.prices.iter().zip(&charge_targets) {
            let currency = price.price.currency.trim().to_ascii_uppercase();
            for option in options {
                cells
                    .entry((option.as_uuid(), currency.clone()))
                    .or_default()
                    .push(price.price.clone());
            }
        }
        for ((_, currency), prices) in &cells {
            if crate::constants::Currencies::resolve_currency(currency).is_none() {
                return Err(invalid(format!(
                    "{}: '{currency}' is not a supported currency",
                    charge.name
                )));
            }
            check_cell_prices(prices, currency).map_err(|error| {
                let message = match error.current_context() {
                    StoreError::InvalidArgument(message) => message.clone(),
                    other => other.to_string(),
                };
                error.change_context(StoreError::InvalidArgument(format!(
                    "{}: {message}",
                    charge.name
                )))
            })?;
        }

        // Prices are made per currency: a charge resolves its new prices in one currency.
        let mut by_currency: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, price) in charge.prices.iter().enumerate() {
            by_currency
                .entry(price.price.currency.clone())
                .or_default()
                .push(index);
        }
        let mut price_ids = vec![None; charge.prices.len()];
        let mut structure = None;
        if charge.prices.is_empty() {
            let resolved = resolve_component_internal(
                conn,
                &PriceComponentNewInternal {
                    name: charge.name.clone(),
                    product_ref: ProductRef::Existing(charge.product_id),
                    fee_structure: charge.fee_structure.clone(),
                    prices: Vec::new(),
                },
                tenant_id,
                product_family_id,
                &version.currency,
                true,
            )
            .await?;
            structure = resolved.fee_structure;
        }
        for (currency, indexes) in &by_currency {
            let resolved = resolve_component_internal(
                conn,
                &PriceComponentNewInternal {
                    name: charge.name.clone(),
                    product_ref: ProductRef::Existing(charge.product_id),
                    fee_structure: charge.fee_structure.clone(),
                    prices: indexes
                        .iter()
                        .map(|index| PriceEntry::New(charge.prices[*index].price.clone()))
                        .collect(),
                },
                tenant_id,
                product_family_id,
                currency,
                true,
            )
            .await?;
            structure = resolved.fee_structure;
            for (index, price_id) in indexes.iter().zip(resolved.price_ids) {
                price_ids[*index] = Some(price_id);
            }
        }

        let row: diesel_models::price_components::PriceComponentRowNew = PriceComponentNew {
            name: charge.name.clone(),
            product_id: Some(charge.product_id),
            plan_version_id: version.id,
            fee_structure: structure,
        }
        .try_into()?;
        let component = PriceComponentRow::insert(conn, row)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        charged_products.push(charge.product_id);
        for (price_id, options) in price_ids.into_iter().zip(charge_targets) {
            let price_id = price_id.ok_or_else(|| {
                Report::new(StoreError::InvalidPriceComponents(format!(
                    "{}: a price was not resolved",
                    charge.name
                )))
            })?;
            links.extend(
                options
                    .into_iter()
                    .map(|option_id| PlanComponentPriceRowNew {
                        plan_component_id: component.id,
                        price_id,
                        option_id,
                    }),
            );
        }
    }
    PlanComponentPriceRowNew::insert_batch(conn, &links)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    if let Some(minimum) = minimum {
        if minimum.amount_subunit <= 0 {
            return Err(invalid("A minimum commitment is a positive amount"));
        }
        if let MinimumScope::Products(products) = &minimum.scope {
            if products.is_empty() {
                return Err(invalid("A minimum scoped to products names one at least"));
            }
            if let Some(stray) = products.iter().find(|p| !charged_products.contains(p)) {
                return Err(invalid(format!(
                    "The minimum counts product {stray}, which no charge of the version bills"
                )));
            }
        }
        let new = PlanMinimumNew {
            plan_version_id: version.id,
            amount_subunit: minimum.amount_subunit,
            scope: minimum.scope,
        };
        new.to_row()
            .upsert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let rows = new.component_rows();
        if !rows.is_empty() {
            use diesel_models::plan_version_adjustments::PlanVersionAdjustmentComponentRow;
            PlanVersionAdjustmentComponentRow::insert_batch(conn, &rows)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        }
    }

    let mut attached = HashSet::new();
    for add_on in add_ons {
        if !attached.insert(add_on.add_on_id) {
            return Err(invalid(format!(
                "Add-on {} is attached twice",
                add_on.add_on_id
            )));
        }
        let row: diesel_models::plan_version_add_ons::PlanVersionAddOnRowNew =
            crate::domain::plan_version_add_ons::PlanVersionAddOnNew {
                plan_version_id: version.id,
                add_on_id: add_on.add_on_id,
                price_id: None,
                self_serviceable: add_on.self_serviceable,
                max_instances_per_subscription: add_on.max_instances_per_subscription,
                tenant_id,
            }
            .into();
        row.insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }

    if !entitlements.is_empty() {
        insert_entitlement_specs(
            conn,
            entitlements,
            EntitlementEntityId::PlanVersion(version.id),
            tenant_id,
        )
        .await?;
    }

    Ok(version)
}
