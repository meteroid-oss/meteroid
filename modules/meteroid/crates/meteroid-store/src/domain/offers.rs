//! Offers (ADR 0019): what a subscription is given beside its plan, a trial and grants. Each grant
//! writes into its own engine's rows: a discount or credit grant owns a coupon row, an
//! entitlement grant writes an entitlement row, an add-on grant a dated add-on.
use std::collections::BTreeMap;

use chrono::{DateTime, Days, Months, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    AddOnId, AppliedCouponId, CouponId, CreditGrantId, CustomerId, EntitlementId, FeatureId,
    OfferApplicationId, OfferGrantId, OfferId, PlanId, SubscriptionAddOnId, SubscriptionId,
};
use error_stack::Report;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::domain::coupons::{CouponCreditScope, CouponDiscount, CouponDuration};
use crate::domain::entitlement_fold::{Effect, GrantValue};
use crate::domain::trials::{
    TrialEligibility, TrialEndAction, TrialOffer, TrialTerms, TrialWithoutCard,
};
use crate::errors::{StoreError, StoreErrorReport};

/// An offer's trial: a free period before billing (R-153).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfferTrial {
    pub days: u32,
    /// True but for offers stored billed before trials v2.
    pub free: bool,
    pub trial_plan_id: Option<PlanId>,
    pub card_required: bool,
    pub end_action: TrialEndAction,
    pub without_card: TrialWithoutCard,
}

impl OfferTrial {
    pub fn of(offer: &TrialOffer) -> Self {
        OfferTrial {
            days: offer.duration_days,
            free: offer.free,
            trial_plan_id: offer.trial_plan_id,
            card_required: offer.card_required,
            end_action: offer.end_action,
            without_card: offer.without_card,
        }
    }

    /// The trial a plan editor's terms give, those left out the defaults.
    pub fn from_terms(terms: TrialTerms) -> Self {
        let terms = terms.over(None);
        OfferTrial {
            days: terms.duration_days,
            free: terms.free,
            trial_plan_id: terms.trial_plan_id,
            card_required: terms.card_required.unwrap_or(true),
            end_action: terms.end_action.unwrap_or(TrialEndAction::Convert),
            without_card: terms.without_card.unwrap_or_default(),
        }
    }

    pub fn terms(&self) -> TrialTerms {
        TrialTerms {
            duration_days: self.days,
            free: self.free,
            trial_plan_id: self.trial_plan_id,
            card_required: Some(self.card_required),
            end_action: Some(self.end_action),
            without_card: Some(self.without_card),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrantKind {
    Discount,
    Credit,
    Entitlement,
    AddOn,
}

impl GrantKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            GrantKind::Discount => "DISCOUNT",
            GrantKind::Credit => "CREDIT",
            GrantKind::Entitlement => "ENTITLEMENT",
            GrantKind::AddOn => "ADD_ON",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "DISCOUNT" => GrantKind::Discount,
            "CREDIT" => GrantKind::Credit,
            "ENTITLEMENT" => GrantKind::Entitlement,
            "ADD_ON" => GrantKind::AddOn,
            _ => return None,
        })
    }
}

/// What a grant's window is counted from (R-171).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantAnchor {
    /// The subscription's start: its trial's first day when it has one.
    Start,
    /// When billing starts: a free trial's end, else the billing start.
    BillingStart,
}

