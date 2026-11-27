use crate::StoreResult;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::plan_options::PlanOption;
use crate::domain::price_cells::{resolve_option, sold_options};
use crate::domain::price_components::resolve_legacy_subscription_fee;
use crate::domain::prices::{
    LegacyPricingData, Price, Pricing, extract_legacy_pricing, fee_type_billing_period,
    resolve_subscription_fee,
};
use crate::domain::scheduled_events::{
    ComponentMapping, ScheduledEvent, ScheduledEventData, ScheduledEventNew,
};
use crate::domain::subscription_changes::{
    AddedComponent, ChangeDirection, ImmediatePlanChangeResult, MatchedComponent, PlanChangeMode,
    PlanChangeOutcome, PlanChangePreview, PlanChangePreviewExtended, PlanChangePreviewFull,
    PreviewPart, ProrationSummary, RemovedComponent,
};
use crate::domain::subscription_changes::{PlanTarget, TargetPrice};
use crate::domain::subscription_components::{
    ComponentParameterization, ComponentParameters, SubscriptionComponent,
    SubscriptionComponentNew, SubscriptionComponentNewInternal, SubscriptionFee,
};
use crate::domain::subscription_phases::RewriteEntry;
use crate::domain::{SubscriptionAddOn, SubscriptionStatusEnum};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::repositories::price_components::PriceComponentInterface;
use crate::services::Services;
use crate::services::evaluate::change::{Change, GiveBack, change_documents};
use crate::services::subscriptions::currency::{plan_options_of, record_plan_option};
use crate::services::subscriptions::proration::{DayFraction, OwnGrid, detect_change_direction};
use crate::services::subscriptions::timeline_rewrite;
use crate::store::PgConn;
use crate::utils::periods::calculate_advance_period_range;
use chrono::{Datelike, NaiveDate, NaiveTime};
use common_domain::ids::{
    AddOnId, BaseId, CreditNoteId, PlanVersionId, PriceComponentId, ProductId, SubscriptionId,
    SubscriptionPriceComponentId, TenantId,
};
use diesel_models::plan_component_prices::PlanComponentPriceRow;
use diesel_models::plan_version_add_ons::PlanVersionAddOnRow;
use diesel_models::plans::PlanRow;
use diesel_models::price_components::PriceComponentRow;
use diesel_models::prices::PriceRow;
use diesel_models::query::credit_inputs;
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_changes::SubscriptionChangeRow;
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use diesel_models::subscription_facts::SubscriptionEndingRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone)]
enum TargetPricing {
    V2 {
        product_id: ProductId,
        prices: Vec<Price>,
        /// How the charge bills its product.
        fee_structure: crate::domain::prices::FeeStructure,
    },
    Legacy(LegacyPricingData),
}

struct TargetComponent {
    id: PriceComponentId,
    name: String,
    pricing: TargetPricing,
}

impl TargetComponent {
    fn product_id(&self) -> Option<ProductId> {
        match &self.pricing {
            TargetPricing::V2 { product_id, .. } => Some(*product_id),
            TargetPricing::Legacy(_) => None,
        }
    }

    /// The current component the target continues: the same product billed the same way, its
    /// metric and its quantity included (`FeeStructure::same_shape`). A product charged another
    /// way by the target (per seat where it was a flat fee, usage of another metric) is another
    /// charge: the old one is removed, the new one added.
    fn matching<'a>(
        &self,
        current: &'a [SubscriptionComponent],
    ) -> Option<&'a SubscriptionComponent> {
        let TargetPricing::V2 {
            product_id,
            fee_structure,
            ..
        } = &self.pricing
        else {
            return None;
        };
        current.iter().find(|c| {
            c.product_id == Some(*product_id)
                && crate::domain::prices::FeeStructure::of_fee(&c.fee).same_shape(fee_structure)
        })
    }
}

struct ValidatedPlanChangeContext {
    subscription_details: crate::domain::SubscriptionDetails,
    target_components: Vec<TargetComponent>,
    units: crate::domain::credit_units::CreditUnitsById,
    target_minimum: Option<crate::domain::plan_minimums::PlanMinimum>,
    compatible_add_on_ids: Vec<AddOnId>,
    /// The option of the target version moved to.
    target_option: Option<PlanOption>,
    /// The prices the change sets on the target's charges (R-163).
    prices: Vec<TargetPrice>,
}

/// A minimum commitment is a term of the period: it changes at the period's end, never within.
const MINIMUM_COMMITMENT_AT_PERIOD_END: &str = "This subscription or the target plan has a minimum commitment; plan changes take effect at the end of the period.";

impl ValidatedPlanChangeContext {
    fn immediate_allowed(&self) -> bool {
        self.subscription_details.minimum.is_none() && self.target_minimum.is_none()
    }
}

async fn load_validated_plan_change_context(
    conn: &mut PgConn,
    store: &crate::Store,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    target: PlanTarget,
) -> StoreResult<ValidatedPlanChangeContext> {
    load_plan_change_context(conn, store, tenant_id, subscription_id, target, None).await
}

/// The plan version and fees a switch written ahead starts from: those in force on its day,
/// earlier changes written ahead included.
pub(crate) struct SwitchFrom {
    pub plan_version_id: PlanVersionId,
    pub plan_option_id: Option<common_domain::ids::PlanOptionId>,
    pub components: Vec<SubscriptionComponent>,
}

async fn load_plan_change_context(
    conn: &mut PgConn,
    store: &crate::Store,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    target: PlanTarget,
    from: Option<SwitchFrom>,
) -> StoreResult<ValidatedPlanChangeContext> {
    let new_plan_version_id = target.version;
    let prices = target.prices.clone();
    let mut subscription_details = store
        .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
        .await?;

    validate_subscription_for_plan_change(&subscription_details.subscription.status)?;

    let in_force = match from {
        Some(from) => {
            subscription_details.price_components = from.components;
            subscription_details.subscription.plan_option_id = from.plan_option_id;
            subscription_details.plan_option = match from.plan_option_id {
                Some(option) => Some(
                    diesel_models::plan_version_options::PlanVersionOptionRow::find_by_id(
                        conn, tenant_id, option,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .into(),
                ),
                None => None,
            };
            from.plan_version_id
        }
        None => subscription_details.subscription.plan_version_id,
    };
    let sub_currency = &subscription_details.subscription.currency;
    let target_option = target_option(
        conn,
        store,
        tenant_id,
        &target,
        subscription_details.plan_option.as_ref(),
        sub_currency,
        fixed_fee_cadence(
            &subscription_details.price_components,
            subscription_details.subscription.period,
        ),
    )
    .await?;
    // On its own plan version, a change moves to another option, asked.
    if in_force == new_plan_version_id
        && (target.option.is_none()
            || target_option.as_ref().map(|option| option.id)
                == subscription_details.subscription.plan_option_id)
    {
        return Err(Report::new(StoreError::InvalidArgument(
            "Cannot change to the current plan version".to_string(),
        )));
    }

    let target_plan = PlanRow::get_with_version(conn, new_plan_version_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let target_version = target_plan.version.ok_or_else(|| {
        Report::new(StoreError::ValueNotFound(
            "Target plan version not found".to_string(),
        ))
    })?;

    if target_version.is_draft_version {
        return Err(Report::new(StoreError::InvalidArgument(
            "Cannot switch to a draft plan version".to_string(),
        )));
    }

    // A subscription reads all of its amounts one way, and a change prorates the old plan
    // against the new one: a credit quoted tax-included cannot be netted against a charge
    // quoted net. Like currency, the basis is fixed for the subscription's life.
    if target_version.tax_inclusive != subscription_details.subscription.tax_inclusive {
        let describe = |inclusive: bool| {
            if inclusive {
                "tax-inclusive"
            } else {
                "tax-exclusive"
            }
        };
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "Tax basis mismatch: subscription prices are {} but target plan prices are {}",
            describe(subscription_details.subscription.tax_inclusive),
            describe(target_version.tax_inclusive)
        ))));
    }

    // A change keeps the subscription's currency, its charges priced in the option's cells.
    let target_components = load_target_components_with_prices(
        conn,
        tenant_id,
        new_plan_version_id,
        target_version.currency.clone(),
        sub_currency,
        target_option.as_ref(),
    )
    .await?;
    refuse_legacy_in_another_currency(&target_components, &target_version.currency, sub_currency)?;

    let mut target_minimum =
        crate::repositories::plans::load_plan_minimum(conn, new_plan_version_id).await?;
    // A plan minimum is in its version's currency: it never lands on a subscription billed in
    // another, as a creation would refuse it. An overridden minimum stays as it is.
    let overridden = subscription_details
        .minimum
        .as_ref()
        .is_some_and(|m| m.is_override);
    if target_minimum.is_some()
        && !overridden
        && !target_version.currency.eq_ignore_ascii_case(sub_currency)
    {
        // On its own version the subscription was created without it: it stays so.
        if new_plan_version_id == subscription_details.subscription.plan_version_id {
            target_minimum = None;
        } else {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "The target plan's minimum commitment is in {}, the subscription is billed in {sub_currency}: override its minimum first",
                target_version.currency
            ))));
        }
    }

    let units = crate::repositories::credit_units::load_credit_units(
        conn,
        tenant_id,
        target_components
            .iter()
            .flat_map(|target| match &target.pricing {
                TargetPricing::V2 { prices, .. } => prices.as_slice(),
                TargetPricing::Legacy(_) => &[],
            })
            .flat_map(Price::credit_unit_ids),
    )
    .await?;

    let compatible_add_on_ids =
        PlanVersionAddOnRow::list_by_plan_version_id(conn, new_plan_version_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|pva| pva.add_on_id)
            .collect();

    if let Some(unknown) = prices.iter().find(|price| {
        !target_components
            .iter()
            .any(|target| target.id == price.component_id)
    }) {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "The plan changed to has no component {}",
            unknown.component_id
        ))));
    }
    for price in &prices {
        crate::services::subscriptions::phases::validate_price(price.price)?;
    }

    Ok(ValidatedPlanChangeContext {
        subscription_details,
        target_components,
        units,
        target_minimum,
        compatible_add_on_ids,
        target_option,
        prices,
    })
}

/// The cadence the subscription's fixed fees bill on, the longest of them when several: its
/// term, which an option sells. Its `period` is the shortest of all its fees, usage included (an
/// annual platform fee beside monthly usage bills on a monthly period): read only without a fixed
/// fee.
pub(crate) fn fixed_fee_cadence(
    components: &[SubscriptionComponent],
    period: crate::domain::enums::BillingPeriodEnum,
) -> crate::domain::enums::BillingPeriodEnum {
    components
        .iter()
        .filter(|component| {
            !matches!(
                component.fee,
                SubscriptionFee::Usage { .. }
                    | SubscriptionFee::OneTime { .. }
                    | SubscriptionFee::Credit { .. }
            )
        })
        .filter_map(|component| component.period.as_billing_period_opt())
        .max()
        .unwrap_or(period)
}

/// The option of `version` a change naming none moves the subscription of `details` to, as
/// [`target_option`] picks it: a change written before options were recorded on it.
pub(crate) async fn unnamed_target_option(
    conn: &mut PgConn,
    store: &crate::Store,
    details: &crate::domain::SubscriptionDetails,
    version: PlanVersionId,
) -> StoreResult<Option<PlanOption>> {
    let sub = &details.subscription;
    target_option(
        conn,
        store,
        sub.tenant_id,
        &PlanTarget::from(version),
        details.plan_option.as_ref(),
        &sub.currency,
        fixed_fee_cadence(&details.price_components, sub.period),
    )
    .await
}

/// The option a change moves to: the one asked, else the subscription's by alias when the target
/// version has it, else the one billing its fixed fees on `period` (see [`fixed_fee_cadence`]),
/// else the first sold in the subscription's currency. None for a version without options.
async fn target_option(
    conn: &mut PgConn,
    store: &crate::Store,
    tenant_id: TenantId,
    target: &PlanTarget,
    current: Option<&PlanOption>,
    currency: &str,
    period: crate::domain::enums::BillingPeriodEnum,
) -> StoreResult<Option<PlanOption>> {
    let options = plan_options_of(conn, tenant_id, &[target.version])
        .await?
        .remove(&target.version)
        .unwrap_or_default();
    if options.is_empty() {
        return Ok(None);
    }
    if let Some(asked) = target.option.as_ref() {
        return resolve_option(&options, Some(asked)).map(|option| Some(option.clone()));
    }
    if let Some(kept) = current.and_then(|current| {
        options
            .iter()
            .find(|option| option.alias.eq_ignore_ascii_case(&current.alias))
    }) {
        return Ok(Some(kept.clone()));
    }
    let components = store
        .list_price_components_with_conn(conn, target.version, tenant_id)
        .await?;
    let sold = sold_options(&options, &components, currency);
    // No option of the same alias: the one billing a fixed fee on the cadence the subscription's
    // fixed fees bill on, as a change kept it before options; else the first sold.
    let on_period = sold.iter().find(|option| {
        let cell = crate::domain::price_cells::OptionPricing::new(option.id, currency);
        components.iter().any(|component| {
            cell.prices(component).iter().any(|price| {
                !crate::domain::price_cells::cadence_free(&price.pricing) && price.cadence == period
            })
        })
    });
    Ok(on_period
        .or(sold.first())
        .map(|option| (*option).clone())
        .or_else(|| options.first().cloned()))
}

