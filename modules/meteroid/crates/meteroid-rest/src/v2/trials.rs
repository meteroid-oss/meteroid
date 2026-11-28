//! What trials read and write in v2 (ADR-0015 §3): a trial is a free period before billing, its
//! end action (`at_end`), what a converting trial does without a payment method (`without_card`),
//! and the estimates read beside it. Shared by plans, subscriptions, checkouts, pricing pages and
//! the trial offer catalog.
use crate::v2::Reader;
use crate::v2::charges::Cadence;
use crate::v2::ids;
use crate::v2::problem::Problem;
use crate::v2::refs::PlanRef;
use chrono::NaiveDate;
use common_domain::ids::{PlanId, PlanVersionId, TenantId};
use error_stack::Report;
use meteroid_store::domain::trials as domain;
use meteroid_store::domain::{PlanVersionFilter, SubscriptionDetails};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CatalogCodesInterface;
use meteroid_store::repositories::plans::{PlansInterface, PlansInterfaceAuto};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub use meteroid_store::domain::trials::TRIAL_IS_FREE;

/// What a trial's end does: `convert` the subscription goes on, billed from then; `manual`
/// nothing is billed and the trial's access goes on until it is converted (`…/trial/end`),
/// extended or cancelled; `cancel` the subscription ends with it; `switch` it moves to `plan`.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrialAtEndType {
    Convert,
    Manual,
    Cancel,
    Switch,
}

/// What a trial's end does.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct TrialAtEnd {
    /// Convert, decide manually, cancel or switch plan.
    #[serde(rename = "type")]
    pub kind: TrialAtEndType,
    /// The plan switched to, at its version, for `switch`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanRef>,
}

/// What a trial's end does, as written: `switch` names its plan.
#[derive(Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TrialAtEndInput {
    /// Convert, decide manually, cancel or switch plan.
    #[serde(rename = "type")]
    pub kind: TrialAtEndType,
    /// For `switch`: the plan, by code or id.
    #[serde(default)]
    pub plan: Option<String>,
    /// For `switch`: a published version's number; the active one when left out.
    #[serde(default)]
    pub version: Option<i32>,
}

/// What a converting trial's end does if the subscription is charged automatically and has no
/// payment method then: `await_payment` its opening invoice is issued and it turns active once
/// paid; `cancel` it ends with the trial, nothing invoiced; `switch` it moves to `plan`.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrialWithoutCardType {
    AwaitPayment,
    Cancel,
    Switch,
}

/// What a converting trial's end does if the subscription is charged automatically and has no
/// payment method then. A subscription paid by invoice converts and is invoiced on its terms.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct TrialWithoutCard {
    /// Await the payment, cancel or switch plan.
    #[serde(rename = "type")]
    pub kind: TrialWithoutCardType,
    /// The plan switched to, at its version, for `switch`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanRef>,
}

/// What a converting trial's end does if the subscription is charged automatically and has no
/// payment method then, as written: `switch` names its plan.
#[derive(Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TrialWithoutCardInput {
    /// Await the payment, cancel or switch plan.
    #[serde(rename = "type")]
    pub kind: TrialWithoutCardType,
    /// For `switch`: the plan, by code or id.
    #[serde(default)]
    pub plan: Option<String>,
    /// For `switch`: a published version's number; the active one when left out.
    #[serde(default)]
    pub version: Option<i32>,
}

/// Whether sign-up, checkout and the pricing page ask for a card to start the trial. A
/// subscription's activation decides for itself.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrialCard {
    Required,
    Optional,
}

/// Who is given the trial once: per `offer`, per `plan`, per `product_family`, or `none` (every
/// subscription).
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrialEligibility {
    Offer,
    Plan,
    ProductFamily,
    None,
}

/// Where a subscription's trial stands: `active` running; `converted` billing started;
/// `cancelled` the subscription ended with it; `switched` it moved plan; `awaiting_payment`
/// converted without a payment method, billing waiting for its opening invoice's payment;
/// `awaiting_decision` ended with a manual end action, nothing billed until it is decided.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrialStatus {
    Active,
    Converted,
    Cancelled,
    Switched,
    AwaitingPayment,
    AwaitingDecision,
}

/// How a trial ended.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrialOutcome {
    Converted,
    Cancelled,
    Switched,
    AwaitingPayment,
    AwaitingDecision,
}

