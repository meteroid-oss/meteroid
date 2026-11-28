//! Phases at offsets from a subscription's billing start: a plan version's template (R-154) and a
//! quote's phases (R-155). Creating the subscription writes them: the phase at offset 0 prices the
//! fees it starts with, each later one is a phase of its own on its renewal. Terms are absolute
//! over the rows it starts with, so what offset 0 prices every later phase restates.
use chrono::{Datelike, Months};
use common_domain::actor::Actor;
use common_domain::ids::{PlanVersionId, PriceComponentId, QuoteId, TenantId};
use diesel_models::PgConn;
use diesel_models::plans::PlanRow;
use diesel_models::price_components::PriceComponentRow;
use diesel_models::subscriptions::SubscriptionRowNew;
use diesel_models::trial_offers::{PlanVersionPhaseRow, QuotePhaseRow, QuoteTermRow};
use error_stack::Report;
use std::collections::HashMap;

use crate::StoreResult;
use crate::domain::quotes::DetailedQuote;
use crate::domain::subscription_components::SubscriptionFee;
use crate::domain::subscription_phases::{PhaseMinimum, QuoteTerm, SubscriptionPhaseInput};
use crate::domain::trial_offers::PlanPhaseTemplate;
use crate::errors::StoreError;
use crate::services::Services;
use scoped_futures::ScopedFutureExt;

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::into)
}