impl Services {
    pub(in crate::services) async fn schedule_plan_change(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: impl Into<PlanTarget>,
        component_params: Vec<ComponentParameterization>,
    ) -> StoreResult<ScheduledEvent> {
        self.schedule_plan_change_reporting(
            actor,
            subscription_id,
            tenant_id,
            target,
            component_params,
        )
        .await
        .map(|(event, _)| event)
    }

    /// [`Self::schedule_plan_change`], with what it did to the subscription's timeline (R-162).
    pub(in crate::services) async fn schedule_plan_change_reporting(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: impl Into<PlanTarget>,
        component_params: Vec<ComponentParameterization>,
    ) -> StoreResult<(ScheduledEvent, Vec<RewriteEntry>)> {
        let target: PlanTarget = target.into();
        self.store
            .transaction(|conn| {
                let actor = &actor;
                let component_params = &component_params;
                let target = target.clone();
                async move {
                    self.schedule_plan_change_tx(
                        conn,
                        actor,
                        subscription_id,
                        tenant_id,
                        target,
                        component_params,
                    )
                    .await
                }
                .scope_boxed()
            })
            .await
    }

    /// [`Self::schedule_plan_change_reporting`] inside the caller's transaction.
    pub(in crate::services) async fn schedule_plan_change_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: PlanTarget,
        component_params: &[ComponentParameterization],
    ) -> StoreResult<(ScheduledEvent, Vec<RewriteEntry>)> {
        let new_plan_version_id = target.version;
        let prices = target.prices.clone();
        let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id).await?;
        credit_inputs::bump_customer(conn, &lock).await?;
        refuse_while_a_paid_change_is_pending(conn, tenant_id, subscription_id, None).await?;

        let ctx = load_validated_plan_change_context(
            conn,
            &self.store,
            tenant_id,
            subscription_id,
            target,
        )
        .await?;
        let customer_id = ctx.subscription_details.subscription.customer_id;

        let component_mappings = build_component_mappings(
            &ctx.subscription_details.price_components,
            &ctx.target_components,
            &ctx.units,
            component_params,
            &ctx.prices,
        )?;

        let effective_date = ctx
            .subscription_details
            .subscription
            .current_period_end
            .ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(
                    "Subscription has no current_period_end".to_string(),
                ))
            })?;

        refuse_after_a_cancellation(conn, tenant_id, subscription_id, effective_date).await?;

        // The timeline from its day is written again over it (R-162); a phase
        // switching plan that day takes this plan instead.
        let mut lifted =
            timeline_rewrite::lift_tx(conn, tenant_id, subscription_id, effective_date).await?;
        crate::services::subscriptions::phases::withdraw_one_offs_tx(
            conn,
            tenant_id,
            subscription_id,
            &[diesel_models::enums::SubscriptionChangeKindEnum::PlanChange],
            "Replaced by new plan change",
        )
        .await?;
        let replaced = lifted.replace_switch_on(
            effective_date,
            crate::domain::subscription_phases::PhasePlan {
                plan_version_id: new_plan_version_id,
                option: ctx.target_option.as_ref().map(|o| o.alias.clone()),
                component_params: component_params.to_vec(),
                name: None,
            },
            &prices,
        );
        let event = match replaced {
            Some(_) => None,
            // Written as facts now, the change's rows dated from the period's end.
            None => Some(
                crate::services::facts::pending::schedule_as_facts_tx(
                    &self.store,
                    conn,
                    ScheduledEventNew {
                        subscription_id,
                        tenant_id,
                        scheduled_time: effective_date.and_time(NaiveTime::MIN),
                        event_data: ScheduledEventData::ApplyPlanChange {
                            source_plan_version_id: Some(
                                ctx.subscription_details.subscription.plan_version_id,
                            ),
                            new_plan_version_id,
                            new_plan_option_id: ctx.target_option.as_ref().map(|o| o.id),
                            component_mappings,
                            addon_close: Vec::new(),
                            addon_insert: Vec::new(),
                        },
                        source: "api".to_string(),
                        created_by: actor.clone(),
                    },
                )
                .await?,
            ),
        };
        let mut timeline = timeline_rewrite::replay_tx(self, conn, actor, lifted).await?;
        let event = match (event, replaced) {
            (Some(event), _) => event,
            (None, Some(phase)) => {
                timeline_rewrite::mark_switch_replaced(&mut timeline, phase);
                timeline_rewrite::phase_change_tx(conn, tenant_id, subscription_id, phase).await?
            }
            (None, None) => {
                return Err(Report::new(StoreError::InvalidArgument(
                    "the plan change was not written".to_string(),
                )));
            }
        };
        // A period's end already past (the controller behind): dated as a late change.
        if effective_date <= common_clock::now().date_naive() {
            crate::repositories::subscriptions::usage_attribution::settle_late_change(
                conn,
                tenant_id,
                subscription_id,
                effective_date,
                "The plan change",
            )
            .await?;
        }
        self.derive_state_tx(conn, actor, tenant_id, subscription_id, common_clock::now())
            .await?;

        let activity = Activity::new(
            ActivityType::SubscriptionPlanChangeScheduled,
            EntityType::Subscription,
            subscription_id.as_uuid(),
        )
        .with_metadata(serde_json::json!({
            "new_plan_version_id": new_plan_version_id.to_string(),
            "effective_at": event.scheduled_time.to_string(),
        }))
        .agg_customer(customer_id);
        self.store
            .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
            .await?;

        Ok((event, timeline))
    }

    pub(in crate::services) async fn preview_plan_change(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: impl Into<PlanTarget>,
        component_params: Vec<ComponentParameterization>,
        mode: Option<PlanChangeMode>,
    ) -> StoreResult<PlanChangePreviewExtended> {
        Ok(self
            .preview_plan_change_with(
                subscription_id,
                tenant_id,
                target.into(),
                component_params,
                mode,
                false,
            )
            .await?
            .preview)
    }

    /// [`Self::preview_plan_change`], with what the change issues (its documents, with their
    /// lines), the renewal under it and the subscription it leaves.
    pub(in crate::services) async fn preview_plan_change_in_full(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: impl Into<PlanTarget>,
        component_params: Vec<ComponentParameterization>,
        mode: Option<PlanChangeMode>,
    ) -> StoreResult<PlanChangePreviewFull> {
        self.preview_plan_change_with(
            subscription_id,
            tenant_id,
            target.into(),
            component_params,
            mode,
            true,
        )
        .await
    }

    async fn preview_plan_change_with(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: PlanTarget,
        component_params: Vec<ComponentParameterization>,
        mode: Option<PlanChangeMode>,
        in_full: bool,
    ) -> StoreResult<PlanChangePreviewFull> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let new_plan_version_id = target.version;
            let mut conn = self.store.get_conn().await?;

            let ctx = load_validated_plan_change_context(
                &mut conn,
                &self.store,
                tenant_id,
                subscription_id,
                target.clone(),
            )
            .await?;

            let is_immediate = matches!(mode, Some(PlanChangeMode::Immediate));
            let immediate_allowed = ctx.immediate_allowed();

            let effective_date = if is_immediate {
                common_clock::now().naive_utc().date()
            } else {
                ctx.subscription_details
                    .subscription
                    .current_period_end
                    .unwrap_or(ctx.subscription_details.subscription.current_period_start)
            };

            if is_immediate {
                drop(ctx);
                let prepared = self
                    .prepare_plan_change_inner(
                        &mut conn,
                        subscription_id,
                        tenant_id,
                        target,
                        &component_params,
                        effective_date,
                    )
                    .await?;
                prepared.ensure_immediate_allowed()?;
                let mut outcome = PlanChangeOutcome::default();
                // What the change bills and gives back: its rows laid over the facts at its date; a
                // change ending a free trial bills the new plan's first invoice.
                let (charged, credited, incoming) = if prepared.is_free_trial() {
                    let first = self
                        .plan_change_quote(&mut conn, &prepared, new_plan_version_id, effective_date)
                        .await?;
                    let charged = (
                        first.subtotal,
                        0,
                        pack_charges(&first.invoice_lines, prepared.precision),
                    );
                    if in_full {
                        outcome.invoice = Some(first);
                    }
                    charged
                } else {
                    let overlay = self
                        .plan_change_overlay(&mut conn, &prepared, new_plan_version_id, effective_date)
                        .await?;
                    let documents = self
                        .preview_change_documents(
                            &mut conn,
                            &prepared.subscription_details,
                            &overlay,
                            Change::of_rows(effective_date, GiveBack::Unused, "plan change"),
                            Vec::new(),
                        )
                        .await?;
                    let incoming = documents
                        .adjustment
                        .as_ref()
                        .map_or_else(Vec::new, |invoice| {
                            pack_charges(&invoice.invoice_lines, prepared.precision)
                        });
                    if in_full {
                        outcome.invoice = documents.adjustment.clone();
                        outcome.credit_note = documents.credit_note.clone();
                    }
                    (documents.charged, documents.credited, incoming)
                };
                if in_full {
                    self.lay_plan_change_after(
                        &mut conn,
                        &prepared,
                        new_plan_version_id,
                        effective_date,
                        &mut outcome,
                    )
                    .await;
                }
                drop(conn);
                let outgoing = if prepared.is_free_trial() {
                    Default::default()
                } else {
                    self.estimate_outgoing_packs(&prepared, effective_date)
                        .await?
                };
                let proration = prepared.proration_summary(charged, credited, incoming, &outgoing);
                return Ok(PlanChangePreviewFull {
                    preview: PlanChangePreviewExtended {
                        preview: prepared.preview,
                        proration: Some(proration),
                        change_direction: prepared.direction,
                        immediate_allowed,
                    },
                    outcome,
                });
            }

            let mut preview = build_plan_change_preview(
                &ctx.subscription_details.price_components,
                &ctx.target_components,
                &ctx.units,
                &component_params,
                &ctx.prices,
                effective_date,
                &HashMap::new(),
            )?;

            resolve_preview_slot_counts(&mut conn, tenant_id, subscription_id, &mut preview).await?;

            let precision = crate::constants::Currencies::resolve_currency_precision(
                &ctx.subscription_details.subscription.currency,
            )
            .unwrap_or(2);

            let change_direction = detect_change_direction(
                &preview.matched,
                &preview.added,
                &preview.removed,
                precision,
            );

            // At the period's end the change issues nothing of its own: its renewal bills it.
            let mut outcome = PlanChangeOutcome::default();
            if in_full {
                drop(ctx);
                match self
                    .prepare_plan_change_inner(
                        &mut conn,
                        subscription_id,
                        tenant_id,
                        target,
                        &component_params,
                        effective_date,
                    )
                    .await
                {
                    Ok(prepared) => {
                        self.lay_plan_change_after(
                            &mut conn,
                            &prepared,
                            new_plan_version_id,
                            effective_date,
                            &mut outcome,
                        )
                        .await;
                    }
                    Err(error) => {
                        log::warn!(
                            "The plan change of subscription {subscription_id} at its period's end was not previewed in full: {error:?}"
                        );
                        outcome.missing = vec![PreviewPart::NextInvoice, PreviewPart::SubscriptionAfter];
                    }
                }
            }

            Ok(PlanChangePreviewFull {
                preview: PlanChangePreviewExtended {
                    preview,
                    proration: None,
                    change_direction,
                    immediate_allowed,
                },
                outcome,
            })
        })
        .await
    }

    /// The subscription `prepared` leaves from `on`, the renewal under it and its MRR, laid on
    /// `outcome`; best-effort: a preview never fails on them.
    async fn lay_plan_change_after(
        &self,
        conn: &mut PgConn,
        prepared: &PreparedPlanChange,
        new_plan_version_id: PlanVersionId,
        on: NaiveDate,
        outcome: &mut PlanChangeOutcome,
    ) {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        let laid: StoreResult<()> = Box::pin(async {
            let overlay = self
                .plan_change_overlay(conn, prepared, new_plan_version_id, on)
                .await?;
            let before = &prepared.subscription_details;
            let mut after = crate::services::evaluate::preview::details_after(before, &overlay)?;
            after.add_ons.retain(|a| {
                !prepared
                    .ending_add_ons
                    .iter()
                    .any(|ending| ending.id == a.id)
            });
            after.minimum = prepared
                .hypothetical_details(new_plan_version_id, before, on)
                .minimum;
            after.plan_option = prepared.target_option.clone();
            match self
                .preview_next_invoice(conn, &after, &overlay, None)
                .await
            {
                Ok(invoice) => outcome.next_invoice = Some(invoice),
                Err(error) => {
                    log::warn!(
                        "The renewal after a plan change of {} was not previewed: {error:?}",
                        before.subscription.id
                    );
                    outcome.missing.push(PreviewPart::NextInvoice);
                }
            }
            outcome.mrr_after_cents = Some(
                self.mrr_after_change(conn, before, &after, on, HashMap::new())
                    .await?,
            );
            outcome.subscription_after = Some(after);
            Ok(())
        })
        .await;
        if let Err(error) = laid {
            log::warn!(
                "The subscription after a plan change of {} was not previewed: {error:?}",
                prepared.subscription_details.subscription.id
            );
            if outcome.next_invoice.is_none()
                && !outcome.missing.contains(&PreviewPart::NextInvoice)
            {
                outcome.missing.push(PreviewPart::NextInvoice);
            }
            outcome.missing.push(PreviewPart::SubscriptionAfter);
        }
    }

    /// The MRR of `after`, the subscription `before` changed, stated as applying the change would:
    /// both component sets floored by their minimum and netted, the difference laid on the stored
    /// figure.
    pub(in crate::services) async fn mrr_after_change(
        &self,
        conn: &mut PgConn,
        before: &crate::domain::SubscriptionDetails,
        after: &crate::domain::SubscriptionDetails,
        on: NaiveDate,
        seats_after: HashMap<common_domain::ids::ManagedQuantityId, i64>,
    ) -> StoreResult<i64> {
        use crate::services::subscriptions::utils::{subscription_mrr, with_add_on_lines};
        let sub = &before.subscription;
        let precision =
            crate::constants::Currencies::resolve_currency_precision(&sub.currency).unwrap_or(2);
        // Seat add-ons at their quantity's live count, after the change at the counts it sets.
        let before_add_ons =
            add_ons_at_live_seats(conn, sub.tenant_id, sub.id, &before.add_ons).await?;
        let mut after_add_ons =
            add_ons_at_live_seats(conn, sub.tenant_id, sub.id, &after.add_ons).await?;
        let asked: HashMap<common_domain::ids::ManagedQuantityId, u32> = seats_after
            .iter()
            .map(|(quantity, count)| (*quantity, u32::try_from(*count).unwrap_or(0)))
            .collect();
        for add_on in &mut after_add_ons {
            at_live_count(&mut add_on.fee, &asked);
        }
        let before_lines = with_add_on_lines(
            &before_add_ons,
            calculate_components_mrr_with_slots(
                conn,
                sub.tenant_id,
                sub.id,
                &before.price_components,
                precision,
            )
            .await?,
            precision,
        );
        let after_lines = with_add_on_lines(
            &after_add_ons,
            calculate_components_mrr_with_counts(
                conn,
                sub.tenant_id,
                sub.id,
                &after.price_components,
                precision,
                seats_after,
            )
            .await?,
            precision,
        );
        let rate_after = self
            .tax_inclusive_rate_for(conn, after, &after_lines, on)
            .await?;
        let minimum_after = after
            .minimum
            .as_ref()
            .map(|min| (min.amount_subunit, &min.scope));
        let minimum_before = before
            .minimum
            .as_ref()
            .map(|min| (min.amount_subunit, &min.scope));
        Ok(sub.mrr_cents as i64
            + subscription_mrr(
                &after_lines,
                minimum_after,
                after.subscription.period,
                rate_after,
            )
            - subscription_mrr(
                &before_lines,
                minimum_before,
                sub.period,
                sub.tax_inclusive_rate,
            ))
    }

    pub(in crate::services) async fn cancel_plan_change(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id)
                        .await?;
                    credit_inputs::bump_customer(conn, &lock).await?;

                    let pending_changes =
                        crate::services::subscriptions::phases::pending_one_offs_tx(
                            conn,
                            tenant_id,
                            subscription_id,
                            &[diesel_models::enums::SubscriptionChangeKindEnum::PlanChange],
                        )
                        .await?;

                    let event = pending_changes.first().ok_or_else(|| {
                        Report::new(StoreError::ValueNotFound(
                            "No pending plan change found for this subscription".to_string(),
                        ))
                    })?;

                    if !self
                        .cancel_pending_change_tx(conn, actor, event, "Cancelled by user")
                        .await?
                    {
                        return Err(Report::new(StoreError::ValueNotFound(
                            "No pending plan change found for this subscription".to_string(),
                        )));
                    }

                    let customer_id =
                        SubscriptionRow::get_customer_id(conn, &tenant_id, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                    let activity = Activity::new(
                        ActivityType::SubscriptionPlanChangeCancelled,
                        EntityType::Subscription,
                        subscription_id.as_uuid(),
                    )
                    .agg_customer(customer_id);
                    self.store
                        .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                        .await
                }
                .scope_boxed()
            })
            .await
    }

    pub(in crate::services) async fn cancel_scheduled_event(
        &self,
        actor: Actor,
        event_id: common_domain::ids::ScheduledEventId,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    // Before the event is read: the lifecycle runs an event holding the customer,
                    // so an event read pending here is still pending when cancelled below.
                    SubscriptionRow::get_customer_id(conn, &tenant_id, subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    let lock = SubscriptionRow::lock_customer_of(conn, subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    credit_inputs::bump_customer(conn, &lock).await?;
                    let change = SubscriptionChangeRow::find_by_id(conn, event_id, &tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    let activity_type = match change {
                        Some(change) => {
                            if change.subscription_id != subscription_id {
                                return Err(Report::new(StoreError::InvalidArgument(
                                    "Event does not belong to the specified subscription"
                                        .to_string(),
                                )));
                            }
                            // Customers may only cancel changes they themselves scheduled.
                            if actor.is_customer() && !change.created_by_customer {
                                return Err(Report::new(StoreError::InvalidArgument(
                                    "This scheduled change can only be modified by your account manager"
                                        .to_string(),
                                )));
                            }
                            // A phase's boundary goes with its phase, from the subscription's phases.
                            if crate::services::subscriptions::phases::is_timeline_change(&change) {
                                return Err(Report::new(StoreError::InvalidArgument(
                                    "This change is a phase of the subscription: edit its phases"
                                        .to_string(),
                                )));
                            }
                            if !self
                                .cancel_pending_change_tx(conn, actor, &change, "Cancelled by user")
                                .await?
                            {
                                return Err(Report::new(StoreError::InvalidArgument(
                                    "Only pending events can be cancelled".to_string(),
                                )));
                            }
                            match change.kind {
                                diesel_models::enums::SubscriptionChangeKindEnum::PlanChange => {
                                    ActivityType::SubscriptionPlanChangeCancelled
                                }
                                diesel_models::enums::SubscriptionChangeKindEnum::Amendment
                                | diesel_models::enums::SubscriptionChangeKindEnum::Phase => {
                                    ActivityType::SubscriptionAmendmentCancelled
                                }
                            }
                        }
                        // Not a change: the subscription's cancellation, named by its ending.
                        None => {
                            let cancellation = SubscriptionEndingRow::find_pending_cancellation(
                                conn,
                                tenant_id,
                                event_id.as_uuid(),
                            )
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                            .ok_or_else(|| {
                                Report::new(StoreError::InvalidArgument(
                                    "Only pending events can be cancelled".to_string(),
                                ))
                            })?;
                            if cancellation.subscription_id != subscription_id {
                                return Err(Report::new(StoreError::InvalidArgument(
                                    "Event does not belong to the specified subscription"
                                        .to_string(),
                                )));
                            }
                            if actor.is_customer()
                                && cancellation.actor_type
                                    != diesel_models::enums::ActorTypeEnum::Customer
                            {
                                return Err(Report::new(StoreError::InvalidArgument(
                                    "This scheduled change can only be modified by your account manager"
                                        .to_string(),
                                )));
                            }
                            crate::services::facts::withdraw_cancellation_tx(conn, subscription_id)
                                .await?;
                            // Undone before its day, its churn was never booked: none is.
                            diesel_models::subscription_events::SubscriptionEventRow::withdraw_unbooked_cancellation(
                                conn,
                                &lock,
                                subscription_id,
                            )
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                            self.derive_state_tx(
                                conn,
                                actor,
                                tenant_id,
                                subscription_id,
                                common_clock::now(),
                            )
                            .await?;
                            ActivityType::SubscriptionCancellationUndone
                        }
                    };
                    let activity_type = Some(activity_type);

                    if let Some(activity_type) = activity_type {
                        let customer_id =
                            SubscriptionRow::get_customer_id(conn, &tenant_id, subscription_id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;

                        let activity = Activity::new(
                            activity_type,
                            EntityType::Subscription,
                            subscription_id.as_uuid(),
                        )
                        .agg_customer(customer_id);
                        self.store
                            .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                            .await?;
                    }

                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }

    /// Cancels a change before its date: its rows from its date go; the state is derived again.
    /// False when it was no longer pending.
    pub(in crate::services) async fn cancel_pending_change_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        event: &SubscriptionChangeRow,
        reason: &str,
    ) -> StoreResult<bool> {
        // The timeline after it was written over it: written again over what it leaves (R-162).
        let lifted = timeline_rewrite::lift_tx(
            conn,
            event.tenant_id,
            event.subscription_id,
            event.effective_at.date(),
        )
        .await?;
        let cancelled =
            crate::services::facts::pending::withdraw_pending_tx(conn, event, reason).await?;
        timeline_rewrite::replay_tx(self, conn, actor, lifted).await?;
        if cancelled {
            self.derive_state_tx(
                conn,
                actor,
                event.tenant_id,
                event.subscription_id,
                common_clock::now(),
            )
            .await?;
        }
        Ok(cancelled)
    }

    pub(in crate::services) async fn apply_plan_change_immediate(
        &self,
        typed_actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: impl Into<PlanTarget>,
        component_params: Vec<ComponentParameterization>,
    ) -> StoreResult<ImmediatePlanChangeResult> {
        let target: PlanTarget = target.into();
        let today = common_clock::now().naive_utc().date();
        self.apply_plan_change_immediate_at(
            typed_actor,
            subscription_id,
            tenant_id,
            target,
            component_params,
            today,
        )
        .await
    }

    pub(crate) async fn apply_plan_change_immediate_at(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: impl Into<PlanTarget>,
        component_params: Vec<ComponentParameterization>,
        change_date: NaiveDate,
    ) -> StoreResult<ImmediatePlanChangeResult> {
        let target: PlanTarget = target.into();
        let new_plan_version_id = target.version;
        let mut conn = self.store.get_conn().await?;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
            let actor = &actor;
            let component_params = &component_params;
            let target = target.clone();
            async move {
                let prepared = services
                    .prepare_plan_change_tx(
                        conn,
                        subscription_id,
                        tenant_id,
                        target,
                        component_params,
                        change_date,
                    )
                    .await?;
                refuse_while_a_paid_change_is_pending(conn, tenant_id, subscription_id, None)
                    .await?;
                refuse_after_a_cancellation(conn, tenant_id, subscription_id, change_date).await?;
                let customer_id = prepared.subscription_details.customer.id;

                let is_free_trial = prepared.is_free_trial();

                let (adjustment, credit_note_ids) = services
                    .apply_and_invoice_plan_change_tx(
                        conn,
                        actor,
                        &prepared,
                        new_plan_version_id,
                        change_date,
                    )
                    .await?;
                let adjustment_invoice_id = adjustment.map(|invoice| invoice.id);

                // Free trial: create first invoice at new plan rate (optimistic)
                let first_invoice_id = if is_free_trial {
                    use crate::services::InvoiceBillingMode;

                    let invoice = services
                        .bill_subscription_tx(
                            conn,
                            actor,
                            tenant_id,
                            subscription_id,
                            InvoiceBillingMode::Immediate,
                        )
                        .await?;
                    invoice.map(|inv| inv.invoice.id)
                } else {
                    None
                };

                let activity = Activity::new(
                    ActivityType::SubscriptionPlanChanged,
                    EntityType::Subscription,
                    subscription_id.as_uuid(),
                )
                .with_metadata(serde_json::json!({
                    "new_plan_version_id": new_plan_version_id.to_string(),
                    "change_date": change_date.to_string(),
                }))
                .agg_customer(customer_id);
                services
                    .store
                    .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;

                Ok(ImmediatePlanChangeResult {
                    adjustment_invoice_id,
                    credit_note_ids,
                    first_invoice_id,
                    effective_date: change_date,
                })
            }
            .scope_boxed()
        })
        .await
    }

    /// Applies the change: the old plan settled first (unused fees credited on notes, its packs'
    /// period credit removed), so that credit never pays the new plan's charges, then what the
    /// facts owe invoiced and finalized. Returns the adjustment invoice.
    pub(crate) async fn apply_and_invoice_plan_change_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        prepared: &PreparedPlanChange,
        new_plan_version_id: PlanVersionId,
        change_date: NaiveDate,
    ) -> StoreResult<(Option<crate::domain::Invoice>, Vec<CreditNoteId>)> {
        let subscription = &prepared.subscription_details.subscription;
        let notes = self
            .execute_plan_change_tx(
                conn,
                actor,
                prepared,
                subscription.id,
                subscription.tenant_id,
                new_plan_version_id,
                change_date,
            )
            .await?;
        if prepared.is_free_trial() {
            return Ok((None, notes));
        }
        let bills = self
            .plan_change_bills(conn, subscription, change_date)
            .await?;
        let invoice = self
            .issue_change_adjustment_tx(
                conn,
                actor,
                subscription,
                &prepared.subscription_details.customer,
                change_date,
                &bills,
            )
            .await?;
        Ok((invoice, notes))
    }

    /// What an immediate plan change on `on` bills, from the facts it wrote: the new fees' stubs
    /// and, on the period's first day after its invoice, the old fees' arrears (R-011, R-080).
    pub(crate) async fn plan_change_bills(
        &self,
        conn: &mut PgConn,
        subscription: &crate::domain::Subscription,
        on: NaiveDate,
    ) -> StoreResult<Vec<crate::domain::LineItem>> {
        let change = Change::of_rows(on, GiveBack::Unused, "plan change");
        Ok(change_documents(
            conn,
            subscription.tenant_id,
            subscription.id,
            &change,
            None,
            Some(self.usage_client.as_ref()),
        )
        .await?
        .bills)
    }

    /// Prepare phase: validates, loads data, calculates proration.
    /// Must be called within a transaction.
    pub(in crate::services) async fn prepare_plan_change_tx(
        &self,
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: impl Into<PlanTarget>,
        component_params: &[ComponentParameterization],
        change_date: NaiveDate,
    ) -> StoreResult<PreparedPlanChange> {
        let target: PlanTarget = target.into();
        let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id).await?;
        credit_inputs::bump_customer(conn, &lock).await?;
        let prepared = self
            .prepare_plan_change_inner(
                conn,
                subscription_id,
                tenant_id,
                target,
                component_params,
                change_date,
            )
            .await?;
        prepared.ensure_immediate_allowed()?;
        Ok(prepared)
    }

    /// Read-only version of prepare (no row lock). For previews/GetCheckout.
    pub(in crate::services) async fn prepare_plan_change_readonly(
        &self,
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: impl Into<PlanTarget>,
        component_params: &[ComponentParameterization],
        change_date: NaiveDate,
    ) -> StoreResult<PreparedPlanChange> {
        let target: PlanTarget = target.into();
        self.prepare_plan_change_inner(
            conn,
            subscription_id,
            tenant_id,
            target,
            component_params,
            change_date,
        )
        .await
    }

    async fn prepare_plan_change_inner(
        &self,
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        target: PlanTarget,
        component_params: &[ComponentParameterization],
        change_date: NaiveDate,
    ) -> StoreResult<PreparedPlanChange> {
        let ctx = load_validated_plan_change_context(
            conn,
            &self.store,
            tenant_id,
            subscription_id,
            target,
        )
        .await?;

        // Ids the new components are inserted with, stamped on the lines billing them so a later
        // change in the period credits those lines.
        let new_component_ids: HashMap<PriceComponentId, SubscriptionPriceComponentId> = ctx
            .target_components
            .iter()
            .map(|target| (target.id, SubscriptionPriceComponentId::new()))
            .collect();

        let mut preview = build_plan_change_preview(
            &ctx.subscription_details.price_components,
            &ctx.target_components,
            &ctx.units,
            component_params,
            &ctx.prices,
            change_date,
            &new_component_ids,
        )?;

        resolve_preview_slot_counts(conn, tenant_id, subscription_id, &mut preview).await?;

        // A component the target bills exactly as before is carried over: nothing is credited
        // or charged for it, and its new row continues the lineage of the lines that billed it.
        let carried: HashMap<PriceComponentId, SubscriptionPriceComponentId> = preview
            .matched
            .iter()
            .filter(|m| m.current_period == m.new_period && same_fee(&m.current_fee, &m.new_fee))
            .filter_map(|m| {
                let key = m.current_net_key.as_deref()?;
                let root = ctx
                    .subscription_details
                    .price_components
                    .iter()
                    .find(|c| c.lineage().to_string() == key)?
                    .lineage();
                Some((m.price_component_id?, root))
            })
            .collect();

        let precision = crate::constants::Currencies::resolve_currency_precision(
            &ctx.subscription_details.subscription.currency,
        )
        .unwrap_or(2);

        let direction = detect_change_direction(
            &preview.matched,
            &preview.added,
            &preview.removed,
            precision,
        );

        let period_start = ctx.subscription_details.subscription.current_period_start;
        let period_end = ctx
            .subscription_details
            .subscription
            .current_period_end
            .ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(
                    "Subscription has no current_period_end".to_string(),
                ))
            })?;

        if change_date < period_start || change_date > period_end {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "Change date {} is outside current period [{}, {}]",
                change_date, period_start, period_end
            ))));
        }

        let ending_add_ons: Vec<SubscriptionAddOn> = ctx
            .subscription_details
            .add_ons
            .iter()
            .filter(|a| !ctx.compatible_add_on_ids.contains(&a.add_on_id))
            .cloned()
            .collect();
        let component_mappings = build_component_mappings(
            &ctx.subscription_details.price_components,
            &ctx.target_components,
            &ctx.units,
            component_params,
            &ctx.prices,
        )?;
        let free_trial = ctx.subscription_details.subscription.status
            == SubscriptionStatusEnum::TrialActive
            && super::trial_is_free_tx(conn, &ctx.subscription_details.subscription).await?;

        let outgoing = OwnGrid::read(
            conn,
            &ctx.subscription_details.subscription,
            (period_start, period_end),
            change_date,
        )
        .await?;
        let period = (period_start, period_end, outgoing.billed(change_date).2);
        Ok(PreparedPlanChange {
            free_trial,
            subscription_details: ctx.subscription_details,
            component_mappings,
            new_component_ids,
            carried,
            settle_outgoing: true,
            preview,
            period,
            change_date,
            direction,
            precision,
            target_minimum: ctx.target_minimum,
            compatible_add_on_ids: ctx.compatible_add_on_ids,
            ending_add_ons,
            target_option: ctx.target_option,
        })
    }

    /// Execute phase: settles what the old plan billed for the rest of the period (credit notes,
    /// pack credit removed), then applies the plan change (component rotation, MRR recalc, event).
    /// Returns the credit notes issued.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn execute_plan_change_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        prepared: &PreparedPlanChange,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        change_date: NaiveDate,
    ) -> StoreResult<Vec<CreditNoteId>> {
        self.execute_plan_change_reporting_tx(
            conn,
            actor,
            prepared,
            subscription_id,
            tenant_id,
            new_plan_version_id,
            change_date,
        )
        .await
        .map(|(notes, _)| notes)
    }

    /// [`Self::execute_plan_change_tx`], with what it did to the subscription's timeline.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn execute_plan_change_reporting_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        prepared: &PreparedPlanChange,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        new_plan_version_id: PlanVersionId,
        change_date: NaiveDate,
    ) -> StoreResult<(Vec<CreditNoteId>, Vec<RewriteEntry>)> {
        // Free trials settle nothing: no money changed hands.
        let settles_outgoing = !prepared.is_free_trial() && prepared.settle_outgoing;

        // The timeline from its day is written again over it (R-162); one in a trial ends the
        // trial, and the switch written for its end with it.
        let mut lifted =
            timeline_rewrite::lift_tx(conn, tenant_id, subscription_id, change_date).await?;
        let mut timeline = Vec::new();
        if prepared.is_trial() {
            timeline = lifted
                .drop_switches_until(
                    conn,
                    prepared.period.1,
                    "the trial ended with the plan change",
                )
                .await?;
        }
        // A change written for a later date holds the rows this one closes: it goes first.
        crate::services::subscriptions::phases::withdraw_one_offs_tx(
            conn,
            tenant_id,
            subscription_id,
            crate::services::facts::pending::CHANGES,
            "Replaced by immediate plan change",
        )
        .await?;

        let component_mappings = &prepared.component_mappings;
        let old_plan_version_id = prepared.subscription_details.subscription.plan_version_id;

        let new_period =
            ComponentMapping::derive_billing_period(component_mappings).map(|p| p.into());

        self.store
            .hooks
            .signals
            .subscription_plan_changed(
                conn,
                tenant_id,
                prepared.subscription_details.subscription.customer_id,
                subscription_id,
                common_clock::now().naive_utc(),
            )
            .await?;

        let mut components_to_close = Vec::new();
        let mut components_to_insert: Vec<SubscriptionComponentRowNew> = Vec::new();

        for mapping in component_mappings {
            match mapping {
                ComponentMapping::Matched {
                    current_component_id,
                    target_component_id,
                    product_id,
                    price_id,
                    name,
                    fee,
                    period,
                    priced,
                    ..
                } => {
                    components_to_close.push(*current_component_id);
                    let mut row_new: SubscriptionComponentRowNew = SubscriptionComponentNew {
                        subscription_id,
                        internal: SubscriptionComponentNewInternal {
                            price_component_id: Some(*target_component_id),
                            product_id: Some(*product_id),
                            name: name.clone(),
                            period: *period,
                            fee: fee.clone(),
                            is_override: *priced,
                            price_id: (!priced).then_some(*price_id),
                            effective_from: change_date,
                        },
                    }
                    .try_into()
                    .map_err(|_| {
                        StoreError::InvalidArgument(
                            "Failed to convert matched component for plan change".to_string(),
                        )
                    })?;
                    if let Some(id) = prepared.new_component_ids.get(target_component_id) {
                        row_new.id = *id;
                    }
                    row_new.lineage_id = prepared.carried.get(target_component_id).copied();
                    components_to_insert.push(row_new);
                }
                ComponentMapping::Added {
                    target_component_id,
                    product_id,
                    price_id,
                    name,
                    fee,
                    period,
                    ..
                } => {
                    let mut row_new: SubscriptionComponentRowNew = SubscriptionComponentNew {
                        subscription_id,
                        internal: SubscriptionComponentNewInternal {
                            price_component_id: Some(*target_component_id),
                            product_id: *product_id,
                            name: name.clone(),
                            period: *period,
                            fee: fee.clone(),
                            is_override: false,
                            price_id: *price_id,
                            effective_from: change_date,
                        },
                    }
                    .try_into()
                    .map_err(|_| {
                        StoreError::InvalidArgument(
                            "Failed to convert new component for plan change".to_string(),
                        )
                    })?;
                    if let Some(id) = prepared.new_component_ids.get(target_component_id) {
                        row_new.id = *id;
                    }
                    components_to_insert.push(row_new);
                }
                ComponentMapping::Removed {
                    current_component_id,
                } => {
                    components_to_close.push(*current_component_id);
                }
            }
        }

        crate::services::facts::close_components_tx(conn, &components_to_close, change_date)
            .await?;

        if !components_to_insert.is_empty() {
            let refs: Vec<&SubscriptionComponentRowNew> = components_to_insert.iter().collect();
            SubscriptionComponentRow::insert_subscription_component_batch(conn, refs)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        }

        // Refresh the subscription minimum snapshot from the new plan version.
        // Operator-set / quote-sourced overrides (`is_override = true`) are preserved —
        // those represent negotiated contracts that survive plan changes. Only plain
        // plan-snapshots (`is_override = false`) get re-snapped.
        refresh_subscription_minimum_on_plan_change(
            conn,
            subscription_id,
            new_plan_version_id,
            change_date,
        )
        .await?;

        // A target seat fee counting a quantity the subscription holds carries its seats.
        let held = held_seat_quantities(&prepared.subscription_details);
        let seeds: Vec<_> = component_mappings
            .iter()
            .filter_map(|mapping| match mapping {
                ComponentMapping::Added { fee, seats, .. } => {
                    plan_change_seat_seed(fee, *seats, false, &held, change_date)
                }
                ComponentMapping::Matched { fee, .. } => {
                    plan_change_seat_seed(fee, None, true, &held, change_date)
                }
                ComponentMapping::Removed { .. } => None,
            })
            .map(|tx| (subscription_id, tx))
            .collect();
        let rows =
            crate::repositories::subscriptions::slots::ledger_rows(conn, tenant_id, &seeds).await?;
        SlotTransactionRow::insert_batch(conn, rows.iter().collect())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let ended_addons = SubscriptionAddOnRow::close_incompatible(
            conn,
            &subscription_id,
            &prepared.compatible_add_on_ids,
            change_date,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        super::utils::rescope_coupons_to_plan_tx(conn, subscription_id, new_plan_version_id)
            .await?;

        if !ended_addons.is_empty() {
            log::info!(
                "Ended {} incompatible add-on(s) during plan change for subscription {}: {:?}",
                ended_addons.len(),
                subscription_id,
                ended_addons.iter().map(|a| a.add_on_id).collect::<Vec<_>>(),
            );
        }

        let renews_on = crate::services::facts::record_plan_change_tx(
            conn,
            tenant_id,
            subscription_id,
            change_date,
            new_plan_version_id,
            new_period
                .clone()
                .unwrap_or_else(|| prepared.subscription_details.subscription.period.into()),
            prepared.is_trial() && prepared.is_free_trial(),
        )
        .await?;
        crate::repositories::subscriptions::usage_attribution::settle_late_change(
            conn,
            tenant_id,
            subscription_id,
            change_date,
            "The plan change",
        )
        .await?;

        // The old plan's give-backs read the facts just written.
        let credit_note_ids = if settles_outgoing {
            self.settle_outgoing_components_tx(conn, actor, prepared, change_date)
                .await?
        } else {
            Vec::new()
        };

        let sub_event = diesel_models::subscription_events::SubscriptionEventRow {
            id: uuid::Uuid::now_v7(),
            subscription_id,
            event_type: diesel_models::enums::SubscriptionEventType::Switch,
            details: Some(if settles_outgoing {
                // Marks the change whose ended lines are settled, now or once their invoice is issued.
                serde_json::json!({
                    "new_plan_version_id": new_plan_version_id.to_string(),
                    "old_plan_version_id": old_plan_version_id.to_string(),
                    "mode": "immediate",
                    "outgoing": "settled",
                    "notes": credit_note_ids.iter().map(|id| id.as_uuid().to_string()).collect::<Vec<_>>(),
                    "seats": prepared.outgoing_seats(),
                })
            } else {
                serde_json::json!({
                    "new_plan_version_id": new_plan_version_id.to_string(),
                    "old_plan_version_id": old_plan_version_id.to_string(),
                    "mode": "immediate",
                })
            }),
            created_at: common_clock::now().naive_utc(),
            mrr_delta: None,
            bi_mrr_movement_log_id: None,
            applies_to: change_date,
        };
        sub_event
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Trial → Active transition
        if prepared.is_trial() {
            use diesel_models::subscriptions::SubscriptionCycleRowPatch;

            if prepared.is_free_trial() {
                // Free trial: billing starts fresh from change_date.
                // Reset billing_start_date and anchor to the change_date so the
                // invoice computation aligns with the new period.
                let new_anchor = change_date.day();
                let billing_period: crate::domain::enums::BillingPeriodEnum = renews_on.into();
                let period_range =
                    calculate_advance_period_range(change_date, new_anchor, true, &billing_period);

                let patch = SubscriptionCycleRowPatch {
                    id: subscription_id,
                    tenant_id,
                    status: Some(SubscriptionStatusEnum::Active.into()),
                    current_period_start: Some(change_date),
                    current_period_end: Some(Some(period_range.end)),
                    pending_checkout: None,
                    billing_start_date: Some(change_date),
                    billing_day_anchor: Some(new_anchor as i16),
                    terminated_at: None,
                };
                patch.patch(conn).await?;

                log::info!(
                    "Free trial ended via plan change for subscription {}: period [{}, {}]",
                    subscription_id,
                    change_date,
                    period_range.end,
                );
            } else {
                // Paid trial: keep billing period (already being billed), just transition status.
                let patch = SubscriptionCycleRowPatch {
                    id: subscription_id,
                    tenant_id,
                    status: Some(SubscriptionStatusEnum::Active.into()),
                    current_period_start: None,
                    current_period_end: None,
                    pending_checkout: None,
                    billing_start_date: None,
                    billing_day_anchor: None,
                    terminated_at: None,
                };
                patch.patch(conn).await?;

                log::info!(
                    "Paid trial ended via plan change for subscription {}",
                    subscription_id,
                );
            }
        }

        timeline.extend(timeline_rewrite::replay_tx(self, conn, actor, lifted).await?);

        // The plan, its cadence and the period it cuts, the MRR and the outbox: the writer's,
        // from the facts just written, read at the change's instant.
        self.derive_state_tx(
            conn,
            actor,
            tenant_id,
            subscription_id,
            crate::services::evaluate::bill::midnight(change_date),
        )
        .await?;
        // A version without options leaves none of the old version's.
        let option = prepared.target_option.as_ref().map(|option| option.id);
        if option.is_some()
            || prepared.subscription_details.subscription.plan_version_id != new_plan_version_id
        {
            record_plan_option(conn, subscription_id, option).await?;
        }

        log::info!(
            "Applied immediate plan change for subscription {}: plan_version={}, direction={:?}",
            subscription_id,
            new_plan_version_id,
            prepared.direction,
        );

        Ok((credit_note_ids, timeline))
    }
}

