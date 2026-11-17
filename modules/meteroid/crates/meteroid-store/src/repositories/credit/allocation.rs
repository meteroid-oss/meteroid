//! The store's grants and requests as `billing_core::credit::allocation` reads them: which
//! grants pay how much of an invoice is decided there.
use crate::StoreResult;
use crate::domain::credit::{
    CreditApplication, CreditGrant, CreditGrantState, GrantClassification, GrantRollover,
    GrantScope,
};
use crate::domain::credit_units::DenominationCode;
use crate::services::credit::invalid;
use billing_core::credit::allocation as core;
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, CreditGrantId, ProductId, SubscriptionId};

/// The grants a draw may take. Money credit is drawn from promotional grants before tax and from
/// paid grants after it; a credit unit draws all of its grants at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantPool {
    All,
    Promotional,
    Paid,
}

impl GrantPool {
    pub fn admits(self, classification: GrantClassification) -> bool {
        core::Pool::from(self).admits(classification.into())
    }

    /// The pools usage of `denomination` is paid from in turn, when no tax stands between them.
    pub fn sequence(denomination: &str) -> &'static [GrantPool] {
        if DenominationCode::parse(denomination).is_ok_and(|code| code.is_unit()) {
            &[Self::All]
        } else {
            &[Self::Promotional, Self::Paid]
        }
    }
}

impl From<GrantPool> for core::Pool {
    fn from(pool: GrantPool) -> Self {
        match pool {
            GrantPool::All => core::Pool::All,
            GrantPool::Promotional => core::Pool::Promotional,
            GrantPool::Paid => core::Pool::Paid,
        }
    }
}

impl From<GrantClassification> for core::Classification {
    fn from(classification: GrantClassification) -> Self {
        match classification {
            GrantClassification::Paid => core::Classification::Paid,
            GrantClassification::Promotional => core::Classification::Promotional,
        }
    }
}

impl From<&GrantScope> for core::Scope {
    fn from(scope: &GrantScope) -> Self {
        core::Scope {
            subscription: scope.subscription_id.map(|id| id.as_uuid()),
            application: match scope.application {
                CreditApplication::Invoice => core::Application::Invoice,
                CreditApplication::Usage => core::Application::Usage,
            },
            products: scope.product_ids.iter().map(|id| id.as_uuid()).collect(),
        }
    }
}

/// A charge credit may pay, at most `amount_minor` of it: the usage, or the other charges, of
/// one subscription (None: of none) and product (None: lines of no product).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditDemand {
    pub subscription_id: Option<SubscriptionId>,
    pub product_id: Option<ProductId>,
    pub usage: bool,
    pub amount_minor: i64,
    /// Usage only: the end of the dates it was consumed in. Only credit effective before it pays
    /// it, so late usage of a closed period never draws credit of a later one.
    pub dated_before: Option<DateTime<Utc>>,
}

/// The subscription and product a usage demand is keyed by.
pub type DemandScope = (Option<SubscriptionId>, Option<ProductId>);

/// What an invoice may draw: at most `requested_minor` in all, each demand at most its amount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditRequest {
    pub requested_minor: i64,
    pub demands: Vec<CreditDemand>,
}

