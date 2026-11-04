//! Purchased credit follows the money that paid for it: a reversed payment, or one refunded without
//! a credit note, takes back the unspent share and reports the spent one (a failed collection's
//! reopened invoice collects it again, a dispute's or a refund's loss is the merchant's), and money
//! that comes back (reinstated, paid again, a refund that failed) restores what was taken and
//! withdraws the report.
use super::invalid;
use crate::StoreResult;
use crate::domain::credit::{GrantClassification, LedgerEntryKind};
use crate::domain::wallet_events::{CreditUnrecoveredCause, CreditUnrecoveredTrigger};
use crate::hooks::{CreditControlHooks, ReversedPayment};
use crate::repositories::credit::customer_credit::{
    PaymentMovement, balance_changed_tx, dispute_follow_up_tx, follow_payment_tx,
    report_unrecovered_credit_tx, source,
};
use crate::store::PgConn;
use chrono::Utc;
use common_domain::ids::{BaseId, CreditGrantId, InvoiceId, TenantId};
use diesel_models::customer_credit::{
    CustomerCreditEntryRow, CustomerCreditGrantRow, CustomerCreditOperationRow,
};
use diesel_models::invoices::InvoiceRow;
use diesel_models::lock::CustomerLock;

/// Aligns a purchase invoice the previous release left reversed, under the invoice lock.
pub(crate) async fn reconcile_reversed_purchase_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
) -> StoreResult<()> {
    let lock = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
        .await?
        .lock;
    follow_purchase_money_tx(conn, controls, &lock, invoice_id, None, true).await
}

/// Brings the invoice's purchased credit in line with the money still missing on it, and its
/// reported loss with the spent credit that money backed, under the invoice lock; idempotent, as
/// both derive from the payments and the ledger alone. `moved` is the payment movement (reversal
/// or reinstatement) that changed the money, if one did.
pub(crate) async fn reconcile_purchased_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    invoice_id: InvoiceId,
    moved: Option<&PaymentMovement>,
) -> StoreResult<()> {
    follow_purchase_money_tx(conn, controls, lock, invoice_id, moved, false).await
}

/// Credit the invoice drew went back to its grants (a void, a credit note, returned settlement
/// excess): a purchase whose money was reversed takes it back at once and withdraws the loss it
/// covers. The caller holds the customer's lock, which every purchase invoice lock takes first.
pub(crate) async fn follow_credit_given_back_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    invoice_id: InvoiceId,
) -> StoreResult<()> {
    let tenant_id = lock.tenant_id();
    let ids = CustomerCreditEntryRow::list_by_invoice(conn, tenant_id, invoice_id)
        .await?
        .into_iter()
        .filter(|entry| entry.kind == LedgerEntryKind::DrawReversal.as_db_str())
        .map(|entry| entry.grant_id)
        .collect::<Vec<_>>();
    follow_grants_given_back_tx(conn, controls, lock, ids, Some(invoice_id)).await
}

/// `follow_credit_given_back_tx` for credit given back to these grants, skipping the purchase of
/// `spending` itself. The caller holds the customer's lock (`lock_customer_tx`) and these grants.
pub(crate) async fn follow_grants_given_back_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    mut ids: Vec<CreditGrantId>,
    spending: Option<InvoiceId>,
) -> StoreResult<()> {
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let given_back = CustomerCreditGrantRow::lock_by_ids(conn, lock, &ids).await?;
    super::collected_payment::follow_collected_given_back_tx(conn, controls, lock, &given_back)
        .await?;
    let mut purchases = given_back
        .into_iter()
        .filter(|grant| grant.source_kind == source::CREDIT_PURCHASE)
        .filter_map(|grant| grant.invoice_id)
        .filter(|purchase| Some(*purchase) != spending)
        .collect::<Vec<_>>();
    purchases.sort_by_key(|purchase| purchase.as_uuid());
    purchases.dedup();
    for purchase in &purchases {
        InvoiceRow::lock_by_id(conn, lock, *purchase).await?;
    }
    for purchase in purchases {
        follow_purchase_money_tx(conn, controls, lock, purchase, None, false).await?;
    }
    Ok(())
}