/// Intermediate struct holding validated and computed data from the prepare phase.
pub struct PreparedPlanChange {
    pub subscription_details: crate::domain::SubscriptionDetails,
    component_mappings: Vec<ComponentMapping>,
    new_component_ids: HashMap<PriceComponentId, SubscriptionPriceComponentId>,
    /// Target components carried over unchanged, with the lineage root they continue.
    pub(crate) carried: HashMap<PriceComponentId, SubscriptionPriceComponentId>,
    /// False when an earlier release already netted the old plan's credits on the adjustment.
    settle_outgoing: bool,
    preview: PlanChangePreview,
    /// The period the change lands in and the share of it left from its day.
    period: (NaiveDate, NaiveDate, DayFraction),
    change_date: NaiveDate,
    direction: ChangeDirection,
    precision: u8,
    target_minimum: Option<crate::domain::plan_minimums::PlanMinimum>,
    /// In a trial that bills nothing, as activation decides it (`trial_is_free_tx`).
    free_trial: bool,
    compatible_add_on_ids: Vec<AddOnId>,
    /// The subscription's add-ons the target plan version does not offer: they end at the change.
    pub(crate) ending_add_ons: Vec<SubscriptionAddOn>,
    /// The option of the target version moved to.
    pub(crate) target_option: Option<PlanOption>,
}