impl CreditRequest {
    /// The usage of one subscription, of no product in particular, dated before `dated_before`.
    pub fn usage_for(
        subscription_id: Option<SubscriptionId>,
        amount_minor: i64,
        dated_before: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            requested_minor: amount_minor,
            demands: vec![CreditDemand {
                subscription_id,
                product_id: None,
                usage: true,
                amount_minor,
                dated_before,
            }],
        }
    }

    /// Usage only, by subscription, product and the end of its dates; what one owes below zero
    /// counts as nothing.
    pub fn usage_by_scope(
        amounts: impl IntoIterator<Item = (DemandScope, Option<DateTime<Utc>>, i64)>,
    ) -> StoreResult<Self> {
        let mut demands = amounts
            .into_iter()
            .map(
                |((subscription_id, product_id), dated_before, amount_minor)| CreditDemand {
                    subscription_id,
                    product_id,
                    usage: true,
                    amount_minor: amount_minor.max(0),
                    dated_before,
                },
            )
            .collect::<Vec<_>>();
        demands.sort_by_key(|demand| {
            (
                demand.subscription_id.map(|id| id.as_uuid()),
                demand.product_id.map(|id| id.as_uuid()),
                demand.dated_before,
            )
        });
        let requested_minor = demands
            .iter()
            .try_fold(0_i64, |sum, demand| sum.checked_add(demand.amount_minor))
            .ok_or_else(overflow)?;
        Ok(Self {
            requested_minor,
            demands,
        })
    }

    /// Without the usage dates: credit an invoice holds goes back to the lines it paid, whatever
    /// rule placed it there (an invoice drawn before usage was dated included).
    pub(crate) fn undated(mut self) -> Self {
        for demand in &mut self.demands {
            demand.dated_before = None;
        }
        self
    }

    pub(crate) fn capped(mut self, requested_minor: i64) -> Self {
        self.requested_minor = self.requested_minor.min(requested_minor).max(0);
        self
    }

    /// What is left to pay once `allocation` paid its share.
    pub fn remainder(&self, allocation: &Allocation) -> StoreResult<Self> {
        core::Request::from(self)
            .remainder(&allocation.into())
            .map(Into::into)
            .map_err(|_| overflow())
    }
}

impl From<&CreditDemand> for core::Demand {
    fn from(demand: &CreditDemand) -> Self {
        core::Demand {
            subscription: demand.subscription_id.map(|id| id.as_uuid()),
            product: demand.product_id.map(|id| id.as_uuid()),
            usage: demand.usage,
            amount_minor: demand.amount_minor,
            dated_before: demand.dated_before,
        }
    }
}

impl From<core::Demand> for CreditDemand {
    fn from(demand: core::Demand) -> Self {
        CreditDemand {
            subscription_id: demand.subscription.map(SubscriptionId::from),
            product_id: demand.product.map(ProductId::from),
            usage: demand.usage,
            amount_minor: demand.amount_minor,
            dated_before: demand.dated_before,
        }
    }
}

impl From<&CreditRequest> for core::Request {
    fn from(request: &CreditRequest) -> Self {
        core::Request {
            requested_minor: request.requested_minor,
            demands: request.demands.iter().map(Into::into).collect(),
        }
    }
}

impl From<core::Request> for CreditRequest {
    fn from(request: core::Request) -> Self {
        CreditRequest {
            requested_minor: request.requested_minor,
            demands: request.demands.into_iter().map(Into::into).collect(),
        }
    }
}

/// The draw plan, `(grant, minor units)` in the order the grants are drawn, and what it covers of
/// each demand of the request, in the request's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Allocation {
    pub draws: Vec<(CreditGrantId, i64)>,
    pub covered: Vec<i64>,
    /// What each grant drawn paid of each demand, then of what no demand names: the revenue
    /// facts place its credit on those lines.
    pub placed: Vec<(CreditGrantId, Vec<i64>)>,
}

impl Allocation {
    pub fn total(&self) -> StoreResult<i64> {
        core::Allocation::from(self).total().map_err(|_| overflow())
    }

    /// Adds `other`, drawn after this one on the same request.
    pub fn then(self, other: Allocation) -> StoreResult<Self> {
        core::Allocation::from(&self)
            .then((&other).into())
            .map(Into::into)
            .map_err(|_| overflow())
    }
}

impl From<&Allocation> for core::Allocation {
    fn from(allocation: &Allocation) -> Self {
        core::Allocation {
            draws: allocation
                .draws
                .iter()
                .map(|(id, amount)| (id.as_uuid(), *amount))
                .collect(),
            covered: allocation.covered.clone(),
            placed: allocation
                .placed
                .iter()
                .map(|(id, columns)| (id.as_uuid(), columns.clone()))
                .collect(),
        }
    }
}

