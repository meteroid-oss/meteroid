//! OpenTelemetry instruments of the credit money paths. Labels are bounded enums (kind, reason,
//! outcome, currency): never tenant, customer or wallet ids. Counters recorded inside a
//! transaction can over-count by a rolled-back attempt.
use common_logging::{GLOBAL_METER, KeyValue};
use opentelemetry::metrics::{Counter, Histogram};
use std::sync::LazyLock;

const RATIO: &[f64] = &[0.0, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0];

fn counter(name: &'static str, description: &'static str) -> Counter<u64> {
    GLOBAL_METER
        .u64_counter(name)
        .with_description(description)
        .build()
}

fn histogram(
    name: &'static str,
    description: &'static str,
    unit: &'static str,
    boundaries: &[f64],
) -> Histogram<f64> {
    GLOBAL_METER
        .f64_histogram(name)
        .with_description(description)
        .with_unit(unit)
        .with_boundaries(boundaries.to_vec())
        .build()
}

static EXPIRY_SETTLEMENTS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    counter(
        "credit_expiry_settlements",
        "Usage settlements paid from credit expiring mid-period, by source",
    )
});

static EXPIRY_SETTLED_MINOR: LazyLock<Counter<u64>> = LazyLock::new(|| {
    counter(
        "credit_expiry_settled_minor",
        "Usage paid from credit expiring mid-period, in minor units, by currency",
    )
});

static ROLLOVERS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    counter(
        "credit_rollovers",
        "Expired grants whose unspent credit rolled over into a new grant",
    )
});

static ROLLED_MINOR: LazyLock<Counter<u64>> = LazyLock::new(|| {
    counter(
        "credit_rolled_minor",
        "Unspent credit rolled over at expiry, in minor units, by denomination",
    )
});

static EXPIRY_LAPSES: LazyLock<Counter<u64>> = LazyLock::new(|| {
    counter(
        "credit_expiry_lapses",
        "Expired grants lapsed, settled or without their usage settled",
    )
});

static SUBSCRIPTION_END_REPAIRS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    counter(
        "credit_subscription_end_repaired",
        "Ended subscriptions whose credit the sweep ended because their termination did not",
    )
});

static SETTLEMENT_RETURNS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    counter(
        "credit_settlement_returns",
        "Usage settlements given back because no invoice can bill their period",
    )
});

static SETTLEMENT_RETURNED_MINOR: LazyLock<Counter<u64>> = LazyLock::new(|| {
    counter(
        "credit_settlement_returned_minor",
        "Credit given back from orphaned usage settlements, in minor units, by currency",
    )
});

static RECONCILIATIONS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    counter(
        "credit_reconciliations",
        "Credit documents the reversal reconciler brought in line, by kind",
    )
});

static ESTIMATE_DRIFT: LazyLock<Histogram<f64>> = LazyLock::new(|| {
    histogram(
        "credit_live_estimate_drift_ratio",
        "Relative gap between the usage the live estimate carried and the finalized invoice",
        "1",
        RATIO,
    )
});

static REVERSAL_EXCESS_MINOR: LazyLock<Counter<u64>> = LazyLock::new(|| {
    counter(
        "payment_reversal_excess_minor",
        "Minor units a provider withdrew above what the payment still held, never applied to the invoice or credit",
    )
});

fn label(key: &'static str, value: &'static str) -> KeyValue {
    KeyValue::new(key, value)
}

fn minor(amount: i64) -> u64 {
    u64::try_from(amount).unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementSource {
    Sweep,
    Invoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LapseKind {
    Settled,
    Unsettled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciliationKind {
    ReversedPurchase,
    RefundNote,
    DisputedInvoice,
    KeptPayment,
}

pub fn expiry_settled(source: SettlementSource, currency: &str, settlements: usize, amount: i64) {
    if settlements == 0 {
        return;
    }
    let source = match source {
        SettlementSource::Sweep => "sweep",
        SettlementSource::Invoice => "invoice",
    };
    EXPIRY_SETTLEMENTS.add(settlements as u64, &[label("source", source)]);
    EXPIRY_SETTLED_MINOR.add(
        minor(amount),
        &[KeyValue::new("currency", currency.to_owned())],
    );
}

pub fn expiry_lapsed(kind: LapseKind, grants: usize) {
    if grants == 0 {
        return;
    }
    let kind = match kind {
        LapseKind::Settled => "settled",
        LapseKind::Unsettled => "unsettled",
    };
    EXPIRY_LAPSES.add(grants as u64, &[label("kind", kind)]);
}

pub fn subscription_end_repaired(subscriptions: usize) {
    if subscriptions > 0 {
        SUBSCRIPTION_END_REPAIRS.add(subscriptions as u64, &[]);
    }
}

pub fn credit_rolled_over(currency: &str, grants: usize, amount: i64) {
    if grants == 0 {
        return;
    }
    ROLLOVERS.add(grants as u64, &[]);
    ROLLED_MINOR.add(
        minor(amount),
        &[KeyValue::new("currency", currency.to_owned())],
    );
}

pub fn settlements_returned(currency: &str, settlements: usize, amount: i64) {
    if settlements == 0 {
        return;
    }
    SETTLEMENT_RETURNS.add(settlements as u64, &[]);
    SETTLEMENT_RETURNED_MINOR.add(
        minor(amount),
        &[KeyValue::new("currency", currency.to_owned())],
    );
}

pub fn reconciled(kind: ReconciliationKind) {
    let kind = match kind {
        ReconciliationKind::ReversedPurchase => "reversed_purchase",
        ReconciliationKind::RefundNote => "refund_note",
        ReconciliationKind::DisputedInvoice => "disputed_invoice",
        ReconciliationKind::KeptPayment => "kept_payment",
    };
    RECONCILIATIONS.add(1, &[label("kind", kind)]);
}

/// Nothing is recorded when the invoice bills no usage to compare against.
pub fn estimate_drift(estimated_minor: i64, invoiced_minor: i64) {
    if invoiced_minor <= 0 {
        return;
    }
    let gap = (i128::from(estimated_minor) - i128::from(invoiced_minor)).unsigned_abs();
    ESTIMATE_DRIFT.record(gap as f64 / invoiced_minor as f64, &[]);
}

pub fn reversal_excess(currency: &str, disputed: bool, amount: i64) {
    REVERSAL_EXCESS_MINOR.add(
        minor(amount),
        &[
            KeyValue::new("currency", currency.to_owned()),
            label("nature", if disputed { "dispute" } else { "failure" }),
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruments_register_and_record() {
        expiry_settled(SettlementSource::Sweep, "EUR", 1, 500);
        expiry_settled(SettlementSource::Invoice, "EUR", 1, -5);
        expiry_lapsed(LapseKind::Settled, 1);
        expiry_lapsed(LapseKind::Unsettled, 1);
        settlements_returned("EUR", 1, 250);
        reconciled(ReconciliationKind::ReversedPurchase);
        reconciled(ReconciliationKind::RefundNote);
        estimate_drift(105, 100);
        estimate_drift(0, 0);
        estimate_drift(i64::MAX, 1);
    }
}