impl PreparedPlanChange {
    /// The change without the old plan's settlement: what a preview writes, its documents
    /// computed apart.
    pub(crate) fn without_settlement(mut self) -> Self {
        self.settle_outgoing = false;
        self
    }

    /// What the change maps each component to, as its rows are written from.
    pub(crate) fn component_mappings(&self) -> &[ComponentMapping] {
        &self.component_mappings
    }

    /// The seats of each unit the old plan had when the change read them: a deferred settlement
    /// cannot read them back once the new plan's seats are recorded under the same unit.
    pub(crate) fn outgoing_seats(&self) -> BTreeMap<String, u32> {
        self.preview
            .matched
            .iter()
            .map(|m| &m.current_fee)
            .chain(self.preview.removed.iter().map(|r| &r.current_fee))
            .filter_map(|fee| match fee {
                SubscriptionFee::Slot {
                    unit,
                    initial_slots,
                    ..
                } => Some((unit.clone(), *initial_slots)),
                _ => None,
            })
            .collect()
    }

    /// A change paid on an adjustment invoice an earlier release issued: that invoice netted the
    /// old plan's credits as negative lines and billed no pack, so nothing more is settled; the
    /// change inserts the components it billed.
    pub(crate) fn adopt_legacy_adjustment(&mut self, invoice: &crate::domain::Invoice) {
        self.settle_outgoing = false;
        for line in &invoice.line_items {
            if let (Some(target), Some(id)) = (line.price_component_id, line.sub_component_id)
                && let Some(slot) = self.new_component_ids.get_mut(&target)
            {
                *slot = id;
            }
        }
    }