/// `healing`: the sweep applies money the previous release moved without following it.
async fn follow_purchase_money_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    invoice_id: InvoiceId,
    moved: Option<&PaymentMovement>,
    healing: bool,
) -> StoreResult<()> {
    let tenant_id = lock.tenant_id();
    let grants = CustomerCreditGrantRow::lock_by_invoice(conn, lock, invoice_id)
        .await?
        .into_iter()
        .filter(|grant| grant.source_kind == source::CREDIT_PURCHASE)
        .collect::<Vec<_>>();
    let backing = Backing::of_invoice_tx(conn, tenant_id, invoice_id).await?;
    let moved = moved.filter(|moved| !moved.follow_up);
    let trigger =
        CreditUnrecoveredTrigger::Payments(moved.map_or(backing.cause, PaymentMovement::cause));
    let payment = moved.map(|moved| moved.payment);
    if grants.is_empty() {
        return Ok(());
    }
    let entries = CustomerCreditEntryRow::list_by_invoice(conn, tenant_id, invoice_id).await?;
    let followed = grants
        .iter()
        .map(|grant| followed_by_grant(&entries, grant))
        .collect::<StoreResult<Vec<_>>>()?;
    let shares = grant_shares(&followed, &backing)?;

    if shares.iter().any(|share| *share > 0)
        && let Some(moved) = moved.filter(|moved| moved_out(moved, &followed, &shares))
        && let Some(operation) = CustomerCreditOperationRow::find_by_payment_transaction_for_update(
            conn,
            lock,
            moved.payment,
        )
        .await?
    {
        // Before any movement, so the lower balance cannot queue a top-up on the card the money
        // went back to.
        let reversed = reversed_payment_tx(conn, tenant_id, moved).await?;
        controls
            .purchase_payment_reversed(conn, lock, &operation, reversed, Utc::now())
            .await?;
    }

    let remaining = grants
        .iter()
        .map(|grant| grant.remaining)
        .collect::<Vec<_>>();
    let plan = plan_movements(&remaining, &followed, &shares);
    // A refund or dispute the previous release recorded stopped no top-up: stop it now, before
    // the take-back lowers the balance onto the card the money went back to.
    if healing
        && moved.is_none()
        && matches!(
            backing.cause,
            CreditUnrecoveredCause::PaymentRefunded | CreditUnrecoveredCause::PaymentDisputed
        )
        && plan.taken.iter().any(|taken| *taken > 0)
    {
        let currency = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
            .await?
            .currency;
        let missing = backing.unbacked.min(backing.standing);
        suspend_top_up_of_refunded_purchase_tx(
            conn, controls, lock, invoice_id, &currency, missing,
        )
        .await?;
    }
    let mut touched = Vec::new();
    let now = Utc::now();
    let follow_up = match moved {
        None if plan.movements.iter().any(|movement| *movement != 0) => {
            dispute_follow_up_tx(conn, tenant_id, Some(invoice_id), None, now).await?
        }
        _ => None,
    };
    let linked = moved.or(follow_up.as_ref());
    let worded = match follow_up {
        Some(_) => CreditUnrecoveredCause::PaymentDisputed,
        None => trigger.cause(),
    };
    for (grant, movement) in grants.iter().zip(&plan.movements) {
        if *movement != 0 {
            touched.push(
                follow_payment_tx(
                    conn,
                    grant.id,
                    *movement,
                    Some(invoice_id),
                    linked,
                    worded,
                    now,
                )
                .await?,
            );
        }
    }
    balance_changed_tx(conn, controls, tenant_id, &touched, now).await?;

    // (first grant of the wallet, taken back now, spent credit the missing money backed)
    let mut wallets: Vec<(usize, i64, i64)> = Vec::new();
    for (index, grant) in grants.iter().enumerate() {
        // Bonus credit sold with the purchase was never paid for: its use is no loss.
        let short = if grant.classification == GrantClassification::Promotional.as_db_str() {
            0
        } else {
            plan.short[index]
        };
        let taken = plan.taken[index];
        match wallets
            .iter_mut()
            .find(|(first, _, _)| grants[*first].account_id == grant.account_id)
        {
            Some(wallet) => {
                wallet.1 = wallet.1.checked_add(taken).ok_or_else(out_of_range)?;
                wallet.2 = wallet.2.checked_add(short).ok_or_else(out_of_range)?;
            }
            None => wallets.push((index, taken, short)),
        }
    }
    for (first, taken, short) in wallets {
        report_unrecovered_credit_tx(
            conn,
            lock,
            &grants[first],
            Some(invoice_id),
            payment,
            trigger,
            taken,
            short,
        )
        .await?;
    }
    Ok(())
}

