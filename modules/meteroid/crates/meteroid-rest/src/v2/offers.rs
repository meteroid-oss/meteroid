//! What offers read and write in v2: an offer gives a subscription a trial, grants or both beside
//! its plan. Shared by the offer catalog (`/offers`, Enterprise), a subscription's `grants` and the
//! `offer` a creation asks for.
use crate::AppState;
use crate::v2::Reader;
use crate::v2::bounds;
use crate::v2::coupons::CreditScope;
use crate::v2::entitlements::model::{EntitlementEffect, EntitlementValue, grant_value};
use crate::v2::ids;
use crate::v2::problem::{Problem, store_message};
use crate::v2::refs::{AddOnRef, FeatureRef, ReferenceIds, Refs};
use chrono::NaiveDate;
use common_domain::ids::{
    AddOnId, AppliedCouponId, CouponId, CreditGrantId, EntitlementId, OfferGrantId, OfferId,
    SubscriptionAddOnId, SubscriptionId, TenantId,
};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_app::features::gates;
use meteroid_store::domain::coupons::{CouponCreditScope, CouponDiscount};
use meteroid_store::domain::entitlement_fold::{Effect, Tier};
use meteroid_store::domain::enums::FeatureStatusEnum;
use meteroid_store::domain::offers as domain;
use meteroid_store::repositories::CatalogCodesInterface;
use meteroid_store::repositories::add_ons::AddOnInterface;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use utoipa::ToSchema;

/// The most grants an offer gives.
pub const MAX_GRANTS: usize = 50;

pub const OFFERS_NOT_IN_PLAN: &str = "Your plan does not include the offer catalog";

// ── The offer a creation asks for ───────────────────────────────────

/// The plan's start offer: its trial and grants, unless the customer already had it.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct OfferOfThePlan {}

/// Nothing the plan's start offer gives: no grants, and a trial only when `trial` asks one.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct NoOffer {}

/// A catalog offer in place of the plan's (Enterprise): refused when the customer already had it.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NamedOffer {
    /// The offer's id, `ofr_…`.
    pub offer: String,
}

/// What a new subscription is given beside its plan: the plan's start offer, none, or a catalog
/// offer. Its trial is the offer's unless `trial` asks another.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[schema(as = SubscriptionOfferInput)]
pub enum OfferInput {
    Plan(OfferOfThePlan),
    None(NoOffer),
    Offer(NamedOffer),
}

/// The offer asked at `pointer`, the plan's when left out. A catalog offer needs the
/// `subscription_phases` feature.
pub async fn offer_request(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    input: Option<&OfferInput>,
    pointer: &str,
) -> Result<domain::OfferRequest, Problem> {
    Ok(match input {
        None | Some(OfferInput::Plan(_)) => domain::OfferRequest::Plan,
        Some(OfferInput::None(_)) => domain::OfferRequest::None,
        Some(OfferInput::Offer(named)) => {
            if !state
                .feature_resolver
                .resolve(auth.organization_id)
                .await
                .is_enabled(gates::SUBSCRIPTION_PHASES)
            {
                return Err(Problem::feature_not_available(OFFERS_NOT_IN_PLAN));
            }
            let raw = named.offer.trim();
            let id = ids::parse_id::<OfferId>(raw).ok_or_else(|| {
                Problem::field(
                    format!("{pointer}/offer"),
                    "invalid",
                    format!("`{raw}` is not an offer's id, `ofr_…`"),
                )
            })?;
            domain::OfferRequest::Offer(id)
        }
    })
}

/// An offer request as it was asked, read back.
pub fn offer_input(request: domain::OfferRequest) -> OfferInput {
    match request {
        domain::OfferRequest::Plan => OfferInput::Plan(OfferOfThePlan {}),
        domain::OfferRequest::None => OfferInput::None(NoOffer {}),
        domain::OfferRequest::Offer(id) => OfferInput::Offer(NamedOffer {
            offer: id.to_string(),
        }),
    }
}

// ── Grants ──────────────────────────────────────────────────────────

/// What a grant gives: an invoice `discount`, promotional `credit`, a feature's `entitlement`
/// or an `add_on` (Enterprise).
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OfferGrantKind {
    Discount,
    Credit,
    Entitlement,
    AddOn,
}

