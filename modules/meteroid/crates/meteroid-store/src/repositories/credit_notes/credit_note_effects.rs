//! How a credit note's amount splits between cancelled debt and returned value. Forgiven debt is
//! never funding.
use crate::{StoreResult, domain::Invoice, errors::StoreError};
use diesel_models::{credit_notes::CreditNoteRow, enums::CreditNoteStatus};

pub(crate) fn total(value: i64) -> StoreResult<i64> {
    value.checked_abs().ok_or_else(|| {
        StoreError::InvalidArgument("credit-note total exceeds the currency range".into()).into()
    })
}

pub(crate) fn sum(mut values: impl Iterator<Item = i64>) -> StoreResult<i64> {
    values
        .try_fold(0_i64, |sum, value| sum.checked_add(value))
        .ok_or_else(|| {
            StoreError::InvalidArgument("credit-note effects exceed the currency range".into())
                .into()
        })
}

/// `(credited, debt_cancelled)` of a credit-to-balance note. Debt is cancelled first, then value
/// returns, capped by what the invoice actually collected. With a payment in flight the charge
/// collects the full total, so cancelling debt would be clamped away: all of it is returned.
pub(crate) fn balance_effects_in_flight(
    invoice: &Invoice,
    amount: i64,
    prior: &[CreditNoteRow],
    externally_restored: bool,
    payment_in_flight: bool,
) -> StoreResult<(i64, i64)> {
    let finalized_debt = sum(prior
        .iter()
        .filter(|note| note.status == CreditNoteStatus::Finalized)
        .map(|note| note.debt_cancelled_amount_cents))?;
    let reserved_debt = sum(prior
        .iter()
        .filter(|note| note.status == CreditNoteStatus::Draft)
        .map(|note| note.debt_cancelled_amount_cents))?;
    let converted = sum(prior
        .iter()
        .filter(|note| note.status != CreditNoteStatus::Voided)
        .flat_map(|note| [note.credited_amount_cents, note.refunded_amount_cents]))?;
    let received = invoice
        .total
        .saturating_sub(invoice.amount_due)
        .saturating_sub(finalized_debt)
        .max(0);
    let in_flight = payment_in_flight
        || invoice.payment_status == crate::domain::enums::InvoicePaymentStatus::Processing;
    let received = if externally_restored {
        received.saturating_sub(invoice.applied_credits).max(0)
    } else if in_flight {
        invoice.total.saturating_sub(finalized_debt).max(0)
    } else {
        received
    };
    let debt = if externally_restored || in_flight {
        0
    } else {
        amount.min(invoice.amount_due.saturating_sub(reserved_debt).max(0))
    };
    let credited = amount
        .saturating_sub(debt)
        .min(received.saturating_sub(converted).max(0));
    Ok((credited, debt))
}

/// Share of `original` owed back once `prior_invoice + requested` of `invoice_applied` is
/// returned, minus what earlier returns already took: cumulative rounding never over-returns.
pub(crate) fn cumulative_return(
    original: i64,
    invoice_applied: i64,
    prior_invoice: i64,
    prior_original: i64,
    requested: i64,
) -> StoreResult<i64> {
    let refused = |message: &str| {
        error_stack::Report::new(StoreError::FailedPrecondition(message.to_owned()))
    };
    if original <= 0
        || invoice_applied <= 0
        || prior_invoice < 0
        || prior_original < 0
        || requested <= 0
    {
        return Err(refused("the invoice refund allocation is invalid"));
    }
    let cumulative = prior_invoice
        .checked_add(requested)
        .filter(|total| *total <= invoice_applied)
        .ok_or_else(|| refused("the invoice refunds exceed the invoiced amount"))?;
    let numerator = i128::from(original) * i128::from(cumulative);
    let rounded = (numerator + i128::from(invoice_applied) / 2) / i128::from(invoice_applied);
    i64::try_from(rounded)
        .ok()
        .and_then(|value| value.checked_sub(prior_original))
        .filter(|value| *value >= 0)
        .ok_or_else(|| refused("earlier invoice refunds exceed their original allocation"))
}

#[cfg(test)]
mod tests {
    use super::cumulative_return;

    #[test]
    fn partial_refunds_carry_rounding_and_never_exceed_the_original_amount() {
        assert_eq!(cumulative_return(1, 3, 0, 0, 1).unwrap(), 0);
        assert_eq!(cumulative_return(1, 3, 1, 0, 1).unwrap(), 1);
        assert_eq!(cumulative_return(1, 3, 2, 1, 1).unwrap(), 0);
        assert_eq!(cumulative_return(1001, 3000, 0, 0, 1000).unwrap(), 334);
        assert_eq!(cumulative_return(1001, 3000, 1000, 334, 1000).unwrap(), 333);
        assert_eq!(cumulative_return(1001, 3000, 2000, 667, 1000).unwrap(), 334);
        assert!(cumulative_return(1001, 3000, 3000, 1001, 1).is_err());
    }
}