/// Credit a top-up bought is taken back for money returned (a refund credit note, or a refund or
/// dispute the sweep applies): the wallet's automatic top-up stops before the lower balance
/// charges that card again. The email names `refunded_minor` of `currency`.
pub(crate) async fn suspend_top_up_of_refunded_purchase_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    lock: &CustomerLock,
    invoice_id: InvoiceId,
    currency: &str,
    refunded_minor: i64,
) -> StoreResult<()> {
    if refunded_minor <= 0 {
        return Ok(());
    }
    let Some(operation) =
        CustomerCreditOperationRow::find_settled_by_invoice_for_update(conn, lock, invoice_id)
            .await?
    else {
        return Ok(());
    };
    let refunded = ReversedPayment {
        currency: currency.to_owned(),
        amount_minor: refunded_minor,
    };
    controls
        .purchase_payment_reversed(conn, lock, &operation, refunded, Utc::now())
        .await
}

/// The movement takes credit back: some grant's share now exceeds what reversals hold back.
/// What this movement took back from the payment, in the money paid: a partial refund or
/// dispute is its own amount, never the whole top-up.
async fn reversed_payment_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    moved: &PaymentMovement,
) -> StoreResult<ReversedPayment> {
    use diesel_models::payment_reversal_movements::PaymentReversalMovementRow;
    use diesel_models::payments::PaymentTransactionRow;
    let payment = PaymentTransactionRow::get_by_id(conn, moved.payment, tenant_id).await?;
    if let Some(refunded) = moved.refunded.filter(|amount| *amount > 0) {
        return Ok(ReversedPayment {
            currency: payment.currency,
            amount_minor: refunded,
        });
    }
    let movements =
        PaymentReversalMovementRow::list_by_payment(conn, tenant_id, moved.payment).await?;
    let applied: i64 = movements.iter().map(|row| row.amount).sum();
    let amount_minor = moved
        .movement
        .and_then(|id| movements.iter().find(|row| row.id == id))
        .map_or(applied, |row| row.amount);
    Ok(ReversedPayment {
        currency: payment.currency,
        amount_minor: if amount_minor > 0 {
            amount_minor
        } else {
            payment.amount
        },
    })
}

fn moved_out(_moved: &PaymentMovement, followed: &[Followed], shares: &[i64]) -> bool {
    followed
        .iter()
        .zip(shares)
        .any(|(followed, share)| *share > followed.clawed)
}

fn out_of_range() -> error_stack::Report<crate::errors::StoreError> {
    invalid("the purchased credit exceeds the supported range")
}

/// Per grant: the signed ledger movement (negative takes back, positive restores), the value
/// taken back now, and the spent credit the missing money backed once moved.
struct Plan {
    movements: Vec<i64>,
    taken: Vec<i64>,
    short: Vec<i64>,
}

/// A take-back first covers the share the missing money calls for; what no unspent credit can
/// cover is the shortfall, reported as the current loss.
fn plan_movements(remaining: &[i64], followed: &[Followed], shares: &[i64]) -> Plan {
    let mut plan = Plan {
        movements: Vec::with_capacity(followed.len()),
        taken: Vec::with_capacity(followed.len()),
        short: Vec::with_capacity(followed.len()),
    };
    for ((remaining, followed), share) in remaining.iter().zip(followed).zip(shares) {
        let delta = share - followed.clawed;
        let movement = if delta > 0 {
            -delta.min((*remaining).max(0))
        } else {
            -delta
        };
        plan.movements.push(movement);
        plan.taken.push((-movement).max(0));
        plan.short
            .push((share - (followed.clawed - movement)).max(0));
    }
    plan
}