impl From<domain::GrantKind> for OfferGrantKind {
    fn from(kind: domain::GrantKind) -> Self {
        match kind {
            domain::GrantKind::Discount => OfferGrantKind::Discount,
            domain::GrantKind::Credit => OfferGrantKind::Credit,
            domain::GrantKind::Entitlement => OfferGrantKind::Entitlement,
            domain::GrantKind::AddOn => OfferGrantKind::AddOn,
        }
    }
}

impl OfferGrantKind {
    /// The kind of an event's grant, as the event names it.
    pub fn named(raw: &str) -> Option<Self> {
        [
            domain::GrantKind::Discount,
            domain::GrantKind::Credit,
            domain::GrantKind::Entitlement,
            domain::GrantKind::AddOn,
        ]
        .into_iter()
        .find(|kind| kind.as_str() == raw)
        .map(Into::into)
    }
}

/// What a grant's window counts from: `start` the subscription's start, its trial's first day
/// when it has one; `billing_start` when billing starts, a free trial's end.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OfferGrantFrom {
    Start,
    BillingStart,
}

/// For as long as the subscription runs.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct OfferGrantForever {}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OfferGrantDays {
    /// 1 to 3,660.
    pub days: u32,
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OfferGrantMonths {
    /// 1 to 120.
    pub months: u32,
}

/// Until billing starts: the trial's free days.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct OfferGrantUntilBillingStarts {}

/// A discount's first invoices, as coupons made before durations in months count them. Read
/// only.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OfferGrantInvoices {
    pub invoices: u32,
}

/// How long a grant lasts from where its window counts.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OfferGrantDuration {
    Forever(OfferGrantForever),
    Days(OfferGrantDays),
    Months(OfferGrantMonths),
    Trial(OfferGrantUntilBillingStarts),
    Invoices(OfferGrantInvoices),
}

/// When a grant applies on a subscription. A discount counts from `billing_start`, for months
/// or forever; credit lasts days, months or forever; an add-on is given from the `start`, for
/// the trial or for months.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OfferGrantWindow {
    pub from: OfferGrantFrom,
    pub duration: OfferGrantDuration,
}

/// A discount on each invoice in its window: a `percent`, or an `amount` in a `currency`.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct OfferGrantDiscount {
    /// Above 0, at most 100.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal", example = "20")]
    pub percent: Option<Decimal>,
    /// In the major unit of `currency`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal", example = "10")]
    pub amount: Option<Decimal>,
    /// ISO 4217, with `amount`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    #[schema(schema_with = other_amounts)]
    pub amounts: BTreeMap<String, Decimal>,
    /// Only these add-ons' lines; the whole invoice when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub add_ons: Vec<AddOnRef>,
}

/// A discount as written: a `percent`, or an `amount` in a `currency`.
#[derive(Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OfferGrantDiscountInput {
    /// Above 0, at most 100.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "decimal", example = "20")]
    pub percent: Option<Decimal>,
    /// Positive, in the major unit of `currency`.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "decimal", example = "10")]
    pub amount: Option<Decimal>,
    /// ISO 4217, with `amount`.
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    #[schema(schema_with = other_amounts)]
    pub amounts: BTreeMap<String, Decimal>,
    /// Only these add-ons' lines, by code or id; the whole invoice when left out.
    #[serde(default)]
    pub add_ons: Vec<String>,
}

fn other_amounts() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
    crate::v2::charges::decimal_map(
        "With `amount`: the amount in other currencies, by ISO 4217 code; a subscription in a \
         currency with no amount is not discounted.",
        false,
    )
}

/// Promotional credit funded when the subscription starts, spendable for the window.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OfferGrantCredit {
    /// Positive, in the major unit of `currency`.
    #[schema(value_type = String, format = "decimal", example = "25")]
    pub amount: Decimal,
    /// ISO 4217: the currency of the subscriptions given it.
    pub currency: String,
    /// `customer` when left out.
    #[serde(default)]
    pub scope: CreditScope,
}

/// A feature given for the window, above what the plan gives.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct OfferGrantEntitlement {
    pub feature: FeatureRef,
    pub value: EntitlementValue,
    /// Absent: the feature's usual one for a subscription.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<EntitlementEffect>,
}

