//! What a subscription's facts say its documents owe: each charge the facts put on a
//! written document billed exactly once across them, notes included.

use std::collections::HashSet;

use billing_core::charge::ChargeKind;
use billing_core::evaluate::{DocumentSlot, evaluate};
pub use billing_core::reconcile::Adjustment;
use billing_core::reconcile::{TakeBack, reconcile};
use chrono::NaiveTime;
use common_domain::ids::{SubscriptionId, TenantId};
use meteroid_store::domain::Invoice;
use meteroid_store::domain::enums::{InvoiceStatusEnum, InvoiceType};
use meteroid_store::services::evaluate::issued::load_billed;
use meteroid_store::services::evaluate::load::load_facts;
use meteroid_store::services::evaluate::pricer::StorePricer;
use meteroid_store::store::PgConn;

/// The slot a written invoice is.
fn slot_of(invoice: &Invoice) -> Option<DocumentSlot> {
    match invoice.invoice_type {
        InvoiceType::Recurring => Some(DocumentSlot::Recurring {
            period_start: invoice.billing_period_start,
        }),
        InvoiceType::Final => Some(DocumentSlot::Final),
        InvoiceType::Adjustment => Some(DocumentSlot::Adjustment {
            on: invoice.invoice_date,
        }),
        _ => None,
    }
}

/// Every fixed charge the facts put on one of `invoices` (the subscription's, as written), or on
/// any document unless `written_only`, due by the latest period they settle: what reconcile still owes for
/// each against all of them, after their notes. Empty when each is billed exactly once.
pub async fn owed_on_written_documents(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    invoices: &[Invoice],
    written_only: bool,
) -> Vec<Adjustment> {
    let written: Vec<&Invoice> = invoices
        .iter()
        .filter(|invoice| invoice.status != InvoiceStatusEnum::Void)
        .collect();
    let slots: HashSet<DocumentSlot> = written.iter().filter_map(|i| slot_of(i)).collect();
    let until = written
        .iter()
        .map(|invoice| invoice.billing_period_start)
        .max()
        .expect("a written invoice");
    let loaded = load_facts(conn, tenant_id, subscription_id, until, None)
        .await
        .expect("facts");
    let now = until.and_time(NaiveTime::MIN).and_utc();
    let evaluation = evaluate(&loaded.facts, now, None, &StorePricer).expect("evaluation");
    let mut expected = evaluation.expected();
    expected.retain(|charge| {
        matches!(charge.key.kind, ChargeKind::Fee | ChargeKind::Credit)
            && evaluation
                .charges
                .iter()
                .any(|c| c.key == charge.key && (!written_only || slots.contains(&c.document)))
    });
    let from = expected
        .iter()
        .map(|charge| charge.key.from)
        .min()
        .unwrap_or(until);
    let billed = load_billed(conn, tenant_id, subscription_id, from, None, None)
        .await
        .expect("billed");
    let lines: Vec<_> = billed.lines.into_iter().map(|on| on.line).collect();
    reconcile(&expected, &lines, TakeBack::LatestFirst)
}