fn invalid(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

fn template_of(
    offset_months: i32,
    label: Option<String>,
    terms: serde_json::Value,
) -> StoreResult<PlanPhaseTemplate> {
    Ok(PlanPhaseTemplate {
        offset_months: u32::try_from(offset_months).unwrap_or_default(),
        label,
        terms: serde_json::from_value(terms)
            .map_err(|error| Report::new(StoreError::SerdeError("phase terms".into(), error)))?,
    })
}

fn terms_json(phase: &PlanPhaseTemplate) -> StoreResult<serde_json::Value> {
    serde_json::to_value(&phase.terms)
        .map_err(|error| Report::new(StoreError::SerdeError("phase terms".into(), error)))
}

fn seq_of(seq: usize) -> i32 {
    i32::try_from(seq).unwrap_or(i32::MAX)
}

impl Services {
    pub(in crate::services) async fn list_plan_version_phases(
        &self,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
    ) -> StoreResult<Vec<PlanPhaseTemplate>> {
        let mut conn = self.store.get_conn().await?;
        template_tx(&mut conn, tenant_id, plan_version_id).await
    }

    /// Replaces the plan version's template; subscriptions already created keep their phases.
    pub(in crate::services) async fn set_plan_version_phases(
        &self,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
        mut phases: Vec<PlanPhaseTemplate>,
    ) -> StoreResult<Vec<PlanPhaseTemplate>> {
        let mut conn = self.store.get_conn().await?;
        for phase in &mut phases {
            crate::services::subscriptions::phases::name_seats_tx(
                &mut conn,
                tenant_id,
                &mut phase.terms.min_seats,
            )
            .await?;
        }
        let components = {
            use crate::repositories::price_components::PriceComponentInterface;
            self.store
                .list_price_components_with_conn(&mut conn, plan_version_id, tenant_id)
                .await?
        };
        let currencies = crate::domain::price_cells::currencies(&components);
        validate_tx(&mut conn, tenant_id, plan_version_id, &mut phases, false).await?;
        on_every_period(&phases, &template_fees(&components))?;
        if currencies.len() > 1
            && phases
                .iter()
                .flat_map(|phase| &phase.terms.components)
                .any(|component| component.list_rate.is_none())
        {
            return Err(invalid(format!(
                "the plan is sold in {}: a template prices each fee as a share of its list price",
                currencies.join(", ")
            )));
        }
        let now = common_clock::now().naive_utc();
        let mut rows = Vec::new();
        for (seq, phase) in phases.iter().enumerate() {
            rows.push(PlanVersionPhaseRow {
                id: uuid::Uuid::now_v7(),
                tenant_id,
                plan_version_id,
                seq: seq_of(seq),
                offset_months: i32::try_from(phase.offset_months).unwrap_or(i32::MAX),
                label: phase.label.clone(),
                terms: terms_json(phase)?,
                created_at: now,
            });
        }
        db(PlanVersionPhaseRow::replace_for_version(
            &mut conn,
            &tenant_id,
            plan_version_id,
            &rows,
        )
        .await)?;
        Ok(phases)
    }

    pub(in crate::services) async fn get_quote_term(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Option<QuoteTerm>> {
        let mut conn = self.store.get_conn().await?;
        Ok(
            db(QuoteTermRow::find(&mut conn, tenant_id, quote_id).await)?
                .as_ref()
                .map(QuoteTerm::from_row),
        )
    }

    /// Replaces a draft quote's contract term, or removes it, and the quote's document with it:
    /// what a recipient signs states its term (ADR-0022 §2).
    pub(in crate::services) async fn set_quote_term(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        term: Option<QuoteTerm>,
    ) -> StoreResult<Option<QuoteTerm>> {
        self.store
            .transaction(|conn| {
                async move {
                    let quote = db(diesel_models::quotes::QuoteRow::find_by_id_for_update(
                        conn, tenant_id, quote_id,
                    )
                    .await)?;
                    if quote.status != diesel_models::enums::QuoteStatusEnum::Draft {
                        return Err(invalid(
                            "a quote's term is set while it is a draft: what was sent is signed as it was",
                        ));
                    }
                    let mut detailed =
                        crate::repositories::quotes::detailed_quote_tx(conn, tenant_id, quote_id)
                            .await?;
                    detailed.term = term.clone();
                    check_quote_term(&detailed, false)?;
                    let row = term.as_ref().map(|term| QuoteTermRow {
                        quote_id,
                        tenant_id,
                        term_months: i32::try_from(term.term_months).unwrap_or(i32::MAX),
                        auto_renew: term.auto_renew,
                        renewal_months: i32::try_from(term.renewal_months).unwrap_or(i32::MAX),
                        notice_days: i32::try_from(term.notice_days).unwrap_or(i32::MAX),
                        uplift_percent: term.uplift_percent,
                        updated_at: common_clock::now().naive_utc(),
                    });
                    db(QuoteTermRow::replace(conn, tenant_id, quote_id, row.as_ref()).await)?;
                    crate::repositories::quote_signing::refresh_quote_document_tx(
                        conn, tenant_id, quote_id,
                    )
                    .await?;
                    Ok(term)
                }
                .scope_boxed()
            })
            .await
    }

    pub(in crate::services) async fn list_quote_phases(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Vec<PlanPhaseTemplate>> {
        let mut conn = self.store.get_conn().await?;
        quote_phases_tx(&mut conn, tenant_id, quote_id).await
    }

    /// Replaces a draft quote's phases, written on the subscription it converts to, and the
    /// quote's document with them: what a recipient signs states its ramp (R-155).
    pub(in crate::services) async fn set_quote_phases(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        phases: Vec<PlanPhaseTemplate>,
    ) -> StoreResult<Vec<PlanPhaseTemplate>> {
        self.store
            .transaction(|conn| {
                async move {
                    let mut phases = phases;
                    for phase in &mut phases {
                        crate::services::subscriptions::phases::name_seats_tx(
                            conn,
                            tenant_id,
                            &mut phase.terms.min_seats,
                        )
                        .await?;
                    }
                    let quote = db(diesel_models::quotes::QuoteRow::find_by_id_for_update(
                        conn, tenant_id, quote_id,
                    )
                    .await)?;
                    if quote.status != diesel_models::enums::QuoteStatusEnum::Draft {
                        return Err(invalid(
                            "a quote's phases are set while it is a draft: what was sent is signed as it was",
                        ));
                    }
                    validate_tx(conn, tenant_id, quote.plan_version_id, &mut phases, true).await?;
                    let detailed =
                        crate::repositories::quotes::detailed_quote_tx(conn, tenant_id, quote_id)
                            .await?;
                    let fees: Vec<_> = detailed
                        .components
                        .iter()
                        .map(|c| (c.price_component_id, c.list_fee.as_ref().unwrap_or(&c.fee)))
                        .collect();
                    price_list_rates(
                        &mut phases,
                        &fees,
                        crate::domain::subscription_phases::precision_of(&detailed.quote.currency),
                    )?;
                    check_quote_schedule(&detailed, &phases)?;
                    let now = common_clock::now().naive_utc();
                    let mut rows = Vec::new();
                    for (seq, phase) in phases.iter().enumerate() {
                        rows.push(QuotePhaseRow {
                            id: uuid::Uuid::now_v7(),
                            tenant_id,
                            quote_id,
                            seq: seq_of(seq),
                            offset_months: i32::try_from(phase.offset_months).unwrap_or(i32::MAX),
                            label: phase.label.clone(),
                            terms: terms_json(phase)?,
                            created_at: now,
                        });
                    }
                    db(QuotePhaseRow::replace_for_quote(conn, &tenant_id, quote_id, &rows).await)?;
                    crate::repositories::quote_signing::refresh_quote_document_tx(
                        conn, tenant_id, quote_id,
                    )
                    .await?;
                    Ok(phases)
                }
                .scope_boxed()
            })
            .await
    }
}

/// A fee by plan component: its name, the months of each cadence it bills on, and whether it
/// counts seats (a phase's floor changes it).
struct PeriodicFee {
    component: Option<PriceComponentId>,
    name: String,
    months: Vec<u32>,
    seats: bool,
}

fn template_fees(
    components: &[crate::domain::price_components::PriceComponent],
) -> Vec<PeriodicFee> {
    components
        .iter()
        .map(|component| {
            let mut months: Vec<u32> = component
                .prices
                .iter()
                .filter(|price| !crate::domain::price_cells::cadence_free(&price.pricing))
                .map(|price| price.cadence.as_months())
                .collect();
            months.sort_unstable();
            months.dedup();
            PeriodicFee {
                component: Some(component.id),
                name: component.name.clone(),
                months,
                seats: matches!(
                    component.fee_structure,
                    Some(crate::domain::prices::FeeStructure::Slot { .. })
                ),
            }
        })
        .collect()
}

fn quote_fees(quote: &DetailedQuote) -> Vec<PeriodicFee> {
    quote
        .components
        .iter()
        .map(|component| PeriodicFee {
            component: component.price_component_id,
            name: component.name.clone(),
            months: u32::try_from(component.period.as_months())
                .ok()
                .filter(|months| *months > 0)
                .into_iter()
                .collect(),
            seats: matches!(component.fee, SubscriptionFee::Slot { .. }),
        })
        .collect()
}

/// A phase at an offset starts on a renewal of every fee it changes, whatever cadence it is sold
/// on: its offset is a whole number of each of their periods (R-154). A phase from a switch on
/// prices another plan's fees, checked when it is written.
fn on_every_period(phases: &[PlanPhaseTemplate], fees: &[PeriodicFee]) -> StoreResult<()> {
    for phase in phases
        .iter()
        .take_while(|phase| phase.terms.plan.is_none())
        .filter(|phase| phase.offset_months > 0)
    {
        let changed = fees.iter().filter(|fee| {
            (fee.seats && !phase.terms.min_seats.is_empty())
                || phase
                    .terms
                    .components
                    .iter()
                    .any(|c| c.plan_component.is_some() && c.plan_component == fee.component)
        });
        for fee in changed {
            if let Some(months) = fee
                .months
                .iter()
                .find(|months| !phase.offset_months.is_multiple_of(**months))
            {
                return Err(invalid(format!(
                    "{} is billed every {months} months: a phase at {} months does not start one of its periods",
                    fee.name, phase.offset_months
                )));
            }
        }
    }
    Ok(())
}

/// What a quote's phases need for the subscription it converts to to bill them as signed: each
/// on a renewal of the fees it changes, and renewals on the billing start's day (R-155).
pub(crate) fn check_quote_schedule(
    quote: &DetailedQuote,
    phases: &[PlanPhaseTemplate],
) -> StoreResult<()> {
    if phases.iter().all(|phase| phase.offset_months == 0) {
        return Ok(());
    }
    on_every_period(phases, &quote_fees(quote))?;
    let quote = &quote.quote;
    if let Some(anchor) = quote.billing_day_anchor {
        let on_start = quote.trial_duration_days == Some(0)
            && quote
                .billing_start_date
                .is_some_and(|start| i32::try_from(start.day()).ok() == Some(anchor));
        if !on_start {
            return Err(invalid(
                "a quote with phases renews on its billing start's day: remove its billing day anchor",
            ));
        }
    }
    Ok(())
}

/// What a quote's term needs for the subscription it converts to to renew as signed (ADR-0022
/// §2): its ends on a renewal of every recurring fee, on the billing start's day, after today.
pub(crate) fn check_quote_term(quote: &DetailedQuote, sending: bool) -> StoreResult<()> {
    let Some(term) = &quote.term else {
        return Ok(());
    };
    let q = &quote.quote;
    if q.billing_end_date.is_some() {
        return Err(invalid(
            "a quote with a contract term ends with it: remove its end date",
        ));
    }
    if !(1..=120).contains(&term.term_months) || !(1..=120).contains(&term.renewal_months) {
        return Err(invalid("a term and its renewal last 1 to 120 months"));
    }
    for fee in quote_fees(quote) {
        for months in &fee.months {
            for (what, length) in [("term", term.term_months), ("renewal", term.renewal_months)] {
                if !length.is_multiple_of(*months) {
                    return Err(invalid(format!(
                        "{} is billed every {months} months: a {length}-month {what} does not end one of its periods",
                        fee.name
                    )));
                }
            }
        }
    }
    if let Some(anchor) = q.billing_day_anchor {
        let on_start = q
            .billing_start_date
            .is_some_and(|start| i32::try_from(start.day()).ok() == Some(anchor));
        if !on_start {
            return Err(invalid(
                "a quote with a contract term renews on its billing start's day: remove its billing day anchor",
            ));
        }
    }
    if u64::from(term.notice_days) >= u64::from(term.term_months) * 28 {
        return Err(invalid("the notice is shorter than the term"));
    }
    if sending
        && let Some(start) = q.billing_start_date
        && start
            .checked_add_months(Months::new(term.term_months))
            .is_some_and(|end| end <= common_clock::now().date_naive())
    {
        return Err(invalid("the contract term has already ended"));
    }
    Ok(())
}

async fn components_of(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_version_id: PlanVersionId,
) -> StoreResult<HashMap<PriceComponentId, String>> {
    Ok(
        db(PriceComponentRow::list_by_plan_version_id(conn, tenant_id, plan_version_id).await)?
            .into_iter()
            .map(|component| (component.id, component.name))
            .collect(),
    )
}

/// Phases at offsets, fees by plan component: of the plan version, or of a plan a phase before
/// switched to (a quote's only). What offset 0 prices and floors, every later phase restates.
async fn validate_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_version_id: PlanVersionId,
    phases: &mut [PlanPhaseTemplate],
    allow_switch: bool,
) -> StoreResult<()> {
    let mut components = components_of(conn, tenant_id, plan_version_id).await?;
    let own: Vec<PriceComponentId> = components.keys().copied().collect();
    let mut previous: Option<u32> = None;
    for phase in phases.iter_mut() {
        if previous.is_some_and(|offset| offset >= phase.offset_months) {
            return Err(invalid("phases at offsets start one after another"));
        }
        previous = Some(phase.offset_months);
        let terms = &mut phase.terms;
        if terms.prorated {
            return Err(invalid("phases at offsets start on renewals"));
        }
        if let Some(plan) = &mut terms.plan {
            if !allow_switch || phase.offset_months == 0 {
                return Err(invalid(
                    "a plan's template never switches plan, and nothing switches at offset 0",
                ));
            }
            plan.name = Some(
                db(PlanRow::get_with_version(conn, plan.plan_version_id, tenant_id).await)?
                    .plan
                    .name,
            );
            components.extend(components_of(conn, tenant_id, plan.plan_version_id).await?);
        }
        for component in &mut terms.components {
            if let Some(rate) = component.list_rate {
                crate::services::subscriptions::phases::validate_price(rate)?;
                if !component.plan_component.is_some_and(|id| own.contains(&id)) {
                    return Err(invalid(
                        "a share of the list price prices a fee of the template's own plan",
                    ));
                }
                component.price = rust_decimal::Decimal::ZERO;
            }
            crate::services::subscriptions::phases::validate_price(component.price)?;
            match component.plan_component.and_then(|id| components.get(&id)) {
                Some(name) if component.lineage.is_none() => component.name = Some(name.clone()),
                _ => {
                    return Err(invalid(
                        "a phase at an offset names each fee by a plan component of its plan",
                    ));
                }
            }
        }
    }
    if let Some(first) = phases.first().filter(|phase| phase.offset_months == 0) {
        if first.terms.minimum != PhaseMinimum::Kept {
            return Err(invalid(
                "the plan's minimum is its own: a later phase sets another",
            ));
        }
        for later in phases[1..]
            .iter()
            .take_while(|phase| phase.terms.plan.is_none())
        {
            let restates = first.terms.components.iter().all(|c| {
                later
                    .terms
                    .components
                    .iter()
                    .any(|l| l.plan_component == c.plan_component)
            }) && first.terms.min_seats.iter().all(|s| {
                later
                    .terms
                    .min_seats
                    .iter()
                    .any(|l| l.quantity_id == s.quantity_id)
            });
            if !restates {
                return Err(invalid(format!(
                    "the phase at {} months restates what the first prices: terms are absolute",
                    later.offset_months
                )));
            }
        }
    }
    Ok(())
}

/// The plan version's template, in order.
pub(crate) async fn template_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_version_id: PlanVersionId,
) -> StoreResult<Vec<PlanPhaseTemplate>> {
    db(PlanVersionPhaseRow::list_for_version(conn, &tenant_id, plan_version_id).await)?
        .into_iter()
        .map(|row| template_of(row.offset_months, row.label, row.terms))
        .collect()
}

/// The quote's phases, in order.
pub(crate) async fn quote_phases_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    quote_id: QuoteId,
) -> StoreResult<Vec<PlanPhaseTemplate>> {
    db(QuotePhaseRow::list_for_quote(conn, &tenant_id, quote_id).await)?
        .into_iter()
        .map(|row| template_of(row.offset_months, row.label, row.terms))
        .collect()
}