impl From<domain::TrialOutcome> for TrialOutcome {
    fn from(outcome: domain::TrialOutcome) -> Self {
        match outcome {
            domain::TrialOutcome::Converted => TrialOutcome::Converted,
            domain::TrialOutcome::Cancelled => TrialOutcome::Cancelled,
            domain::TrialOutcome::Switched => TrialOutcome::Switched,
            domain::TrialOutcome::AwaitingPayment => TrialOutcome::AwaitingPayment,
            domain::TrialOutcome::AwaitingDecision => TrialOutcome::AwaitingDecision,
        }
    }
}

/// What the trial's end is estimated to charge first: the opening invoice billed from its end,
/// with coupons and tax as they stand and before any usage. An estimate: usage, coupons and tax
/// may change it.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct FirstChargeEstimate {
    /// In minor units of `currency`.
    pub amount: i64,
    /// ISO 4217.
    pub currency: String,
    /// The day it is charged: the trial's end.
    pub on: NaiveDate,
}

/// What the subscription bills once its trial ends: its recurring fees per cadence, before
/// coupons, usage and tax.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[schema(example = json!({
    "currency": "EUR",
    "starts_on": "2026-12-15",
    "recurring": [
        {"amount": 49000, "interval": "annual"},
        {"amount": 4000, "interval": "monthly"}
    ]
}))]
pub struct AfterTrial {
    /// ISO 4217.
    pub currency: String,
    /// The trial's end, when billing starts.
    pub starts_on: NaiveDate,
    /// One per cadence, the subscription's own first; empty when only usage is billed.
    pub recurring: Vec<RecurringAmount>,
}

/// The recurring fees billed on one cadence.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecurringAmount {
    /// In minor units of the currency.
    pub amount: i64,
    /// How often it is billed.
    pub interval: Cadence,
}

impl From<meteroid_store::domain::trials::AfterTrial> for AfterTrial {
    fn from(after: meteroid_store::domain::trials::AfterTrial) -> Self {
        AfterTrial {
            currency: after.currency,
            starts_on: after.starts_on,
            recurring: after
                .recurring
                .into_iter()
                .map(|entry| RecurringAmount {
                    amount: entry.amount,
                    interval: entry.period.into(),
                })
                .collect(),
        }
    }
}

pub fn card_out(card_required: bool) -> TrialCard {
    if card_required {
        TrialCard::Required
    } else {
        TrialCard::Optional
    }
}

pub fn eligibility_out(eligibility: domain::TrialEligibility) -> TrialEligibility {
    match eligibility {
        domain::TrialEligibility::Offer => TrialEligibility::Offer,
        domain::TrialEligibility::Plan => TrialEligibility::Plan,
        domain::TrialEligibility::ProductFamily => TrialEligibility::ProductFamily,
        domain::TrialEligibility::None => TrialEligibility::None,
    }
}

pub fn eligibility_in(eligibility: TrialEligibility) -> domain::TrialEligibility {
    match eligibility {
        TrialEligibility::Offer => domain::TrialEligibility::Offer,
        TrialEligibility::Plan => domain::TrialEligibility::Plan,
        TrialEligibility::ProductFamily => domain::TrialEligibility::ProductFamily,
        TrialEligibility::None => domain::TrialEligibility::None,
    }
}

/// A plan version, named by its plan at its number.
pub async fn version_ref(
    reader: &impl Reader,
    tenant_id: TenantId,
    version_id: PlanVersionId,
) -> Result<PlanRef, Problem> {
    let plan = reader
        .store()
        .get_plan_by_version_id(version_id, tenant_id)
        .await?;
    Ok(PlanRef {
        id: plan.plan.id,
        code: plan.plan.code,
        version: plan.version.map(|version| version.version),
        option: None,
    })
}

/// A plan alone, by its code.
pub async fn plan_ref(
    reader: &impl Reader,
    tenant_id: TenantId,
    plan_id: PlanId,
) -> Result<PlanRef, Problem> {
    let code = reader
        .store()
        .plan_codes(tenant_id, &[plan_id])
        .await?
        .remove(&plan_id)
        .unwrap_or_else(|| plan_id.to_string());
    Ok(PlanRef {
        id: plan_id,
        code,
        version: None,
        option: None,
    })
}

