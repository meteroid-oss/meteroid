//! What an issued invoice's money comes to: one fold over everything that moved it (R-125), and
//! the decisions taken from it: a charge (R-127), a credit note's split (R-126), a void (R-128).
//!
//! Every figure is in minor units of the invoice's currency. Credit note amounts are positive.

use uuid::Uuid;

/// Where a payment row stands, as the store records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveStatus {
    /// Committed, not sent yet.
    Ready,
    /// Sent, its outcome not known yet.
    Pending,
    Settled,
    /// Settled, then fully given back or clawed back.
    Refunded,
    Failed,
    Cancelled,
}

impl MoveStatus {
    fn in_flight(self) -> bool {
        matches!(self, MoveStatus::Ready | MoveStatus::Pending)
    }

    fn collected(self) -> bool {
        matches!(self, MoveStatus::Settled | MoveStatus::Refunded)
    }

    fn live(self) -> bool {
        matches!(
            self,
            MoveStatus::Ready | MoveStatus::Pending | MoveStatus::Settled
        )
    }
}

/// Money collected toward the invoice: a payment, or a top-up it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    pub amount: i64,
    /// Clawed back by the bank or the provider, disputes included.
    pub reversed: i64,
    /// Of `reversed`, what a dispute withdrew: lost to the merchant, the invoice stays paid.
    pub disputed: i64,
    pub status: MoveStatus,
    /// An in-flight collection its provider accepted (a delayed-notification debit): money is
    /// moving, the invoice is neither charged again nor chased.
    pub accepted: bool,
    /// A saved method was charged and declined: the invoice reads Errored. A customer's attempt on
    /// a hosted page, or a decline whose outcome is unconfirmed, leaves it payable as it was.
    pub declined: bool,
}

/// Money given back on one of the invoice's collections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refund {
    pub amount: i64,
    pub status: MoveStatus,
    /// The Refund note it draws on; `None` for a refund made without one.
    pub note: Option<Uuid>,
    /// Of a refund without a note, the excess over what the invoice asks it gave back when issued.
    pub excess: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    CreditToBalance,
    Refund,
    DebtCancellation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteStatus {
    Draft,
    Finalized,
    Voided,
}

/// A credit note of the invoice, with the split its decision recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub id: Uuid,
    pub kind: NoteKind,
    pub status: NoteStatus,
    /// What it credits, positive.
    pub total: i64,
    /// Returned to the customer's balance.
    pub credited: i64,
    /// Refunded in cash.
    pub refunded: i64,
    pub debt_cancelled: i64,
    /// The note of the invoice's void: the void returns the draws itself.
    pub voids_invoice: bool,
}

/// An issued invoice and everything that moved its money. Collections and notes in the order
/// they happened (notes by finalization).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    pub total: i64,
    /// The paid credit finalize drew (`applied_credits`).
    pub drawn: i64,
    pub collections: Vec<Collection>,
    pub refunds: Vec<Refund>,
    pub notes: Vec<Note>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaymentStatus {
    #[default]
    Unpaid,
    PartiallyPaid,
    Paid,
    Errored,
    Processing,
}

/// The account folded (R-125).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// What the invoice still bills: its total less its finalized notes'.
    pub remaining: i64,
    /// Drawn credit still paying it.
    pub credit: i64,
    /// Cash still paying it: collected, less what was clawed back (a dispute's loss stays paid),
    /// what notes turned into credit or promised back, and the excess refunds gave back.
    pub cash: i64,
    /// Of `cash`, what disputes withdrew.
    pub disputed: i64,
    /// What refunds without a note gave back beyond the excess: the invoice stays paid.
    pub goodwill: i64,
    /// Money clawed back as failed collections: the invoice owes it again.
    pub failed: i64,
    pub due: i64,
    /// Cash kept above what the invoice asks: OVERPAYMENT credit (OQ12).
    pub excess: i64,
    /// Reversed, disputed or goodwill-refunded money that nothing collected again: what the credit
    /// the invoice sold still stands on is missing by this much.
    pub uncollected: i64,
    /// Collections committed or sent, not settled yet.
    pub in_flight: i64,
    pub status: PaymentStatus,
}

