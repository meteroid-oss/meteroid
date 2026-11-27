//! The changes of a subscription (ADR-0013 §10): a plan change, a seat count, a level, an
//! amendment and a cancellation, each with a `/preview` taking the same body and writing nothing.
//! Each runs on the store service the gRPC API runs it on; `when` is checked in `when.rs`.
use super::model::{
    AmendmentInput, CancelInput, Cancellation, ChangePreview, ChangeResult, ChangeStatus,
    Direction, DocumentType, LevelChangeInput, PlanChangeInput, PlanRef, PreviewPart,
    QuantityChangeInput, Schedule, SeatActivation, Subscription, SubscriptionCharge,
    SubscriptionStatus, WhenType,
};
use super::render::{self, AfterChange, ChargeSource};
use super::resolve::{
    cells, conflict, count, counts, not_found_at, option_named, override_price, own_charge,
    plan_charge, published_version, seat_delta, seats,
};
use super::when::{self, ChangeKind, Rules, Timing};
use crate::AppState;
use crate::v2::extract::{Body, Path};
use crate::v2::idempotency::{IDEMPOTENCY_KEY_HEADER, committed};
use crate::v2::ids;
use crate::v2::problem::{Problem, store_message};
use crate::v2::refs::{ReferenceIds, Refs};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use chrono::NaiveDate;
use common_domain::ids::{
    BaseId, CreditNoteId, InvoiceId, ProductId, ScheduledEventId, SubscriptionId, TenantId,
};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::HeaderMap;
use meteroid_app::features::ensure_credit_pricing_allowed;
use meteroid_store::domain::enums::ScheduledEventTypeEnum;
use meteroid_store::domain::managed_quantities::{ManagedQuantity, SeatChange};
use meteroid_store::domain::plan_options::PlanOptionRef;
use meteroid_store::domain::price_components::{PriceEntry, ProductRef};
use meteroid_store::domain::slot_transactions::SeatCharge;
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_amendment::{
    AddExtraComponent, AddOnChanges as DomainAddOnChanges, ComponentChanges, ComponentLevels,
    EditComponent, EditSubscriptionAddOn, LevelChangeOutcome, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::{PlanChangeMode, PlanChangePreview, PlanTarget};
use meteroid_store::domain::usage_levels::{LevelPick, plan_change_levels};
use meteroid_store::domain::{
    ComponentParameterization, ComponentParameters, FullPlan, SlotUpgradeBillingMode,
    SubscriptionComponent, SubscriptionDetails, SubscriptionFee,
};
use meteroid_store::repositories::products::ProductInterface;
use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionInterfaceAuto,
};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashMap, HashSet};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

// ── Shared ──────────────────────────────────────────────────────────

/// The subscription a path names, read whole.
async fn target(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<(SubscriptionId, SubscriptionDetails), Problem> {
    let id = ids::subscription_id(state, tenant_id, raw).await?;
    let details = state.store.get_subscription_details(tenant_id, id).await?;
    Ok((id, details))
}

/// A write that may invoice is sent with an Idempotency-Key: a retry must never apply it twice.
pub(super) fn require_key(headers: &HeaderMap, what: &str) -> Result<(), Problem> {
    if headers.contains_key(IDEMPOTENCY_KEY_HEADER) {
        return Ok(());
    }
    Err(Problem::key_required(format!(
        "{what} is sent with an Idempotency-Key: a retry must never apply it twice"
    )))
}

/// The subscription once changed, read whole; none when v2 cannot read it back (the change is
/// made all the same: a write committed never answers an error).
async fn read_back(
    state: &AppState,
    tenant_id: TenantId,
    id: SubscriptionId,
) -> Option<Subscription> {
    match render::subscription(state, tenant_id, id).await {
        Ok(subscription) => Some(subscription),
        Err(problem) => {
            log::warn!("Subscription {id} was changed but is not read back in v2: {problem:?}");
            None
        }
    }
}

/// A change made, its response kept for its Idempotency-Key whatever its status.
fn made(result: ChangeResult) -> Response {
    committed(Json(result).into_response())
}

#[allow(clippy::too_many_arguments)]
async fn result(
    state: &AppState,
    tenant_id: TenantId,
    id: SubscriptionId,
    status: ChangeStatus,
    effective_date: NaiveDate,
    scheduled_change: Option<ScheduledEventId>,
    invoices: Vec<InvoiceId>,
    credit_notes: Vec<CreditNoteId>,
) -> Response {
    made(
        change_result(
            state,
            tenant_id,
            id,
            status,
            effective_date,
            scheduled_change,
            (invoices, credit_notes),
            None,
        )
        .await,
    )
}

/// The change made, the documents it issued named by number when they read (best-effort: the
/// change is made whatever is read back).
#[allow(clippy::too_many_arguments)]
async fn change_result(
    state: &AppState,
    tenant_id: TenantId,
    id: SubscriptionId,
    status: ChangeStatus,
    effective_date: NaiveDate,
    scheduled_change: Option<ScheduledEventId>,
    (invoices, credit_notes): (Vec<InvoiceId>, Vec<CreditNoteId>),
    awaiting_payment: Option<InvoiceId>,
) -> ChangeResult {
    let ids = ReferenceIds {
        invoices: invoices.iter().chain(&awaiting_payment).copied().collect(),
        credit_notes: credit_notes.clone(),
        ..ReferenceIds::default()
    };
    let refs = Refs::load(&state.store, tenant_id, &ids)
        .await
        .unwrap_or_else(|problem| {
            log::warn!(
                "The documents subscription {id}'s change issued are not named: {problem:?}"
            );
            Refs::default()
        });
    ChangeResult {
        status,
        effective_date,
        scheduled_change,
        invoices: invoices.into_iter().map(|id| refs.invoice(id)).collect(),
        credit_notes: credit_notes
            .into_iter()
            .map(|id| refs.credit_note(id))
            .collect(),
        awaiting_payment: awaiting_payment.map(|id| refs.invoice(id)),
        subscription: read_back(state, tenant_id, id).await,
    }
}

fn today() -> NaiveDate {
    common_clock::now().date_naive()
}

/// What a store preview left out, unpriced: the preview says so (`incomplete`), its documents
/// never reading as the change billing nothing.
fn left_out(
    parts: &[meteroid_store::domain::subscription_changes::PreviewPart],
) -> Vec<PreviewPart> {
    use meteroid_store::domain::subscription_changes::PreviewPart as Part;
    let mut missing: Vec<PreviewPart> = parts
        .iter()
        .map(|part| match part {
            Part::Invoice => PreviewPart::Invoice,
            Part::CreditNote => PreviewPart::CreditNote,
            Part::NextInvoice => PreviewPart::NextInvoice,
            Part::SubscriptionAfter => PreviewPart::SubscriptionAfter,
        })
        .collect();
    missing.dedup();
    missing
}

/// The codes of the products the subscription's charges bill.
async fn product_codes(
    state: &AppState,
    tenant_id: TenantId,
    details: &SubscriptionDetails,
) -> Result<HashMap<ProductId, String>, Problem> {
    let mut ids: Vec<ProductId> = details
        .price_components
        .iter()
        .filter_map(|component| component.product_id)
        .collect();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(state
        .store
        .find_products_by_ids(&ids, tenant_id)
        .await?
        .into_iter()
        .map(|product| (product.id, product.code))
        .collect())
}

fn names_product(codes: &HashMap<ProductId, String>, product_id: ProductId, raw: &str) -> bool {
    codes.get(&product_id).map(String::as_str) == Some(raw)
        || product_id.to_string() == raw
        || product_id.as_uuid().to_string() == raw
}

/// The subscription's charge billing the product `raw` names, refused at `path`.
fn subscription_charge<'a>(
    details: &'a SubscriptionDetails,
    codes: &HashMap<ProductId, String>,
    raw: &str,
    path: &str,
) -> Result<&'a SubscriptionComponent, Problem> {
    let raw = raw.trim();
    let found: Vec<&SubscriptionComponent> = details
        .price_components
        .iter()
        .filter(|c| c.product_id.is_some_and(|p| names_product(codes, p, raw)))
        .collect();
    match found.as_slice() {
        [component] => Ok(component),
        [] => Err(Problem::field(
            path,
            "not_found",
            format!("The subscription has no charge of the product {raw}"),
        )),
        _ => Err(Problem::field(
            path,
            "ambiguous",
            format!("Several charges of the subscription bill the product {raw}"),
        )),
    }
}