impl GrantAnchor {
    pub fn as_str(&self) -> &'static str {
        match self {
            GrantAnchor::Start => "START",
            GrantAnchor::BillingStart => "BILLING_START",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "START" => GrantAnchor::Start,
            "BILLING_START" => GrantAnchor::BillingStart,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantDuration {
    Forever,
    Days(u32),
    Months(u32),
    /// Until billing starts: the trial's free segment.
    UntilBillingStarts,
    /// A discount's first invoices, as coupons made before durations in months count them.
    Invoices(u32),
}

impl GrantDuration {
    fn columns(&self) -> (&'static str, Option<i32>) {
        let value = |n: u32| Some(i32::try_from(n).unwrap_or(i32::MAX));
        match self {
            GrantDuration::Forever => ("FOREVER", None),
            GrantDuration::Days(n) => ("DAYS", value(*n)),
            GrantDuration::Months(n) => ("MONTHS", value(*n)),
            GrantDuration::UntilBillingStarts => ("TRIAL", None),
            GrantDuration::Invoices(n) => ("INVOICES", value(*n)),
        }
    }

    fn parse(kind: &str, value: Option<i32>) -> Option<Self> {
        let n = || value.map(|v| u32::try_from(v).unwrap_or(0));
        Some(match kind {
            "FOREVER" => GrantDuration::Forever,
            "DAYS" => GrantDuration::Days(n()?),
            "MONTHS" => GrantDuration::Months(n()?),
            "TRIAL" => GrantDuration::UntilBillingStarts,
            "INVOICES" => GrantDuration::Invoices(n()?),
            _ => return None,
        })
    }
}

/// When a grant applies on a subscription: from its anchor, for its duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantWindow {
    pub anchor: GrantAnchor,
    pub duration: GrantDuration,
}

/// Longest a grant lasts in days or months.
pub const MAX_GRANT_DAYS: u32 = 3_660;
pub const MAX_GRANT_MONTHS: u32 = 120;

impl GrantWindow {
    /// Its first day and its end (exclusive, `None` forever or counted in invoices) on a
    /// subscription starting `start` and billing from `billing_start`.
    pub fn dates(
        &self,
        start: NaiveDate,
        billing_start: NaiveDate,
    ) -> (NaiveDate, Option<NaiveDate>) {
        let from = match self.anchor {
            GrantAnchor::Start => start,
            GrantAnchor::BillingStart => billing_start,
        };
        let until = match self.duration {
            GrantDuration::Forever | GrantDuration::Invoices(_) => None,
            GrantDuration::Days(days) => Some(
                from.checked_add_days(Days::new(u64::from(days)))
                    .unwrap_or(NaiveDate::MAX),
            ),
            GrantDuration::Months(months) => Some(
                from.checked_add_months(Months::new(months))
                    .unwrap_or(NaiveDate::MAX),
            ),
            GrantDuration::UntilBillingStarts => Some(billing_start),
        };
        (from, until)
    }

    /// When the row a grant of `kind` wrote ends on its own (R-178), `None` never: credit when its
    /// funded grant expires (`funded_expires_at`, `None` for its applied coupon), the others at
    /// their window's end, midnight UTC.
    pub fn ends_at(
        &self,
        kind: GrantKind,
        start: NaiveDate,
        billing_start: NaiveDate,
        funded_expires_at: Option<DateTime<Utc>>,
    ) -> Option<NaiveDateTime> {
        if matches!(
            self.duration,
            GrantDuration::Forever | GrantDuration::Invoices(_)
        ) {
            return None;
        }
        match kind {
            GrantKind::Credit => funded_expires_at.map(|at| at.naive_utc()),
            _ => self
                .dates(start, billing_start)
                .1
                .map(|until| until.and_time(NaiveTime::MIN)),
        }
    }

    /// The windows each kind takes (R-171): a discount bills, so it counts from billing.
    pub fn validate(&self, kind: GrantKind) -> Result<(), StoreError> {
        let invalid = |message: &str| Err(StoreError::InvalidArgument(message.to_owned()));
        match self.duration {
            GrantDuration::Days(days) if !(1..=MAX_GRANT_DAYS).contains(&days) => {
                return invalid("a grant lasts 1 to 3,660 days");
            }
            GrantDuration::Months(months) if !(1..=MAX_GRANT_MONTHS).contains(&months) => {
                return invalid("a grant lasts 1 to 120 months");
            }
            GrantDuration::Invoices(_) => {
                return invalid("a grant lasts days, months, until billing starts or forever");
            }
            _ => {}
        }
        match kind {
            GrantKind::Discount => {
                if self.anchor != GrantAnchor::BillingStart
                    || !matches!(
                        self.duration,
                        GrantDuration::Forever | GrantDuration::Months(_)
                    )
                {
                    return invalid(
                        "a discount counts from billing's start, for months or forever",
                    );
                }
            }
            GrantKind::Credit => {
                if self.duration == GrantDuration::UntilBillingStarts {
                    return invalid("credit lasts days, months or forever");
                }
            }
            GrantKind::Entitlement => {}
            GrantKind::AddOn => {
                if self.anchor != GrantAnchor::Start
                    || !matches!(
                        self.duration,
                        GrantDuration::UntilBillingStarts | GrantDuration::Months(_)
                    )
                {
                    return invalid(
                        "an add-on is given from the start, until billing starts or for months",
                    );
                }
            }
        }
        Ok(())
    }
}

/// What a dated add-on does at its window's end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOnEnd {
    /// It is removed.
    Revert,
    /// It stays, at its catalog price.
    Convert,
}