    /// The day the change was prepared for.
    pub fn change_date(&self) -> NaiveDate {
        self.change_date
    }

    /// The summary of what the change bills (`charged`, the packs it buys among it) and takes
    /// back: `credited` of its fees, as the facts reconcile them, and the old packs' fees.
    pub(crate) fn proration_summary(
        &self,
        charged: i64,
        credited: i64,
        incoming: Vec<crate::domain::subscription_changes::PackChange>,
        outgoing: &crate::services::subscriptions::plan_change_credit::OutgoingEstimate,
    ) -> ProrationSummary {
        // The credits as the notes would issue them: the fees' unused time and the packs', net.
        let pack_credits: i64 = outgoing.packs.iter().map(|pack| pack.fee_credit).sum();
        let credits_total_cents = -(credited + pack_credits);
        let (period_start, period_end, factor) = self.period;
        let mut pack_changes = incoming;
        pack_changes.extend(outgoing.packs.iter().flat_map(|pack| pack.changes()));
        ProrationSummary {
            document: if self.free_trial {
                crate::domain::subscription_changes::ChangeDocument::OpeningInvoice
            } else {
                crate::domain::subscription_changes::ChangeDocument::Adjustment
            },
            credits_total_cents,
            charges_total_cents: charged,
            net_amount_cents: charged + credits_total_cents,
            // Credits are issued on credit notes, never netted into a charge.
            net_credit_cents: credits_total_cents,
            // Plan changes don't introduce new arrears components, so no deferred amount.
            arrears_charge_cents: 0,
            proration_factor: factor.as_f64(),
            days_remaining: (period_end - self.change_date).num_days().max(0) as u32,
            days_in_period: (period_end - period_start).num_days().max(0) as u32,
            pack_changes,
        }
    }

    /// `base` (the subscription now, or as it was at a past cycle) moved to the target version, in
    /// memory, for a preview to price. Each component keeps its row (id, effective dates, lineage)
    /// and takes the target fee of its product: the biller's history then never counts a row twice,
    /// and a component that started later is not backdated. Components the target drops are left
    /// out; a product the change does not know (only in `base`'s history) keeps its fee; the
    /// target's new components start on `added_from`. The minimum and the cadence are the target's.
    pub fn hypothetical_details(
        &self,
        target_version_id: PlanVersionId,
        base: &crate::domain::SubscriptionDetails,
        added_from: NaiveDate,
    ) -> crate::domain::SubscriptionDetails {
        let mut details = base.clone();
        let subscription_id = details.subscription.id;
        let current = &self.subscription_details.price_components;

        let mut by_component = HashMap::new();
        let mut by_product = HashMap::new();
        let mut removed_components = std::collections::HashSet::new();
        let mut removed_products = std::collections::HashSet::new();
        let mut added = Vec::new();
        for mapping in &self.component_mappings {
            match mapping {
                ComponentMapping::Matched {
                    current_component_id,
                    product_id,
                    ..
                } => {
                    by_component.insert(*current_component_id, mapping);
                    by_product.insert(*product_id, mapping);
                }
                ComponentMapping::Removed {
                    current_component_id,
                } => {
                    removed_components.insert(*current_component_id);
                    if let Some(product_id) = current
                        .iter()
                        .find(|c| c.id == *current_component_id)
                        .and_then(|c| c.product_id)
                    {
                        removed_products.insert(product_id);
                    }
                }
                ComponentMapping::Added {
                    target_component_id,
                    product_id,
                    price_id,
                    name,
                    fee,
                    period,
                    ..
                } => added.push(SubscriptionComponent {
                    id: common_domain::ids::SubscriptionPriceComponentId::new(),
                    price_component_id: Some(*target_component_id),
                    product_id: *product_id,
                    subscription_id,
                    name: name.clone(),
                    period: *period,
                    fee: fee.clone(),
                    price_id: *price_id,
                    effective_from: added_from,
                    effective_to: None,
                    lineage_id: None,
                    added_by_amendment: false,
                }),
            }
        }

        details.price_components = base
            .price_components
            .iter()
            .filter_map(|c| {
                let target = by_component
                    .get(&c.id)
                    .or_else(|| c.product_id.and_then(|p| by_product.get(&p)));
                match target {
                    Some(ComponentMapping::Matched {
                        target_component_id,
                        product_id,
                        price_id,
                        name,
                        fee,
                        period,
                        ..
                    }) => Some(SubscriptionComponent {
                        price_component_id: Some(*target_component_id),
                        product_id: Some(*product_id),
                        price_id: Some(*price_id),
                        name: name.clone(),
                        fee: fee.clone(),
                        period: *period,
                        ..c.clone()
                    }),
                    _ if removed_components.contains(&c.id)
                        || c.product_id.is_some_and(|p| removed_products.contains(&p)) =>
                    {
                        None
                    }
                    _ => Some(c.clone()),
                }
            })
            .chain(added)
            .collect();
        if !details.minimum.as_ref().is_some_and(|m| m.is_override) {
            details.minimum = self.target_minimum.as_ref().map(|m| {
                crate::domain::subscription_minimums::SubscriptionMinimum {
                    subscription_id,
                    amount_subunit: m.amount_subunit,
                    scope: m.scope.clone(),
                    is_override: false,
                }
            });
        }
        if let Some(period) = ComponentMapping::derive_billing_period(&self.component_mappings) {
            details.subscription.period = period;
        }
        details.subscription.plan_version_id = target_version_id;
        details
    }