impl State {
    /// Cash at hand to give back: collected and kept, less disputes and goodwill refunds.
    pub fn held(&self) -> i64 {
        (self.cash - self.disputed - self.goodwill).max(0)
    }

    /// What a charge may collect (R-127).
    pub fn charge(&self) -> Charge {
        if self.in_flight > 0 {
            Charge::InFlight
        } else if self.due == 0 {
            Charge::Covered
        } else {
            Charge::Due(self.due)
        }
    }
}

/// What a charge may collect (R-127).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charge {
    /// Nothing in flight: collect what is due.
    Due(i64),
    /// Nothing is due.
    Covered,
    /// A collection is committed or sent: it owns the invoice.
    InFlight,
}

/// How a credit note's amount is paid back (R-126). The three parts add up to its total (I17).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Split {
    pub credited: i64,
    pub refunded: i64,
    pub debt_cancelled: i64,
}

/// What a void gives back (R-128).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Void {
    /// The draws still on the invoice, back to their grants.
    pub credit: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A note amount must be positive.
    NotPositive,
    /// A collection in flight collects the whole amount due: no debt can be cancelled, no note
    /// voided, no invoice voided under it.
    InFlight,
    /// A debt cancellation above what is due.
    AboveDue {
        due: i64,
    },
    /// A refund above the credit and cash the invoice holds: the rest was never received.
    AboveHeld {
        credit: i64,
        cash: i64,
    },
    /// A credit to the balance above what the invoice owes and holds.
    AboveInvoice {
        left: i64,
    },
    /// A credit to the balance that would return money a collection in flight has not brought in.
    AwaitingPayment {
        left: i64,
    },
    /// The invoice is paid, or holds cash: it is given back by a credit note.
    Paid,
    /// A refund already drew on the note.
    Refunded,
    UnknownNote,
}

impl Account {
    /// The account folded (R-125).
    pub fn state(&self) -> State {
        let mut remaining = self.total;
        let mut credit = self.drawn.max(0);
        let (mut cash, mut disputed, mut failed, mut in_flight) = (0_i64, 0_i64, 0_i64, 0_i64);
        let mut accepted = false;
        let mut errored = false;
        for collection in &self.collections {
            if collection.status.collected() {
                cash += collection.amount - (collection.reversed - collection.disputed);
                disputed += collection.disputed;
                failed += collection.reversed - collection.disputed;
                errored = false;
            } else if collection.status.in_flight() {
                in_flight += collection.amount;
                accepted |= collection.accepted && collection.status == MoveStatus::Pending;
            } else if collection.status == MoveStatus::Failed && collection.declined {
                errored = true;
            }
        }
        let (mut excess_refunded, mut without_note) = (0_i64, 0_i64);
        for refund in self.refunds.iter().filter(|r| r.note.is_none()) {
            if refund.status.live() {
                excess_refunded += refund.excess;
                without_note += refund.amount;
            }
        }
        cash -= excess_refunded;
        for note in self.finalized() {
            remaining -= note.total;
            // The void returns every draw itself; its note converts only collected cash.
            let (from_draws, from_cash) = if note.voids_invoice {
                (credit, note.credited)
            } else {
                let from_draws = note.credited.min(credit);
                (from_draws, note.credited - from_draws)
            };
            credit -= from_draws;
            cash -= from_cash + note.refunded;
        }
        let goodwill = (without_note - excess_refunded).max(0);
        let owed_cash = (remaining - credit).max(0);
        let due = (owed_cash - cash).max(0);
        let excess = (cash - disputed - goodwill - owed_cash).max(0);
        let lost = failed + disputed + goodwill;
        let in_hand = cash - disputed - goodwill;
        let uncollected = lost.min(owed_cash - in_hand).max(0);
        let status = if due == 0 {
            PaymentStatus::Paid
        } else if accepted {
            PaymentStatus::Processing
        } else if errored {
            PaymentStatus::Errored
        } else if cash > 0 {
            PaymentStatus::PartiallyPaid
        } else {
            PaymentStatus::Unpaid
        };
        State {
            remaining,
            credit,
            cash,
            disputed,
            goodwill,
            failed,
            due,
            excess,
            uncollected,
            in_flight,
            status,
        }
    }