/// Each of `fees` (by plan component) as the phase at offset 0 of `phases` prices and floors it:
/// none for one it leaves as it is. Refused when that phase names a plan component none of them
/// has, or prices one by tiers or packages, which one amount cannot (R-154, R-155).
pub(crate) fn priced_at_start(
    phases: &[PlanPhaseTemplate],
    fees: &[(Option<PriceComponentId>, &SubscriptionFee)],
    precision: u32,
) -> StoreResult<Vec<Option<SubscriptionFee>>> {
    let Some(first) = phases.iter().find(|phase| phase.offset_months == 0) else {
        return Ok(vec![None; fees.len()]);
    };
    let mut first = [first.clone()];
    price_list_rates(&mut first, fees, precision)?;
    let terms = &first[0].terms;
    if let Some(missing) = terms
        .components
        .iter()
        .filter_map(|c| c.plan_component)
        .find(|id| !fees.iter().any(|(component, _)| *component == Some(*id)))
    {
        return Err(invalid(format!(
            "the first phase prices plan component {missing}, which the subscription does not have"
        )));
    }
    fees.iter()
        .map(|(component, fee)| {
            let price = terms
                .components
                .iter()
                .find(|c| c.plan_component.is_some() && c.plan_component == *component)
                .map(|c| c.price);
            let mut priced = match price {
                Some(price) => crate::domain::subscription_phases::repriced(fee, price)
                    .ok_or_else(|| {
                        invalid(format!(
                            "plan component {} is priced by tiers or packages: a phase sets one price",
                            component.map_or_else(String::new, |id| id.to_string())
                        ))
                    })?,
                None => (*fee).clone(),
            };
            let floor = terms.min_seats.iter().find(|s| s.counts(&priced)).cloned();
            if let SubscriptionFee::Slot { min_slots, .. } = &mut priced
                && let Some(floor) = floor
            {
                *min_slots = Some(min_slots.map_or(floor.slots, |min| min.max(floor.slots)));
            }
            Ok((serde_json::to_value(&priced).ok() != serde_json::to_value(*fee).ok())
                .then_some(priced))
        })
        .collect()
}