/// The invoice money the purchased credit still stands on, and how much of it is missing.
struct Backing {
    /// Reversed or refunded collections nothing collected again (`InvoiceCollections::uncollected`).
    unbacked: i64,
    /// What the invoice still collects: refunds and forgiven debt took the rest. A refund of a
    /// purchase line also took back its credit, one of another line left the credit whole, so
    /// the purchase stands on a larger share of what remains.
    standing: i64,
    /// What took its money, when no payment movement says.
    cause: CreditUnrecoveredCause,
}

impl Backing {
    async fn of_invoice_tx(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Self> {
        let collections = InvoiceRow::collections(conn, invoice_id, tenant_id).await?;
        let cause = if collections.failed == 0 && collections.disputed > 0 {
            CreditUnrecoveredCause::PaymentDisputed
        } else if collections.failed == 0 && collections.refunded > 0 {
            CreditUnrecoveredCause::PaymentRefunded
        } else {
            CreditUnrecoveredCause::PaymentReversed
        };
        Ok(Self {
            unbacked: collections.uncollected(),
            standing: collections.owed.max(0),
            cause,
        })
    }
}

/// Each grant loses the invoice's missing fraction of its own capacity, in its own denomination:
/// pooling grants of different units or prices would take back more cost than money was lost.
fn grant_shares(followed: &[Followed], backing: &Backing) -> StoreResult<Vec<i64>> {
    followed
        .iter()
        .map(|followed| credit_share(followed.capacity, backing.unbacked, backing.standing))
        .collect()
}

/// `bought × unbacked / standing`, rounded up: a partial reversal never leaves a fraction of a
/// unit of unbacked credit behind.
fn credit_share(bought: i64, unbacked: i64, standing: i64) -> StoreResult<i64> {
    if bought <= 0 || unbacked <= 0 || standing <= 0 {
        return Ok(0);
    }
    let unbacked = unbacked.min(standing);
    let numerator = i128::from(bought) * i128::from(unbacked);
    let share = (numerator + i128::from(standing) - 1) / i128::from(standing);
    i64::try_from(share)
        .map(|share| share.min(bought))
        .map_err(|_| invalid("the purchased credit exceeds the supported range"))
}

struct Followed {
    /// What the grant can still lose to a payment reversal: credit notes took the rest.
    capacity: i64,
    /// Net value payment reversals hold back from the grant.
    clawed: i64,
}

fn followed_by_grant(
    entries: &[CustomerCreditEntryRow],
    grant: &CustomerCreditGrantRow,
) -> StoreResult<Followed> {
    let (mut by_notes, mut clawed) = (0_i64, 0_i64);
    for entry in entries.iter().filter(|entry| entry.grant_id == grant.id) {
        let slot = match LedgerEntryKind::from_db_str(&entry.kind)? {
            LedgerEntryKind::Reversal if entry.credit_note_id.is_some() => &mut by_notes,
            LedgerEntryKind::Reversal | LedgerEntryKind::Reinstatement => &mut clawed,
            _ => continue,
        };
        *slot = slot
            .checked_sub(entry.amount)
            .ok_or_else(|| invalid("the purchased credit exceeds the supported range"))?;
    }
    Ok(Followed {
        capacity: grant.amount.saturating_sub(by_notes),
        clawed,
    })
}

#[cfg(test)]
mod tests {
    use super::{Backing, Followed, credit_share, grant_shares, plan_movements};
    use crate::domain::wallet_events::CreditUnrecoveredCause;

    fn fresh(capacities: &[i64]) -> Vec<Followed> {
        capacities
            .iter()
            .map(|capacity| Followed {
                capacity: *capacity,
                clawed: 0,
            })
            .collect()
    }

    fn missing(unbacked: i64, standing: i64) -> Backing {
        Backing {
            unbacked,
            standing,
            cause: CreditUnrecoveredCause::PaymentReversed,
        }
    }

