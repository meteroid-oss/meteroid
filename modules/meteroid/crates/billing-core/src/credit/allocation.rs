//! Which grants pay how much of an invoice. Pure and in whole minor units. A request names the
//! charges credit may pay (its demands), by subscription and product: a grant of a subscription
//! pays only that subscription's, usage credit only usage, a grant restricted to products only
//! theirs, and unrestricted customer-wide invoice credit also what no demand names (the rounding
//! between the total and its lines). Within a pool, grants of a subscription are drawn first, then
//! product-restricted, then usage-only, then by priority, soonest lapse, promotional before paid
//! and age. Money draws promotional credit before tax and paid credit after it. Usage is paid only
//! by credit effective before the end of the dates it was consumed in (Orb).

use std::cmp::Ordering;

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// An amount beyond what minor units in `i64` hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overflow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Classification {
    Paid,
    Promotional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Application {
    Invoice,
    Usage,
}

/// What a grant may pay: one subscription's charges (None: any), usage only or the invoice, and
/// some products (sorted; empty: every product).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub subscription: Option<Uuid>,
    pub application: Application,
    pub products: Vec<Uuid>,
}

/// The grants a draw may take. Money credit is drawn from promotional grants before tax and from
/// paid grants after it; a credit unit draws all of its grants at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pool {
    All,
    Promotional,
    Paid,
}

impl Pool {
    pub fn admits(self, classification: Classification) -> bool {
        match self {
            Self::All => true,
            Self::Promotional => classification == Classification::Promotional,
            Self::Paid => classification == Classification::Paid,
        }
    }
}

/// A charge credit may pay, at most `amount_minor` of it: the usage, or the other charges, of
/// one subscription (None: of none) and product (None: lines of no product).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Demand {
    pub subscription: Option<Uuid>,
    pub product: Option<Uuid>,
    pub usage: bool,
    pub amount_minor: i64,
    /// Usage only: the end of the dates it was consumed in. Only credit effective before it pays
    /// it, so late usage of a closed period never draws credit of a later one.
    pub dated_before: Option<DateTime<Utc>>,
}

/// What an invoice may draw: at most `requested_minor` in all, each demand at most its amount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub requested_minor: i64,
    pub demands: Vec<Demand>,
}

impl Request {
    /// What is left to pay once `allocation` paid its share.
    pub fn remainder(&self, allocation: &Allocation) -> Result<Self, Overflow> {
        Ok(Self {
            requested_minor: self
                .requested_minor
                .max(0)
                .checked_sub(allocation.total()?)
                .ok_or(Overflow)?
                .max(0),
            demands: self
                .demands
                .iter()
                .enumerate()
                .map(|(index, demand)| Demand {
                    amount_minor: (demand.amount_minor.max(0)
                        - allocation.covered.get(index).copied().unwrap_or(0))
                    .max(0),
                    ..*demand
                })
                .collect(),
        })
    }
}

/// The draw plan, `(grant, minor units)` in the order the grants are drawn, and what it covers of
/// each demand of the request, in the request's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Allocation {
    pub draws: Vec<(Uuid, i64)>,
    pub covered: Vec<i64>,
    /// What each grant drawn paid of each demand, then of what no demand names: the revenue
    /// facts place its credit on those lines.
    pub placed: Vec<(Uuid, Vec<i64>)>,
}

impl Allocation {
    pub fn total(&self) -> Result<i64, Overflow> {
        self.draws
            .iter()
            .try_fold(0_i64, |sum, (_, amount)| sum.checked_add(*amount))
            .ok_or(Overflow)
    }