/// Each share of a list price in `phases` as an amount: the share of the fee its plan component
/// lists in `fees`, which are in the subscription's or quote's currency (R-154).
pub(crate) fn price_list_rates(
    phases: &mut [PlanPhaseTemplate],
    fees: &[(Option<PriceComponentId>, &SubscriptionFee)],
    precision: u32,
) -> StoreResult<()> {
    for component in phases
        .iter_mut()
        .flat_map(|phase| &mut phase.terms.components)
    {
        let Some(rate) = component.list_rate.take() else {
            continue;
        };
        let id = component.plan_component;
        let fee = fees
            .iter()
            .find(|(of, _)| of.is_some() && *of == id)
            .map(|(_, fee)| *fee)
            .ok_or_else(|| {
                invalid(format!(
                    "a phase prices plan component {} as a share of its list price, which the subscription does not have",
                    id.map_or_else(String::new, |id| id.to_string())
                ))
            })?;
        let list = crate::domain::subscription_phases::price_of(fee).ok_or_else(|| {
            invalid(format!(
                "plan component {} is priced by tiers or packages: a phase sets one price",
                id.map_or_else(String::new, |id| id.to_string())
            ))
        })?;
        component.price = crate::domain::subscription_phases::scaled_price(list, rate, precision);
    }
    Ok(())
}