    /// The target's new slot components: the change seeds their ledger with `initial_slots`.
    pub fn added_slot_units(&self) -> Vec<String> {
        self.component_mappings
            .iter()
            .filter_map(|m| match m {
                ComponentMapping::Added {
                    fee: SubscriptionFee::Slot { unit, .. },
                    ..
                } => Some(unit.clone()),
                _ => None,
            })
            .collect()
    }

    /// A minimum commitment, held or targeted, keeps the change to the period's end.
    pub(crate) fn ensure_immediate_allowed(&self) -> StoreResult<()> {
        if self.subscription_details.minimum.is_some() || self.target_minimum.is_some() {
            return Err(Report::new(StoreError::FailedPrecondition(
                MINIMUM_COMMITMENT_AT_PERIOD_END.to_owned(),
            )));
        }
        Ok(())
    }

    pub fn is_free_trial(&self) -> bool {
        self.free_trial
    }

    pub fn is_trial(&self) -> bool {
        self.subscription_details.subscription.status == SubscriptionStatusEnum::TrialActive
    }
}

/// A plan change taking effect once the subscription is cancelled changes nothing (R-092).
async fn refuse_after_a_cancellation(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    effective_date: NaiveDate,
) -> StoreResult<()> {
    let cancels_at = SubscriptionEndingRow::list_of(conn, subscription_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .filter(|ending| {
            ending.tenant_id == tenant_id
                && ending.kind == diesel_models::subscription_facts::ENDING_CANCEL
        })
        .map(|ending| ending.at)
        .min();
    match cancels_at.filter(|at| at.date() <= effective_date) {
        Some(cancels_at) => Err(Report::new(StoreError::InvalidArgument(format!(
            "The subscription is cancelled on {}, before this plan change would take effect on \
             {effective_date}",
            cancels_at.date()
        )))),
        None => Ok(()),
    }
}

/// A plan change paid by a checkout still awaiting its payment applies when that payment settles:
/// another change meanwhile would be overtaken, or overtake it on stale prices. Refused: the
/// pending checkout is cancelled first (its payment, if it still settles, becomes credit).
/// The mappings a phase's switch to `target` writes from the fees in force on its day, as a
/// scheduled plan change maps them, and the option it moves to (R-150).
pub(crate) async fn switch_mappings_tx(
    conn: &mut PgConn,
    store: &crate::Store,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    target: PlanTarget,
    from: SwitchFrom,
    component_params: &[ComponentParameterization],
) -> StoreResult<(
    Vec<ComponentMapping>,
    Option<common_domain::ids::PlanOptionId>,
)> {
    let ctx = load_plan_change_context(conn, store, tenant_id, subscription_id, target, Some(from))
        .await?;
    let mappings = build_component_mappings(
        &ctx.subscription_details.price_components,
        &ctx.target_components,
        &ctx.units,
        component_params,
        &ctx.prices,
    )?;
    Ok((mappings, ctx.target_option.map(|option| option.id)))
}

pub(crate) async fn refuse_while_a_paid_change_is_pending(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    except: Option<common_domain::ids::CheckoutSessionId>,
) -> StoreResult<()> {
    if let Some(pending) =
        diesel_models::checkout_sessions::CheckoutSessionRow::plan_change_awaiting_payment(
            conn,
            tenant_id,
            subscription_id,
            except,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
    {
        return Err(Report::new(StoreError::FailedPrecondition(format!(
            "A plan change for this subscription is awaiting its payment (checkout session {pending}). Wait for the payment to complete or fail, or cancel that checkout session, before changing the subscription again."
        ))));
    }
    Ok(())
}

fn validate_subscription_for_plan_change(status: &SubscriptionStatusEnum) -> StoreResult<()> {
    match status {
        SubscriptionStatusEnum::Active | SubscriptionStatusEnum::TrialActive => Ok(()),
        _ => Err(Report::new(StoreError::InvalidArgument(format!(
            "Cannot schedule plan change for subscription in {:?} status",
            status
        )))),
    }
}

/// A legacy charge has one amount, in its version's currency: it never bills a subscription in
/// another, where €49 would be billed as $49.
fn refuse_legacy_in_another_currency(
    components: &[TargetComponent],
    version_currency: &str,
    sub_currency: &str,
) -> StoreResult<()> {
    if version_currency.eq_ignore_ascii_case(sub_currency) {
        return Ok(());
    }
    match components
        .iter()
        .find(|c| matches!(c.pricing, TargetPricing::Legacy(_)))
    {
        Some(legacy) => Err(Report::new(StoreError::InvalidArgument(format!(
            "{} is priced in {version_currency} only (a pre-currency price), the subscription is billed in {sub_currency}: price it in {sub_currency} first",
            legacy.name
        )))),
        None => Ok(()),
    }
}

/// Load target components with prices. For v1 components (no v2 prices, Row has legacy_fee),
/// extract legacy pricing data (no fake IDs). Returns components with TargetPricing.
async fn load_target_components_with_prices(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_version_id: PlanVersionId,
    currency: String,
    subscription_currency: &str,
    option: Option<&PlanOption>,
) -> StoreResult<Vec<TargetComponent>> {
    let component_rows =
        PriceComponentRow::list_by_plan_version_id(conn, tenant_id, plan_version_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

    let component_ids: Vec<PriceComponentId> = component_rows.iter().map(|c| c.id).collect();

    // Load v2 prices via plan_component_price join table
    let mut prices_by_component: HashMap<PriceComponentId, Vec<Price>> = HashMap::new();
    let mut priced_elsewhere: std::collections::HashSet<PriceComponentId> =
        std::collections::HashSet::new();
    if !component_ids.is_empty() {
        let pcp_rows = PlanComponentPriceRow::list_by_component_ids(conn, &component_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if !pcp_rows.is_empty() {
            let price_ids: Vec<_> = pcp_rows.iter().map(|pcp| pcp.price_id).collect();
            let price_rows = PriceRow::list_by_ids(conn, &price_ids, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            let prices_by_id: HashMap<_, _> = price_rows
                .into_iter()
                .map(|row| {
                    let id = row.id;
                    Price::try_from(row).map(|p| (id, p))
                })
                .collect::<Result<_, _>>()?;

            let in_option = |pcp: &PlanComponentPriceRow| {
                option.is_none_or(|option| option.id == pcp.option_id)
            };
            for pcp in &pcp_rows {
                match prices_by_id.get(&pcp.price_id) {
                    Some(price)
                        if in_option(pcp)
                            && price.currency.eq_ignore_ascii_case(subscription_currency) =>
                    {
                        let prices = prices_by_component
                            .entry(pcp.plan_component_id)
                            .or_default();
                        // A price the same in several options is linked once per option.
                        if !prices.iter().any(|p| p.id == price.id) {
                            prices.push(price.clone());
                        }
                    }
                    Some(_) => {
                        priced_elsewhere.insert(pcp.plan_component_id);
                    }
                    None => {}
                }
            }
        }
    }

    // A fee priced in other currencies or options only would be dropped from the bill: refuse.
    if let Some(row) = component_rows.iter().find(|row| {
        priced_elsewhere.contains(&row.id) && !prices_by_component.contains_key(&row.id)
    }) {
        return Err(Report::new(StoreError::InvalidArgument(match option {
            Some(option) => format!(
                "{} has no price in {subscription_currency} for {}",
                row.name, option.name
            ),
            None => format!("{} has no price in {subscription_currency}", row.name),
        })));
    }

    // Extract legacy pricing for v1 components
    let mut legacy_by_component: HashMap<PriceComponentId, LegacyPricingData> = HashMap::new();
    for row in &component_rows {
        if !prices_by_component.contains_key(&row.id)
            && let Some(legacy_json) = &row.legacy_fee
        {
            let legacy = extract_legacy_pricing(legacy_json, currency.clone())?;
            legacy_by_component.insert(row.id, legacy);
        }
    }

    let component_rows: Vec<_> = component_rows
        .into_iter()
        .filter(|row| {
            prices_by_component.contains_key(&row.id)
                || legacy_by_component.contains_key(&row.id)
                || row.product_id.is_none()
        })
        .collect();
    let components = component_rows
        .into_iter()
        .map(|row| {
            let id = row.id;
            if let Some(prices) = prices_by_component.remove(&id) {
                let product_id = row.product_id.ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(format!(
                        "V2 component {} has no product_id",
                        row.name
                    )))
                })?;
                let fee_structure =
                    crate::domain::price_components::structure_of_json(row.fee_structure)?
                        .ok_or_else(|| {
                            Report::new(StoreError::InvalidArgument(format!(
                                "Charge {} has no fee structure",
                                row.name
                            )))
                        })?;
                Ok(TargetComponent {
                    id: row.id,
                    name: row.name,
                    pricing: TargetPricing::V2 {
                        product_id,
                        prices,
                        fee_structure,
                    },
                })
            } else if let Some(legacy) = legacy_by_component.remove(&id) {
                Ok(TargetComponent {
                    id: row.id,
                    name: row.name,
                    pricing: TargetPricing::Legacy(legacy),
                })
            } else {
                Err(Report::new(StoreError::InvalidArgument(format!(
                    "Component {} has no pricing data",
                    row.name
                ))))
            }
        })
        .collect::<Result<Vec<_>, Report<StoreError>>>()?;

    Ok(components)
}

/// What a matched current component (same product) carries over to the target's price choice.
#[derive(Clone, Copy)]
struct Ported {
    period: SubscriptionFeeBillingPeriod,
    /// The committed capacity of a capacity fee.
    capacity: Option<u64>,
}

impl Ported {
    fn of(current: &SubscriptionComponent) -> Self {
        Ported {
            period: current.period,
            capacity: match current.fee {
                SubscriptionFee::Capacity { included, .. } => Some(included),
                _ => None,
            },
        }
    }

    /// For a component of a product the subscription doesn't have yet: the subscription's own
    /// cadence, its shortest recurring one.
    fn subscription_cadence(current: &[SubscriptionComponent]) -> Option<Self> {
        current
            .iter()
            .map(|component| component.period)
            .filter(|period| *period != SubscriptionFeeBillingPeriod::OneTime)
            .min_by_key(SubscriptionFeeBillingPeriod::as_months)
            .map(|period| Ported {
                period,
                capacity: None,
            })
    }
}

/// Resolve the fee and period for a target component.
///
/// Price selection priority:
/// 1. Explicit parametrization (from API caller) → use its billing_period / committed_capacity
/// 2. Ported cadence, then committed capacity, of the matched current component (same product_id);
///    for a new product, the subscription's cadence
/// 3. Single price → use it
/// 4. Multiple prices with no selection → error
fn resolve_target_fee(
    target: &TargetComponent,
    units: &crate::domain::credit_units::CreditUnitsById,
    explicit_params: Option<&ComponentParameters>,
    ported: Option<Ported>,
) -> Result<
    (
        SubscriptionFeeBillingPeriod,
        crate::domain::SubscriptionFee,
        Option<common_domain::ids::PriceId>,
    ),
    Report<StoreError>,
> {
    match &target.pricing {
        TargetPricing::V2 {
            prices,
            fee_structure,
            ..
        } => {
            let price = select_target_price(prices, explicit_params, ported, &target.name)?;

            let fee = resolve_subscription_fee(
                fee_structure,
                &price.pricing,
                price.credit_unit.as_ref(),
                units,
                explicit_params,
            )?;
            let period = fee_type_billing_period(fee_structure)
                .unwrap_or_else(|| price.cadence.as_subscription_billing_period());

            Ok((period, fee, Some(price.id)))
        }
        TargetPricing::Legacy(legacy) => {
            let params = build_legacy_params(explicit_params, ported);
            let resolved = resolve_legacy_subscription_fee(legacy, params.as_ref())?;
            Ok((resolved.period, resolved.fee, None))
        }
    }
}

/// Select the right price from a V2 component's price list.
fn select_target_price<'a>(
    prices: &'a [Price],
    explicit_params: Option<&ComponentParameters>,
    ported: Option<Ported>,
    component_name: &str,
) -> Result<&'a Price, Report<StoreError>> {
    // Single price → always use it
    if prices.len() == 1 {
        return Ok(&prices[0]);
    }

    let mut candidates: Vec<&Price> = prices.iter().collect();

    // 1. Filter by explicit params
    if let Some(params) = explicit_params {
        if let Some(bp) = &params.billing_period {
            let target = bp.as_subscription_billing_period();
            candidates.retain(|p| p.cadence.as_subscription_billing_period() == target);
        }
        if let Some(cap) = params.committed_capacity {
            candidates.retain(|p| match &p.pricing {
                Pricing::Capacity { included, .. } => *included == cap,
                _ => true,
            });
        }
    }

    // 2. If still ambiguous, try the ported cadence, then the ported committed capacity, of the
    //    matched current component. Each narrows only when it matches something: no guess.
    if candidates.len() > 1
        && let Some(ported) = ported
    {
        let by_period: Vec<&Price> = candidates
            .iter()
            .filter(|p| p.cadence.as_subscription_billing_period() == ported.period)
            .copied()
            .collect();
        if !by_period.is_empty() {
            candidates = by_period;
        }
        if candidates.len() > 1
            && let Some(cap) = ported.capacity
        {
            let by_capacity: Vec<&Price> = candidates
                .iter()
                .filter(|p| matches!(&p.pricing, Pricing::Capacity { included, .. } if *included == cap))
                .copied()
                .collect();
            if !by_capacity.is_empty() {
                candidates = by_capacity;
            }
        }
    }

    match candidates.len() {
        1 => Ok(candidates[0]),
        0 => Err(Report::new(StoreError::InvalidArgument(format!(
            "No matching price found for component '{}'",
            component_name
        )))),
        _ => Err(Report::new(StoreError::InvalidArgument(format!(
            "Multiple prices match for component '{}' — provide billing_period or committed_capacity to disambiguate",
            component_name
        )))),
    }
}