    fn finalized(&self) -> impl Iterator<Item = &Note> {
        self.notes
            .iter()
            .filter(|note| note.status == NoteStatus::Finalized)
    }

    /// What a charge may collect (R-127).
    pub fn charge(&self) -> Charge {
        self.state().charge()
    }

    /// How a note of `amount` (positive) is paid back, read against the account as it stands
    /// without the note `except` (a draft decided again at its finalization) (R-126).
    pub fn note(
        &self,
        kind: NoteKind,
        amount: i64,
        except: Option<Uuid>,
    ) -> Result<Split, Refusal> {
        if amount <= 0 {
            return Err(Refusal::NotPositive);
        }
        let account = self.without(except);
        let state = account.state();
        match kind {
            NoteKind::DebtCancellation => {
                if state.in_flight > 0 {
                    return Err(Refusal::InFlight);
                }
                if amount > state.due {
                    return Err(Refusal::AboveDue { due: state.due });
                }
                Ok(Split {
                    debt_cancelled: amount,
                    ..Split::default()
                })
            }
            NoteKind::CreditToBalance => {
                let reserved: i64 = account
                    .notes
                    .iter()
                    .filter(|note| note.status == NoteStatus::Draft)
                    .map(|note| note.debt_cancelled)
                    .sum();
                let debt = if state.in_flight > 0 {
                    0
                } else {
                    amount.min((state.due - reserved).max(0))
                };
                let rest = amount - debt;
                // Money not received yet is never returned: it may never arrive.
                let returnable = state.credit + state.held();
                if rest > returnable {
                    let left = debt + returnable;
                    return Err(if state.in_flight > 0 {
                        Refusal::AwaitingPayment { left }
                    } else {
                        Refusal::AboveInvoice { left }
                    });
                }
                Ok(Split {
                    credited: rest,
                    refunded: 0,
                    debt_cancelled: debt,
                })
            }
            NoteKind::Refund => {
                let credited = amount.min(state.credit);
                let refunded = amount - credited;
                if refunded > state.held() {
                    return Err(Refusal::AboveHeld {
                        credit: state.credit,
                        cash: state.held(),
                    });
                }
                Ok(Split {
                    credited,
                    refunded,
                    debt_cancelled: 0,
                })
            }
        }
    }

    /// Whether the finalized note `id` may be voided: never under a collection in flight, nor a
    /// Refund note a live refund drew on.
    pub fn void_note(&self, id: Uuid) -> Result<(), Refusal> {
        let note = self
            .notes
            .iter()
            .find(|note| note.id == id)
            .ok_or(Refusal::UnknownNote)?;
        if self
            .refunds
            .iter()
            .any(|refund| refund.note == Some(note.id) && refund.status != MoveStatus::Failed)
        {
            return Err(Refusal::Refunded);
        }
        if self.state().in_flight > 0 {
            return Err(Refusal::InFlight);
        }
        Ok(())
    }

    /// What voiding the invoice gives back (R-128): refused with a collection in flight (a 3-D
    /// Secure challenge included, its payment could still land) or collected cash still paying it,
    /// a dispute's loss included; an invoice paid by credit alone gives that credit back.
    pub fn void(&self) -> Result<Void, Refusal> {
        let state = self.state();
        if state.in_flight > 0 {
            return Err(Refusal::InFlight);
        }
        if state.cash > 0 {
            return Err(Refusal::Paid);
        }
        Ok(Void {
            credit: state.credit,
        })
    }

    /// Of a refund of `amount` without a note, just recorded and its excess not set yet, what gives
    /// back the excess (Q1): read without that refund, which would otherwise count as goodwill.
    pub fn refund_excess(&self, amount: i64) -> i64 {
        let mut before = self.clone();
        if let Some(recorded) = before.refunds.iter().position(|refund| {
            refund.note.is_none()
                && refund.excess == 0
                && refund.amount == amount
                && refund.status.live()
        }) {
            before.refunds.remove(recorded);
        }
        amount.min(before.state().excess).max(0)
    }

