//! The words of a credit ledger movement, shared by the dashboard and the customer portal.

use crate::domain::credit::{
    CreditGrantSource, CreditLedgerEntry, ENDED_WITH_SUBSCRIPTION, GrantClassification,
    LedgerEntryKind, is_removal_reason,
};
use std::borrow::Cow;

pub const PAYMENT_REVERSED: &str = "Payment reversed by the bank";
pub const PAYMENT_DISPUTE_LOST: &str = "Payment lost to a dispute";
pub const PAYMENT_REFUNDED: &str = "Payment refunded";
pub const PAYMENT_RECOVERED: &str = "Reversed payment recovered";

/// A ledger entry with what its grant was, when the grant is known.
#[derive(Debug, Clone, Copy)]
pub struct LedgerMovement<'a> {
    pub entry: &'a CreditLedgerEntry,
    pub source: Option<CreditGrantSource>,
    pub classification: Option<GrantClassification>,
    /// The bonus half of paid credit sold below its face (`CreditGrant::is_bonus`).
    pub bonus: bool,
    pub automatic_top_up: bool,
}

impl LedgerMovement<'_> {
    fn promotional(&self) -> bool {
        self.classification == Some(GrantClassification::Promotional)
    }

    pub fn label(&self) -> Cow<'static, str> {
        let entry = self.entry;
        match entry.kind {
            LedgerEntryKind::Fund => Cow::Borrowed(self.fund_label()),
            LedgerEntryKind::Draw if entry.invoice_id.is_some() => {
                Cow::Borrowed("Applied to invoice")
            }
            LedgerEntryKind::Draw => Cow::Borrowed("Applied to usage"),
            LedgerEntryKind::DrawReversal if entry.credit_note_id.is_some() => {
                Cow::Borrowed("Credit note")
            }
            LedgerEntryKind::DrawReversal if entry.invoice_id.is_some() => {
                Cow::Borrowed("Returned from a voided invoice")
            }
            LedgerEntryKind::DrawReversal => Cow::Borrowed("Credit returned"),
            LedgerEntryKind::Reversal | LedgerEntryKind::Expiry
                if is_removal_reason(&entry.reason) =>
            {
                Cow::Owned(entry.reason.clone())
            }
            LedgerEntryKind::Reversal if entry.credit_note_id.is_some() => {
                Cow::Borrowed("Reversed by a credit note")
            }
            LedgerEntryKind::Reversal => Cow::Borrowed("Credit removed"),
            LedgerEntryKind::Reinstatement => Cow::Borrowed("Credit restored"),
            LedgerEntryKind::Expiry if entry.reason == ENDED_WITH_SUBSCRIPTION => {
                Cow::Borrowed(ENDED_WITH_SUBSCRIPTION)
            }
            LedgerEntryKind::Expiry => Cow::Borrowed("Credit expired"),
            LedgerEntryKind::Rollover => Cow::Borrowed("Rolled over"),
        }
    }

    fn fund_label(&self) -> &'static str {
        match self.source {
            _ if self.bonus => "Free credit",
            Some(CreditGrantSource::Purchase) if self.automatic_top_up => "Automatic top-up",
            // Credit sold for nothing is promotional too.
            Some(CreditGrantSource::Purchase) if self.promotional() => "Free credit",
            Some(CreditGrantSource::Purchase) => "Purchased credit",
            Some(CreditGrantSource::Included) => "Included with plan",
            Some(CreditGrantSource::CreditNote) => "Credit note",
            Some(CreditGrantSource::NegativeInvoice) => "Credit from an invoice",
            Some(CreditGrantSource::InvoiceVoid) => "Returned from a voided invoice",
            Some(CreditGrantSource::UsageSettlementReturn) => "Credit returned",
            Some(CreditGrantSource::CustomerBalance) => "Previous balance",
            Some(CreditGrantSource::Rollover) => "Rolled over from last period",
            Some(CreditGrantSource::Overpayment) => "Overpayment",
            Some(CreditGrantSource::Coupon) => "Free credit",
            Some(CreditGrantSource::Manual) if self.promotional() => "Free credit",
            Some(CreditGrantSource::Manual) | None => "Credit added",
        }
    }

    /// The reason a customer may read: one the system wrote, never an operator's own words.
    pub fn customer_reason(&self) -> Option<&'static str> {
        let reason = self.entry.reason.as_str();
        match self.entry.kind {
            LedgerEntryKind::Reversal | LedgerEntryKind::Reinstatement => [
                PAYMENT_REVERSED,
                PAYMENT_DISPUTE_LOST,
                PAYMENT_REFUNDED,
                PAYMENT_RECOVERED,
            ]
            .into_iter()
            .find(|known| *known == reason),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::credit::{CANCELLATION_REMOVAL, CreditApplication};
    use chrono::Utc;
    use common_domain::ids::{
        BaseId, CreditGrantId, CreditNoteId, InvoiceId, LedgerEntryId, WalletId,
    };

    fn entry(kind: LedgerEntryKind, reason: &str) -> CreditLedgerEntry {
        CreditLedgerEntry {
            id: LedgerEntryId::new(),
            wallet_id: WalletId::new(),
            grant_id: CreditGrantId::new(),
            sequence: 1,
            kind,
            amount_minor: 100,
            balance_after_minor: 100,
            invoice_id: None,
            credit_note_id: None,
            reason: reason.to_owned(),
            reference: None,
            created_by: None,
            effective_at: Utc::now(),
            recorded_at: Utc::now(),
            cost: None,
            application: CreditApplication::Invoice,
            subscription_id: None,
        }
    }

    fn label(
        entry: &CreditLedgerEntry,
        source: CreditGrantSource,
        classification: GrantClassification,
    ) -> String {
        LedgerMovement {
            entry,
            source: Some(source),
            classification: Some(classification),
            bonus: false,
            automatic_top_up: false,
        }
        .label()
        .into_owned()
    }

    #[test]
    fn bonus_and_promotional_credit_read_apart_from_paid_credit() {
        let fund = entry(LedgerEntryKind::Fund, "");
        use CreditGrantSource::*;
        use GrantClassification::*;
        assert_eq!(label(&fund, Purchase, Promotional), "Free credit");
        assert_eq!(label(&fund, Purchase, Paid), "Purchased credit");
        assert_eq!(label(&fund, Manual, Promotional), "Free credit");
        assert_eq!(label(&fund, Manual, Paid), "Credit added");
        assert_eq!(label(&fund, Coupon, Promotional), "Free credit");
        assert_eq!(label(&fund, Included, Promotional), "Included with plan");
        let top_up = LedgerMovement {
            entry: &fund,
            source: Some(Purchase),
            classification: Some(Paid),
            bonus: false,
            automatic_top_up: true,
        };
        assert_eq!(top_up.label(), "Automatic top-up");
        let bonus = LedgerMovement {
            source: Some(Manual),
            classification: Some(Promotional),
            bonus: true,
            ..top_up
        };
        assert_eq!(bonus.label(), "Free credit");
    }

    #[test]
    fn returns_and_removals_name_what_moved_the_credit() {
        use CreditGrantSource::Purchase;
        use GrantClassification::Paid;
        let mut returned = entry(LedgerEntryKind::DrawReversal, "");
        assert_eq!(label(&returned, Purchase, Paid), "Credit returned");
        returned.invoice_id = Some(InvoiceId::new());
        assert_eq!(
            label(&returned, Purchase, Paid),
            "Returned from a voided invoice"
        );
        returned.credit_note_id = Some(CreditNoteId::new());
        assert_eq!(label(&returned, Purchase, Paid), "Credit note");

        let mut reversed = entry(LedgerEntryKind::Reversal, "");
        reversed.credit_note_id = Some(CreditNoteId::new());
        assert_eq!(
            label(&reversed, Purchase, Paid),
            "Reversed by a credit note"
        );
        let removed = entry(LedgerEntryKind::Reversal, CANCELLATION_REMOVAL);
        assert_eq!(label(&removed, Purchase, Paid), CANCELLATION_REMOVAL);
        let expired = entry(LedgerEntryKind::Expiry, ENDED_WITH_SUBSCRIPTION);
        assert_eq!(label(&expired, Purchase, Paid), ENDED_WITH_SUBSCRIPTION);
    }

    #[test]
    fn customers_read_system_reasons_only() {
        let reason = |kind, reason| {
            LedgerMovement {
                entry: &entry(kind, reason),
                source: None,
                classification: None,
                bonus: false,
                automatic_top_up: false,
            }
            .customer_reason()
        };
        assert_eq!(
            reason(LedgerEntryKind::Reversal, PAYMENT_DISPUTE_LOST),
            Some(PAYMENT_DISPUTE_LOST)
        );
        assert_eq!(
            reason(LedgerEntryKind::Reversal, PAYMENT_REFUNDED),
            Some(PAYMENT_REFUNDED)
        );
        assert_eq!(
            reason(LedgerEntryKind::Reinstatement, PAYMENT_RECOVERED),
            Some(PAYMENT_RECOVERED)
        );
        assert_eq!(
            reason(LedgerEntryKind::Reversal, "Goodwill clawback, ticket 42"),
            None
        );
    }
}