#[derive(Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OfferGrantEntitlementInput {
    /// The feature, by code or id.
    pub feature: String,
    pub value: EntitlementValue,
    /// The feature's usual one for a subscription when left out.
    #[serde(default)]
    pub effect: Option<EntitlementEffect>,
}

/// What a granted add-on does at its window's end: `revert` it is removed; `convert` it stays,
/// at its catalog price.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OfferGrantAddOnEnd {
    #[default]
    Revert,
    Convert,
}

/// An add-on given free for the window (Enterprise).
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct OfferGrantAddOn {
    pub add_on: AddOnRef,
    pub quantity: u32,
    pub at_end: OfferGrantAddOnEnd,
}

#[derive(Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OfferGrantAddOnInput {
    /// The add-on, by code or id.
    pub add_on: String,
    /// 1 when left out.
    #[serde(default = "one")]
    pub quantity: u32,
    /// `revert` when left out.
    #[serde(default)]
    pub at_end: OfferGrantAddOnEnd,
}

fn one() -> u32 {
    1
}

/// What an offer gives, for a window: exactly one of `discount`, `credit`, `entitlement` and
/// `add_on`, as `kind` says.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct OfferGrant {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: OfferGrantId,
    pub kind: OfferGrantKind,
    pub window: OfferGrantWindow,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discount: Option<OfferGrantDiscount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<OfferGrantCredit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entitlement: Option<OfferGrantEntitlement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_on: Option<OfferGrantAddOn>,
}

/// A grant written: exactly one of `discount`, `credit`, `entitlement` and `add_on`, naming
/// features and add-ons by code or id. A grant's `id` sent is ignored, its `kind` checked.
#[derive(Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OfferGrantInput {
    /// Read-only: ignored.
    #[serde(default)]
    #[schema(read_only)]
    pub id: Option<String>,
    /// Checked against the definition sent when given.
    #[serde(default)]
    pub kind: Option<OfferGrantKind>,
    pub window: OfferGrantWindow,
    #[serde(default)]
    pub discount: Option<OfferGrantDiscountInput>,
    #[serde(default)]
    pub credit: Option<OfferGrantCredit>,
    #[serde(default)]
    pub entitlement: Option<OfferGrantEntitlementInput>,
    #[serde(default)]
    pub add_on: Option<OfferGrantAddOnInput>,
}

fn window_out(window: domain::GrantWindow) -> OfferGrantWindow {
    OfferGrantWindow {
        from: match window.anchor {
            domain::GrantAnchor::Start => OfferGrantFrom::Start,
            domain::GrantAnchor::BillingStart => OfferGrantFrom::BillingStart,
        },
        duration: match window.duration {
            domain::GrantDuration::Forever => OfferGrantDuration::Forever(OfferGrantForever {}),
            domain::GrantDuration::Days(days) => OfferGrantDuration::Days(OfferGrantDays { days }),
            domain::GrantDuration::Months(months) => {
                OfferGrantDuration::Months(OfferGrantMonths { months })
            }
            domain::GrantDuration::UntilBillingStarts => {
                OfferGrantDuration::Trial(OfferGrantUntilBillingStarts {})
            }
            domain::GrantDuration::Invoices(invoices) => {
                OfferGrantDuration::Invoices(OfferGrantInvoices { invoices })
            }
        },
    }
}

fn window_in(window: &OfferGrantWindow) -> domain::GrantWindow {
    domain::GrantWindow {
        anchor: match window.from {
            OfferGrantFrom::Start => domain::GrantAnchor::Start,
            OfferGrantFrom::BillingStart => domain::GrantAnchor::BillingStart,
        },
        duration: match &window.duration {
            OfferGrantDuration::Forever(_) => domain::GrantDuration::Forever,
            OfferGrantDuration::Days(days) => domain::GrantDuration::Days(days.days),
            OfferGrantDuration::Months(months) => domain::GrantDuration::Months(months.months),
            OfferGrantDuration::Trial(_) => domain::GrantDuration::UntilBillingStarts,
            OfferGrantDuration::Invoices(invoices) => {
                domain::GrantDuration::Invoices(invoices.invoices)
            }
        },
    }
}