impl AddOnEnd {
    pub fn as_str(&self) -> &'static str {
        match self {
            AddOnEnd::Revert => "REVERT",
            AddOnEnd::Convert => "CONVERT",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "REVERT" => AddOnEnd::Revert,
            "CONVERT" => AddOnEnd::Convert,
            _ => return None,
        })
    }
}

/// A discount grant's definition: its coupon row's discount and scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantDiscount {
    /// A percentage or a fixed amount; never credit.
    pub discount: CouponDiscount,
    pub fixed_amounts: BTreeMap<String, Decimal>,
    /// Only these add-ons' lines; empty: the whole invoice.
    pub add_on_ids: Vec<AddOnId>,
}

/// Promotional credit, funded when the subscription starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantCredit {
    pub currency: String,
    pub amount: Decimal,
    pub scope: CouponCreditScope,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrantEntitlement {
    pub feature_id: FeatureId,
    pub value: GrantValue,
    /// `None` takes the feature's default.
    pub effect: Option<Effect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantAddOn {
    pub add_on_id: AddOnId,
    pub quantity: u32,
    pub at_end: AddOnEnd,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GrantDefinition {
    Discount(GrantDiscount),
    Credit(GrantCredit),
    Entitlement(GrantEntitlement),
    AddOn(GrantAddOn),
}

impl GrantDefinition {
    pub fn kind(&self) -> GrantKind {
        match self {
            GrantDefinition::Discount(_) => GrantKind::Discount,
            GrantDefinition::Credit(_) => GrantKind::Credit,
            GrantDefinition::Entitlement(_) => GrantKind::Entitlement,
            GrantDefinition::AddOn(_) => GrantKind::AddOn,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OfferGrant {
    pub id: OfferGrantId,
    pub offer_id: OfferId,
    pub seq: u32,
    pub window: GrantWindow,
    pub definition: GrantDefinition,
    /// A discount or credit grant's coupon row.
    pub coupon_id: Option<CouponId>,
    /// Replaced by an edit: applications made before keep it.
    pub removed_at: Option<NaiveDateTime>,
}

impl OfferGrant {
    pub fn kind(&self) -> GrantKind {
        self.definition.kind()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OfferGrantNew {
    pub window: GrantWindow,
    pub definition: GrantDefinition,
}

/// The `entitlement` column of an entitlement grant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredEntitlement {
    pub value: GrantValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<Effect>,
}

pub(crate) fn kind_and_window_of_row(
    row: &diesel_models::offers::OfferGrantRow,
) -> Result<(GrantKind, GrantWindow), StoreErrorReport> {
    let bad = |what: &str| {
        Report::new(StoreError::InvalidArgument(format!(
            "offer grant {}: {what}",
            row.id
        )))
    };
    let kind = GrantKind::parse(&row.kind).ok_or_else(|| bad("unknown kind"))?;
    let window = GrantWindow {
        anchor: GrantAnchor::parse(&row.anchor).ok_or_else(|| bad("unknown anchor"))?,
        duration: GrantDuration::parse(&row.duration_kind, row.duration_value)
            .ok_or_else(|| bad("unknown duration"))?,
    };
    Ok((kind, window))
}

/// A grant row's window and kind-specific columns, the coupon's definition read beside it.
pub(crate) fn grant_of_row(
    row: diesel_models::offers::OfferGrantRow,
    coupon: Option<&crate::domain::coupons::Coupon>,
) -> Result<OfferGrant, StoreErrorReport> {
    let bad = |what: &str| {
        Report::new(StoreError::InvalidArgument(format!(
            "offer grant {}: {what}",
            row.id
        )))
    };
    let (kind, window) = kind_and_window_of_row(&row)?;
    let definition = match kind {
        GrantKind::Discount | GrantKind::Credit => {
            let coupon = coupon.ok_or_else(|| bad("its coupon is missing"))?;
            match &coupon.discount {
                CouponDiscount::Credit {
                    currency,
                    amount,
                    scope,
                    ..
                } => GrantDefinition::Credit(GrantCredit {
                    currency: currency.clone(),
                    amount: *amount,
                    scope: *scope,
                }),
                discount => GrantDefinition::Discount(GrantDiscount {
                    discount: discount.clone(),
                    fixed_amounts: coupon.fixed_amounts.clone(),
                    add_on_ids: coupon.add_on_ids.clone(),
                }),
            }
        }
        GrantKind::Entitlement => {
            let stored: StoredEntitlement = serde_json::from_value(
                row.entitlement
                    .clone()
                    .ok_or_else(|| bad("no entitlement"))?,
            )
            .map_err(|_| bad("unreadable entitlement"))?;
            GrantDefinition::Entitlement(GrantEntitlement {
                feature_id: row.feature_id.ok_or_else(|| bad("no feature"))?,
                value: stored.value,
                effect: stored.effect,
            })
        }
        GrantKind::AddOn => GrantDefinition::AddOn(GrantAddOn {
            add_on_id: row.add_on_id.ok_or_else(|| bad("no add-on"))?,
            quantity: row
                .quantity
                .and_then(|q| u32::try_from(q).ok())
                .ok_or_else(|| bad("no quantity"))?,
            at_end: row
                .add_on_end
                .as_deref()
                .and_then(AddOnEnd::parse)
                .ok_or_else(|| bad("unknown add-on end"))?,
        }),
    };
    Ok(OfferGrant {
        id: row.id,
        offer_id: row.offer_id,
        seq: u32::try_from(row.seq).unwrap_or(0),
        window,
        definition,
        coupon_id: row.coupon_id,
        removed_at: row.removed_at,
    })
}

/// A grant's row, its coupon (`coupon_id`) written beforehand.
pub(crate) fn grant_row(
    tenant_id: common_domain::ids::TenantId,
    offer_id: OfferId,
    seq: u32,
    grant: &OfferGrantNew,
    coupon_id: Option<CouponId>,
    now: NaiveDateTime,
) -> Result<diesel_models::offers::OfferGrantRow, StoreErrorReport> {
    let (duration_kind, duration_value) = grant.window.duration.columns();
    let mut row = diesel_models::offers::OfferGrantRow {
        id: OfferGrantId::from(uuid::Uuid::now_v7()),
        tenant_id,
        offer_id,
        seq: i32::try_from(seq).unwrap_or(i32::MAX),
        kind: grant.definition.kind().as_str().to_string(),
        anchor: grant.window.anchor.as_str().to_string(),
        duration_kind: duration_kind.to_string(),
        duration_value,
        coupon_id,
        feature_id: None,
        entitlement: None,
        add_on_id: None,
        quantity: None,
        add_on_end: None,
        removed_at: None,
        created_at: now,
    };
    match &grant.definition {
        GrantDefinition::Discount(_) | GrantDefinition::Credit(_) => {}
        GrantDefinition::Entitlement(entitlement) => {
            row.feature_id = Some(entitlement.feature_id);
            row.entitlement = Some(
                serde_json::to_value(StoredEntitlement {
                    value: entitlement.value.clone(),
                    effect: entitlement.effect,
                })
                .map_err(|e| {
                    Report::new(StoreError::SerdeError("grant entitlement".to_string(), e))
                })?,
            );
        }
        GrantDefinition::AddOn(add_on) => {
            row.add_on_id = Some(add_on.add_on_id);
            row.quantity = Some(i32::try_from(add_on.quantity).unwrap_or(i32::MAX));
            row.add_on_end = Some(add_on.at_end.as_str().to_string());
        }
    }
    Ok(row)
}

/// A legacy coupon's window, as its wrapper's grant states it.
pub fn window_of_coupon(coupon: &crate::domain::coupons::Coupon) -> GrantWindow {
    match &coupon.discount {
        CouponDiscount::Credit {
            expires_in_days, ..
        } => GrantWindow {
            anchor: GrantAnchor::Start,
            duration: expires_in_days.map_or(GrantDuration::Forever, GrantDuration::Days),
        },
        _ => GrantWindow {
            anchor: GrantAnchor::BillingStart,
            duration: match coupon.duration() {
                CouponDuration::Forever => GrantDuration::Forever,
                CouponDuration::Months(n) => GrantDuration::Months(n),
                CouponDuration::Invoices(n) => GrantDuration::Invoices(n),
            },
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub id: OfferId,
    pub name: String,
    pub description: Option<String>,
    /// `None`: it gives its grants only.
    pub trial: Option<OfferTrial>,
    pub grants: Vec<OfferGrant>,
    /// Who is given it once; the trial's and every grant's (R-173).
    pub eligibility: TrialEligibility,
    /// Listed in the catalog; a plan's own offer, or a coupon's, is not.
    pub standalone: bool,
    pub archived_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl Offer {
    /// Today's coupon in the Core edition: one discount or credit grant, no trial.
    pub fn is_one_coupon_grant(&self) -> bool {
        self.trial.is_none()
            && matches!(self.grants.as_slice(), [grant] if matches!(
                grant.kind(),
                GrantKind::Discount | GrantKind::Credit
            ))
    }
}

#[derive(Debug, Clone)]
pub struct OfferNew {
    pub name: String,
    pub description: Option<String>,
    pub trial: Option<OfferTrial>,
    pub grants: Vec<OfferGrantNew>,
    pub eligibility: TrialEligibility,
}

/// An offer's edit, field by field, for applications to come: subscriptions it was applied to
/// keep what they were given. `grants` replaces them all.
#[derive(Debug, Clone, Default)]
pub struct OfferPatch {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub trial: Option<Option<OfferTrial>>,
    pub grants: Option<Vec<OfferGrantNew>>,
    pub eligibility: Option<TrialEligibility>,
}

/// The offer a creation asks for (R-170).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OfferRequest {
    /// The plan's start offer.
    #[default]
    Plan,
    None,
    /// A catalog offer, in place of the plan's.
    Offer(OfferId),
}

/// An offer request as a checkout session stores it.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StoredOfferRequest {
    Plan,
    None,
    Offer { id: OfferId },
}

impl OfferRequest {
    /// As a checkout session stores it; the plan's, asked by default, is stored as nothing.
    pub(crate) fn stored(&self) -> Option<serde_json::Value> {
        let stored = match self {
            OfferRequest::Plan => return None,
            OfferRequest::None => StoredOfferRequest::None,
            OfferRequest::Offer(id) => StoredOfferRequest::Offer { id: *id },
        };
        serde_json::to_value(stored).ok()
    }

    pub(crate) fn from_stored(
        session: common_domain::ids::CheckoutSessionId,
        value: Option<serde_json::Value>,
    ) -> Self {
        let Some(value) = value else {
            return OfferRequest::Plan;
        };
        match serde_json::from_value::<StoredOfferRequest>(value) {
            Ok(StoredOfferRequest::Plan) => OfferRequest::Plan,
            Ok(StoredOfferRequest::None) => OfferRequest::None,
            Ok(StoredOfferRequest::Offer { id }) => OfferRequest::Offer(id),
            Err(error) => {
                log::error!("Checkout session {session} stores an offer it cannot read: {error}");
                OfferRequest::None
            }
        }
    }
}

/// Where an application of an offer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OfferSource {
    Plan,
    Operator,
    Coupon,
    Retention,
    Quote,
    Api,
}

impl OfferSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            OfferSource::Plan => "PLAN",
            OfferSource::Operator => "OPERATOR",
            OfferSource::Coupon => "COUPON",
            OfferSource::Retention => "RETENTION",
            OfferSource::Quote => "QUOTE",
            OfferSource::Api => "API",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "PLAN" => OfferSource::Plan,
            "OPERATOR" => OfferSource::Operator,
            "COUPON" => OfferSource::Coupon,
            "RETENTION" => OfferSource::Retention,
            "QUOTE" => OfferSource::Quote,
            "API" => OfferSource::Api,
            _ => return None,
        })
    }

    /// An offer named at a creation, by who asked.
    pub fn of_actor(actor: &Actor) -> Self {
        match actor {
            Actor::ApiToken { .. } | Actor::OAuthApp { .. } => OfferSource::Api,
            _ => OfferSource::Operator,
        }
    }
}

/// A plan's start offer written from the plan: its own terms and grants, a catalog offer (EE),
/// or none.
#[derive(Debug, Clone)]
pub enum StartOfferChange {
    Inline {
        trial: Option<TrialTerms>,
        grants: Vec<OfferGrantNew>,
        /// `None` keeps the plan's own offer's, else once per product family.
        eligibility: Option<TrialEligibility>,
    },
    Catalog(OfferId),
    None,
}

/// What gives an offer: the plans naming it as their start offer, the coupons whose code gives
/// it, the retention offers; and the subscriptions it was given to, withdrawn ones aside.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OfferUse {
    pub plans: Vec<PlanId>,
    pub coupons: Vec<OfferCoupon>,
    pub retention: Vec<common_domain::ids::RetentionOfferId>,
    pub applications: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferCoupon {
    pub id: CouponId,
    pub code: String,
}

/// A plan's start offer, and every plan naming it.
#[derive(Debug, Clone)]
pub struct PlanStartOffer {
    pub offer: Offer,
    pub plans: Vec<PlanId>,
}

/// The offer a creation gets, resolved once with its trial (R-170).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedOffer {
    pub offer_id: OfferId,
    pub source: OfferSource,
    pub grants: Vec<OfferGrant>,
}

/// The engine row a grant wrote on a subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantTarget {
    AppliedCoupon(AppliedCouponId),
    CreditGrant(CreditGrantId),
    Entitlement(EntitlementId),
    /// A dated add-on: its timeline item, and the add-on it gives.
    AddOn {
        item_id: uuid::Uuid,
        add_on: Option<SubscriptionAddOnId>,
    },
}