async fn ensure_credit_allowed(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    introduces: bool,
) -> Result<(), Problem> {
    ensure_credit_pricing_allowed(
        state.feature_resolver.as_ref(),
        auth.organization_id,
        introduces,
    )
    .await
    .map_err(Problem::gated)
}

// ── Plan change ─────────────────────────────────────────────────────

/// A plan change asked, its codes resolved.
struct PlanChangeAsk {
    id: SubscriptionId,
    details: SubscriptionDetails,
    fp: FullPlan,
    target: PlanTarget,
    /// The option the new plan's charges are read in.
    option_alias: Option<String>,
    params: Vec<ComponentParameterization>,
    /// The seat counts asked on the new plan, by managed quantity code.
    quantities: BTreeMap<String, Decimal>,
    /// The products the new plan sells in levels.
    level_products: HashSet<ProductId>,
}

async fn plan_change_ask(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
    body: &PlanChangeInput,
) -> Result<PlanChangeAsk, Problem> {
    let (id, details) = target(state, tenant_id, raw).await?;
    let plan_id = ids::plan_id(state, tenant_id, &body.plan)
        .await
        .map_err(not_found_at("/plan".to_owned(), "plan", &body.plan))?;
    let fp = published_version(state, tenant_id, plan_id, body.version, "").await?;
    let currency = details.subscription.currency.clone();
    let options = &fp.version.options;
    // The option named, else the one the store moves the subscription to (the alias it holds,
    // else the one billing its fixed fees' cadence, else the first sold): named to the store
    // explicitly, so the change, its levels and `subscription_after` read the same option.
    let read_in = match body.option.as_deref() {
        Some(alias) => Some(option_named(&fp, alias, "/option")?),
        None => {
            let picked = state
                .services
                .plan_change_option(tenant_id, id, fp.version.id)
                .await?;
            picked.and_then(|picked| options.iter().find(|option| option.id == picked.id))
        }
    };
    let cells = cells(&fp, read_in, &currency);

    let mut params = Vec::new();
    let mut quantities = BTreeMap::new();
    for (code, raw) in &body.quantities {
        let path = format!("/quantities/{code}");
        let count = seats(raw, &path)?;
        quantities.insert(code.clone(), Decimal::from(count));
        let quantity = ids::quantity(state, tenant_id, code)
            .await
            .map_err(not_found_at(path.clone(), "managed quantity", code))?;
        let seat_charges: Vec<_> = cells
            .iter()
            .filter(|component| counts(component.fee_structure.as_ref(), &quantity))
            .collect();
        if seat_charges.is_empty() {
            return Err(Problem::field(
                path,
                "not_found",
                format!("No seat charge of the plan counts {code}"),
            ));
        }
        for component in seat_charges {
            params.push(ComponentParameterization {
                component_id: component.id,
                parameters: ComponentParameters {
                    initial_slot_count: Some(count),
                    billing_period: None,
                    committed_capacity: None,
                },
            });
        }
    }
    let mut prices = Vec::with_capacity(body.prices.len());
    for (code, raw) in &body.prices {
        let path = format!("/prices/{code}");
        let component = plan_charge(&fp, &cells, code, &path)?;
        let price: Decimal = raw.trim().parse().map_err(|_| {
            Problem::field(&path, "invalid", format!("'{raw}' is not a decimal price"))
        })?;
        prices.push(meteroid_store::domain::subscription_changes::TargetPrice {
            component_id: component.id,
            price,
        });
    }
    let mut chosen = Vec::with_capacity(body.levels.len());
    for (code, raw) in &body.levels {
        let path = format!("/levels/{code}");
        let included = count(raw, &path)?;
        let component = plan_charge(&fp, &cells, code, &path)?;
        chosen.push((component.id, included));
    }
    // The level held of each product the new plan sells in levels, else the one asked or its
    // lowest, as the portal picks them.
    let picks = plan_change_levels(&details.price_components, &cells, &chosen)
        .map_err(|error| Problem::field("/levels", "invalid", store_message(&error)))?;
    params.extend(picks.iter().map(LevelPick::parameterization));

    let target = PlanTarget {
        version: fp.version.id,
        option: read_in.map(|option| PlanOptionRef::Id(option.id)),
        prices,
    };
    let option_alias = read_in.map(|option| option.alias.clone());
    let level_products = cells
        .iter()
        .filter(|component| !meteroid_store::domain::usage_levels::levels(component).is_empty())
        .filter_map(|component| component.product_id)
        .collect();
    Ok(PlanChangeAsk {
        id,
        details,
        fp,
        target,
        option_alias,
        params,
        quantities,
        level_products,
    })
}

/// When the change takes effect, checked against which way it goes and what holds it.
async fn plan_change_timing(
    state: &AppState,
    tenant_id: TenantId,
    ask: &PlanChangeAsk,
    body: &PlanChangeInput,
) -> Result<
    (
        Timing,
        meteroid_store::domain::subscription_changes::PlanChangePreviewExtended,
    ),
    Problem,