fn currency_in(raw: &str, pointer: &str) -> Result<String, Problem> {
    let code = raw.trim().to_ascii_uppercase();
    if code.len() == 3 && code.chars().all(|c| c.is_ascii_uppercase()) {
        Ok(code)
    } else {
        Err(Problem::field(
            pointer,
            "invalid",
            "An ISO 4217 currency code",
        ))
    }
}

async fn add_on_in(
    reader: &impl Reader,
    tenant_id: TenantId,
    raw: &str,
    pointer: &str,
) -> Result<AddOnId, Problem> {
    let raw = raw.trim();
    let found = match ids::parse_id::<AddOnId>(raw) {
        Some(id) => reader.store().get_add_on_by_id(tenant_id, id).await,
        None => match reader.store().find_add_on_id_by_code(tenant_id, raw).await {
            Ok(id) => reader.store().get_add_on_by_id(tenant_id, id).await,
            Err(error) => Err(error),
        },
    };
    let add_on = found
        .map_err(|_| Problem::field(pointer, "not_found", format!("add-on {raw} not found")))?;
    if add_on.archived_at.is_some() {
        return Err(Problem::field(
            pointer,
            "archived",
            format!("The add-on {} is archived", add_on.code),
        ));
    }
    Ok(add_on.id)
}

async fn discount_in(
    reader: &impl Reader,
    tenant_id: TenantId,
    input: &OfferGrantDiscountInput,
    at: &str,
) -> Result<domain::GrantDiscount, Problem> {
    let discount = match (input.percent, input.amount, input.currency.as_deref()) {
        (Some(percent), None, None) => {
            if percent <= Decimal::ZERO || percent > Decimal::ONE_HUNDRED {
                return Err(Problem::field(
                    format!("{at}/percent"),
                    "invalid",
                    "A percentage is above 0 and at most 100",
                ));
            }
            if !input.amounts.is_empty() {
                return Err(Problem::field(
                    format!("{at}/amounts"),
                    "invalid",
                    "Only an amount discount has amounts in other currencies",
                ));
            }
            CouponDiscount::Percentage(percent)
        }
        (None, Some(amount), Some(currency)) => {
            if amount <= Decimal::ZERO {
                return Err(Problem::field(
                    format!("{at}/amount"),
                    "invalid",
                    "The amount is positive",
                ));
            }
            CouponDiscount::Fixed {
                currency: currency_in(currency, &format!("{at}/currency"))?,
                amount,
            }
        }
        (None, Some(_), None) => {
            return Err(Problem::field(
                format!("{at}/currency"),
                "required",
                "An amount discount names its currency",
            ));
        }
        _ => {
            return Err(Problem::field(
                at,
                "invalid",
                "A discount is a `percent`, or an `amount` in a `currency`",
            ));
        }
    };
    let mut fixed_amounts = BTreeMap::new();
    for (code, amount) in &input.amounts {
        let pointer = format!("{at}/amounts/{code}");
        let code = currency_in(code, &pointer)?;
        if discount.currency() == Some(code.as_str()) {
            return Err(Problem::field(
                pointer,
                "invalid",
                format!("The discount is already in {code}: `amount`"),
            ));
        }
        if *amount <= Decimal::ZERO {
            return Err(Problem::field(pointer, "invalid", "The amount is positive"));
        }
        fixed_amounts.insert(code, *amount);
    }
    bounds::count(
        input.add_ons.len(),
        bounds::MAX_ADD_ONS,
        &format!("{at}/add_ons"),
        "add-ons",
    )?;
    let mut add_on_ids = Vec::with_capacity(input.add_ons.len());
    for (index, raw) in input.add_ons.iter().enumerate() {
        let id = add_on_in(reader, tenant_id, raw, &format!("{at}/add_ons/{index}")).await?;
        if !add_on_ids.contains(&id) {
            add_on_ids.push(id);
        }
    }
    Ok(domain::GrantDiscount {
        discount,
        fixed_amounts,
        add_on_ids,
    })
}