/// The tenant's plan named by code or id, refused at `pointer` when it has none.
pub async fn plan_in(
    reader: &impl Reader,
    tenant_id: TenantId,
    raw: &str,
    pointer: &str,
) -> Result<PlanId, Problem> {
    let raw = raw.trim();
    let not_found = || Problem::field(pointer, "not_found", format!("plan {raw} not found"));
    match ids::parse_id::<PlanId>(raw) {
        Some(id) => reader
            .store()
            .plan_codes(tenant_id, &[id])
            .await?
            .contains_key(&id)
            .then_some(id)
            .ok_or_else(not_found),
        None => reader
            .store()
            .find_plan_id_by_code(tenant_id, raw)
            .await
            .map_err(|_| not_found()),
    }
}

/// The published version of a plan switched to: `version`'s number, else the active one.
async fn switch_version_in(
    reader: &impl Reader,
    tenant_id: TenantId,
    plan: Option<&str>,
    version: Option<i32>,
    pointer: &str,
) -> Result<PlanVersionId, Problem> {
    let plan = plan.ok_or_else(|| {
        Problem::field(
            format!("{pointer}/plan"),
            "required",
            "A switch names its plan",
        )
    })?;
    let plan_id = plan_in(reader, tenant_id, plan, &format!("{pointer}/plan")).await?;
    let filter = match version {
        Some(number) => PlanVersionFilter::Version(number),
        None => PlanVersionFilter::Active,
    };
    let full = reader
        .store()
        .get_full_plan(plan_id, tenant_id, filter)
        .await
        .map_err(|_| {
            Problem::field(
                format!("{pointer}/version"),
                "not_found",
                "The plan has no such published version",
            )
        })?;
    Ok(full.version.id)
}

fn no_plan_unless_switch(
    switch: bool,
    plan: Option<&str>,
    version: Option<i32>,
    pointer: &str,
) -> Result<(), Problem> {
    if switch || (plan.is_none() && version.is_none()) {
        return Ok(());
    }
    Err(Problem::field(
        format!("{pointer}/plan"),
        "invalid",
        "Only a switch names a plan",
    ))
}

pub async fn at_end_in(
    reader: &impl Reader,
    tenant_id: TenantId,
    input: &TrialAtEndInput,
    pointer: &str,
) -> Result<domain::TrialEndAction, Problem> {
    let switch = input.kind == TrialAtEndType::Switch;
    no_plan_unless_switch(switch, input.plan.as_deref(), input.version, pointer)?;
    Ok(match input.kind {
        TrialAtEndType::Convert => domain::TrialEndAction::Convert,
        TrialAtEndType::Manual => domain::TrialEndAction::Manual,
        TrialAtEndType::Cancel => domain::TrialEndAction::Cancel,
        TrialAtEndType::Switch => domain::TrialEndAction::Fallback {
            plan_version_id: switch_version_in(
                reader,
                tenant_id,
                input.plan.as_deref(),
                input.version,
                pointer,
            )
            .await?,
        },
    })
}

pub async fn at_end_out(
    reader: &impl Reader,
    tenant_id: TenantId,
    action: &domain::TrialEndAction,
) -> Result<TrialAtEnd, Problem> {
    Ok(match action {
        domain::TrialEndAction::Convert => TrialAtEnd {
            kind: TrialAtEndType::Convert,
            plan: None,
        },
        domain::TrialEndAction::Manual => TrialAtEnd {
            kind: TrialAtEndType::Manual,
            plan: None,
        },
        domain::TrialEndAction::Cancel => TrialAtEnd {
            kind: TrialAtEndType::Cancel,
            plan: None,
        },
        domain::TrialEndAction::Fallback { plan_version_id } => TrialAtEnd {
            kind: TrialAtEndType::Switch,
            plan: Some(version_ref(reader, tenant_id, *plan_version_id).await?),
        },
    })
}