> {
    let preview = state
        .services
        .preview_plan_change(
            ask.id,
            tenant_id,
            ask.target.clone(),
            ask.params.clone(),
            None,
        )
        .await?;
    let timing = when::timing(
        body.when.as_ref(),
        ChangeKind::PlanChange,
        Rules {
            direction: Some(preview.change_direction.into()),
            period_end_only: !preview.immediate_allowed,
        },
    )?;
    Ok((timing, preview))
}

/// The new plan's charges, as the change bills them.
async fn charges_after(
    state: &AppState,
    tenant_id: TenantId,
    ask: &PlanChangeAsk,
    preview: &PlanChangePreview,
) -> Result<Vec<SubscriptionCharge>, Problem> {
    let sub = &ask.details.subscription;
    let mut sources = Vec::new();
    for matched in &preview.matched {
        sources.push(ChargeSource {
            origin: super::model::ChargeOrigin::Plan,
            add_on: None,
            stored: render::fee_charge(
                &matched.new_name,
                matched.product_id,
                &matched.new_fee,
                matched.new_period,
                sub.period,
                &sub.currency,
            ),
        });
    }
    for added in &preview.added {
        let product_id = added
            .price_component_id
            .and_then(|id| ask.fp.price_components.iter().find(|c| c.id == id))
            .and_then(|component| component.product_id);
        if let Some(product_id) = product_id {
            sources.push(ChargeSource {
                origin: super::model::ChargeOrigin::Plan,
                add_on: None,
                stored: render::fee_charge(
                    &added.name,
                    product_id,
                    &added.fee,
                    added.period,
                    sub.period,
                    &sub.currency,
                ),
            });
        }
    }
    render::render_charges(state, tenant_id, &sources, &HashMap::new()).await
}