fn credit_in(input: &OfferGrantCredit, at: &str) -> Result<domain::GrantCredit, Problem> {
    if input.amount <= Decimal::ZERO {
        return Err(Problem::field(
            format!("{at}/amount"),
            "invalid",
            "The amount is positive",
        ));
    }
    Ok(domain::GrantCredit {
        currency: currency_in(&input.currency, &format!("{at}/currency"))?,
        amount: input.amount,
        scope: match input.scope {
            CreditScope::Customer => CouponCreditScope::Customer,
            CreditScope::Subscription => CouponCreditScope::Subscription,
        },
    })
}

async fn entitlement_in(
    reader: &impl Reader,
    tenant_id: TenantId,
    input: &OfferGrantEntitlementInput,
    at: &str,
) -> Result<domain::GrantEntitlement, Problem> {
    let raw = input.feature.trim();
    let pointer = format!("{at}/feature");
    let feature = ids::feature_in(reader.store(), tenant_id, raw)
        .await
        .map_err(|_| Problem::field(&pointer, "not_found", format!("No feature '{raw}'")))?;
    if feature.status == FeatureStatusEnum::Archived {
        return Err(Problem::field(
            pointer,
            "archived",
            format!("The feature '{}' is archived", feature.code),
        ));
    }
    let value = grant_value(&input.value, &feature.kind)
        .map_err(|message| Problem::field(format!("{at}/value"), "invalid", message))?;
    let effect = input.effect.map(Effect::from);
    if let Some(effect) = effect
        && !Tier::Subscription
            .allowed_effects(&feature.value_order())
            .contains(&effect)
    {
        return Err(Problem::field(
            format!("{at}/effect"),
            "invalid",
            "This feature's grant takes another effect",
        ));
    }
    Ok(domain::GrantEntitlement {
        feature_id: feature.id,
        value,
        effect,
    })
}

async fn grant_in(
    reader: &impl Reader,
    tenant_id: TenantId,
    input: &OfferGrantInput,
    at: &str,
) -> Result<domain::OfferGrantNew, Problem> {
    let definition = match (
        &input.discount,
        &input.credit,
        &input.entitlement,
        &input.add_on,
    ) {
        (Some(discount), None, None, None) => domain::GrantDefinition::Discount(
            discount_in(reader, tenant_id, discount, &format!("{at}/discount")).await?,
        ),
        (None, Some(credit), None, None) => {
            domain::GrantDefinition::Credit(credit_in(credit, &format!("{at}/credit"))?)
        }
        (None, None, Some(entitlement), None) => domain::GrantDefinition::Entitlement(
            entitlement_in(reader, tenant_id, entitlement, &format!("{at}/entitlement")).await?,
        ),
        (None, None, None, Some(add_on)) => {
            if add_on.quantity == 0 {
                return Err(Problem::field(
                    format!("{at}/add_on/quantity"),
                    "invalid",
                    "An add-on is given once at least",
                ));
            }
            domain::GrantDefinition::AddOn(domain::GrantAddOn {
                add_on_id: add_on_in(
                    reader,
                    tenant_id,
                    &add_on.add_on,
                    &format!("{at}/add_on/add_on"),
                )
                .await?,
                quantity: add_on.quantity,
                at_end: match add_on.at_end {
                    OfferGrantAddOnEnd::Revert => domain::AddOnEnd::Revert,
                    OfferGrantAddOnEnd::Convert => domain::AddOnEnd::Convert,
                },
            })
        }
        _ => {
            return Err(Problem::field(
                at,
                "invalid",
                "A grant gives one of `discount`, `credit`, `entitlement` and `add_on`",
            ));
        }
    };
    let kind = definition.kind();
    if input
        .kind
        .is_some_and(|sent| sent != OfferGrantKind::from(kind))
    {
        return Err(Problem::field(
            format!("{at}/kind"),
            "invalid",
            "The kind is not the definition's",
        ));
    }
    let window = window_in(&input.window);
    window.validate(kind).map_err(|error| {
        Problem::field(format!("{at}/window"), "invalid", store_message(&error))
    })?;
    Ok(domain::OfferGrantNew { window, definition })
}