    /// Adds `other`, drawn after this one on the same request.
    pub fn then(mut self, other: Allocation) -> Result<Self, Overflow> {
        if self.covered.len() < other.covered.len() {
            self.covered.resize(other.covered.len(), 0);
        }
        for (slot, more) in self.covered.iter_mut().zip(other.covered) {
            *slot = slot.checked_add(more).ok_or(Overflow)?;
        }
        for (grant_id, amount) in other.draws {
            match self.draws.iter_mut().find(|(id, _)| *id == grant_id) {
                Some((_, drawn)) => *drawn = drawn.checked_add(amount).ok_or(Overflow)?,
                None => self.draws.push((grant_id, amount)),
            }
        }
        for (grant_id, columns) in other.placed {
            match self.placed.iter_mut().find(|(id, _)| *id == grant_id) {
                Some((_, placed)) => {
                    if placed.len() < columns.len() {
                        placed.resize(columns.len(), 0);
                    }
                    for (slot, more) in placed.iter_mut().zip(columns) {
                        *slot = slot.checked_add(more).ok_or(Overflow)?;
                    }
                }
                None => self.placed.push((grant_id, columns)),
            }
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub id: Uuid,
    pub scope: Scope,
    pub remaining_minor: i64,
    pub priority: i32,
    pub classification: Classification,
    pub effective_at: DateTime<Utc>,
    /// The start of the usage it may pay: its effective date; a rollover's is its parent's, as it
    /// carries what that credit left of its own period.
    pub pays_usage_from: DateTime<Utc>,
    /// The earlier of its expiry and the end of its subscription.
    pub lapses_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl Grant {
    pub fn spendable_at(&self, now: DateTime<Utc>) -> bool {
        self.remaining_minor > 0
            && self.effective_at <= now
            && self.lapses_at.is_none_or(|lapses_at| lapses_at > now)
    }

    /// Whether this grant may pay `demand`: of its subscription, usage or the invoice, of its
    /// products, and usage dated after the credit it carries took effect.
    pub fn pays(&self, demand: &Demand) -> bool {
        self.scope
            .subscription
            .is_none_or(|own| demand.subscription == Some(own))
            && (demand.usage || self.scope.application == Application::Invoice)
            && (self.scope.products.is_empty()
                || demand
                    .product
                    .is_some_and(|product| self.scope.products.contains(&product)))
            && demand
                .dated_before
                .is_none_or(|before| self.pays_usage_from < before)
    }

    fn pays_unnamed(&self) -> bool {
        self.scope.subscription.is_none()
            && self.scope.application == Application::Invoice
            && self.scope.products.is_empty()
    }
}

/// What orders grants of one scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawKey {
    pub priority: i32,
    pub lapses_at: Option<DateTime<Utc>>,
    pub classification: Classification,
    pub effective_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub id: Uuid,
}

impl From<&Grant> for DrawKey {
    fn from(grant: &Grant) -> Self {
        Self {
            priority: grant.priority,
            lapses_at: grant.lapses_at,
            classification: grant.classification,
            effective_at: grant.effective_at,
            created_at: grant.created_at,
            id: grant.id,
        }
    }
}

/// The credit that can pay least first: a subscription's before customer-wide, restricted to
/// products (the fewest first), then usage credit before invoice credit, then the draw order.
pub fn grant_order(left: &Grant, right: &Grant) -> Ordering {
    scope_order(&left.scope, &right.scope)
        .then_with(|| draw_order(DrawKey::from(left), DrawKey::from(right)))
}

pub fn scope_order(left: &Scope, right: &Scope) -> Ordering {
    scope_rank(left)
        .cmp(&scope_rank(right))
        .then_with(|| application_rank(left.application).cmp(&application_rank(right.application)))
}

fn scope_rank(scope: &Scope) -> (bool, bool, usize) {
    (
        scope.subscription.is_none(),
        scope.products.is_empty(),
        scope.products.len(),
    )
}

const fn application_rank(application: Application) -> u8 {
    match application {
        Application::Usage => 0,
        Application::Invoice => 1,
    }
}

/// Lower priority first, then the soonest lapse (lapsing before never-lapsing), promotional
/// before paid, then the oldest; the id makes it a total order.
pub fn draw_order(left: DrawKey, right: DrawKey) -> Ordering {
    left.priority
        .cmp(&right.priority)
        .then_with(|| match (left.lapses_at, right.lapses_at) {
            (Some(left), Some(right)) => left.cmp(&right),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
        .then_with(|| {
            classification_rank(left.classification).cmp(&classification_rank(right.classification))
        })
        .then_with(|| left.effective_at.cmp(&right.effective_at))
        .then_with(|| left.created_at.cmp(&right.created_at))
        .then_with(|| left.id.cmp(&right.id))
}

const fn classification_rank(classification: Classification) -> u8 {
    match classification {
        Classification::Promotional => 0,
        Classification::Paid => 1,
    }
}

/// Pays `request` from `grants`, each at most its paired amount, in grant order. Each grant pays
/// first the charges the fewest later grants could pay (other charges before usage, the tax last),
/// then [`augment`] reroutes what overlapping restrictions left unpaid.
/// `afterwards` are the grants a later stage draws: they pay nothing here, but count among the
/// later payers, so this stage leaves to them the charges they can pay.
fn cover(
    mut grants: Vec<(&Grant, i64)>,
    request: &Request,
    afterwards: &[&Grant],
) -> Result<Allocation, Overflow> {
    grants.sort_by(|(left, _), (right, _)| grant_order(left, right));
    let demands = request.demands.len();
    // The last column is what no demand names (the tax).
    let mut open = request
        .demands
        .iter()
        .map(|demand| demand.amount_minor.max(0))
        .collect::<Vec<_>>();
    let named = open
        .iter()
        .try_fold(0_i64, |sum, amount| sum.checked_add(*amount))
        .ok_or(Overflow)?;
    let mut left = request.requested_minor.max(0);
    open.push(left.saturating_sub(named).max(0));
    let pays = |grant: &Grant, column: usize| {
        if column == demands {
            grant.pays_unnamed()
        } else {
            grant.pays(&request.demands[column])
        }
    };
    let mut budget = grants
        .iter()
        .map(|(_, available)| (*available).max(0))
        .collect::<Vec<_>>();
    let mut paid = vec![vec![0_i64; demands + 1]; grants.len()];
    for index in 0..grants.len() {
        let later_payers = |column: usize| {
            (index + 1..grants.len())
                .filter(|later| budget[*later] > 0 && pays(grants[*later].0, column))
                .count()
                + afterwards
                    .iter()
                    .filter(|grant| pays(grant, column))
                    .count()
        };
        let mut columns = (0..=demands)
            .filter(|column| pays(grants[index].0, *column))
            .collect::<Vec<_>>();
        // Among charges as many later grants can pay, the earliest-dated usage first: credit
        // effective later (drawn in a later stage, or fresh after kept settlements) pays only later.
        columns.sort_by_key(|column| {
            (
                *column == demands,
                later_payers(*column),
                *column < demands && request.demands[*column].usage,
                request
                    .demands
                    .get(*column)
                    .and_then(|demand| demand.dated_before)
                    .unwrap_or(DateTime::<Utc>::MAX_UTC),
                *column,
            )
        });
        for column in columns {
            let take = open[column].min(budget[index]).min(left);
            open[column] -= take;
            budget[index] -= take;
            left -= take;
            paid[index][column] += take;
        }
    }
    augment(&grants, &pays, &mut open, &mut budget, &mut left, &mut paid);
    let covered = (0..demands)
        .map(|column| paid.iter().map(|row| row[column]).sum())
        .collect();
    let draws = grants
        .iter()
        .zip(&paid)
        .filter_map(|((grant, _), row)| {
            let drawn = row.iter().sum::<i64>();
            (drawn > 0).then_some((grant.id, drawn))
        })
        .collect();
    let placed = grants
        .iter()
        .zip(paid)
        .filter(|(_, row)| row.iter().any(|amount| *amount > 0))
        .map(|((grant, _), row)| (grant.id, row))
        .collect();
    Ok(Allocation {
        draws,
        covered,
        placed,
    })
}

/// Pays more of what is still open where a grant with budget left can take over a charge another
/// grant paid, freeing that one for the open charge (augmenting paths, breadth first and in grant
/// and charge order, so the result is deterministic).
fn augment(
    grants: &[(&Grant, i64)],
    pays: &impl Fn(&Grant, usize) -> bool,
    open: &mut [i64],
    budget: &mut [i64],
    left: &mut i64,
    paid: &mut [Vec<i64>],
) {
    let columns = open.len();
    while *left > 0 {
        let Some(start) = (0..columns).find(|column| {
            open[*column] > 0 && (0..grants.len()).any(|grant| pays(grants[grant].0, *column))
        }) else {
            return;
        };
        // Breadth first over charges; each grant reached records the charge it was reached from.
        let mut reached_grant: Vec<Option<usize>> = vec![None; grants.len()];
        let mut reached_column: Vec<Option<usize>> = vec![None; columns];
        reached_column[start] = Some(usize::MAX);
        let mut queue = std::collections::VecDeque::from([start]);
        let mut sink = None;
        'search: while let Some(column) = queue.pop_front() {
            for grant in 0..grants.len() {
                if reached_grant[grant].is_some() || !pays(grants[grant].0, column) {
                    continue;
                }
                reached_grant[grant] = Some(column);
                if budget[grant] > 0 {
                    sink = Some(grant);
                    break 'search;
                }
                for next in 0..columns {
                    if paid[grant][next] > 0 && reached_column[next].is_none() {
                        reached_column[next] = Some(grant);
                        queue.push_back(next);
                    }
                }
            }
        }
        let Some(sink) = sink else {
            // Nothing can pay more of this charge: stop asking for it.
            open[start] = 0;
            continue;
        };
        // Walk back from the grant with budget to the open charge, taking the bottleneck.
        let mut amount = open[start].min(budget[sink]).min(*left);
        let mut grant = sink;
        loop {
            let column = reached_grant[grant].unwrap_or(start);
            if column == start {
                break;
            }
            amount = amount.min(paid[reached_column[column].unwrap_or(grant)][column]);
            grant = reached_column[column].unwrap_or(grant);
        }
        let mut grant = sink;
        budget[sink] -= amount;
        loop {
            let column = reached_grant[grant].unwrap_or(start);
            paid[grant][column] += amount;
            if column == start {
                break;
            }
            let previous = reached_column[column].unwrap_or(grant);
            paid[previous][column] -= amount;
            grant = previous;
        }
        open[start] -= amount;
        *left -= amount;
    }
}

/// The draw plan of the spendable grants of `pool`. Never more than requested, never more than a
/// grant holds.
pub fn allocate(
    grants: &[Grant],
    request: &Request,
    now: DateTime<Utc>,
    pool: Pool,
) -> Result<Allocation, Overflow> {
    // Promotional credit lands where the paid credit drawn after tax cannot pay, such as
    // another subscription's usage, never where only that paid credit could.
    let afterwards = match pool {
        Pool::Promotional => grants
            .iter()
            .filter(|grant| Pool::Paid.admits(grant.classification))
            .filter(|grant| grant.spendable_at(now))
            .collect(),
        Pool::All | Pool::Paid => Vec::new(),
    };
    cover(
        grants
            .iter()
            .filter(|grant| pool.admits(grant.classification) && grant.spendable_at(now))
            .map(|grant| (grant, grant.remaining_minor))
            .collect(),
        request,
        &afterwards,
    )
}

/// `pools` in turn, each on what the previous ones left, after what grants already paid (settlements an invoice will keep), placed
/// first as an invoice nets them: a later pool's (paid) before an earlier one's, so promotional
/// credit never pays usage paid credit settled. Returns what is kept, then what is drawn.
pub fn allocate_in_turn_keeping(
    grants: &[Grant],
    held: &[(Grant, i64)],
    request: &Request,
    now: DateTime<Utc>,
    pools: &[Pool],
) -> Result<(Allocation, Allocation), Overflow> {
    let empty = || Allocation {
        draws: Vec::new(),
        covered: vec![0; request.demands.len()],
        placed: Vec::new(),
    };
    let mut kept = empty();
    for pool in pools.iter().rev() {
        let pool_held = held
            .iter()
            .filter(|(grant, _)| pool.admits(grant.classification))
            .cloned()
            .collect::<Vec<_>>();
        let kept_now = assign(&pool_held, &request.remainder(&kept)?)?;
        kept = kept.then(kept_now)?;
    }
    let mut drawn = empty();
    for pool in pools {
        let paid = kept.clone().then(drawn.clone())?;
        drawn = drawn.then(allocate(grants, &request.remainder(&paid)?, now, *pool)?)?;
    }
    Ok((kept, drawn))
}

/// Places amounts grants already paid (settlements an invoice keeps, a replayed draw) on the
/// request, in grant order and whatever their expiry: what does not fit is not kept.
pub fn assign(held: &[(Grant, i64)], request: &Request) -> Result<Allocation, Overflow> {
    cover(
        held.iter()
            .map(|(grant, amount)| (grant, *amount))
            .collect(),
        request,
        &[],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::sync::atomic::{AtomicU64, Ordering as Atomic};

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, 12, 0, 0).unwrap()
    }

    /// Ids as random as the store's: no test may lean on their order.
    fn id() -> Uuid {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Atomic::Relaxed);
        Uuid::from_u128(u128::from(n).wrapping_mul(0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835))
    }

    fn grant(
        remaining: i64,
        priority: i32,
        expires_at: Option<DateTime<Utc>>,
        classification: Classification,
        application: Application,
    ) -> Grant {
        Grant {
            id: id(),
            scope: Scope {
                subscription: None,
                application,
                products: Vec::new(),
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

    fn request(requested_minor: i64, usage_cap_minor: i64) -> Request {
        Request {
            requested_minor,
            demands: vec![
                Demand {
                    subscription: None,
                    product: None,
                    usage: true,
                    amount_minor: usage_cap_minor,
                    dated_before: None,
                },
                Demand {
                    subscription: None,
                    product: None,
                    usage: false,
                    amount_minor: (requested_minor - usage_cap_minor).max(0),
                    dated_before: None,
                },
            ],
        }
    }

    fn drawn(grants: &[Grant], request: Request, now: DateTime<Utc>) -> Vec<(Uuid, i64)> {
        allocate(grants, &request, now, Pool::All).unwrap().draws
    }

    use Application::{Invoice, Usage};
    use Classification::{Paid, Promotional};

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
    fn grants_follow_the_waterfall_and_skip_expired_value() {
        let grants = [
            grant(1_000, 100, None, Paid, Invoice),
            grant(1_000, 100, Some(at(20)), Paid, Invoice),
            grant(1_000, 100, None, Promotional, Invoice),
            grant(1_000, 10, None, Paid, Invoice),
            grant(1_000, 1, Some(at(5)), Paid, Invoice),
        ];
        let plan = drawn(&grants, request(3_500, 0), at(10));
        assert_eq!(
            plan,
            vec![
                (grants[3].id, 1_000),
                (grants[1].id, 1_000),
                (grants[2].id, 1_000),
                (grants[0].id, 500),
            ]
        );
    }

    #[test]
    fn never_draws_more_than_requested_or_available() {
        let grants = [grant(700, 100, None, Paid, Invoice)];
        assert_eq!(
            drawn(&grants, request(1_000, 0), at(10)),
            vec![(grants[0].id, 700)]
        );
        assert!(drawn(&grants, request(0, 0), at(10)).is_empty());
        assert!(drawn(&grants, request(-5, 3), at(10)).is_empty());
        let usage_only = [grant(700, 100, None, Paid, Usage)];
        assert!(drawn(&usage_only, request(1_000, 0), at(10)).is_empty());
        // The usage cap never exceeds the request.
        assert_eq!(
            drawn(&usage_only, request(300, 1_000), at(10)),
            vec![(usage_only[0].id, 300)]
        );
    }

    #[test]
    fn future_and_depleted_grants_are_not_drawn() {
        let mut future = grant(1_000, 1, None, Paid, Invoice);
        future.effective_at = at(15);
        let depleted = grant(0, 1, None, Paid, Invoice);
        let live = grant(1_000, 100, None, Paid, Invoice);
        let plan = drawn(
            &[future.clone(), depleted, live.clone()],
            request(500, 0),
            at(10),
        );
        assert_eq!(plan, vec![(live.id, 500)]);
        // Expiry is exclusive: a grant expiring now is no longer spendable.
        let mut lapsing = grant(1_000, 1, Some(at(10)), Paid, Invoice);
        lapsing.effective_at = at(1);
        assert!(drawn(&[lapsing], request(500, 0), at(10)).is_empty());
    }

    #[test]
    fn grant_order_is_total_over_every_key() {
        let base = grant(1, 5, Some(at(20)), Paid, Invoice);
        let later_created = Grant {
            id: id(),
            created_at: at(2),
            ..base.clone()
        };
        let later_effective = Grant {
            id: id(),
            effective_at: at(2),
            ..base.clone()
        };
        let promotional = Grant {
            id: id(),
            classification: Promotional,
            ..base.clone()
        };
        let never_expiring = Grant {
            id: id(),
            lapses_at: None,
            ..base.clone()
        };
        let lower_priority = Grant {
            id: id(),
            priority: 1,
            ..never_expiring.clone()
        };
        assert_eq!(grant_order(&lower_priority, &base), Ordering::Less);
        assert_eq!(grant_order(&base, &never_expiring), Ordering::Less);
        assert_eq!(grant_order(&promotional, &base), Ordering::Less);
        assert_eq!(grant_order(&base, &later_effective), Ordering::Less);
        assert_eq!(grant_order(&base, &later_created), Ordering::Less);
        let twin = Grant {
            id: id(),
            ..base.clone()
        };
        assert_ne!(grant_order(&base, &twin), Ordering::Equal);
        assert_eq!(grant_order(&base, &base), Ordering::Equal);
    }

    #[test]
    fn promotional_credit_pays_before_paid_credit_on_what_it_leaves() {
        let grants = [
            grant(1_000, 1, None, Paid, Invoice),
            grant(600, 100, None, Promotional, Invoice),
            grant(300, 100, None, Promotional, Usage),
        ];
        let promotional =
            allocate(&grants, &request(2_000, 500), at(10), Pool::Promotional).unwrap();
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
            &[Pool::Promotional, Pool::Paid],
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
    fn invoice_credit_also_pays_what_no_demand_names() {
        let grants = [
            grant(5_000, 100, None, Paid, Usage),
            grant(5_000, 100, None, Paid, Invoice),
        ];
        let tax_on_top = Request {
            requested_minor: 1_200,
            demands: vec![Demand {
                subscription: None,
                product: None,
                usage: true,
                amount_minor: 1_000,
                dated_before: None,
            }],
        };
        let allocation = allocate(&grants, &tax_on_top, at(10), Pool::All).unwrap();
        assert_eq!(
            allocation.draws,
            vec![(grants[0].id, 1_000), (grants[1].id, 200)]
        );
        assert_eq!(allocation.covered, vec![1_000]);
        assert_eq!(
            tax_on_top.remainder(&allocation).unwrap().requested_minor,
            0
        );
    }

    #[test]
    fn kept_amounts_are_placed_whatever_their_expiry() {
        let mut expired = grant(0, 100, Some(at(2)), Promotional, Usage);
        expired.remaining_minor = 0;
        let kept = assign(&[(expired.clone(), 800)], &request(500, 500)).unwrap();
        assert_eq!(kept.draws, vec![(expired.id, 500)]);
    }

    fn product_demand(product: Uuid, usage: bool, amount_minor: i64) -> Demand {
        Demand {
            subscription: None,
            product: Some(product),
            usage,
            amount_minor,
            dated_before: None,
        }
    }

    fn scoped(mut grant: Grant, products: &[Uuid]) -> Grant {
        let mut products = products.to_vec();
        products.sort();
        products.dedup();
        grant.scope.products = products;
        grant
    }

    #[test]
    fn overlapping_scopes_never_pay_the_same_charge_twice() {
        let (a, b) = (id(), id());
        let request = Request {
            requested_minor: 1_000,
            demands: vec![product_demand(a, true, 600), product_demand(b, true, 400)],
        };
        let wide = scoped(grant(1_000, 100, None, Paid, Invoice), &[a, b]);
        let narrow = scoped(grant(1_000, 100, None, Paid, Invoice), &[a]);
        let allocation =
            allocate(&[wide.clone(), narrow.clone()], &request, at(10), Pool::All).unwrap();
        // The narrower grant pays its one product first; the wider one only what is left.
        assert_eq!(allocation.draws, vec![(narrow.id, 600), (wide.id, 400)]);
        assert_eq!(allocation.covered, vec![600, 400]);
    }

    #[test]
    fn non_nested_restrictions_pay_every_charge_they_can() {
        let (p1, p2, p3) = (id(), id(), id());
        let request = Request {
            requested_minor: 100,
            demands: vec![product_demand(p2, false, 50), product_demand(p1, false, 50)],
        };
        let a = scoped(grant(50, 100, None, Paid, Invoice), &[p1, p2]);
        let c = scoped(grant(50, 100, None, Paid, Invoice), &[p2, p3]);
        for grants in [[a.clone(), c.clone()], [c.clone(), a.clone()]] {
            let allocation = allocate(&grants, &request, at(10), Pool::All).unwrap();
            assert_eq!(allocation.covered, vec![50, 50], "nothing is left to cash");
            assert_eq!(allocation.total().unwrap(), 100);
        }
        // Three overlapping grants where only rerouting pays everything.
        let request = Request {
            requested_minor: 150,
            demands: vec![
                product_demand(p1, false, 50),
                product_demand(p2, false, 50),
                product_demand(p3, false, 50),
            ],
        };
        let ab = scoped(grant(50, 1, None, Paid, Invoice), &[p1, p2]);
        let bc = scoped(grant(50, 2, None, Paid, Invoice), &[p2, p3]);
        let a_only = scoped(grant(50, 3, None, Paid, Invoice), &[p1]);
        let allocation = allocate(&[ab, bc, a_only], &request, at(10), Pool::All).unwrap();
        assert_eq!(allocation.covered, vec![50, 50, 50]);
    }

    #[test]
    fn rerouting_frees_a_grant_for_the_charge_only_it_can_pay() {
        let (p1, p2, p3) = (id(), id(), id());
        let request = Request {
            requested_minor: 100,
            demands: vec![product_demand(p1, false, 50), product_demand(p2, false, 50)],
        };
        let a = scoped(grant(50, 100, None, Paid, Invoice), &[p1, p2]);
        let c = scoped(grant(50, 100, None, Paid, Invoice), &[p2, p3]);
        let grants = [(&a, 50), (&c, 50)];
        let pays =
            |grant: &Grant, column: usize| column < 2 && grant.pays(&request.demands[column]);
        // `a` paid the charge `c` could have paid, leaving p1 open.
        let mut paid = vec![vec![0, 50, 0], vec![0, 0, 0]];
        let (mut open, mut budget, mut left) = (vec![50, 0, 0], vec![0, 50], 50);
        augment(&grants, &pays, &mut open, &mut budget, &mut left, &mut paid);
        assert_eq!(paid, vec![vec![50, 0, 0], vec![0, 50, 0]]);
        assert_eq!((left, budget), (0, vec![0, 0]));
    }

    #[test]
    fn scoped_then_usage_then_unrestricted_credit() {
        let (a, b) = (id(), id());
        let request = Request {
            requested_minor: 3_200,
            demands: vec![
                product_demand(a, false, 1_000),
                product_demand(a, true, 500),
                product_demand(b, true, 1_000),
            ],
        };
        let unrestricted = grant(5_000, 1, None, Paid, Invoice);
        let usage = grant(5_000, 1, None, Paid, Usage);
        let only_b = scoped(grant(300, 100, None, Paid, Invoice), &[b]);
        let allocation = allocate(
            &[unrestricted.clone(), usage.clone(), only_b.clone()],
            &request,
            at(10),
            Pool::All,
        )
        .unwrap();
        assert_eq!(
            allocation.draws,
            vec![
                (only_b.id, 300),
                (usage.id, 1_200),
                (unrestricted.id, 1_700)
            ]
        );
        // The tax (what no demand names) only unrestricted invoice credit pays.
        assert_eq!(allocation.covered, vec![1_000, 500, 1_000]);
        let scoped_alone =
            allocate(std::slice::from_ref(&only_b), &request, at(10), Pool::All).unwrap();
        assert_eq!(scoped_alone.covered, vec![0, 0, 300]);
    }

    fn of_subscription(mut grant: Grant, subscription: Uuid) -> Grant {
        grant.scope.subscription = Some(subscription);
        grant
    }

    fn usage_of(subscription: Uuid, amount_minor: i64) -> Demand {
        Demand {
            subscription: Some(subscription),
            product: None,
            usage: true,
            amount_minor,
            dated_before: None,
        }
    }

    fn requested(demands: Vec<Demand>) -> Request {
        Request {
            requested_minor: demands.iter().map(|demand| demand.amount_minor).sum(),
            demands,
        }
    }

    #[test]
    fn a_subscription_grant_never_pays_another_subscription() {
        let (a, b) = (id(), id());
        let pack = of_subscription(grant(10_000, 100, Some(at(30)), Promotional, Usage), a);
        let request = requested(vec![usage_of(b, 4_000)]);
        assert!(drawn(std::slice::from_ref(&pack), request, at(10)).is_empty());
        // Nor a charge of no subscription, nor the unnamed remainder.
        let anyone = Request {
            requested_minor: 500,
            demands: vec![Demand {
                subscription: None,
                product: None,
                usage: true,
                amount_minor: 400,
                dated_before: None,
            }],
        };
        let invoice_wide = of_subscription(grant(10_000, 100, None, Paid, Invoice), a);
        assert!(drawn(&[pack, invoice_wide], anyone, at(10)).is_empty());
    }

    #[test]
    fn a_customer_wide_grant_pays_any_subscription() {
        let (a, b) = (id(), id());
        let top_up = grant(5_000, 100, None, Paid, Invoice);
        let allocation = allocate(
            std::slice::from_ref(&top_up),
            &requested(vec![usage_of(a, 1_000), usage_of(b, 2_000)]),
            at(10),
            Pool::All,
        )
        .unwrap();
        assert_eq!(allocation.draws, vec![(top_up.id, 3_000)]);
        assert_eq!(allocation.covered, vec![1_000, 2_000]);
    }

    #[test]
    fn subscription_credit_is_drawn_before_customer_credit_in_the_same_stage() {
        let a = id();
        // The customer-wide grant would win on every other key: priority, lapse and age.
        let shared = grant(5_000, 1, Some(at(12)), Paid, Usage);
        let mut pack = of_subscription(grant(5_000, 100, None, Paid, Invoice), a);
        pack.created_at = at(2);
        let allocation = allocate(
            &[shared.clone(), pack.clone()],
            &requested(vec![usage_of(a, 3_000)]),
            at(10),
            Pool::Paid,
        )
        .unwrap();
        assert_eq!(allocation.draws, vec![(pack.id, 3_000)]);
        assert_eq!(grant_order(&pack, &shared), Ordering::Less);
        // Stages come first for money: a customer-wide promotional grant pays before the tax,
        // ahead of the subscription's paid grant.
        let promo = grant(1_000, 100, None, Promotional, Invoice);
        let (_, in_turn) = allocate_in_turn_keeping(
            &[pack.clone(), promo.clone()],
            &[],
            &requested(vec![usage_of(a, 3_000)]),
            at(10),
            &[Pool::Promotional, Pool::Paid],
        )
        .unwrap();
        assert_eq!(in_turn.draws, vec![(promo.id, 1_000), (pack.id, 2_000)]);
    }

    #[test]
    fn usage_credit_never_pays_a_fee_or_the_unnamed_remainder() {
        let a = id();
        let pack = of_subscription(grant(10_000, 100, None, Promotional, Usage), a);
        let request = Request {
            requested_minor: 3_100,
            demands: vec![
                Demand {
                    subscription: Some(a),
                    product: None,
                    usage: false,
                    amount_minor: 2_000,
                    dated_before: None,
                },
                usage_of(a, 1_000),
            ],
        };
        let allocation =
            allocate(std::slice::from_ref(&pack), &request, at(10), Pool::All).unwrap();
        assert_eq!(allocation.draws, vec![(pack.id, 1_000)]);
        assert_eq!(allocation.covered, vec![0, 1_000]);
        assert_eq!(
            request.remainder(&allocation).unwrap().requested_minor,
            2_100
        );
    }

    #[test]
    fn augmenting_moves_shared_credit_to_where_scoped_credit_cannot_pay() {
        let (a, b) = (id(), id());
        let pack = of_subscription(grant(100, 100, None, Paid, Usage), a);
        let shared = grant(100, 100, None, Paid, Invoice);
        for demands in [
            vec![usage_of(a, 100), usage_of(b, 100)],
            vec![usage_of(b, 100), usage_of(a, 100)],
        ] {
            for grants in [
                [pack.clone(), shared.clone()],
                [shared.clone(), pack.clone()],
            ] {
                let allocation =
                    allocate(&grants, &requested(demands.clone()), at(10), Pool::All).unwrap();
                assert_eq!(allocation.covered, vec![100, 100]);
                assert_eq!(allocation.total().unwrap(), 200);
            }
        }
    }

    #[test]
    fn a_grant_past_its_ends_at_is_not_spendable() {
        let a = id();
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
    }

    /// A's lines first or B's: the customer-wide promotion pays B's usage, which A's paid pack
    /// cannot, and the pack pays all of A's after tax; nothing is left to pay.
    #[test]
    fn promotional_credit_leaves_to_scoped_paid_credit_what_it_can_pay() {
        let (a, b) = (id(), id());
        let pack = of_subscription(grant(10_000, 100, None, Paid, Usage), a);
        let promo = grant(5_000, 100, None, Promotional, Invoice);
        for demands in [
            vec![usage_of(a, 10_000), usage_of(b, 5_000)],
            vec![usage_of(b, 5_000), usage_of(a, 10_000)],
        ] {
            let (_, drawn) = allocate_in_turn_keeping(
                &[pack.clone(), promo.clone()],
                &[],
                &requested(demands),
                at(10),
                &[Pool::Promotional, Pool::Paid],
            )
            .unwrap();
            assert_eq!(drawn.total().unwrap(), 15_000);
            assert_eq!(drawn.draws, vec![(promo.id, 5_000), (pack.id, 10_000)]);
        }
    }

    /// September's usage reaches its invoice on Oct 1: September's rollover and credit live in
    /// September pay it; credit bought on Oct 1 pays only October's fee and usage.
    #[test]
    fn usage_is_paid_only_by_credit_effective_before_its_dates_end() {
        let october = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        let hour = chrono::Duration::hours(1);
        let september = grant(1_000, 100, None, Paid, Invoice);
        let rolled = Grant {
            effective_at: october,
            lapses_at: Some(october + chrono::Duration::days(31)),
            ..grant(500, 99, None, Paid, Invoice)
        };
        let bought = Grant {
            effective_at: october + hour,
            pays_usage_from: october + hour,
            created_at: october + hour,
            ..grant(5_000, 100, None, Paid, Invoice)
        };
        let grants = [september.clone(), rolled.clone(), bought.clone()];
        let dated = |dated_before| Request {
            requested_minor: 3_000,
            demands: vec![Demand {
                subscription: None,
                product: None,
                usage: true,
                amount_minor: 3_000,
                dated_before,
            }],
        };
        let late = allocate(
            &grants,
            &dated(Some(october)),
            october + hour * 2,
            Pool::All,
        )
        .unwrap();
        assert_eq!(late.draws, vec![(rolled.id, 500), (september.id, 1_000)]);
        assert_eq!(
            late.covered,
            vec![1_500],
            "the rest is billed beyond credit"
        );
        let current = allocate(
            &grants,
            &dated(Some(october + chrono::Duration::days(31))),
            october + hour * 2,
            Pool::All,
        )
        .unwrap();
        assert_eq!(current.covered, vec![3_000]);
        assert!(current.draws.contains(&(bought.id, 1_500)));
        // A fee of the same invoice is no usage: any credit spendable then pays it.
        let fee = Request {
            requested_minor: 3_000,
            demands: vec![Demand {
                usage: false,
                ..dated(None).demands[0]
            }],
        };
        assert_eq!(
            allocate(&grants, &fee, october + hour * 2, Pool::All)
                .unwrap()
                .covered,
            vec![3_000]
        );
    }

    /// A settlement kept by a grant eligible for both plans' lines of a period (an immediate change)
    /// is placed on the old plan's line first: the credit drawn after it may pay only the new one.
    #[test]
    fn kept_credit_pays_the_earliest_dated_usage_first() {
        let change = Utc.with_ymd_and_hms(2026, 9, 16, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        let pack = grant(0, 100, None, Paid, Usage);
        let usage = |amount_minor, dated_before| Demand {
            subscription: None,
            product: None,
            usage: true,
            amount_minor,
            dated_before: Some(dated_before),
        };
        let request = Request {
            requested_minor: 3_000,
            demands: vec![usage(2_000, end), usage(1_000, change)],
        };
        let kept = assign(&[(pack, 1_000)], &request).unwrap();
        assert_eq!(kept.covered, vec![0, 1_000]);
    }

    /// The rerouting never moves late usage onto credit not effective before it ended.
    #[test]
    fn rerouting_keeps_late_usage_off_later_credit() {
        let october = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        let september = grant(1_000, 100, None, Paid, Invoice);
        let bought = Grant {
            effective_at: october,
            pays_usage_from: october,
            ..grant(1_000, 100, None, Paid, Invoice)
        };
        let request = Request {
            requested_minor: 2_000,
            demands: vec![
                Demand {
                    subscription: None,
                    product: None,
                    usage: false,
                    amount_minor: 1_000,
                    dated_before: None,
                },
                Demand {
                    subscription: None,
                    product: None,
                    usage: true,
                    amount_minor: 1_000,
                    dated_before: Some(october),
                },
            ],
        };
        let paid = allocate(
            &[september.clone(), bought],
            &request,
            october + chrono::Duration::hours(1),
            Pool::All,
        )
        .unwrap();
        assert_eq!(paid.covered, vec![1_000, 1_000]);
        let usage_payer = paid
            .placed
            .iter()
            .find(|(_, columns)| columns[1] > 0)
            .map(|(id, _)| *id);
        assert_eq!(usage_payer, Some(september.id));
    }
}