/// Build ComponentParameters for legacy pricing resolution from explicit params or what the
/// matched current component carries over.
fn build_legacy_params(
    explicit_params: Option<&ComponentParameters>,
    ported: Option<Ported>,
) -> Option<ComponentParameters> {
    if explicit_params.is_some() {
        return explicit_params.cloned();
    }
    let ported = ported?;
    let billing_period = ported.period.as_billing_period_opt();
    (billing_period.is_some() || ported.capacity.is_some()).then_some(ComponentParameters {
        initial_slot_count: None,
        billing_period,
        committed_capacity: ported.capacity,
    })
}

/// `fee` of `target` at the price the change sets on it, one amount (R-163); whether it sets one.
fn at_target_price(
    fee: SubscriptionFee,
    target: &TargetComponent,
    prices: &[TargetPrice],
) -> StoreResult<(SubscriptionFee, bool)> {
    let Some(asked) = prices.iter().find(|price| price.component_id == target.id) else {
        return Ok((fee, false));
    };
    let priced = crate::domain::subscription_phases::repriced(&fee, asked.price).ok_or_else(|| {
        Report::new(StoreError::InvalidArgument(format!(
            "{} is not priced by one amount (a rate, a seat, a capacity): its price cannot be set",
            target.name
        )))
    })?;
    Ok((priced, true))
}

fn build_component_mappings(
    current_components: &[SubscriptionComponent],
    target_components: &[TargetComponent],
    units: &crate::domain::credit_units::CreditUnitsById,
    component_params: &[ComponentParameterization],
    prices: &[TargetPrice],
) -> StoreResult<Vec<ComponentMapping>> {
    let params_by_id: HashMap<PriceComponentId, &ComponentParameters> = component_params
        .iter()
        .map(|p| (p.component_id, &p.parameters))
        .collect();

    let mut mappings = Vec::new();
    let mut matched_current_ids = std::collections::HashSet::new();

    // Match target components to current by product and how it is billed
    for target in target_components {
        let explicit_params = params_by_id.get(&target.id).copied();

        // Try to find a matching current component (v2 only)
        let matched_current = target.matching(current_components);

        if let Some(current) = matched_current {
            matched_current_ids.insert(current.id);

            let (period, fee, price_id) =
                resolve_target_fee(target, units, explicit_params, Some(Ported::of(current)))?;
            let (fee, priced) = at_target_price(fee, target, prices)?;

            let price_id = price_id.ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(format!(
                    "Matched component {} has no price_id",
                    target.name
                )))
            })?;

            mappings.push(ComponentMapping::Matched {
                current_component_id: current.id,
                target_component_id: target.id,
                product_id: target.product_id().ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(format!(
                        "Matched component {} has no product_id",
                        target.name
                    )))
                })?,
                price_id,
                name: target.name.clone(),
                seats: explicit_params
                    .and_then(|params| params.initial_slot_count)
                    .filter(|_| matches!(fee, SubscriptionFee::Slot { .. })),
                priced,
                fee,
                period,
            });
            continue;
        }

        // No match found — this is a new component
        let (period, fee, price_id) = resolve_target_fee(
            target,
            units,
            explicit_params,
            Ported::subscription_cadence(current_components),
        )?;
        let (fee, priced) = at_target_price(fee, target, prices)?;
        let price_id = price_id.filter(|_| !priced);

        mappings.push(ComponentMapping::Added {
            target_component_id: target.id,
            product_id: target.product_id(),
            price_id,
            name: target.name.clone(),
            seats: explicit_params
                .and_then(|params| params.initial_slot_count)
                .filter(|_| matches!(fee, SubscriptionFee::Slot { .. })),
            fee,
            period,
        });
    }

    // Current components not matched → removed
    for current in current_components {
        if !matched_current_ids.contains(&current.id) {
            mappings.push(ComponentMapping::Removed {
                current_component_id: current.id,
            });
        }
    }

    Ok(mappings)
}

fn same_fee(current: &SubscriptionFee, new: &SubscriptionFee) -> bool {
    matches!(
        (serde_json::to_value(current), serde_json::to_value(new)),
        (Ok(current), Ok(new)) if current == new
    )
}

#[allow(clippy::too_many_arguments)]
fn build_plan_change_preview(
    current_components: &[SubscriptionComponent],
    target_components: &[TargetComponent],
    units: &crate::domain::credit_units::CreditUnitsById,
    component_params: &[ComponentParameterization],
    prices: &[TargetPrice],
    effective_date: chrono::NaiveDate,
    component_ids: &HashMap<PriceComponentId, SubscriptionPriceComponentId>,
) -> StoreResult<PlanChangePreview> {
    let params_by_id: HashMap<PriceComponentId, &ComponentParameters> = component_params
        .iter()
        .map(|p| (p.component_id, &p.parameters))
        .collect();

    let mut matched = Vec::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut matched_current_ids = std::collections::HashSet::new();

    for target in target_components {
        let explicit_params = params_by_id.get(&target.id).copied();

        let matched_current = target.matching(current_components);

        if let Some(current) = matched_current {
            matched_current_ids.insert(current.id);

            let (new_period, new_fee, _) =
                resolve_target_fee(target, units, explicit_params, Some(Ported::of(current)))?;
            let (new_fee, _) = at_target_price(new_fee, target, prices)?;

            matched.push(MatchedComponent {
                product_id: target.product_id().ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(format!(
                        "Matched component {} has no product_id",
                        target.name
                    )))
                })?,
                current_name: current.name.clone(),
                current_fee: current.fee.clone(),
                current_period: current.period,
                new_name: target.name.clone(),
                new_fee,
                new_period,
                current_net_key: Some(current.lineage().to_string()),
                billed_component_id: component_ids.get(&target.id).copied(),
                price_component_id: Some(target.id),
            });
            continue;
        }

        let (period, fee, _) = resolve_target_fee(
            target,
            units,
            explicit_params,
            Ported::subscription_cadence(current_components),
        )?;
        let (fee, _) = at_target_price(fee, target, prices)?;

        let instance_quantity = crate::services::subscriptions::proration::fee_instance_count(&fee);
        added.push(AddedComponent {
            name: target.name.clone(),
            fee,
            period,
            net_key: None,
            billed_component_id: component_ids.get(&target.id).copied(),
            billed_add_on_id: None,
            instance_quantity: Some(instance_quantity),
            price_component_id: Some(target.id),
        });
    }

    for current in current_components {
        if !matched_current_ids.contains(&current.id) {
            removed.push(RemovedComponent {
                name: current.name.clone(),
                current_fee: current.fee.clone(),
                current_period: current.period,
                net_key: Some(current.lineage().to_string()),
            });
        }
    }

    Ok(PlanChangePreview {
        matched,
        added,
        removed,
        effective_date,
    })
}

/// The managed quantities the subscription's seat fees count, plan charges and add-ons.
pub(crate) fn held_seat_quantities(
    details: &crate::domain::SubscriptionDetails,
) -> std::collections::HashSet<common_domain::ids::ManagedQuantityId> {
    details
        .price_components
        .iter()
        .map(|c| &c.fee)
        .chain(details.add_ons.iter().map(|a| &a.fee))
        .filter_map(SubscriptionFee::seat_quantity)
        .collect()
}

/// What a plan change writes for a target's seat fee: nothing when the subscription holds its
/// quantity already and the change sets no count, its seats carrying whatever charge or product
/// counted them (R-108, R-137); the count the change sets; else the fee's initial count. A fee
/// naming no quantity (a v1 target) carries when `matched`, as before quantities.
pub(crate) fn plan_change_seat_seed(
    fee: &SubscriptionFee,
    sets: Option<u32>,
    matched: bool,
    held: &std::collections::HashSet<common_domain::ids::ManagedQuantityId>,
    on: NaiveDate,
) -> Option<crate::domain::slot_transactions::SlotTransactionNewInternal> {
    let mut seed = crate::domain::slot_transactions::SlotTransactionNewInternal::from_fee(fee, on)?;
    let carried = match fee.seat_quantity() {
        Some(quantity) => held.contains(&quantity),
        None => matched,
    };
    match (carried, sets) {
        (true, None) => None,
        (_, Some(count)) => {
            seed.prev_active_slots = i32::try_from(count).unwrap_or(i32::MAX);
            Some(seed)
        }
        (false, None) => Some(seed),
    }
}

/// Resolves actual slot counts from `slot_transaction` into the preview's fees.
///
/// `initial_slots` in a `SubscriptionFee::Slot` is only a seed value from subscription creation.
/// The real slot count lives in the `slot_transaction` table. For proration and direction
/// detection we need the actual count, so we patch it into the fee before those calculations.
///
/// For matched components, both `current_fee` and `new_fee` are patched — the customer
/// keeps their seats across plan changes.
async fn resolve_preview_slot_counts(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    preview: &mut PlanChangePreview,
) -> StoreResult<()> {
    // The quantities held, each counted once
    let mut quantities: Vec<common_domain::ids::ManagedQuantityId> = Vec::new();
    for fee in preview
        .matched
        .iter()
        .map(|m| &m.current_fee)
        .chain(preview.removed.iter().map(|r| &r.current_fee))
    {
        if let Some(quantity) = fee.seat_quantity()
            && !quantities.contains(&quantity)
        {
            quantities.push(quantity);
        }
    }

    let mut counts: HashMap<common_domain::ids::ManagedQuantityId, u32> = HashMap::new();
    for quantity in quantities {
        let actual = SlotTransactionRow::fetch_active_slots(
            conn,
            tenant_id,
            subscription_id,
            quantity,
            common_clock::now().naive_utc(),
        )
        .await
        .map(|slots| slots.unwrap_or(0) as u32)
        .unwrap_or(0);
        counts.insert(quantity, actual);
    }

    // Patch fees: a target fee counting a quantity held keeps its seats (R-137)
    fn patch(
        fee: &mut SubscriptionFee,
        counts: &HashMap<common_domain::ids::ManagedQuantityId, u32>,
    ) {
        if let SubscriptionFee::Slot {
            quantity_id: Some(quantity),
            initial_slots,
            ..
        } = fee
            && let Some(&actual) = counts.get(quantity)
        {
            *initial_slots = actual;
        }
    }

    for m in &mut preview.matched {
        patch(&mut m.current_fee, &counts);
        patch(&mut m.new_fee, &counts);
    }
    for a in &mut preview.added {
        patch(&mut a.fee, &counts);
    }
    for r in &mut preview.removed {
        patch(&mut r.current_fee, &counts);
    }

    Ok(())
}