/// The grants written at `pointer`, each checked at its index.
pub async fn grants_in(
    reader: &impl Reader,
    tenant_id: TenantId,
    inputs: &[OfferGrantInput],
    pointer: &str,
) -> Result<Vec<domain::OfferGrantNew>, Problem> {
    bounds::count(inputs.len(), MAX_GRANTS, pointer, "grants")?;
    let mut grants = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        grants.push(grant_in(reader, tenant_id, input, &format!("{pointer}/{index}")).await?);
    }
    Ok(grants)
}

/// The codes of the features and add-ons grants name, read once.
pub struct GrantNames(Refs);

impl GrantNames {
    pub async fn load<'a>(
        reader: &impl Reader,
        tenant_id: TenantId,
        grants: impl IntoIterator<Item = &'a domain::OfferGrant>,
    ) -> Result<Self, Problem> {
        let mut ids = ReferenceIds::default();
        for grant in grants {
            match &grant.definition {
                domain::GrantDefinition::Discount(discount) => {
                    ids.add_ons.extend(&discount.add_on_ids)
                }
                domain::GrantDefinition::Entitlement(entitlement) => {
                    ids.features.push(entitlement.feature_id)
                }
                domain::GrantDefinition::AddOn(add_on) => ids.add_ons.push(add_on.add_on_id),
                domain::GrantDefinition::Credit(_) => {}
            }
        }
        if ids.add_ons.is_empty() && ids.features.is_empty() {
            return Ok(GrantNames(Refs::default()));
        }
        Ok(GrantNames(
            Refs::load(reader.store(), tenant_id, &ids).await?,
        ))
    }

    fn terms(&self, definition: &domain::GrantDefinition) -> GrantTerms {
        let mut terms = GrantTerms::default();
        match definition {
            domain::GrantDefinition::Discount(discount) => {
                let (percent, amount, currency) = match &discount.discount {
                    CouponDiscount::Percentage(percent) => (Some(percent.normalize()), None, None),
                    CouponDiscount::Fixed { currency, amount }
                    | CouponDiscount::Credit {
                        currency, amount, ..
                    } => (None, Some(amount.normalize()), Some(currency.clone())),
                };
                terms.discount = Some(OfferGrantDiscount {
                    percent,
                    amount,
                    currency,
                    amounts: discount
                        .fixed_amounts
                        .iter()
                        .map(|(currency, amount)| (currency.clone(), amount.normalize()))
                        .collect(),
                    add_ons: discount
                        .add_on_ids
                        .iter()
                        .map(|id| self.0.add_on(*id))
                        .collect(),
                });
            }
            domain::GrantDefinition::Credit(credit) => {
                terms.credit = Some(OfferGrantCredit {
                    amount: credit.amount.normalize(),
                    currency: credit.currency.clone(),
                    scope: match credit.scope {
                        CouponCreditScope::Customer => CreditScope::Customer,
                        CouponCreditScope::Subscription => CreditScope::Subscription,
                    },
                });
            }
            domain::GrantDefinition::Entitlement(entitlement) => {
                terms.entitlement = Some(OfferGrantEntitlement {
                    feature: self.0.feature(entitlement.feature_id),
                    value: EntitlementValue::from(&entitlement.value),
                    effect: entitlement.effect.map(Into::into),
                });
            }
            domain::GrantDefinition::AddOn(add_on) => {
                terms.add_on = Some(OfferGrantAddOn {
                    add_on: self.0.add_on(add_on.add_on_id),
                    quantity: add_on.quantity,
                    at_end: match add_on.at_end {
                        domain::AddOnEnd::Revert => OfferGrantAddOnEnd::Revert,
                        domain::AddOnEnd::Convert => OfferGrantAddOnEnd::Convert,
                    },
                });
            }
        }
        terms
    }

    pub fn grant(&self, grant: &domain::OfferGrant) -> OfferGrant {
        let terms = self.terms(&grant.definition);
        OfferGrant {
            id: grant.id,
            kind: grant.kind().into(),
            window: window_out(grant.window),
            discount: terms.discount,
            credit: terms.credit,
            entitlement: terms.entitlement,
            add_on: terms.add_on,
        }
    }

    pub fn subscription_grant(&self, given: &domain::SubscriptionGrant) -> SubscriptionGrant {
        let terms = self.terms(&given.grant.definition);
        let mut rows = SubscriptionGrantRef::default();
        for target in &given.targets {
            match *target {
                domain::GrantTarget::AppliedCoupon(id) => {
                    rows.applied_coupon.get_or_insert(id);
                }
                domain::GrantTarget::CreditGrant(id) => {
                    rows.credit_grant.get_or_insert(id);
                }
                domain::GrantTarget::Entitlement(id) => {
                    rows.entitlement.get_or_insert(id);
                }
                domain::GrantTarget::AddOn {
                    add_on: Some(id), ..
                } => {
                    rows.add_on.get_or_insert(id);
                }
                domain::GrantTarget::AddOn { add_on: None, .. } => {}
            }
        }
        SubscriptionGrant {
            id: given.grant.id,
            kind: given.grant.kind().into(),
            status: given.status.into(),
            window: SubscriptionGrantDates {
                from: given.from,
                until: given.until,
            },
            source: SubscriptionGrantSource {
                kind: given.application.source.into(),
                offer: given.application.offer_id,
                coupon: given.application.coupon_id,
            },
            discount: terms.discount,
            credit: terms.credit,
            entitlement: terms.entitlement,
            add_on: terms.add_on,
            rows,
        }
    }
}