pub async fn without_card_in(
    reader: &impl Reader,
    tenant_id: TenantId,
    input: &TrialWithoutCardInput,
    pointer: &str,
) -> Result<domain::TrialWithoutCard, Problem> {
    let switch = input.kind == TrialWithoutCardType::Switch;
    no_plan_unless_switch(switch, input.plan.as_deref(), input.version, pointer)?;
    Ok(match input.kind {
        TrialWithoutCardType::AwaitPayment => domain::TrialWithoutCard::AwaitPayment,
        TrialWithoutCardType::Cancel => domain::TrialWithoutCard::Cancel,
        TrialWithoutCardType::Switch => domain::TrialWithoutCard::Switch {
            plan_version_id: switch_version_in(
                reader,
                tenant_id,
                input.plan.as_deref(),
                input.version,
                pointer,
            )
            .await?,
        },
    })
}

pub async fn without_card_out(
    reader: &impl Reader,
    tenant_id: TenantId,
    without_card: &domain::TrialWithoutCard,
) -> Result<TrialWithoutCard, Problem> {
    Ok(match without_card {
        domain::TrialWithoutCard::AwaitPayment => TrialWithoutCard {
            kind: TrialWithoutCardType::AwaitPayment,
            plan: None,
        },
        domain::TrialWithoutCard::Cancel => TrialWithoutCard {
            kind: TrialWithoutCardType::Cancel,
            plan: None,
        },
        domain::TrialWithoutCard::Switch { plan_version_id } => TrialWithoutCard {
            kind: TrialWithoutCardType::Switch,
            plan: Some(version_ref(reader, tenant_id, *plan_version_id).await?),
        },
    })
}

/// A store refusal of the trial asked at `pointer`, at the field it refuses; none for another
/// refusal.
pub fn trial_refusal(
    error: &Report<StoreError>,
    request: &domain::TrialRequest,
    pointer: &str,
) -> Option<Problem> {
    use meteroid_store::errors::{OfferRefusal, TrialRefusal};
    if let Some(OfferRefusal::AlreadyGiven) = OfferRefusal::of(error) {
        let at = match request {
            domain::TrialRequest::Offer(_) => format!("{pointer}/offer"),
            domain::TrialRequest::Plan => pointer.to_owned(),
            _ => return None,
        };
        return Some(Problem::field(
            at,
            "invalid",
            "The customer already had this trial",
        ));
    }
    Some(match TrialRefusal::of(error)? {
        TrialRefusal::Billed => Problem::field(format!("{pointer}/free"), "invalid", TRIAL_IS_FREE),
        TrialRefusal::AlreadyHad => {
            let at = match request {
                domain::TrialRequest::Offer(_) => format!("{pointer}/offer"),
                _ => pointer.to_owned(),
            };
            Problem::field(at, "invalid", "The customer already had this trial")
        }
    })
}

/// A trial of its own ending `until` checked against its billing start, at `pointer`.
pub fn check_until(
    until: NaiveDate,
    billing_start: NaiveDate,
    pointer: &str,
) -> Result<(), Problem> {
    let days = (until - billing_start).num_days();
    if days <= 0 {
        return Err(Problem::field(
            pointer,
            "invalid",
            format!("A trial ends after its start, {billing_start}"),
        ));
    }
    if days > i64::from(domain::MAX_TRIAL_DAYS) {
        return Err(Problem::field(
            pointer,
            "invalid",
            format!("A trial lasts {} days at most", domain::MAX_TRIAL_DAYS),
        ));
    }
    Ok(())
}

/// A subscription's trial as its fact records it, with the offer it came from.
pub struct RecordedTrial {
    pub trial: domain::SubscriptionTrial,
    pub offer: Option<domain::TrialOffer>,
}

pub async fn recorded_trial(
    reader: &impl Reader,
    tenant_id: TenantId,
    details: &SubscriptionDetails,
) -> Result<Option<RecordedTrial>, Problem> {
    Ok(reader
        .services()
        .subscription_trial(tenant_id, details.subscription.id)
        .await?
        .map(|(trial, offer)| RecordedTrial { trial, offer }))
}

impl RecordedTrial {
    pub fn outcome(&self) -> Option<TrialOutcome> {
        self.trial.outcome.map(Into::into)
    }

    pub fn status(&self) -> TrialStatus {
        match self.trial.status() {
            domain::TrialStatus::Active => TrialStatus::Active,
            domain::TrialStatus::Converted => TrialStatus::Converted,
            domain::TrialStatus::Cancelled => TrialStatus::Cancelled,
            domain::TrialStatus::Switched => TrialStatus::Switched,
            domain::TrialStatus::AwaitingPayment => TrialStatus::AwaitingPayment,
            domain::TrialStatus::AwaitingDecision => TrialStatus::AwaitingDecision,
        }
    }