impl From<core::Allocation> for Allocation {
    fn from(allocation: core::Allocation) -> Self {
        Allocation {
            draws: allocation
                .draws
                .into_iter()
                .map(|(id, amount)| (CreditGrantId::from(id), amount))
                .collect(),
            covered: allocation.covered,
            placed: allocation
                .placed
                .into_iter()
                .map(|(id, columns)| (CreditGrantId::from(id), columns))
                .collect(),
        }
    }
}

fn overflow() -> error_stack::Report<crate::errors::StoreError> {
    invalid("invoice credit exceeds the supported range")
}

#[derive(Debug, Clone)]
pub struct GrantCandidate {
    pub id: CreditGrantId,
    pub scope: GrantScope,
    pub remaining_minor: i64,
    pub priority: i32,
    pub classification: GrantClassification,
    pub effective_at: DateTime<Utc>,
    /// The start of the usage it may pay: its effective date; a rollover's is its parent's, as it
    /// carries what that credit left of its own period.
    pub pays_usage_from: DateTime<Utc>,
    /// The earlier of its expiry and the end of its subscription.
    pub lapses_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl GrantCandidate {
    pub fn spendable_at(&self, now: DateTime<Utc>) -> bool {
        self.remaining_minor > 0
            && self.effective_at <= now
            && self.lapses_at.is_none_or(|lapses_at| lapses_at > now)
    }
}

impl From<&GrantCandidate> for core::Grant {
    fn from(grant: &GrantCandidate) -> Self {
        core::Grant {
            id: grant.id.as_uuid(),
            scope: (&grant.scope).into(),
            remaining_minor: grant.remaining_minor,
            priority: grant.priority,
            classification: grant.classification.into(),
            effective_at: grant.effective_at,
            pays_usage_from: grant.pays_usage_from,
            lapses_at: grant.lapses_at,
            created_at: grant.created_at,
        }
    }
}

impl GrantCandidate {
    pub(crate) fn pays(&self, demand: &CreditDemand) -> bool {
        core::Grant::from(self).pays(&demand.into())
    }
}

fn draw_key(grant: &CreditGrant) -> core::DrawKey {
    core::DrawKey {
        priority: grant.priority,
        lapses_at: grant.lapses_at(),
        classification: grant.classification.into(),
        effective_at: grant.effective_at,
        created_at: grant.created_at,
        id: grant.id.as_uuid(),
    }
}

/// A wallet's grants as invoices use them: spendable ones in draw order, then scheduled ones by
/// start date, then used-up or expired ones, most recent first.
pub fn grants_in_draw_order(grants: &[CreditGrant]) -> Vec<&CreditGrant> {
    let group = |grant: &CreditGrant| match grant.state {
        CreditGrantState::Active if grant.remaining_minor > 0 => 0,
        CreditGrantState::Pending => 1,
        _ => 2,
    };
    let mut ordered = grants
        .iter()
        .map(|grant| (grant, core::Scope::from(&grant.scope), draw_key(grant)))
        .collect::<Vec<_>>();
    ordered.sort_by(
        |(left, left_scope, left_key), (right, right_scope, right_key)| {
            group(left)
                .cmp(&group(right))
                .then_with(|| match group(left) {
                    0 => core::scope_order(left_scope, right_scope)
                        .then_with(|| core::draw_order(*left_key, *right_key)),
                    1 => left
                        .effective_at
                        .cmp(&right.effective_at)
                        .then_with(|| core::draw_order(*left_key, *right_key)),
                    _ => right
                        .created_at
                        .cmp(&left.created_at)
                        .then_with(|| right.id.as_uuid().cmp(&left.id.as_uuid())),
                })
        },
    );
    ordered.into_iter().map(|(grant, _, _)| grant).collect()
}

fn core_grants(grants: &[GrantCandidate]) -> Vec<core::Grant> {
    grants.iter().map(Into::into).collect()
}

fn core_held(held: &[(GrantCandidate, i64)]) -> Vec<(core::Grant, i64)> {
    held.iter()
        .map(|(grant, amount)| (grant.into(), *amount))
        .collect()
}

/// The draw plan of the spendable grants of `pool`. Never more than requested, never more than a
/// grant holds.
pub fn allocate(
    grants: &[GrantCandidate],
    request: &CreditRequest,
    now: DateTime<Utc>,
    pool: GrantPool,
) -> StoreResult<Allocation> {
    core::allocate(&core_grants(grants), &request.into(), now, pool.into())
        .map(Into::into)
        .map_err(|_| overflow())
}

/// `pools` in turn, each on what the previous ones left, after what grants already paid
/// (settlements an invoice will keep). Returns what is kept, then what is drawn.
pub fn allocate_in_turn_keeping(
    grants: &[GrantCandidate],
    held: &[(GrantCandidate, i64)],
    request: &CreditRequest,
    now: DateTime<Utc>,
    pools: &[GrantPool],
) -> StoreResult<(Allocation, Allocation)> {
    let pools: Vec<core::Pool> = pools.iter().map(|pool| (*pool).into()).collect();
    core::allocate_in_turn_keeping(
        &core_grants(grants),
        &core_held(held),
        &request.into(),
        now,
        &pools,
    )
    .map(|(kept, drawn)| (kept.into(), drawn.into()))
    .map_err(|_| overflow())
}

/// Places amounts grants already paid (settlements an invoice keeps, a replayed draw) on the
/// request, in grant order and whatever their expiry: what does not fit is not kept.
pub fn assign(held: &[(GrantCandidate, i64)], request: &CreditRequest) -> StoreResult<Allocation> {
    core::assign(&core_held(held), &request.into())
        .map(Into::into)
        .map_err(|_| overflow())
}

/// Stands in for the grant an expired, not yet swept grant will roll over to: the sweep first pays
/// the usage before the expiry from it, then carries up to the cap. That usage is part of
/// `pending_minor`, so the cap is raised by it: the live balance never dips below what the sweep leaves.
pub fn rollover_stand_in(
    expired: &GrantCandidate,
    rollover: &GrantRollover,
    pending_minor: i64,
    now: DateTime<Utc>,
) -> Option<GrantCandidate> {
    let expired_at = expired.lapses_at.filter(|at| *at <= now)?;
    let rolled_expiry = rollover.rolled_expiry(expired_at).filter(|at| *at > now)?;
    let bound = GrantRollover {
        max_minor: rollover
            .max_minor
            .map(|max| max.saturating_add(pending_minor.max(0))),
        ..*rollover
    };
    let carried = bound.carried(expired.remaining_minor);
    (carried > 0).then(|| GrantCandidate {
        remaining_minor: carried,
        priority: expired.priority.saturating_sub(1),
        effective_at: expired_at,
        lapses_at: Some(rolled_expiry),
        ..expired.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, 12, 0, 0).unwrap()
    }

    fn grant(
        remaining: i64,
        priority: i32,
        expires_at: Option<DateTime<Utc>>,
        classification: GrantClassification,
        application: CreditApplication,
    ) -> GrantCandidate {
        GrantCandidate {
            id: CreditGrantId::new(),
            scope: GrantScope {
                subscription_id: None,
                application,
                product_ids: Vec::new(),
            },
            remaining_minor: remaining,
            priority,
            classification,
            effective_at: at(1),
            pays_usage_from: at(1),
            lapses_at: expires_at,
            created_at: at(1),
        }
    }

    fn request(requested_minor: i64, usage_cap_minor: i64) -> CreditRequest {
        CreditRequest {
            requested_minor,
            demands: vec![
                CreditDemand {
                    subscription_id: None,
                    product_id: None,
                    usage: true,
                    amount_minor: usage_cap_minor,
                    dated_before: None,
                },
                CreditDemand {
                    subscription_id: None,
                    product_id: None,
                    usage: false,
                    amount_minor: (requested_minor - usage_cap_minor).max(0),
                    dated_before: None,
                },
            ],
        }
    }

    fn drawn(
        grants: &[GrantCandidate],
        request: CreditRequest,
        now: DateTime<Utc>,
    ) -> Vec<(CreditGrantId, i64)> {
        allocate(grants, &request, now, GrantPool::All)
            .unwrap()
            .draws
    }

    use CreditApplication::{Invoice, Usage};
    use GrantClassification::{Paid, Promotional};

    #[test]
    fn usage_credit_is_capped_and_invoice_credit_covers_the_rest() {
        let grants = [
            grant(5_000, 100, None, Paid, Usage),
            grant(5_000, 100, None, Paid, Invoice),
        ];
        let plan = drawn(&grants, request(4_000, 1_500), at(10));
        assert_eq!(plan, vec![(grants[0].id, 1_500), (grants[1].id, 2_500)]);
        let plan = drawn(&grants, request(4_000, 0), at(10));
        assert_eq!(plan, vec![(grants[1].id, 4_000)]);
    }

    #[test]
    fn promotional_credit_pays_before_paid_credit_on_what_it_leaves() {
        let grants = [
            grant(1_000, 1, None, Paid, Invoice),
            grant(600, 100, None, Promotional, Invoice),
            grant(300, 100, None, Promotional, Usage),
        ];
        let promotional = allocate(
            &grants,
            &request(2_000, 500),
            at(10),
            GrantPool::Promotional,
        )
        .unwrap();
        assert_eq!(
            promotional.draws,
            vec![(grants[2].id, 300), (grants[1].id, 600)]
        );
        // The invoice credit pays the other charges first, leaving usage to usage credit.
        assert_eq!(promotional.covered, vec![300, 600]);
        let (_, in_turn) = allocate_in_turn_keeping(
            &grants,
            &[],
            &request(2_000, 500),
            at(10),
            &[GrantPool::Promotional, GrantPool::Paid],
        )
        .unwrap();
        assert_eq!(
            in_turn.draws,
            vec![
                (grants[2].id, 300),
                (grants[1].id, 600),
                (grants[0].id, 1_000)
            ]
        );
        assert_eq!(in_turn.covered, vec![400, 1_500]);
    }

    #[test]
    fn kept_amounts_are_placed_whatever_their_expiry() {
        let mut expired = grant(0, 100, Some(at(2)), Promotional, Usage);
        expired.remaining_minor = 0;
        let kept = assign(&[(expired.clone(), 800)], &request(500, 500)).unwrap();
        assert_eq!(kept.draws, vec![(expired.id, 500)]);
    }

    fn wallet_grant(
        state: CreditGrantState,
        remaining: i64,
        priority: i32,
        classification: GrantClassification,
        effective_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
    ) -> CreditGrant {
        CreditGrant {
            id: CreditGrantId::new(),
            wallet_id: common_domain::ids::WalletId::new(),
            amount_minor: 1_000,
            remaining_minor: remaining,
            expired_minor: None,
            classification,
            priority,
            source: crate::domain::credit::CreditGrantSource::Manual,
            source_id: uuid::Uuid::now_v7(),
            invoice_id: None,
            credit_note_id: None,
            effective_at,
            expires_at: None,
            state,
            reason: String::new(),
            reference: None,
            created_by: None,
            created_at,
            cost: None,
            scope: GrantScope::customer_wide(),
            ends_at: None,
            end_reason: None,
            rollover: None,
        }
    }

    #[test]
    fn wallet_grants_list_spendable_in_draw_order_then_scheduled_then_spent() {
        let paid = wallet_grant(CreditGrantState::Active, 500, 100, Paid, at(1), at(1));
        let promotional = wallet_grant(
            CreditGrantState::Active,
            500,
            100,
            Promotional,
            at(2),
            at(2),
        );
        let first = wallet_grant(CreditGrantState::Active, 500, 10, Paid, at(3), at(3));
        let scheduled_late = wallet_grant(CreditGrantState::Pending, 500, 1, Paid, at(9), at(3));
        let scheduled_soon = wallet_grant(CreditGrantState::Pending, 500, 100, Paid, at(8), at(3));
        let used_up_old = wallet_grant(CreditGrantState::Depleted, 0, 1, Paid, at(1), at(1));
        let expired_recent = wallet_grant(CreditGrantState::Expired, 0, 1, Paid, at(4), at(4));
        let grants = vec![
            used_up_old.clone(),
            scheduled_late.clone(),
            paid.clone(),
            expired_recent.clone(),
            promotional.clone(),
            scheduled_soon.clone(),
            first.clone(),
        ];
        let ordered = grants_in_draw_order(&grants)
            .into_iter()
            .map(|grant| grant.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ordered,
            vec![
                first.id,
                promotional.id,
                paid.id,
                scheduled_soon.id,
                scheduled_late.id,
                expired_recent.id,
                used_up_old.id,
            ]
        );
    }

    fn of_subscription(
        mut grant: GrantCandidate,
        subscription_id: SubscriptionId,
    ) -> GrantCandidate {
        grant.scope.subscription_id = Some(subscription_id);
        grant
    }

    fn usage_of(subscription_id: SubscriptionId, amount_minor: i64) -> CreditDemand {
        CreditDemand {
            subscription_id: Some(subscription_id),
            product_id: None,
            usage: true,
            amount_minor,
            dated_before: None,
        }
    }

    fn requested(demands: Vec<CreditDemand>) -> CreditRequest {
        CreditRequest {
            requested_minor: demands.iter().map(|demand| demand.amount_minor).sum(),
            demands,
        }
    }

    #[test]
    fn usage_credit_never_pays_a_fee_or_the_unnamed_remainder() {
        let a = SubscriptionId::new();
        let pack = of_subscription(grant(10_000, 100, None, Promotional, Usage), a);
        let request = CreditRequest {
            requested_minor: 3_100,
            demands: vec![
                CreditDemand {
                    subscription_id: Some(a),
                    product_id: None,
                    usage: false,
                    amount_minor: 2_000,
                    dated_before: None,
                },
                usage_of(a, 1_000),
            ],
        };
        let allocation = allocate(
            std::slice::from_ref(&pack),
            &request,
            at(10),
            GrantPool::All,
        )
        .unwrap();
        assert_eq!(allocation.draws, vec![(pack.id, 1_000)]);
        assert_eq!(allocation.covered, vec![0, 1_000]);
        assert_eq!(
            request.remainder(&allocation).unwrap().requested_minor,
            2_100
        );
    }

    #[test]
    fn a_grant_past_its_ends_at_is_not_spendable() {
        let a = SubscriptionId::new();
        let mut ended = of_subscription(grant(1_000, 100, Some(at(30)), Promotional, Usage), a);
        ended.lapses_at = Some(at(8));
        assert!(!ended.spendable_at(at(8)));
        assert!(ended.spendable_at(at(7)));
        assert!(
            drawn(
                std::slice::from_ref(&ended),
                requested(vec![usage_of(a, 500)]),
                at(9)
            )
            .is_empty()
        );

        let wallet = CreditGrant {
            expires_at: Some(at(30)),
            ends_at: Some(at(8)),
            end_reason: Some(crate::domain::credit::GrantEndReason::SubscriptionEnded),
            ..wallet_grant(CreditGrantState::Expired, 0, 100, Promotional, at(1), at(1))
        };
        assert_eq!(wallet.lapses_at(), Some(at(8)));
        let never = wallet_grant(CreditGrantState::Active, 1, 100, Paid, at(1), at(1));
        assert_eq!(never.lapses_at(), None);
        let ending_later = CreditGrant {
            expires_at: Some(at(5)),
            ends_at: Some(at(8)),
            ..never.clone()
        };
        assert_eq!(ending_later.lapses_at(), Some(at(5)));
    }
}