#[derive(Default)]
struct GrantTerms {
    discount: Option<OfferGrantDiscount>,
    credit: Option<OfferGrantCredit>,
    entitlement: Option<OfferGrantEntitlement>,
    add_on: Option<OfferGrantAddOn>,
}

// ── A subscription's grants ─────────────────────────────────────────

/// Where a subscription's grant stands: `scheduled` its window, or the subscription, has not
/// started; `active`; `ended` its window is over or it ended with the subscription;
/// `withdrawn` the subscription never started.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionGrantStatus {
    Scheduled,
    Active,
    Ended,
    Withdrawn,
}

impl From<domain::GrantStatus> for SubscriptionGrantStatus {
    fn from(status: domain::GrantStatus) -> Self {
        match status {
            domain::GrantStatus::Scheduled => SubscriptionGrantStatus::Scheduled,
            domain::GrantStatus::Active => SubscriptionGrantStatus::Active,
            domain::GrantStatus::Ended => SubscriptionGrantStatus::Ended,
            domain::GrantStatus::Withdrawn => SubscriptionGrantStatus::Withdrawn,
        }
    }
}

/// How an offer came to the subscription: `plan` its plan's start offer; `offer` a catalog offer
/// named by the tenant or a quote; `coupon` a coupon's code; `retention` the cancel flow; `api`
/// a catalog offer named through the API.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionGrantSourceType {
    Plan,
    Offer,
    Coupon,
    Retention,
    Api,
}

impl From<domain::OfferSource> for SubscriptionGrantSourceType {
    fn from(source: domain::OfferSource) -> Self {
        match source {
            domain::OfferSource::Plan => SubscriptionGrantSourceType::Plan,
            domain::OfferSource::Operator | domain::OfferSource::Quote => {
                SubscriptionGrantSourceType::Offer
            }
            domain::OfferSource::Coupon => SubscriptionGrantSourceType::Coupon,
            domain::OfferSource::Retention => SubscriptionGrantSourceType::Retention,
            domain::OfferSource::Api => SubscriptionGrantSourceType::Api,
        }
    }
}

impl SubscriptionGrantSourceType {
    /// The source an event names.
    pub fn named(raw: &str) -> Option<Self> {
        [
            domain::OfferSource::Plan,
            domain::OfferSource::Operator,
            domain::OfferSource::Coupon,
            domain::OfferSource::Retention,
            domain::OfferSource::Quote,
            domain::OfferSource::Api,
        ]
        .into_iter()
        .find(|source| source.as_str() == raw)
        .map(Into::into)
    }
}

/// Where a subscription's grant came from: the offer applied, and the coupon whose code gave it.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionGrantSource {
    #[serde(rename = "type")]
    pub kind: SubscriptionGrantSourceType,
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub offer: OfferId,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "common_domain::ids::string_serde_opt::serialize"
    )]
    pub coupon: Option<CouponId>,
}

/// A grant's window on the subscription.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionGrantDates {
    /// Its first day.
    pub from: NaiveDate,
    /// Exclusive; absent when it lasts as long as the subscription or counts invoices.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<NaiveDate>,
}