impl GrantTarget {
    pub(crate) fn columns(&self) -> (&'static str, uuid::Uuid) {
        use common_domain::ids::BaseId;
        match self {
            GrantTarget::AppliedCoupon(id) => ("APPLIED_COUPON", id.as_uuid()),
            GrantTarget::CreditGrant(id) => ("CREDIT_GRANT", id.as_uuid()),
            GrantTarget::Entitlement(id) => ("ENTITLEMENT", id.as_uuid()),
            GrantTarget::AddOn { item_id, .. } => ("ADD_ON", *item_id),
        }
    }

    pub(crate) fn parse(kind: &str, target: uuid::Uuid) -> Option<Self> {
        Some(match kind {
            "APPLIED_COUPON" => GrantTarget::AppliedCoupon(target.into()),
            "CREDIT_GRANT" => GrantTarget::CreditGrant(target.into()),
            "ENTITLEMENT" => GrantTarget::Entitlement(target.into()),
            "ADD_ON" => GrantTarget::AddOn {
                item_id: target,
                add_on: None,
            },
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantStatus {
    /// Its window, or the subscription, has not started.
    Scheduled,
    Active,
    /// Its window is over, or it ended with its subscription.
    Ended,
    /// The subscription never started.
    Withdrawn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferApplication {
    pub id: OfferApplicationId,
    pub offer_id: OfferId,
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
    pub coupon_id: Option<CouponId>,
    pub source: OfferSource,
    pub start_on: NaiveDate,
    pub billing_start_on: NaiveDate,
    pub applied_at: NaiveDateTime,
    pub actor: Actor,
    pub started_at: Option<NaiveDateTime>,
    pub withdrawn_at: Option<NaiveDateTime>,
    /// The grants it gives, as the offer had them when applied.
    pub grant_ids: Vec<OfferGrantId>,
}

impl TryFrom<diesel_models::offers::OfferApplicationRow> for OfferApplication {
    type Error = StoreErrorReport;

    fn try_from(row: diesel_models::offers::OfferApplicationRow) -> Result<Self, Self::Error> {
        Ok(OfferApplication {
            id: row.id,
            offer_id: row.offer_id,
            customer_id: row.customer_id,
            subscription_id: row.subscription_id,
            coupon_id: row.coupon_id,
            source: OfferSource::parse(&row.source).ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(format!(
                    "offer application {}: unknown source",
                    row.id
                )))
            })?,
            start_on: row.start_on,
            billing_start_on: row.billing_start_on,
            applied_at: row.applied_at,
            actor: Actor::from_parts(row.actor_type.into(), row.actor_uuid, row.actor_alias),
            started_at: row.started_at,
            withdrawn_at: row.withdrawn_at,
            grant_ids: row.grant_ids.into_iter().flatten().collect(),
        })
    }
}

/// A grant as a subscription has it: its window's dates there, its status, where it came from and
/// the rows it wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscriptionGrant {
    pub grant: OfferGrant,
    pub application: OfferApplication,
    pub status: GrantStatus,
    pub from: NaiveDate,
    pub until: Option<NaiveDate>,
    pub targets: Vec<GrantTarget>,
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};