    /// Billing starts on its end for a free trial, on its start for one recorded billed.
    pub fn billing_starts_on(&self) -> NaiveDate {
        if self.trial.free {
            self.trial.until
        } else {
            self.trial.from
        }
    }

    /// Its end action as recorded, the plan of a switch read from its scheduled switch, else its
    /// offer's.
    pub fn end_action(&self, details: &SubscriptionDetails) -> domain::TrialEndAction {
        match self.trial.end_action {
            domain::TrialEnd::Convert => domain::TrialEndAction::Convert,
            domain::TrialEnd::Manual => domain::TrialEndAction::Manual,
            domain::TrialEnd::Cancel => domain::TrialEndAction::Cancel,
            domain::TrialEnd::Fallback => {
                let scheduled = details
                    .pending_events
                    .iter()
                    .filter(|event| event.scheduled_date == self.trial.until)
                    .find_map(|event| event.new_plan_version_id);
                let offered = self
                    .offer
                    .as_ref()
                    .and_then(|offer| match offer.end_action {
                        domain::TrialEndAction::Fallback { plan_version_id } => {
                            Some(plan_version_id)
                        }
                        _ => None,
                    });
                match scheduled.or(offered) {
                    Some(plan_version_id) => domain::TrialEndAction::Fallback { plan_version_id },
                    None => domain::TrialEndAction::Convert,
                }
            }
        }
    }

    pub fn without_card(&self) -> domain::TrialWithoutCard {
        self.trial.without_card
    }
}

/// Whether the subscription resolves a payment method it is charged or invoiced with.
pub async fn has_payment_method(
    reader: &impl Reader,
    tenant_id: TenantId,
    details: &SubscriptionDetails,
) -> Result<bool, Problem> {
    use meteroid_store::domain::customer_payment_methods::ResolvedPaymentMethod;
    use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
    let resolved = reader
        .store()
        .resolve_payment_method_for_subscription(tenant_id, details.subscription.id)
        .await?;
    Ok(!matches!(resolved, ResolvedPaymentMethod::NotConfigured))
}

/// The opening invoice of `details` billed from `on`, as the checkout prices a new subscription.
pub async fn opening_invoice(
    reader: &impl Reader,
    details: &SubscriptionDetails,
    on: NaiveDate,
) -> Result<meteroid_store::services::invoice_lines::invoice_lines::ComputedInvoiceContent, Problem>
{
    let mut from = details.clone();
    from.subscription.billing_start_date = Some(on);
    Ok(reader
        .services()
        .compute_new_subscription_invoice(&from)
        .await?)
}

/// What a running converting trial's end is estimated to charge; none when it cannot be priced.
pub async fn first_charge_estimate(
    reader: &impl Reader,
    details: &SubscriptionDetails,
    on: NaiveDate,
) -> Option<FirstChargeEstimate> {
    match opening_invoice(reader, details, on).await {
        Ok(content) => Some(FirstChargeEstimate {
            amount: content.total,
            currency: details.subscription.currency.clone(),
            on,
        }),
        Err(problem) => {
            log::warn!(
                "The first charge of subscription {}'s trial is not estimated: {problem:?}",
                details.subscription.id
            );
            None
        }
    }
}

/// What a self-serve checkout's subscription bills each period after its trial; none without a
/// trial or when it cannot be priced.
pub async fn after_trial_of_session(
    reader: &impl Reader,
    tenant_id: TenantId,
    session: &meteroid_store::domain::checkout_sessions::CheckoutSession,
) -> Option<AfterTrial> {
    let details = match reader
        .services()
        .build_preview_subscription_details(session, tenant_id, None)
        .await
    {
        Ok(details) => details,
        Err(error) => {
            log::warn!(
                "Checkout session {} is not priced after its trial: {error:?}",
                session.id
            );
            return None;
        }
    };
    match reader.services().after_trial(&details).await {
        Ok(after) => after.map(AfterTrial::from),
        Err(error) => {
            log::warn!(
                "Checkout session {} is not priced after its trial: {error:?}",
                session.id
            );
            None
        }
    }
}