/// The later phases of a template or quote written for the subscription just inserted, on its
/// renewals from its billing start (a free trial's end).
pub(in crate::services) async fn materialize_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    subscription: &SubscriptionRowNew,
    free_trial_days: Option<u32>,
    phases: &[PlanPhaseTemplate],
) -> StoreResult<()> {
    let mut billing_start = subscription
        .billing_start_date
        .unwrap_or(subscription.start_date);
    if let Some(days) = free_trial_days {
        billing_start += chrono::Duration::days(i64::from(days));
    }
    let loaded = crate::services::evaluate::load::load_facts(
        conn,
        subscription.tenant_id,
        subscription.id,
        billing_start,
        None,
    )
    .await?;
    // After the phase a trial offer's fallback wrote, if any.
    let first_seq = crate::services::subscriptions::phases::list_phases_tx(
        conn,
        subscription.tenant_id,
        subscription.id,
    )
    .await?
    .len();
    for (seq, template) in phases
        .iter()
        .filter(|phase| phase.offset_months > 0)
        .enumerate()
    {
        let due = billing_start
            .checked_add_months(Months::new(template.offset_months))
            .ok_or_else(|| invalid("a phase's start is out of range"))?;
        // Its first renewal from then: a billing day anchor may move the subscription's renewals.
        let start = billing_core::evaluate::renewal_on_or_after(
            &loaded.facts,
            due,
            due.and_time(chrono::NaiveTime::MIN).and_utc(),
        )
        .unwrap_or(due);
        crate::services::subscriptions::phases::write_phase_tx(
            services,
            conn,
            actor,
            subscription.tenant_id,
            subscription.id,
            u32::try_from(first_seq + seq).unwrap_or(u32::MAX),
            &SubscriptionPhaseInput {
                start_date: start,
                label: template.label.clone(),
                terms: template.terms.clone(),
            },
        )
        .await?;
    }
    Ok(())
}