    use super::{GrantAnchor, GrantDuration, GrantKind, GrantWindow};

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn midnight(y: i32, m: u32, d: u32) -> Option<NaiveDateTime> {
        day(y, m, d).and_hms_opt(0, 0, 0)
    }

    fn window(anchor: GrantAnchor, duration: GrantDuration) -> GrantWindow {
        GrantWindow { anchor, duration }
    }

    const START: (i32, u32, u32) = (2026, 1, 10);
    const BILLING_START: (i32, u32, u32) = (2026, 1, 31);

    fn ends_at(window: GrantWindow, kind: GrantKind) -> Option<NaiveDateTime> {
        let (sy, sm, sd) = START;
        let (by, bm, bd) = BILLING_START;
        window.ends_at(kind, day(sy, sm, sd), day(by, bm, bd), None)
    }

    #[test]
    fn days_count_from_the_anchor() {
        assert_eq!(
            ends_at(
                window(GrantAnchor::Start, GrantDuration::Days(30)),
                GrantKind::Entitlement
            ),
            midnight(2026, 2, 9)
        );
        assert_eq!(
            ends_at(
                window(GrantAnchor::BillingStart, GrantDuration::Days(1)),
                GrantKind::Entitlement
            ),
            midnight(2026, 2, 1)
        );
    }

