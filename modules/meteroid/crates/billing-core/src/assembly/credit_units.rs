//! Usage priced in a credit unit is paid first by the customer's wallet in that unit; only what it
//! does not cover is billed in money (R-039). Each subscription's usage, by the end of its lines'
//! dates, is its own demand: only its own and customer-wide credit effective before that end pay it.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::credit::allocation::{Allocation, Demand, Overflow, Pool, Request, allocate};
use crate::credit::owed::assign_settled;
use crate::credit::settle::{Drawn, Grants};

/// One subscription's usage of one unit, of lines ending at `dated_before`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitUse {
    pub subscription: Uuid,
    pub dated_before: DateTime<Utc>,
    pub consumed: i64,
}

/// One demand per use, in order, each at most `amount_of` it.
pub fn unit_request(uses: &[UnitUse], amount_of: impl Fn(&UnitUse) -> i64) -> Request {
    let demands = uses
        .iter()
        .map(|used| Demand {
            subscription: Some(used.subscription),
            product: None,
            usage: true,
            amount_minor: amount_of(used).max(0),
            dated_before: Some(used.dated_before),
        })
        .collect::<Vec<_>>();
    Request {
        requested_minor: demands.iter().map(|demand| demand.amount_minor).sum(),
        demands,
    }
}

/// What each use would have covered if finalized at `grants.at`: the open settlements of its
/// periods kept as finalizing keeps them (R10), then a draw on the rest; never above its use.
pub fn estimate_cover(uses: &[UnitUse], grants: &Grants) -> Result<(Drawn, Vec<i64>), Overflow> {
    let consumed = unit_request(uses, |used| used.consumed);
    let kept = assign_settled(&grants.settled, &consumed)?;
    let rest = consumed.remainder(&kept)?;
    let fresh = if rest.requested_minor <= 0 {
        Allocation::default()
    } else {
        allocate(&grants.drawable, &rest, grants.at, Pool::All)?
    };
    let covered = uses
        .iter()
        .enumerate()
        .map(|(index, used)| {
            kept.covered
                .get(index)
                .copied()
                .unwrap_or(0)
                .saturating_add(fresh.covered.get(index).copied().unwrap_or(0))
                .min(used.consumed)
        })
        .collect();
    Ok((Drawn { kept, fresh }, covered))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credit::allocation::{Application, Classification, Grant, Scope};
    use crate::credit::owed::Settled;
    use chrono::TimeZone;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, day, 0, 0, 0).unwrap()
    }

    fn wallet(id: u128, subscription: Option<Uuid>, remaining: i64) -> Grant {
        Grant {
            id: Uuid::from_u128(id),
            scope: Scope {
                subscription,
                application: Application::Usage,
                products: Vec::new(),
            },
            remaining_minor: remaining,
            priority: 100,
            classification: Classification::Paid,
            effective_at: at(1),
            pays_usage_from: at(1),
            lapses_at: None,
            created_at: at(1),
        }
    }

    /// A settlement kept pays first, the wallet the rest; another subscription's grant pays none.
    #[test]
    fn kept_settlements_then_the_wallet_cover_the_usage() {
        let (ours, theirs) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let uses = [UnitUse {
            subscription: ours,
            dated_before: at(31),
            consumed: 1_000,
        }];
        let grants = Grants {
            drawable: vec![wallet(10, Some(theirs), 5_000), wallet(11, None, 500)],
            settled: vec![Settled {
                window: None,
                grant: wallet(12, Some(ours), 0),
                amount: 300,
            }],
            at: at(15),
        };
        let (drawn, covered) = estimate_cover(&uses, &grants).unwrap();
        assert_eq!(covered, vec![800]);
        assert_eq!(drawn.kept.total().unwrap(), 300);
        assert_eq!(drawn.fresh.draws, vec![(Uuid::from_u128(11), 500)]);
    }
}