/// Preview a plan change
///
/// What moving to another plan (or option, or version) would bill and leave, nothing written.
/// A downgrade takes effect at the period's end; a minimum commitment holds any change to it.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/plan-change/preview",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    request_body(content = PlanChangeInput, example = json!(crate::v2::examples::request("preview_plan_change"))),
    responses(
        (status = 200, description = "What the change would do", body = ChangePreview, example = json!(crate::v2::examples::response("preview_plan_change"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused, or the change cannot take effect when asked (`when_refused`)", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn preview_plan_change(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
    Body(body): Body<PlanChangeInput>,
) -> Result<Json<ChangePreview>, Problem> {
    let tenant_id = auth.tenant_id;
    let ask = plan_change_ask(&state, tenant_id, &id_or_external_id, &body).await?;
    let (timing, _) = plan_change_timing(&state, tenant_id, &ask, &body).await?;
    let full = state
        .services
        .preview_plan_change_in_full(
            ask.id,
            tenant_id,
            ask.target.clone(),
            ask.params.clone(),
            (timing == Timing::Immediate).then_some(PlanChangeMode::Immediate),
        )
        .await?;
    let timeline = state
        .services
        .plan_change_timeline_effect(
            auth.as_actor(),
            ask.id,
            tenant_id,
            ask.target.clone(),
            ask.params.clone(),
            timing == Timing::Immediate,
        )
        .await?
        .map(Into::into);
    let preview = &full.preview;
    let currency = ask.details.subscription.currency.clone();
    let documents =
        render::plan_change_documents(&state, tenant_id, &full.outcome, &currency).await?;
    let change = AfterChange {
        plan: Some(PlanRef {
            id: ask.fp.plan.id,
            code: ask.fp.plan.code.clone(),
            version: Some(ask.fp.version.version),
            option: ask.option_alias.clone(),
        }),
        mrr: full.outcome.mrr_after_cents,
        quantities: ask.quantities.clone(),
        level_products: ask.level_products.clone(),
        ..AfterChange::default()
    };
    let subscription_after = match &full.outcome.subscription_after {
        Some(after) => render::after_change(&state, tenant_id, &ask.details, after, change).await?,
        // The store could not lay the change over the facts: the new plan's charges on the
        // subscription as it stands.
        None => {
            let mut out =
                render::after_change(&state, tenant_id, &ask.details, &ask.details, change).await?;
            out.charges = Some(charges_after(&state, tenant_id, &ask, &preview.preview).await?);
            out
        }
    };
    let missing = left_out(&full.outcome.missing);
    Ok(Json(ChangePreview {
        effective_date: preview.preview.effective_date,
        direction: preview.change_direction.into(),
        when: timing.kind(),
        documents,
        incomplete: !missing.is_empty(),
        missing,
        subscription_after,
        timeline,
    }))
}

/// Change the plan
///
/// Moves the subscription to another plan, option or version: `immediate` (prorated, an upgrade
/// or a lateral move) or at `period_end`. Seat counts and levels on the new plan default to those
/// held. An `Idempotency-Key` is required.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/plan-change",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    request_body(content = PlanChangeInput, example = json!(crate::v2::examples::request("change_plan"))),
    responses(
        (status = 200, description = "Applied or scheduled", body = ChangeResult, example = json!(crate::v2::examples::response("change_plan"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The subscription's state refuses the change (another change is pending…)", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused, the change cannot take effect when asked (`when_refused`), or no Idempotency-Key was sent", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn change_plan(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id_or_external_id): Path<String>,
    Body(body): Body<PlanChangeInput>,
) -> Result<Response, Problem> {
    require_key(&headers, "A plan change")?;
    let tenant_id = auth.tenant_id;
    let ask = plan_change_ask(&state, tenant_id, &id_or_external_id, &body).await?;
    let (timing, _) = plan_change_timing(&state, tenant_id, &ask, &body).await?;
    let id = ask.id;
    match timing {
        Timing::Immediate => {
            let applied = state
                .services
                .apply_plan_change_immediate(auth.as_actor(), id, tenant_id, ask.target, ask.params)
                .await?;
            Ok(result(
                &state,
                tenant_id,
                id,
                ChangeStatus::Applied,
                applied.effective_date,
                None,
                applied
                    .adjustment_invoice_id
                    .into_iter()
                    .chain(applied.first_invoice_id)
                    .collect(),
                applied.credit_note_ids,
            )
            .await)
        }
        Timing::PeriodEnd => {
            let event = state
                .services
                .schedule_plan_change(auth.as_actor(), id, tenant_id, ask.target, ask.params)
                .await?;
            Ok(result(
                &state,
                tenant_id,
                id,
                ChangeStatus::Scheduled,
                event.scheduled_time.date(),
                Some(event.id),
                Vec::new(),
                Vec::new(),
            )
            .await)
        }
        Timing::On(_) => Err(Problem::internal()),
    }
}

// ── Seats ───────────────────────────────────────────────────────────

/// The charge a seat change of `quantity` goes through: a plan charge counting it, else an add-on.
fn seat_charge(details: &SubscriptionDetails, quantity: &ManagedQuantity) -> Option<SeatCharge> {
    let counted = |fee: &SubscriptionFee| match fee {
        SubscriptionFee::Slot {
            quantity_id: Some(id),
            ..
        } => *id == quantity.id,
        _ => false,
    };
    details
        .price_components
        .iter()
        .find_map(|component| match component.price_component_id {
            Some(id) if counted(&component.fee) => Some(SeatCharge::Component(id)),
            _ => None,
        })
        .or_else(|| {
            details
                .add_ons
                .iter()
                .find(|add_on| counted(&add_on.fee))
                .map(|add_on| SeatCharge::AddOn(add_on.add_on_id))
        })
}

fn seat_change(body: &QuantityChangeInput) -> Result<SeatChange, Problem> {
    match (body.set.as_deref(), body.delta.as_deref()) {
        (Some(set), None) => Ok(SeatChange::To(seats(set, "/set")?)),
        (None, Some(delta)) => Ok(SeatChange::By(seat_delta(delta, "/delta")?)),
        (Some(_), Some(_)) => Err(Problem::field(
            "/delta",
            "invalid",
            "Send `set` or `delta`, not both",
        )),
        (None, None) => Err(Problem::field(
            "/set",
            "required",
            "Send `set`, the count to hold, or `delta`, its change",
        )),
    }
}

struct SeatAsk {
    id: SubscriptionId,
    details: SubscriptionDetails,
    quantity: ManagedQuantity,
    charge: SeatCharge,
    change: SeatChange,
}

async fn seat_ask(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
    code: &str,
    body: &QuantityChangeInput,
) -> Result<SeatAsk, Problem> {
    let change = seat_change(body)?;
    let (id, details) = target(state, tenant_id, raw).await?;
    let quantity = ids::quantity(state, tenant_id, code).await?;
    let charge = seat_charge(&details, &quantity).ok_or_else(|| {
        Problem::not_found(format!(
            "The subscription holds no {}: none of its charges counts it",
            quantity.code
        ))
    })?;
    Ok(SeatAsk {
        id,
        details,
        quantity,
        charge,
        change,
    })
}

/// Preview a seat change
///
/// What setting (`set`) or changing (`delta`) the count of a managed quantity would bill and
/// leave, nothing written: the prorated invoice an increase in force at once issues, and the
/// subscription with its new count and MRR. The count is checked against the bounds of the charge
/// counting it. With `activation: on_payment` the preview is the same: the invoice is the one the
/// increase waits on, and the subscription after is as it reads once that invoice is paid. No
/// `Idempotency-Key` is taken.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/quantities/{quantity_code}/preview",
    params(
        ("id_or_external_id" = String, Path, description = "Subscription id or external id"),
        ("quantity_code" = String, Path, description = "Managed quantity code or id"),
    ),
    request_body(content = QuantityChangeInput, example = json!(crate::v2::examples::request("preview_quantity_change"))),
    responses(
        (status = 200, description = "What the change would do", body = ChangePreview, example = json!(crate::v2::examples::response("preview_quantity_change"))),
        (status = 404, description = "Not found, or the subscription holds no such quantity", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn preview_quantity_change(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_external_id, quantity_code)): Path<(String, String)>,
    Body(body): Body<QuantityChangeInput>,
) -> Result<Json<ChangePreview>, Problem> {
    let tenant_id = auth.tenant_id;
    let ask = seat_ask(&state, tenant_id, &id_or_external_id, &quantity_code, &body).await?;
    // The store counts the change as it applies it: a count holds the seats still paying.
    let path = match ask.change {
        SeatChange::To(_) => "/set",
        SeatChange::By(_) => "/delta",
    };
    let preview = state
        .services
        .preview_slot_update(tenant_id, ask.id, ask.charge, ask.change)
        .await
        .map_err(|error| Problem::at(path, error))?;
    let delta = preview.delta;
    let currency = ask.details.subscription.currency.clone();
    let subscription_after = render::after_change(
        &state,
        tenant_id,
        &ask.details,
        &ask.details,
        AfterChange {
            mrr: preview.mrr_after_cents,
            quantities: BTreeMap::from([(
                ask.quantity.code.clone(),
                Decimal::from(preview.new_slots),
            )]),
            ..AfterChange::default()
        },
    )
    .await?;
    let issued: Vec<_> = preview
        .adjustment_invoice
        .iter()
        .map(|invoice| (DocumentType::Invoice, invoice))
        .collect();
    let documents = render::change_documents(&state, tenant_id, &issued, &currency).await?;
    Ok(Json(ChangePreview {
        effective_date: preview.effective_at,
        direction: if delta > 0 {
            Direction::Upgrade
        } else {
            Direction::Downgrade
        },
        when: if preview.effective_at > today() {
            WhenType::PeriodEnd
        } else {
            WhenType::Immediate
        },
        documents,
        // Its MRR unstated, the subscription after is read without it.
        incomplete: preview.mrr_after_cents.is_none(),
        missing: if preview.mrr_after_cents.is_none() {
            vec![PreviewPart::SubscriptionAfter]
        } else {
            Vec::new()
        },
        subscription_after,
        timeline: None,
    }))
}

/// Change a seat count
///
/// Sets (`set`) or changes (`delta`) the count of a managed quantity the subscription holds,
/// through the charges counting it, as the quantity's policies say: an increase in force at once
/// is prorated and invoiced, a decrease waits for the period's end unless the quantity gives
/// unused time back. With `activation: on_payment` an increase applies once its invoice is paid:
/// the change answers `pending_payment` with that invoice in `awaiting_payment`. An
/// `Idempotency-Key` is required: a retry never adds seats twice.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/quantities/{quantity_code}",
    params(
        ("id_or_external_id" = String, Path, description = "Subscription id or external id"),
        ("quantity_code" = String, Path, description = "Managed quantity code or id"),
    ),
    request_body(content = QuantityChangeInput, example = json!(crate::v2::examples::request("change_quantity"))),
    responses(
        (status = 200, description = "Applied, waiting for the period's end, or for the payment of its invoice", body = ChangeResult, example = json!(crate::v2::examples::response("change_quantity"))),
        (status = 404, description = "Not found, or the subscription holds no such quantity", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The subscription's state refuses the change", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused, the count is out of the charge's bounds, or no Idempotency-Key was sent", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn change_quantity(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id_or_external_id, quantity_code)): Path<(String, String)>,
    Body(body): Body<QuantityChangeInput>,
) -> Result<Response, Problem> {
    require_key(&headers, "A seat change")?;
    let tenant_id = auth.tenant_id;
    let ask = seat_ask(&state, tenant_id, &id_or_external_id, &quantity_code, &body).await?;
    let mode = match body.activation {
        SeatActivation::Immediate => SlotUpgradeBillingMode::Optimistic,
        SeatActivation::OnPayment => SlotUpgradeBillingMode::OnInvoicePaid,
    };
    let updated = state
        .services
        .update_subscription_slots(
            tenant_id,
            ask.id,
            ask.charge,
            ask.change,
            mode,
            auth.as_actor(),
        )
        .await?;
    // Written: from here the change answers what it did, whatever is read back.
    let awaiting_payment = updated.invoice_id.filter(|_| !updated.slots_active);
    let (status, effective_date) = if awaiting_payment.is_some() {
        (ChangeStatus::PendingPayment, today())
    } else {
        let held = state
            .store
            .get_active_slots_value(tenant_id, ask.id, ask.quantity.id, None)
            .await
            .ok();
        if held.is_some_and(|held| i64::from(held) == i64::from(updated.new_slot_count)) {
            (ChangeStatus::Applied, today())
        } else {
            let waiting = render::quantity_state(&state, tenant_id, ask.id, ask.quantity.id)
                .await
                .ok()
                .and_then(|quantity| quantity.pending.last().map(|p| p.effective_at));
            (
                ChangeStatus::Scheduled,
                waiting
                    .map(|at| at.naive_utc().date())
                    .or(ask.details.subscription.current_period_end)
                    .unwrap_or_else(today),
            )
        }
    };
    Ok(made(
        change_result(
            &state,
            tenant_id,
            ask.id,
            status,
            effective_date,
            None,
            (updated.invoice_id.into_iter().collect(), Vec::new()),
            awaiting_payment,
        )
        .await,
    ))
}

// ── Levels ──────────────────────────────────────────────────────────

struct LevelAsk {
    id: SubscriptionId,
    details: SubscriptionDetails,
    code: String,
    levels: ComponentLevels,
    price_id: common_domain::ids::PriceId,
    direction: Direction,
}

async fn level_ask(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
    code: &str,
    body: &LevelChangeInput,
) -> Result<LevelAsk, Problem> {
    let included = count(&body.included, "/included")?;
    let (id, details) = target(state, tenant_id, raw).await?;
    let codes = product_codes(state, tenant_id, &details).await?;
    let code = code.trim();
    let mut matching: Vec<ComponentLevels> = state
        .services
        .list_capacity_levels(tenant_id, id)
        .await?
        .into_iter()
        .filter(|levels| {
            details
                .price_components
                .iter()
                .find(|c| c.id == levels.subscription_component_id)
                .and_then(|c| c.product_id)
                .is_some_and(|product_id| names_product(&codes, product_id, code))
        })
        .collect();
    let levels = match matching.len() {
        1 => matching.remove(0),
        0 => {
            return Err(Problem::not_found(format!(
                "The subscription has no charge of {code} sold in levels"
            )));
        }
        _ => {
            return Err(conflict(format!(
                "Several charges of the subscription bill {code} in levels"
            )));
        }
    };
    let Some(level) = levels
        .levels
        .iter()
        .find(|level| level.included == included)
    else {
        let sizes: Vec<String> = levels
            .levels
            .iter()
            .map(|l| l.included.to_string())
            .collect();
        return Err(Problem::field(
            "/included",
            "not_found",
            format!(
                "{included} is not a level of {}: its levels include {}",
                levels.name,
                sizes.join(", ")
            ),
        ));
    };
    let direction = match included.cmp(&levels.included) {
        std::cmp::Ordering::Greater => Direction::Upgrade,
        std::cmp::Ordering::Less => Direction::Downgrade,
        std::cmp::Ordering::Equal => {
            return Err(Problem::field(
                "/included",
                "invalid",
                format!("{} already includes {included}", levels.name),
            ));
        }
    };
    let price_id = level.price_id;
    // The product's code, whatever the path named it by.
    let code = details
        .price_components
        .iter()
        .find(|c| c.id == levels.subscription_component_id)
        .and_then(|c| c.product_id)
        .and_then(|product_id| codes.get(&product_id).cloned())
        .unwrap_or_else(|| code.to_owned());
    Ok(LevelAsk {
        id,
        details,
        code,
        levels,
        price_id,
        direction,
    })
}

/// Preview a level change
///
/// What moving a usage-levels charge, named by its product, to another of its levels
/// (`included`) would bill and leave, nothing written. Without `when` a higher level applies at
/// once and a lower one at the period's end; a higher level at `period_end` or a lower one
/// `immediate` is refused (`422 when_refused`). No `Idempotency-Key` is taken.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/levels/{product_code}/preview",
    params(
        ("id_or_external_id" = String, Path, description = "Subscription id or external id"),
        ("product_code" = String, Path, description = "The product of the usage-levels charge, by code or id"),
    ),
    request_body(content = LevelChangeInput, example = json!(crate::v2::examples::request("preview_level_change"))),
    responses(
        (status = 200, description = "What the move would do", body = ChangePreview, example = json!(crate::v2::examples::response("preview_level_change"))),
        (status = 404, description = "Not found, or no charge of the product is sold in levels", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "Not a level of the charge, or the move cannot take effect when asked", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn preview_level_change(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_external_id, product_code)): Path<(String, String)>,
    Body(body): Body<LevelChangeInput>,
) -> Result<Json<ChangePreview>, Problem> {
    let tenant_id = auth.tenant_id;
    let ask = level_ask(&state, tenant_id, &id_or_external_id, &product_code, &body).await?;
    let timing = when::timing(
        body.when.as_ref(),
        ChangeKind::Level,
        Rules {
            direction: Some(ask.direction),
            period_end_only: false,
        },
    )?;
    let preview = state
        .services
        .preview_capacity_level_change(
            tenant_id,
            ask.id,
            ask.levels.subscription_component_id,
            ask.price_id,
        )
        .await?;
    let currency = ask.details.subscription.currency.clone();
    let mut subscription_after = render::after_change(
        &state,
        tenant_id,
        &ask.details,
        &preview.preview.details_after,
        AfterChange {
            mrr: Some(preview.preview.mrr_after_cents),
            ..AfterChange::default()
        },
    )
    .await?;
    // The level moved to, as the move prices it.
    subscription_after
        .levels
        .get_or_insert_with(BTreeMap::new)
        .insert(ask.code.clone(), preview.to_included.to_string());
    let missing = left_out(&preview.preview.missing);
    Ok(Json(ChangePreview {
        effective_date: preview.effective_date,
        direction: preview.direction.into(),
        when: timing.kind(),
        documents: render::amendment_documents(&state, tenant_id, &preview.preview, &currency)
            .await?,
        incomplete: !missing.is_empty(),
        missing,
        subscription_after,
        timeline: None,
    }))
}

/// Change a level
///
/// Moves a usage-levels charge, named by its product, to another of its levels (`included`): a
/// higher level applies at once, prorated; a lower one at the period's end. An `Idempotency-Key`
/// is required.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/levels/{product_code}",
    params(
        ("id_or_external_id" = String, Path, description = "Subscription id or external id"),
        ("product_code" = String, Path, description = "The product of the usage-levels charge, by code or id"),
    ),
    request_body(content = LevelChangeInput, example = json!(crate::v2::examples::request("change_level"))),
    responses(
        (status = 200, description = "Applied or scheduled", body = ChangeResult, example = json!(crate::v2::examples::response("change_level"))),
        (status = 404, description = "Not found, or no charge of the product is sold in levels", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "Another change is scheduled", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "Not a level of the charge, the move cannot take effect when asked, or no Idempotency-Key was sent", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn change_level(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id_or_external_id, product_code)): Path<(String, String)>,
    Body(body): Body<LevelChangeInput>,
) -> Result<Response, Problem> {
    require_key(&headers, "A level change")?;
    let tenant_id = auth.tenant_id;
    let ask = level_ask(&state, tenant_id, &id_or_external_id, &product_code, &body).await?;
    when::timing(
        body.when.as_ref(),
        ChangeKind::Level,
        Rules {
            direction: Some(ask.direction),
            period_end_only: false,
        },
    )?;
    let outcome = state
        .services
        .change_capacity_level(
            auth.as_actor(),
            tenant_id,
            ask.id,
            ask.levels.subscription_component_id,
            ask.price_id,
        )
        .await?;
    Ok(match outcome {
        LevelChangeOutcome::Applied(applied) => {
            result(
                &state,
                tenant_id,
                ask.id,
                ChangeStatus::Applied,
                applied.effective_date,
                None,
                applied.adjustment_invoice_id.into_iter().collect(),
                applied.credit_note_ids,
            )
            .await
        }
        LevelChangeOutcome::Scheduled(event) => {
            result(
                &state,
                tenant_id,
                ask.id,
                ChangeStatus::Scheduled,
                event.scheduled_time.date(),
                Some(event.id),
                Vec::new(),
                Vec::new(),
            )
            .await
        }
    })
}

// ── Amendments ──────────────────────────────────────────────────────

/// The add-on of the subscription `raw` names.
async fn held_add_on<'a>(
    state: &AppState,
    tenant_id: TenantId,
    details: &'a SubscriptionDetails,
    raw: &str,
    path: &str,
) -> Result<&'a meteroid_store::domain::SubscriptionAddOn, Problem> {
    let add_on_id = ids::add_on_id(state, tenant_id, raw)
        .await
        .map_err(not_found_at(path.to_owned(), "add-on", raw))?;
    let held: Vec<_> = details
        .add_ons
        .iter()
        .filter(|add_on| add_on.add_on_id == add_on_id)
        .collect();
    match held.as_slice() {
        [add_on] => Ok(add_on),
        [] => Err(Problem::field(
            path,
            "not_found",
            format!("The subscription holds no add-on {}", raw.trim()),
        )),
        _ => Err(Problem::field(
            path,
            "ambiguous",
            format!(
                "The subscription holds the add-on {} more than once",
                raw.trim()
            ),
        )),
    }
}

async fn amendment_of(
    state: &AppState,
    tenant_id: TenantId,
    details: &SubscriptionDetails,
    body: &AmendmentInput,
    apply_mode: PlanChangeMode,
) -> Result<SubscriptionAmendment, Problem> {
    let sub = &details.subscription;
    let codes = product_codes(state, tenant_id, details).await?;
    let mut component_changes = ComponentChanges::default();
    for (index, asked) in body.charges.edit.iter().enumerate() {
        let path = format!("/charges/edit/{index}");
        let component =
            subscription_charge(details, &codes, &asked.charge, &format!("{path}/charge"))?;
        let product_id = component.product_id.ok_or_else(Problem::internal)?;
        let stored = render::fee_charge(
            &component.name,
            product_id,
            &component.fee,
            component.period,
            sub.period,
            &sub.currency,
        );
        let cadence = asked.cadence.unwrap_or_else(|| {
            component
                .period
                .as_billing_period_opt()
                .unwrap_or(sub.period)
                .into()
        });
        let price = override_price(
            state,
            tenant_id,
            &stored,
            asked,
            cadence,
            &sub.currency,
            &path,
        )
        .await?;
        component_changes.edited.push(EditComponent {
            subscription_component_id: component.id,
            name: asked
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
            price_entry: PriceEntry::New(price),
        });
    }
    for (index, charge) in body.charges.add.iter().enumerate() {
        let path = format!("/charges/add/{index}");
        let (name, product_id, structure, price) =
            own_charge(state, tenant_id, charge, &sub.currency, &path).await?;
        component_changes.added.push(AddExtraComponent {
            name,
            product: ProductRef::Existing(product_id),
            price_entry: PriceEntry::New(price),
            fee_structure: Some(structure),
        });
    }
    for (index, raw) in body.charges.remove.iter().enumerate() {
        let component =
            subscription_charge(details, &codes, raw, &format!("/charges/remove/{index}"))?;
        component_changes.removed.push(component.id);
    }

    let mut add_on_changes = DomainAddOnChanges::default();
    for (index, purchase) in body.add_ons.add.iter().enumerate() {
        let path = format!("/add_ons/add/{index}");
        if purchase.quantity == 0 {
            return Err(Problem::field(
                format!("{path}/quantity"),
                "invalid",
                "An add-on is held once at least",
            ));
        }
        let add_on_id = ids::add_on_id(state, tenant_id, &purchase.add_on)
            .await
            .map_err(not_found_at(
                format!("{path}/add_on"),
                "add-on",
                &purchase.add_on,
            ))?;
        add_on_changes.added.push(CreateSubscriptionAddOn {
            add_on_id,
            customization: SubscriptionAddOnCustomization::None,
            quantity: i32::try_from(purchase.quantity).map_err(|_| {
                Problem::field(
                    format!("{path}/quantity"),
                    "invalid",
                    format!("An add-on is held {} times at most", i32::MAX),
                )
            })?,
        });
    }
    for (index, purchase) in body.add_ons.edit.iter().enumerate() {
        let path = format!("/add_ons/edit/{index}");
        if purchase.quantity == 0 {
            return Err(Problem::field(
                format!("{path}/quantity"),
                "invalid",
                "An add-on is held once at least: remove it instead",
            ));
        }
        let held = held_add_on(
            state,
            tenant_id,
            details,
            &purchase.add_on,
            &format!("{path}/add_on"),
        )
        .await?;
        add_on_changes.edited.push(EditSubscriptionAddOn {
            subscription_add_on_id: held.id,
            quantity: Some(purchase.quantity),
            customization: None,
        });
    }
    for (index, raw) in body.add_ons.remove.iter().enumerate() {
        let held = held_add_on(
            state,
            tenant_id,
            details,
            raw,
            &format!("/add_ons/remove/{index}"),
        )
        .await?;
        add_on_changes.removed.push(held.id);
    }

    let amendment = SubscriptionAmendment {
        apply_mode,
        component_changes,
        add_on_changes,
    };
    if amendment.is_empty() {
        return Err(Problem::field(
            "/charges",
            "required",
            "An amendment adds, edits or removes a charge or an add-on",
        ));
    }
    Ok(amendment)
}

fn apply_mode(timing: Timing) -> Result<PlanChangeMode, Problem> {
    match timing {
        Timing::Immediate => Ok(PlanChangeMode::Immediate),
        Timing::PeriodEnd => Ok(PlanChangeMode::EndOfPeriod),
        Timing::On(_) => Err(Problem::internal()),
    }
}

fn introduces_credit(amendment: &SubscriptionAmendment) -> bool {
    amendment.component_changes.added.iter().any(|added| {
        added
            .fee_structure
            .as_ref()
            .is_some_and(|structure| structure.is_credit())
            || added.product.creates_credit_product()
    })
}

/// Preview an amendment
///
/// What changing the subscription's charges and add-ons would bill and leave, nothing written:
/// the documents it issues and the subscription after it. `when` is required, `immediate` or
/// `period_end`. A charge selling credit is refused (`403 feature_not_available`) unless the
/// organization's plan includes credit pricing. No `Idempotency-Key` is taken.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/amendments/preview",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    request_body(content = AmendmentInput, example = json!(crate::v2::examples::request("preview_amendment"))),
    responses(
        (status = 200, description = "What the amendment would do", body = ChangePreview, example = json!(crate::v2::examples::response("preview_amendment"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused, or the model cannot express a charge", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn preview_amendment(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
    Body(body): Body<AmendmentInput>,
) -> Result<Json<ChangePreview>, Problem> {
    let tenant_id = auth.tenant_id;
    let timing = when::timing(body.when.as_ref(), ChangeKind::Amendment, Rules::default())?;
    let (id, details) = target(&state, tenant_id, &id_or_external_id).await?;
    let amendment = amendment_of(&state, tenant_id, &details, &body, apply_mode(timing)?).await?;
    ensure_credit_allowed(&state, &auth, introduces_credit(&amendment)).await?;
    let preview = state
        .services
        .preview_amendment(id, tenant_id, amendment.clone())
        .await?;
    let timeline = state
        .services
        .amendment_timeline_effect(auth.as_actor(), id, tenant_id, amendment)
        .await?
        .map(Into::into);
    let currency = details.subscription.currency.clone();
    let subscription_after = render::after_change(
        &state,
        tenant_id,
        &details,
        &preview.details_after,
        AfterChange {
            status: Some(preview.subscription_after.status.clone().into()),
            mrr: Some(preview.mrr_after_cents),
            ..AfterChange::default()
        },
    )
    .await?;
    let missing = left_out(&preview.missing);
    Ok(Json(ChangePreview {
        effective_date: preview.preview.effective_date,
        direction: preview.change_direction.into(),
        when: timing.kind(),
        documents: render::amendment_documents(&state, tenant_id, &preview, &currency).await?,
        incomplete: !missing.is_empty(),
        missing,
        subscription_after,
        timeline,
    }))
}

/// Amend a subscription
///
/// Changes its charges and add-ons without changing its plan: charges added (each with one price
/// in the subscription's currency), repriced (`edit`) or removed by product code; add-ons added,
/// held at another quantity or removed by code. `immediate` (prorated) or at `period_end`. An
/// `Idempotency-Key` is required.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/amendments",
    operation_id = "amend_subscription",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    request_body(content = AmendmentInput, example = json!(crate::v2::examples::request("amend_subscription"))),
    responses(
        (status = 200, description = "Applied or scheduled", body = ChangeResult, example = json!(crate::v2::examples::response("amend_subscription"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The subscription's state refuses the change", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused, the model cannot express a charge, or no Idempotency-Key was sent", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn amend(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id_or_external_id): Path<String>,
    Body(body): Body<AmendmentInput>,
) -> Result<Response, Problem> {
    require_key(&headers, "An amendment")?;
    let tenant_id = auth.tenant_id;
    let timing = when::timing(body.when.as_ref(), ChangeKind::Amendment, Rules::default())?;
    let (id, details) = target(&state, tenant_id, &id_or_external_id).await?;
    let amendment = amendment_of(&state, tenant_id, &details, &body, apply_mode(timing)?).await?;
    ensure_credit_allowed(&state, &auth, introduces_credit(&amendment)).await?;
    Ok(match amendment.apply_mode {
        PlanChangeMode::Immediate => {
            let applied = state
                .services
                .apply_amendment_immediate(auth.as_actor(), id, tenant_id, amendment)
                .await?;
            result(
                &state,
                tenant_id,
                id,
                ChangeStatus::Applied,
                applied.effective_date,
                None,
                applied.adjustment_invoice_id.into_iter().collect(),
                applied.credit_note_ids,
            )
            .await
        }
        PlanChangeMode::EndOfPeriod => {
            let event = state
                .services
                .schedule_amendment(auth.as_actor(), id, tenant_id, amendment)
                .await?;
            result(
                &state,
                tenant_id,
                id,
                ChangeStatus::Scheduled,
                event.scheduled_time.date(),
                Some(event.id),
                Vec::new(),
                Vec::new(),
            )
            .await
        }
    })
}

// ── Cancellation ────────────────────────────────────────────────────

fn cancellation_at(timing: Timing) -> CancellationEffectiveAt {
    match timing {
        Timing::Immediate => CancellationEffectiveAt::Immediate,
        Timing::PeriodEnd => CancellationEffectiveAt::EndOfBillingPeriod,
        Timing::On(date) => CancellationEffectiveAt::Date(date),
    }
}

/// Preview a cancellation
///
/// When the cancellation would take effect, and what it would issue: the invoice settling the
/// period it ends (what was consumed until then) and the credit note giving back the time billed
/// in advance after it, each as the facts with the ending laid over them price it. One that
/// cannot be priced is left out and named in `missing` (`incomplete: true`).
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/cancel/preview",
    operation_id = "preview_cancellation",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    request_body(content = CancelInput, example = json!(crate::v2::examples::request("preview_cancellation"))),
    responses(
        (status = 200, description = "When the cancellation would take effect and what it would issue", body = ChangePreview, example = json!(crate::v2::examples::response("preview_cancellation"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The subscription has ended", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused: a date before today or after the subscription's end (`when_refused`)", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn preview_cancel(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
    Body(body): Body<CancelInput>,
) -> Result<Json<ChangePreview>, Problem> {
    let tenant_id = auth.tenant_id;
    let timing = when::timing(
        body.when.as_ref(),
        ChangeKind::Cancellation,
        Rules::default(),
    )?;
    let (id, details) = target(&state, tenant_id, &id_or_external_id).await?;
    when::cancellation_date(timing, today(), details.subscription.end_date)?;
    let preview = state
        .services
        .preview_cancellation(id, tenant_id, cancellation_at(timing))
        .await?;
    let currency = details.subscription.currency.clone();
    let ends_now = preview.aborts || preview.effective_date <= today();
    let subscription_after = render::after_change(
        &state,
        tenant_id,
        &details,
        &details,
        AfterChange {
            status: Some(if preview.aborts {
                SubscriptionStatus::Aborted
            } else {
                SubscriptionStatus::Cancelled
            }),
            mrr: Some(0),
            cancellation: (!ends_now).then(|| Cancellation {
                effective_date: preview.effective_date,
                reason: body
                    .reason
                    .as_deref()
                    .map(str::trim)
                    .filter(|reason| !reason.is_empty())
                    .map(str::to_owned),
                initiated_by_customer: false,
            }),
            ..AfterChange::default()
        },
    )
    .await?;
    let missing = left_out(&preview.missing);
    Ok(Json(ChangePreview {
        effective_date: preview.effective_date,
        direction: Direction::Downgrade,
        when: timing.kind(),
        documents: render::cancellation_documents(&state, tenant_id, &preview, &currency).await?,
        incomplete: !missing.is_empty(),
        missing,
        subscription_after,
        timeline: None,
    }))
}

/// Cancel a subscription
///
/// `immediate` (the period settles through now), at `period_end`, or on a `date` (today or later,
/// not after the subscription's own end). Applied now, it answers the final invoice and the
/// credit note it issued. A cancellation waiting for its date is replaced by a new one (moved to
/// its date), or revoked with `POST …/cancel/revoke`; an ended subscription is refused (`409`).
/// An `Idempotency-Key` is required.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/cancel",
    operation_id = "cancel_subscription",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    request_body(content = CancelInput, example = json!(crate::v2::examples::request("cancel_subscription"))),
    responses(
        (status = 200, description = "Cancelled, or scheduled to be", body = ChangeResult, example = json!(crate::v2::examples::response("cancel_subscription"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The subscription has ended, or a customer's request would move the cancellation the account manager scheduled", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused, the subscription cannot be cancelled then, or no Idempotency-Key was sent", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn cancel(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id_or_external_id): Path<String>,
    Body(body): Body<CancelInput>,
) -> Result<Response, Problem> {
    require_key(&headers, "A cancellation")?;
    let tenant_id = auth.tenant_id;
    let timing = when::timing(
        body.when.as_ref(),
        ChangeKind::Cancellation,
        Rules::default(),
    )?;
    let (id, details) = target(&state, tenant_id, &id_or_external_id).await?;
    when::cancellation_date(timing, today(), details.subscription.end_date)?;
    // Committed, the store answers: what follows its commit never fails it (a retry would cancel
    // again), and the documents its ending issued in the request come with it.
    let outcome = state
        .services
        .cancel_subscription_with_documents(
            auth.as_actor(),
            id,
            tenant_id,
            body.reason
                .as_deref()
                .map(str::trim)
                .filter(|reason| !reason.is_empty())
                .map(str::to_owned),
            cancellation_at(timing),
        )
        .await?;
    // Written: from here the change answers what it did, whatever is read back.
    let scheduled = state
        .store
        .get_subscription_details(tenant_id, id)
        .await
        .ok()
        .and_then(|details| {
            details
                .pending_events
                .iter()
                .filter(|event| event.event_type == ScheduledEventTypeEnum::CancelSubscription)
                .min_by_key(|event| event.scheduled_date)
                .map(|event| (event.scheduled_date, event.id))
        });
    let (status, effective_date, scheduled_change) = match scheduled {
        Some((date, event)) => (ChangeStatus::Scheduled, date, Some(event)),
        None => (ChangeStatus::Applied, today(), None),
    };
    Ok(made(
        change_result(
            &state,
            tenant_id,
            id,
            status,
            effective_date,
            scheduled_change,
            (outcome.invoice_ids, outcome.credit_note_ids),
            None,
        )
        .await,
    ))
}

/// Revoke a cancellation
///
/// Withdraws the cancellation waiting for its date: the subscription renews again.
#[utoipa::path(
    post,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/cancel/revoke",
    operation_id = "revoke_cancellation",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    responses(
        (status = 200, description = "Revoked", body = Subscription, example = json!(crate::v2::examples::response("revoke_cancellation"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "No cancellation is scheduled", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn revoke_cancel(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    let (id, details) = target(&state, tenant_id, &id_or_external_id).await?;
    let event = details
        .pending_events
        .iter()
        .find(|event| event.event_type == ScheduledEventTypeEnum::CancelSubscription)
        .ok_or_else(|| conflict("No cancellation is scheduled for this subscription"))?;
    state
        .services
        .cancel_scheduled_event(auth.as_actor(), event.id, id, tenant_id)
        .await?;
    // Written: read back whole, else as lists read it.
    let subscription = match read_back(&state, tenant_id, id).await {
        Some(subscription) => subscription,
        None => match render::summary(&state, tenant_id, id).await {
            Ok(subscription) => subscription,
            Err(problem) => {
                log::warn!("Subscription {id} is not read back in v2: {problem:?}");
                return Ok(committed(
                    (
                        http::StatusCode::OK,
                        Json(serde_json::json!({"id": id.to_string()})),
                    )
                        .into_response(),
                ));
            }
        },
    };
    Ok(committed(Json(subscription).into_response()))
}

/// Get a subscription's schedule
///
/// The changes waiting for their date, soonest first: plan changes, cancellations and amendments
/// scheduled for the period's end or a date. A cancellation is revoked with `POST …/cancel/revoke`.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/schedule",
    operation_id = "get_subscription_schedule",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    responses(
        (status = 200, description = "The changes waiting for their date, soonest first", body = Schedule, example = json!(crate::v2::examples::response("get_subscription_schedule"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_schedule(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
) -> Result<Json<Schedule>, Problem> {
    let tenant_id = auth.tenant_id;
    let (_, details) = target(&state, tenant_id, &id_or_external_id).await?;
    Ok(Json(Schedule {
        changes: render::schedule(&state, tenant_id, &details.pending_events).await?,
    }))
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(change_plan))
        .routes(routes!(preview_plan_change))
        .routes(routes!(change_quantity))
        .routes(routes!(preview_quantity_change))
        .routes(routes!(change_level))
        .routes(routes!(preview_level_change))
        .routes(routes!(amend))
        .routes(routes!(preview_amendment))
        .routes(routes!(cancel))
        .routes(routes!(preview_cancel))
        .routes(routes!(revoke_cancel))
        .routes(routes!(get_schedule))
}