/// What the grant wrote on the subscription: the coupon redemption a discount or credit bills
/// through, the credit grant funded, the entitlement row, the add-on given.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq, Default)]
pub struct SubscriptionGrantRef {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "common_domain::ids::string_serde_opt::serialize"
    )]
    pub applied_coupon: Option<AppliedCouponId>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "common_domain::ids::string_serde_opt::serialize"
    )]
    pub credit_grant: Option<CreditGrantId>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "common_domain::ids::string_serde_opt::serialize"
    )]
    pub entitlement: Option<EntitlementId>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "common_domain::ids::string_serde_opt::serialize"
    )]
    pub add_on: Option<SubscriptionAddOnId>,
}

/// A grant of an offer applied to the subscription: what it gives, its window there, where it
/// came from and what it wrote.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct SubscriptionGrant {
    /// The offer's grant.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: OfferGrantId,
    pub kind: OfferGrantKind,
    pub status: SubscriptionGrantStatus,
    pub window: SubscriptionGrantDates,
    pub source: SubscriptionGrantSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discount: Option<OfferGrantDiscount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<OfferGrantCredit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entitlement: Option<OfferGrantEntitlement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_on: Option<OfferGrantAddOn>,
    #[serde(rename = "ref")]
    pub rows: SubscriptionGrantRef,
}

/// The grants of each of `subscriptions`, read in one round.
pub async fn subscriptions_grants(
    reader: &impl Reader,
    tenant_id: TenantId,
    subscriptions: &[SubscriptionId],
) -> Result<HashMap<SubscriptionId, Vec<SubscriptionGrant>>, Problem> {
    if subscriptions.is_empty() {
        return Ok(HashMap::new());
    }
    let given = reader
        .services()
        .subscriptions_grants(tenant_id, subscriptions)
        .await?;
    if given.is_empty() {
        return Ok(HashMap::new());
    }
    let names = GrantNames::load(
        reader,
        tenant_id,
        given.values().flatten().map(|grant| &grant.grant),
    )
    .await?;
    Ok(given
        .into_iter()
        .map(|(id, grants)| {
            (
                id,
                grants
                    .iter()
                    .map(|grant| names.subscription_grant(grant))
                    .collect(),
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;
    use serde_json::json;

    #[test]
    fn a_window_reads_as_written() {
        for written in [
            json!({"from": "billing_start", "duration": {"type": "months", "months": 3}}),
            json!({"from": "start", "duration": {"type": "days", "days": 30}}),
            json!({"from": "start", "duration": {"type": "trial"}}),
            json!({"from": "billing_start", "duration": {"type": "forever"}}),
        ] {
            let window: OfferGrantWindow = serde_json::from_value(written.clone()).unwrap();
            let read = serde_json::to_value(window_out(window_in(&window))).unwrap();
            assert_eq!(read, written);
        }
    }

    #[test]
    fn an_event_names_kinds_and_sources_as_the_store_does() {
        assert_eq!(
            OfferGrantKind::named(domain::GrantKind::AddOn.as_str()),
            Some(OfferGrantKind::AddOn)
        );
        assert_eq!(OfferGrantKind::named("NOPE"), None);
        assert_eq!(
            SubscriptionGrantSourceType::named(domain::OfferSource::Quote.as_str()),
            Some(SubscriptionGrantSourceType::Offer)
        );
        assert_eq!(
            SubscriptionGrantSourceType::named(domain::OfferSource::Api.as_str()),
            Some(SubscriptionGrantSourceType::Api)
        );
    }

    #[test]
    fn a_creation_offer_reads_back_as_asked() {
        let id = OfferId::new();
        let named = serde_json::to_value(offer_input(domain::OfferRequest::Offer(id))).unwrap();
        assert_eq!(named, json!({"type": "offer", "offer": id.to_string()}));
        let none: OfferInput = serde_json::from_value(json!({"type": "none"})).unwrap();
        assert_eq!(none, OfferInput::None(NoOffer {}));
        assert!(
            serde_json::from_value::<OfferInput>(json!({"type": "plan", "offer": "x"})).is_err()
        );
    }
}