/// Calculates MRR for subscription components, using actual slot counts from `slot_transaction`
/// instead of the fee's `initial_slots` (which is just the seed value at creation time).
///
/// `get_subscription_details_with_conn` resolves V2 fees from product/price definitions,
/// which always returns `initial_slots = min_slots`. For correct MRR we need the current
/// slot count from the `slot_transaction` ledger.
pub(crate) async fn calculate_components_mrr_with_slots(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    components: &[crate::domain::subscription_components::SubscriptionComponent],
    precision: u8,
) -> StoreResult<Vec<crate::services::subscriptions::utils::MrrLine>> {
    calculate_components_mrr_with_counts(
        conn,
        tenant_id,
        subscription_id,
        components,
        precision,
        HashMap::new(),
    )
    .await
}

/// [`calculate_components_mrr_with_slots`], the quantities of `counts` held at those counts
/// instead of the ledger's: a change previewed before it writes them.
pub(crate) async fn calculate_components_mrr_with_counts(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    components: &[crate::domain::subscription_components::SubscriptionComponent],
    precision: u8,
    counts: HashMap<common_domain::ids::ManagedQuantityId, i64>,
) -> StoreResult<Vec<crate::services::subscriptions::utils::MrrLine>> {
    use crate::services::subscriptions::utils::{
        MrrLine, calculate_mrr, fee_period_total_cents, monthly_share,
    };

    // Query the count of each quantity held once
    let mut slot_counts: HashMap<common_domain::ids::ManagedQuantityId, i64> = counts;
    for c in components {
        if let Some(quantity) = c.fee.seat_quantity()
            && !slot_counts.contains_key(&quantity)
        {
            let count = SlotTransactionRow::fetch_active_slots(
                conn,
                tenant_id,
                subscription_id,
                quantity,
                common_clock::now().naive_utc(),
            )
            .await
            .map(|slots| i64::from(slots.unwrap_or(0)))
            .map_err(Into::<Report<StoreError>>::into)?;
            slot_counts.insert(quantity, count);
        }
    }

    let mut lines = Vec::with_capacity(components.len());
    for c in components {
        let (monthly, period_total) = match &c.fee {
            SubscriptionFee::Slot { model, .. } => {
                let count = c
                    .fee
                    .seat_quantity()
                    .and_then(|quantity| slot_counts.get(&quantity).copied())
                    .unwrap_or(0);
                let total = model.period_total_minor(u64::try_from(count).unwrap_or(0), precision);
                // Rounded through the same `monthly_share` `calculate_mrr` uses: this
                // branch recomputes what creation computed, and a figure truncated here
                // against a half-up one there books a phantom one-cent contraction on the
                // first amendment.
                (monthly_share(total, i64::from(c.period.as_months())), total)
            }
            _ => (
                calculate_mrr(&c.fee, &c.period, precision),
                fee_period_total_cents(&c.fee, precision),
            ),
        };
        lines.push(MrrLine::new(&c.fee, monthly, c.product_id).with_period_total(period_total));
    }

    Ok(lines)
}

/// The live count of each of `quantities` the subscription holds, from the ledger; a quantity
/// with no seat transaction yet is left out.
pub(crate) async fn live_seat_counts(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    quantities: Vec<common_domain::ids::ManagedQuantityId>,
) -> StoreResult<HashMap<common_domain::ids::ManagedQuantityId, u32>> {
    let now = common_clock::now().naive_utc();
    let mut counts = HashMap::new();
    for quantity in quantities {
        if counts.contains_key(&quantity) {
            continue;
        }
        let count =
            SlotTransactionRow::fetch_active_slots(conn, tenant_id, subscription_id, quantity, now)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        if let Some(count) = count {
            counts.insert(quantity, u32::try_from(count).unwrap_or(0));
        }
    }
    Ok(counts)
}

/// A seat fee at its quantity's live count in `counts`, when held: what it bills, never the
/// seats it was added with.
pub(crate) fn at_live_count(
    fee: &mut SubscriptionFee,
    counts: &HashMap<common_domain::ids::ManagedQuantityId, u32>,
) {
    if let SubscriptionFee::Slot {
        quantity_id: Some(quantity),
        initial_slots,
        ..
    } = fee
        && let Some(count) = counts.get(quantity)
    {
        *initial_slots = *count;
    }
}

/// `add_ons` with each seat add-on's fee at its quantity's live count, as
/// `calculate_components_mrr_with_slots` reads the components': a seat add-on bills the count it
/// shares with every charge counting its quantity (R-137, R-138), whatever it was added with.
pub(crate) async fn add_ons_at_live_seats(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    add_ons: &[SubscriptionAddOn],
) -> StoreResult<Vec<SubscriptionAddOn>> {
    let quantities = add_ons
        .iter()
        .filter_map(|a| a.fee.seat_quantity())
        .collect();
    let counts = live_seat_counts(conn, tenant_id, subscription_id, quantities).await?;
    Ok(add_ons
        .iter()
        .map(|a| {
            let mut a = a.clone();
            at_live_count(&mut a.fee, &counts);
            a
        })
        .collect())
}

/// On plan change, refresh the `subscription_minimum` snapshot from the new plan version's
/// `plan_version_minimum`. **Preserves operator/quote overrides** (`is_override = true`):
/// those represent negotiated contracts and survive plan changes.
///
/// Behaviour matrix:
///
/// | existing snapshot       | new plan has min | result                                       |
/// |-------------------------|------------------|----------------------------------------------|
/// | none                    | none             | nothing                                      |
/// | none                    | yes              | new snapshot, is_override = false            |
/// | plan-snapshot           | none             | snapshot deleted (no floor on new plan)      |
/// | plan-snapshot           | yes              | snapshot replaced from new plan              |
/// | override                | -                | left untouched                               |
pub(crate) async fn refresh_subscription_minimum_on_plan_change(
    conn: &mut crate::store::PgConn,
    subscription_id: common_domain::ids::SubscriptionId,
    new_plan_version_id: common_domain::ids::PlanVersionId,
    on: NaiveDate,
) -> StoreResult<()> {
    use diesel_models::plan_version_adjustments::{
        PlanVersionAdjustmentComponentRow, PlanVersionAdjustmentRow,
    };
    use diesel_models::subscription_adjustments::{
        SubscriptionAdjustmentComponentRow, SubscriptionAdjustmentComponentRowNew,
        SubscriptionAdjustmentRow, SubscriptionAdjustmentRowNew,
    };

    let existing = SubscriptionAdjustmentRow::find_by_subscription_id(conn, &subscription_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    if matches!(&existing, Some(e) if e.is_override) {
        return Ok(());
    }

    // The plan-snapshot ends with the change: the new plan might have another or none.
    if existing.is_some() {
        SubscriptionAdjustmentRow::end_current(conn, &subscription_id, on)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }

    // Snapshot from the new plan version, if any.
    let Some(plan_min_row) =
        PlanVersionAdjustmentRow::find_by_plan_version_id(conn, &new_plan_version_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
    else {
        return Ok(());
    };
    // Never a minimum in another currency than the subscription's (refused or dropped before).
    {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::errors::IntoDbResult;
        use diesel_models::schema::{plan_version, subscription};
        use error_stack::ResultExt;
        let sub_currency: String = subscription::table
            .filter(subscription::id.eq(subscription_id))
            .select(subscription::currency)
            .first(conn)
            .await
            .attach("Error while reading the subscription's currency")
            .into_db_result()
            .map_err(Into::<Report<StoreError>>::into)?;
        let version_currency: String = plan_version::table
            .filter(plan_version::id.eq(new_plan_version_id))
            .select(plan_version::currency)
            .first(conn)
            .await
            .attach("Error while reading the plan version's currency")
            .into_db_result()
            .map_err(Into::<Report<StoreError>>::into)?;
        if !sub_currency.eq_ignore_ascii_case(&version_currency) {
            return Ok(());
        }
    }

    SubscriptionAdjustmentRowNew {
        subscription_id,
        kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
        amount_subunit: plan_min_row.amount_subunit,
        includes_all_components: plan_min_row.includes_all_components,
        is_override: false,
        effective_from: on,
    }
    .insert(conn)
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    if !plan_min_row.includes_all_components {
        let plan_components =
            PlanVersionAdjustmentComponentRow::list_by_plan_version_id(conn, &new_plan_version_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        let sub_components: Vec<SubscriptionAdjustmentComponentRowNew> = plan_components
            .into_iter()
            .map(|c| SubscriptionAdjustmentComponentRowNew {
                subscription_id,
                kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                product_id: c.product_id,
                effective_from: on,
            })
            .collect();
        SubscriptionAdjustmentComponentRow::insert_batch(conn, &sub_components)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }

    Ok(())
}

/// The packs `lines` bill, as a change's summary shows them bought.
fn pack_charges(
    lines: &[crate::domain::LineItem],
    precision: u8,
) -> Vec<crate::domain::subscription_changes::PackChange> {
    use crate::domain::subscription_changes::{PackChange, PackChangeKind};
    lines
        .iter()
        .filter(|line| line.kind == crate::domain::LineItemKind::Credit)
        .map(|line| PackChange {
            name: line.name.clone(),
            kind: PackChangeKind::Charge,
            amount_cents: line.amount_subtotal,
            credit_minor: line.credit_grant_amount.unwrap_or(0),
            credit_unit: line.credit_grant_unit.clone(),
            credit_precision: line
                .credit_grant_unit
                .as_ref()
                .map_or(precision, |unit| unit.precision),
            promotional: line.credit_grant_included,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::enums::BillingPeriodEnum;
    use common_domain::ids::{BaseId, PriceId, TenantId};

    fn rate_price(cadence: BillingPeriodEnum) -> Price {
        Price {
            id: PriceId::new(),
            product_id: ProductId::new(),
            cadence,
            currency: "USD".to_owned(),
            pricing: Pricing::Rate {
                rate: rust_decimal::Decimal::TEN,
            },
            tenant_id: TenantId::new(),
            created_at: common_clock::now().naive_utc(),
            archived_at: None,
            catalog: true,
            credit_unit: None,
        }
    }

    #[test]
    fn a_new_product_with_several_cadences_takes_the_subscription_one() {
        let prices = [
            rate_price(BillingPeriodEnum::Monthly),
            rate_price(BillingPeriodEnum::Annual),
        ];
        assert!(select_target_price(&prices, None, None, "Support").is_err());

        let monthly = Ported {
            period: SubscriptionFeeBillingPeriod::Monthly,
            capacity: None,
        };
        let price = select_target_price(&prices, None, Some(monthly), "Support").unwrap();
        assert_eq!(price.cadence, BillingPeriodEnum::Monthly);
    }

    fn seats(quantity: Option<common_domain::ids::ManagedQuantityId>) -> SubscriptionFee {
        SubscriptionFee::Slot {
            unit: "Seats".to_owned(),
            quantity_id: quantity,
            model: crate::domain::price_components::RatingModel::PerUnit {
                rate: rust_decimal::Decimal::TEN,
            },
            min_slots: Some(1),
            max_slots: None,
            initial_slots: 1,
        }
    }

    /// A target seat fee of a quantity held carries its count, unless the change sets one; one of
    /// another quantity starts at its initial count. A fee naming no quantity reads as before
    /// quantities: matched it carries, added it starts anew.
    /// Rules: R-108, R-137
    #[test]
    fn a_target_seat_fee_carries_the_count_of_a_quantity_held() {
        use common_domain::ids::ManagedQuantityId;
        let on = chrono::NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
        let (held, other) = (ManagedQuantityId::new(), ManagedQuantityId::new());
        let holding = std::collections::HashSet::from([held]);

        assert!(plan_change_seat_seed(&seats(Some(held)), None, false, &holding, on).is_none());
        assert!(plan_change_seat_seed(&seats(Some(held)), None, true, &holding, on).is_none());
        let set = plan_change_seat_seed(&seats(Some(held)), Some(7), false, &holding, on)
            .expect("a count set");
        assert_eq!((set.quantity_id, set.prev_active_slots), (Some(held), 7));
        let fresh = plan_change_seat_seed(&seats(Some(other)), None, true, &holding, on)
            .expect("another quantity starts anew");
        assert_eq!(
            (fresh.quantity_id, fresh.prev_active_slots),
            (Some(other), 1)
        );

        assert!(plan_change_seat_seed(&seats(None), None, true, &holding, on).is_none());
        assert!(plan_change_seat_seed(&seats(None), None, false, &holding, on).is_some());
        assert!(
            plan_change_seat_seed(
                &SubscriptionFee::Rate {
                    rate: rust_decimal::Decimal::TEN
                },
                None,
                false,
                &holding,
                on
            )
            .is_none(),
            "no seats, no seed"
        );
    }
}