    #[test]
    fn months_are_calendar_months_clamped_to_the_month_end() {
        assert_eq!(
            ends_at(
                window(GrantAnchor::BillingStart, GrantDuration::Months(1)),
                GrantKind::Discount
            ),
            midnight(2026, 2, 28)
        );
        assert_eq!(
            ends_at(
                window(GrantAnchor::BillingStart, GrantDuration::Months(3)),
                GrantKind::Discount
            ),
            midnight(2026, 4, 30)
        );
        assert_eq!(
            ends_at(
                window(GrantAnchor::Start, GrantDuration::Months(2)),
                GrantKind::AddOn
            ),
            midnight(2026, 3, 10)
        );
    }

    #[test]
    fn until_billing_starts_ends_with_the_trial() {
        assert_eq!(
            ends_at(
                window(GrantAnchor::Start, GrantDuration::UntilBillingStarts),
                GrantKind::AddOn
            ),
            midnight(2026, 1, 31)
        );
    }

    #[test]
    fn forever_and_invoices_never_end_on_their_own() {
        assert_eq!(
            ends_at(
                window(GrantAnchor::BillingStart, GrantDuration::Forever),
                GrantKind::Discount
            ),
            None
        );
        assert_eq!(
            ends_at(
                window(GrantAnchor::BillingStart, GrantDuration::Invoices(3)),
                GrantKind::Discount
            ),
            None
        );
        let expires = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
        assert_eq!(
            window(GrantAnchor::Start, GrantDuration::Forever).ends_at(
                GrantKind::Credit,
                day(2026, 1, 10),
                day(2026, 1, 31),
                Some(expires)
            ),
            None
        );
    }

    #[test]
    fn credit_ends_as_funded() {
        let expires = Utc.with_ymd_and_hms(2026, 3, 2, 15, 30, 0).unwrap();
        let credit = window(GrantAnchor::BillingStart, GrantDuration::Days(30));
        assert_eq!(
            credit.ends_at(
                GrantKind::Credit,
                day(2026, 1, 10),
                day(2026, 1, 31),
                Some(expires)
            ),
            Some(expires.naive_utc())
        );
        assert_eq!(
            credit.ends_at(GrantKind::Credit, day(2026, 1, 10), day(2026, 1, 31), None),
            None
        );
    }
}