    /// One grant of 5000 with 1000 left unspent, reversed 30% then 70%: the shortfall is the loss
    /// so far (500, then 4000), never the sum of both.
    #[test]
    fn the_shortfall_is_the_spent_credit_the_missing_money_backed() {
        let first = plan_movements(&[1_000], &fresh(&[5_000]), &[1_500]);
        assert_eq!(
            (first.movements, first.taken, first.short),
            (vec![-1_000], vec![1_000], vec![500])
        );

        let after_first = [Followed {
            capacity: 5_000,
            clawed: 1_000,
        }];
        let second = plan_movements(&[0], &after_first, &[5_000]);
        assert_eq!(
            (second.movements, second.taken, second.short),
            (vec![0], vec![0], vec![4_000])
        );

        // Credit given back after the report is taken back first: the loss shrinks to 2000.
        let refilled = plan_movements(&[2_000], &after_first, &[5_000]);
        assert_eq!(
            (refilled.movements, refilled.taken, refilled.short),
            (vec![-2_000], vec![2_000], vec![2_000])
        );

        // Money back restores what was taken and leaves no loss.
        let won_back = plan_movements(&[0], &after_first, &[0]);
        assert_eq!(
            (won_back.movements, won_back.taken, won_back.short),
            (vec![1_000], vec![0], vec![0])
        );
    }

    /// 7000 invoice: a 5000 credit line and a 2000 line refunded in cash, then the remaining 5000
    /// charged back. Old code spread the reversal over the whole 7000 and reported 3572.
    #[test]
    fn a_refund_of_another_line_leaves_the_purchase_standing_on_what_remains() {
        let old = credit_share(5_000, 5_000, 7_000).unwrap();
        assert_eq!(old, 3_572);
        assert_eq!(credit_share(5_000, 5_000, 7_000 - 2_000).unwrap(), 5_000);
        // A 2000 refund of the credit line itself also took back 2000 of its credit.
        assert_eq!(
            credit_share(5_000 - 2_000, 5_000, 7_000 - 2_000).unwrap(),
            3_000
        );
        assert_eq!(
            credit_share(5_000 - 2_000, 2_500, 7_000 - 2_000).unwrap(),
            1_500
        );
    }

    /// $10 of 100 ALFA and $10 of 10000 BETA, $10 of the $20 charged back. Old code pooled both
    /// into one 5050 target filled in grant order: [100, 4950], $14.95 of cost for $10 lost.
    #[test]
    fn each_grant_loses_its_own_share_of_the_missing_money() {
        assert_eq!(
            grant_shares(&fresh(&[100, 10_000]), &missing(1_000, 2_000)).unwrap(),
            vec![50, 5_000]
        );
        // One unit at two prices: half of each, never all of the first.
        assert_eq!(
            grant_shares(&fresh(&[1_000, 1_000]), &missing(3_000, 6_000)).unwrap(),
            vec![500, 500]
        );
        // Each rounds up within its own capacity; a grant notes took in full loses nothing.
        assert_eq!(
            grant_shares(&fresh(&[1, 3, 0]), &missing(1, 3)).unwrap(),
            vec![1, 1, 0]
        );
        assert_eq!(
            grant_shares(&fresh(&[100, 10_000]), &missing(5_000, 2_000)).unwrap(),
            vec![100, 10_000]
        );
    }

    #[test]
    fn the_share_is_proportional_and_rounds_against_unbacked_credit() {
        assert_eq!(credit_share(5_000, 5_000, 5_000).unwrap(), 5_000);
        assert_eq!(credit_share(5_000, 0, 5_000).unwrap(), 0);
        assert_eq!(credit_share(10_000, 1, 12_000).unwrap(), 1);
        assert_eq!(credit_share(10_000, 6_000, 12_000).unwrap(), 5_000);
        assert_eq!(credit_share(10_000, 20_000, 12_000).unwrap(), 10_000);
        assert_eq!(credit_share(10_000, 1_000, 0).unwrap(), 0);
        assert_eq!(credit_share(-5, 1_000, 2_000).unwrap(), 0);
    }
}