    fn without(&self, note: Option<Uuid>) -> std::borrow::Cow<'_, Account> {
        match note {
            Some(id) if self.notes.iter().any(|n| n.id == id) => {
                let mut account = self.clone();
                account.notes.retain(|n| n.id != id);
                std::borrow::Cow::Owned(account)
            }
            _ => std::borrow::Cow::Borrowed(self),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn paid(amount: i64) -> Collection {
        Collection {
            amount,
            reversed: 0,
            disputed: 0,
            status: MoveStatus::Settled,
            accepted: false,
            declined: true,
        }
    }

    fn note(kind: NoteKind, split: Split) -> Note {
        Note {
            id: Uuid::now_v7(),
            kind,
            status: NoteStatus::Finalized,
            total: split.credited + split.refunded + split.debt_cancelled,
            credited: split.credited,
            refunded: split.refunded,
            debt_cancelled: split.debt_cancelled,
            voids_invoice: false,
        }
    }

    fn with(mut account: Account, kind: NoteKind, amount: i64) -> Account {
        let split = account.note(kind, amount, None).unwrap();
        account.notes.push(note(kind, split));
        account
    }

    fn partly_paid() -> Account {
        Account {
            total: 10_000,
            drawn: 3_000,
            collections: vec![paid(2_000)],
            ..Account::default()
        }
    }

    /// Rules: R-125
    #[test]
    fn an_invoice_is_due_what_credit_and_cash_leave() {
        let state = partly_paid().state();
        assert_eq!(state.due, 5_000);
        assert_eq!(state.status, PaymentStatus::PartiallyPaid);
        let credit_only = Account {
            collections: vec![],
            ..partly_paid()
        };
        assert_eq!(
            credit_only.state().status,
            PaymentStatus::Unpaid,
            "credit alone pays nothing in cash"
        );
    }

    /// Rules: R-126
    #[test]
    fn a_refund_returns_the_credit_first_and_keeps_what_is_due() {
        let account = partly_paid();
        assert_eq!(
            account.note(NoteKind::Refund, 2_000, None),
            Ok(Split {
                credited: 2_000,
                refunded: 0,
                debt_cancelled: 0
            })
        );
        let account = with(account, NoteKind::Refund, 2_000);
        assert_eq!(account.state().due, 5_000);
        assert_eq!(
            account.note(NoteKind::Refund, 3_000, None),
            Ok(Split {
                credited: 1_000,
                refunded: 2_000,
                debt_cancelled: 0
            }),
            "the next note crosses from credit to cash"
        );
        assert_eq!(
            account.note(NoteKind::Refund, 3_001, None),
            Err(Refusal::AboveHeld {
                credit: 1_000,
                cash: 2_000
            }),
            "never more cash than was paid"
        );
        let account = with(account, NoteKind::Refund, 3_000);
        assert_eq!(account.state().due, 5_000);
        assert_eq!(account.state().excess, 0);
    }

    /// Rules: R-126
    #[test]
    fn a_debt_cancellation_is_refused_above_due_or_in_flight() {
        let account = partly_paid();
        assert_eq!(
            account.note(NoteKind::DebtCancellation, 5_001, None),
            Err(Refusal::AboveDue { due: 5_000 })
        );
        let cancelled = with(account.clone(), NoteKind::DebtCancellation, 5_000);
        assert_eq!(cancelled.state().status, PaymentStatus::Paid);
        let mut in_flight = account;
        in_flight.collections.push(Collection {
            status: MoveStatus::Pending,
            accepted: true,
            ..paid(5_000)
        });
        assert_eq!(in_flight.state().status, PaymentStatus::Processing);
        assert_eq!(
            in_flight.note(NoteKind::DebtCancellation, 1_000, None),
            Err(Refusal::InFlight)
        );
        assert_eq!(in_flight.charge(), Charge::InFlight);
    }

    /// Rules: R-126
    #[test]
    fn a_credit_to_balance_cancels_debt_then_returns_credit_then_cash() {
        let account = partly_paid();
        assert_eq!(
            account.note(NoteKind::CreditToBalance, 9_000, None),
            Ok(Split {
                credited: 4_000,
                refunded: 0,
                debt_cancelled: 5_000
            })
        );
        let account = with(account, NoteKind::CreditToBalance, 9_000);
        let state = account.state();
        assert_eq!((state.due, state.credit, state.cash), (0, 0, 1_000));
        assert_eq!(
            account.note(NoteKind::CreditToBalance, 1_001, None),
            Err(Refusal::AboveInvoice { left: 1_000 })
        );
    }

    /// Rules: R-126
    #[test]
    fn a_credit_to_balance_never_returns_money_still_in_flight() {
        let mut account = Account {
            total: 10_000,
            ..Account::default()
        };
        account.collections.push(Collection {
            status: MoveStatus::Pending,
            accepted: true,
            ..paid(10_000)
        });
        assert_eq!(
            account.note(NoteKind::CreditToBalance, 10_000, None),
            Err(Refusal::AwaitingPayment { left: 0 })
        );
        account.drawn = 3_000;
        assert_eq!(
            account.note(NoteKind::CreditToBalance, 3_000, None),
            Ok(Split {
                credited: 3_000,
                refunded: 0,
                debt_cancelled: 0
            }),
            "the credit it drew is the invoice's to return"
        );
    }

    /// Rules: R-127
    #[test]
    fn a_charge_collects_what_is_due_never_the_total() {
        // Half paid by credit, half by a settled payment: nothing left to charge.
        let account = Account {
            total: 100,
            drawn: 50,
            collections: vec![paid(50)],
            ..Account::default()
        };
        assert_eq!(account.charge(), Charge::Covered);
        let mut failed = Account {
            collections: vec![],
            ..account
        };
        assert_eq!(failed.charge(), Charge::Due(50));
        failed.collections.push(Collection {
            status: MoveStatus::Failed,
            ..paid(50)
        });
        assert_eq!(failed.state().status, PaymentStatus::Errored);
        let mut abandoned = failed.clone();
        abandoned.collections[0].declined = false;
        assert_eq!(
            abandoned.state().status,
            PaymentStatus::Unpaid,
            "a hosted attempt the customer left is no decline"
        );
    }

    /// Rules: R-128
    #[test]
    fn a_void_returns_the_draws_still_on_the_invoice() {
        let account = Account {
            total: 10_000,
            drawn: 3_000,
            ..Account::default()
        };
        let account = with(
            Account {
                collections: vec![paid(500)],
                ..account
            },
            NoteKind::Refund,
            3_500,
        );
        assert_eq!(account.state().held(), 0);
        assert_eq!(account.void(), Ok(Void { credit: 0 }));
        assert_eq!(partly_paid().void(), Err(Refusal::Paid));
        let unpaid = Account {
            total: 10_000,
            drawn: 3_000,
            ..Account::default()
        };
        assert_eq!(unpaid.void(), Ok(Void { credit: 3_000 }));
        let paid_by_credit = Account {
            total: 10_000,
            drawn: 10_000,
            ..Account::default()
        };
        assert_eq!(paid_by_credit.state().status, PaymentStatus::Paid);
        assert_eq!(paid_by_credit.void(), Ok(Void { credit: 10_000 }));
        let mut challenged = unpaid.clone();
        challenged.collections.push(Collection {
            status: MoveStatus::Pending,
            ..paid(7_000)
        });
        assert_eq!(challenged.void(), Err(Refusal::InFlight));
        assert_eq!(challenged.charge(), Charge::InFlight);
    }

    #[test]
    fn a_dispute_keeps_the_invoice_paid_a_failure_reopens_it() {
        let disputed = Account {
            total: 100,
            collections: vec![Collection {
                reversed: 100,
                disputed: 100,
                ..paid(100)
            }],
            ..Account::default()
        };
        let state = disputed.state();
        assert_eq!((state.due, state.uncollected, state.held()), (0, 100, 0));
        let failed = Account {
            collections: vec![Collection {
                reversed: 100,
                disputed: 0,
                status: MoveStatus::Refunded,
                ..paid(100)
            }],
            ..disputed
        };
        assert_eq!(failed.state().due, 100);
    }

    #[test]
    fn a_lost_dispute_keeps_the_invoice_paid_and_unvoidable() {
        let disputed = Account {
            total: 100,
            collections: vec![Collection {
                reversed: 100,
                disputed: 100,
                ..paid(100)
            }],
            ..Account::default()
        };
        assert_eq!(disputed.void(), Err(Refusal::Paid));
    }

    #[test]
    fn a_goodwill_refund_is_never_overpaid_again() {
        let account = Account {
            total: 100,
            collections: vec![paid(100), paid(30)],
            refunds: vec![Refund {
                amount: 30,
                status: MoveStatus::Settled,
                note: None,
                excess: 0,
            }],
            ..Account::default()
        };
        assert_eq!(account.state().excess, 0);
    }

    /// Rules: Q1
    #[test]
    fn a_refund_just_recorded_gives_back_the_excess_it_found() {
        let mut account = Account {
            total: 25_000,
            collections: vec![paid(30_000)],
            ..Account::default()
        };
        account.refunds.push(Refund {
            amount: 5_000,
            status: MoveStatus::Settled,
            note: None,
            excess: 0,
        });
        assert_eq!(account.refund_excess(5_000), 5_000);
        account.refunds[0].excess = 5_000;
        assert_eq!(account.state().excess, 0);
        assert_eq!(account.state().due, 0);
    }

    #[test]
    fn a_promised_refund_is_never_an_overpayment() {
        let account = with(
            Account {
                total: 100,
                collections: vec![paid(120)],
                ..Account::default()
            },
            NoteKind::Refund,
            30,
        );
        let state = account.state();
        assert_eq!((state.due, state.excess), (0, 20));
    }

    fn arbitrary_account() -> impl Strategy<Value = Account> {
        (
            0_i64..20_000,
            0_i64..10_000,
            prop::collection::vec((0_i64..10_000, 0_i64..3, any::<bool>()), 0..4),
            prop::collection::vec((0_i64..3, 1_i64..10_000), 0..4),
        )
            .prop_map(|(total, drawn, collections, notes)| {
                let mut account = Account {
                    total,
                    drawn: drawn.min(total),
                    collections: collections
                        .into_iter()
                        .map(|(amount, kind, flag)| Collection {
                            amount,
                            reversed: if kind == 1 { amount / 2 } else { 0 },
                            disputed: if kind == 1 && flag { amount / 2 } else { 0 },
                            status: if kind == 2 {
                                MoveStatus::Failed
                            } else {
                                MoveStatus::Settled
                            },
                            accepted: false,
                            declined: true,
                        })
                        .collect(),
                    ..Account::default()
                };
                for (kind, amount) in notes {
                    let kind = [
                        NoteKind::CreditToBalance,
                        NoteKind::Refund,
                        NoteKind::DebtCancellation,
                    ][kind as usize];
                    if let Ok(split) = account.note(kind, amount, None) {
                        account.notes.push(note(kind, split));
                    }
                }
                account
            })
    }

    proptest! {
        #[test]
        fn the_account_never_goes_negative_and_notes_add_up(account in arbitrary_account()) {
            let state = account.state();
            prop_assert!(state.due >= 0 && state.excess >= 0 && state.credit >= 0);
            prop_assert!(state.due <= state.remaining.max(0));
            prop_assert!(state.credit <= account.drawn);
            for note in &account.notes {
                prop_assert_eq!(note.total, note.credited + note.refunded + note.debt_cancelled);
            }
        }

        #[test]
        fn a_refund_never_gives_back_more_cash_than_was_held(account in arbitrary_account(), amount in 1_i64..20_000) {
            let before = account.state();
            if let Ok(split) = account.note(NoteKind::Refund, amount, None) {
                prop_assert!(split.refunded <= before.held());
                prop_assert!(split.credited <= before.credit);
                let mut after = account.clone();
                after.notes.push(note(NoteKind::Refund, split));
                prop_assert_eq!(after.state().due, before.due);
            }
        }
    }
}
