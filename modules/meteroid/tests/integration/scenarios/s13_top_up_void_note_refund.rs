//! S13. A portal top-up on Pay as you go, partly used by September's usage and a one-off invoice:
//! the void gives its credit back, a credit note on the usage line gives back what credit paid,
//! and a refund of the purchase takes back the credit left, reporting what was already spent.
use chrono::{DateTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{CustomerId, InvoiceId, PaymentTransactionId, WalletId};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use meteroid_store::domain::credit::{CreditCost, GrantClassification};
use meteroid_store::domain::enums::{InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::domain::wallet_events::WalletCreditUnrecoveredEvent;
use meteroid_store::domain::{Invoice, InvoiceNew, LineItem, PaymentStatusEnum};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::{CreditNoteInterface, InvoiceInterface};
use meteroid_store::services::{
    InvoiceDelivery, IssueAt, OfflineRefundCommand, SellCustomerCredit,
};

use super::kit::{Halyard, LITE, M, Tax, at, at_time, day, grant, lines, money};
use crate::helpers::invoice_lines::line;

/// The customer's USD wallet.
pub(super) async fn wallet(h: &Halyard, customer: CustomerId) -> WalletId {
    h.store()
        .create_customer_credit_wallet(h.tenant, customer, "USD")
        .await
        .expect("USD wallet")
        .id
}

/// Credit bought on a purchase invoice issued at `at` and paid then: PAID, customer-wide, no expiry.
pub(super) async fn top_up(
    h: &Halyard,
    customer: CustomerId,
    amount: i64,
    at: DateTime<Utc>,
) -> Invoice {
    let wallet_id = wallet(h, customer).await;
    let sold = h
        .services()
        .sell_customer_credit_at(
            uuid::Uuid::now_v7(),
            SellCustomerCredit {
                expires_at: None,
                tenant_id: h.tenant,
                customer_id: customer,
                wallet_id,
                amount_minor: amount,
                expires_in_months: None,
                note: None,
                actor: Actor::System,
                unit_price: None,
                expected_unit_price: None,
            },
            at,
            InvoiceDelivery::Suppressed,
        )
        .await
        .expect("credit purchase");
    let purchase = sold.invoice.invoice;
    assert_eq!(
        money(&purchase),
        (amount, 0, 0, 0, amount, 0, amount),
        "a currency purchase is untaxed"
    );
    h.pay(purchase.id, at).await
}

/// Credit added by hand at `at`: customer-wide, any charge; `cost` below face splits a bonus off.
pub(super) async fn add_credit(
    h: &Halyard,
    customer: CustomerId,
    amount: i64,
    classification: GrantClassification,
    cost: Option<i64>,
    at: DateTime<Utc>,
) {
    let wallet_id = wallet(h, customer).await;
    h.store()
        .fund_customer_credit_wallet_at(
            FundCustomerCreditWallet {
                tenant_id: h.tenant,
                customer_id: customer,
                wallet_id,
                amount_minor: amount,
                classification,
                priority: DEFAULT_PRIORITY,
                expires_at: None,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "Scenario credit".to_owned(),
                reference: None,
                actor_id: None,
                cost: cost.map(|cost| CreditCost::at_face(cost, "USD")),
            },
            at,
        )
        .await
        .expect("credit added");
}

/// A one-off invoice of hand-written `lines`, issued at `at`.
pub(super) async fn one_off(
    h: &Halyard,
    customer: CustomerId,
    lines: Vec<LineItem>,
    at: DateTime<Utc>,
) -> Invoice {
    let details = h
        .store()
        .find_customer_by_id(customer, h.tenant)
        .await
        .expect("customer");
    let entity = h
        .store()
        .get_invoicing_entity(h.tenant, None)
        .await
        .expect("invoicing entity");
    let subtotal: i64 = lines.iter().map(|l| l.amount_subtotal).sum();
    let total: i64 = lines.iter().map(|l| l.amount_total).sum();
    let tax: i64 = lines.iter().map(|l| l.tax_amount).sum();
    let invoice_date = at.date_naive();
    let draft = h
        .store()
        .insert_invoice(InvoiceNew {
            status: InvoiceStatusEnum::Draft,
            tenant_id: h.tenant,
            customer_id: customer,
            subscription_id: None,
            cycle_index: None,
            currency: "USD".to_owned(),
            invoice_number: "draft".to_owned(),
            line_items: lines,
            coupons: Vec::new(),
            data_updated_at: None,
            invoice_date,
            billing_period_start: invoice_date,
            tax_inclusive: false,
            plan_version_id: None,
            invoice_type: InvoiceType::OneOff,
            finalized_at: None,
            subtotal,
            subtotal_recurring: 0,
            discount: subtotal + tax - total,
            tax_amount: tax,
            total,
            amount_due: total,
            applied_credits: 0,
            promotional_credits: 0,
            net_terms: 0,
            reference: None,
            purchase_order: None,
            memo: None,
            due_at: None,
            plan_name: None,
            customer_details: details.into(),
            seller_details: entity.clone().into(),
            auto_advance: false,
            payment_status: InvoicePaymentStatus::Unpaid,
            tax_breakdown: Vec::new(),
            manual: true,
            invoicing_entity_id: entity.id,
            parent_invoice_id: None,
            consolidated_into_invoice_id: None,
            annexes: Vec::new(),
            custom_properties: serde_json::Value::Null,
        })
        .await
        .expect("one-off draft");
    h.services()
        .finalize_invoice_at(
            Actor::System,
            draft.id,
            h.tenant,
            IssueAt::At(at),
            InvoiceDelivery::Suppressed,
        )
        .await
        .expect("one-off issued")
        .invoice
}

pub(super) async fn settled_payment(h: &Halyard, invoice: InvoiceId) -> PaymentTransactionId {
    h.store()
        .list_payment_tx_by_invoice_id(h.tenant, invoice)
        .await
        .expect("payments")
        .into_iter()
        .find(|tx| tx.transaction.status == PaymentStatusEnum::Settled)
        .expect("a settled payment")
        .transaction
        .id
}

/// The `wallet.credit_unrecovered` notices of an invoice: (cause, unrecovered, total).
pub(super) async fn unrecovered(h: &Halyard, invoice: InvoiceId) -> Vec<(String, i64, i64)> {
    unrecovered_where(h, |event| {
        serde_json::to_value(event.invoice_id).ok() == serde_json::to_value(invoice).ok()
    })
    .await
}

/// The `wallet.credit_unrecovered` notices the webhook queue holds that `keep` selects:
/// (cause, unrecovered, total so far).
pub(super) async fn unrecovered_where(
    h: &Halyard,
    keep: impl Fn(&WalletCreditUnrecoveredEvent) -> bool,
) -> Vec<(String, i64, i64)> {
    h.store()
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(1000),
            MessageReadVtSec(0),
        )
        .await
        .expect("webhook queue")
        .into_iter()
        .filter_map(|message| message.message)
        .filter_map(|message| serde_json::from_value::<OutboxEvent>(message.0).ok())
        .filter_map(|event| match event {
            OutboxEvent::WalletCreditUnrecovered(event) if keep(&event) => {
                let event: WalletCreditUnrecoveredEvent = *event;
                Some((
                    event.cause.as_str().to_owned(),
                    event.unrecovered_minor,
                    event.unrecovered_total_minor,
                ))
            }
            _ => None,
        })
        .collect()
}

/// PAYG from Sep 1, a 20000 top-up on Sep 1; 12000 of September usage (lite 20M before Sep 11,
/// lite 40M after) billed Oct 1, a 5000 one-off on Oct 2 voided Oct 3, 3000 of the usage credited
/// Oct 5. Returns the customer, the purchase and the usage invoice.
async fn through_the_credit_note(h: &Halyard) -> (CustomerId, Invoice, Invoice) {
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, _) = h.subscribe(customer, h.payg, day(9, 1)).await;
    let purchase = top_up(h, customer, 20_000, at(9, 1)).await;
    let grants = h.grants(customer).await;
    let top_up_credit = grant(&grants, "CREDIT_PURCHASE", "PAID", 20_000);
    assert_eq!(
        (
            top_up_credit.subscription_id,
            top_up_credit.application.as_str(),
            top_up_credit.expires_at,
            top_up_credit.remaining,
        ),
        (None, "INVOICE", None, 20_000)
    );
    if let Some(rev) = h.revenue(customer, day(9, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized),
            (20_000, 0),
            "billed as prepaid credit"
        );
    }
    h.check().await;

    h.tokens(customer, at_time(9, 5, 10, 0, 0), LITE, 20 * M);
    h.tokens(customer, at_time(9, 20, 10, 0, 0), LITE, 40 * M);
    let september = h.renew(sub, at(10, 1)).await.expect("INV-1");
    assert_eq!(
        money(&september),
        (12_000, 0, 0, 0, 12_000, 12_000, 0),
        "{:?}",
        lines(&september)
    );
    let top_up_id = top_up_credit.id;
    let remaining = |grants: &[diesel_models::customer_credit::CustomerCreditGrantRow]| {
        grants
            .iter()
            .find(|g| g.id == top_up_id)
            .map(|g| g.remaining)
            .expect("the top-up")
    };
    assert_eq!(remaining(&h.grants(customer).await), 8_000);
    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!((rev.recognized, rev.deferred()), (12_000, 8_000));
    }
    h.check().await;

    let onboarding = one_off(
        h,
        customer,
        vec![line("Onboarding", 5_000, false)],
        at(10, 2),
    )
    .await;
    assert_eq!(money(&onboarding), (5_000, 0, 0, 0, 5_000, 5_000, 0));
    assert_eq!(remaining(&h.grants(customer).await), 3_000);
    h.check().await;

    h.store()
        .void_invoice(Actor::System, onboarding.id, h.tenant)
        .await
        .expect("INV-X voided");
    assert_eq!(
        remaining(&h.grants(customer).await),
        8_000,
        "the void gives 5000 back"
    );
    h.check().await;

    let usage_line = september
        .line_items
        .iter()
        .find(|l| l.metric_id.is_some())
        .expect("the usage line")
        .local_id
        .clone();
    h.store()
        .create_and_finalize_credit_note(
            Actor::System,
            h.tenant,
            CreateCreditNoteParams {
                invoice_id: september.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: usage_line,
                    amount_cents: 3_000,
                }],
                reason: Some("Usage credited".into()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
                custom_properties: None,
            },
        )
        .await
        .expect("CN-1");
    assert_eq!(
        remaining(&h.grants(customer).await),
        11_000,
        "the note gives the 3000 back to the top-up that paid it"
    );
    h.check().await;
    (customer, purchase, september)
}

/// Refunds `amount` of the purchase in cash, by a refund note on its line.
async fn refund_purchase(h: &Halyard, purchase: &Invoice, amount: i64) {
    let note = h
        .store()
        .create_and_finalize_credit_note(
            Actor::System,
            h.tenant,
            CreateCreditNoteParams {
                invoice_id: purchase.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: purchase.line_items[0].local_id.clone(),
                    amount_cents: amount,
                }],
                reason: Some("Refund".into()),
                memo: None,
                credit_type: CreditType::Refund,
                custom_properties: None,
            },
        )
        .await
        .expect("CN-2");
    let payment = settled_payment(h, purchase.id).await;
    h.services()
        .record_offline_refund(
            Actor::System,
            h.tenant,
            OfflineRefundCommand {
                parent_transaction_id: payment,
                amount,
                credit_note_id: Some(note.id),
                refund_date: at(10, 6).naive_utc(),
                reference: Some("card refund".to_owned()),
                reason: None,
            },
        )
        .await
        .expect("card refunded");
}

#[tokio::test]
async fn s13_top_up_partly_used_voided_credited_and_refunded() {
    let h = Halyard::new().await;
    let (customer, purchase, _) = through_the_credit_note(&h).await;
    // Hand-issued notes and refunds are dated when made (DECISIONS), so revenue is read after them.
    let today = chrono::Utc::now().date_naive();
    if let Some(rev) = h.revenue(customer, today).await {
        assert_eq!(rev.recognized, 9_000, "12000 less the 3000 credited");
    }

    refund_purchase(&h, &purchase, 11_000).await;
    let grants = h.grants(customer).await;
    let top_up_credit = grant(&grants, "CREDIT_PURCHASE", "PAID", 20_000);
    assert_eq!(top_up_credit.remaining, 0);
    assert_eq!(
        h.movements(top_up_credit.id)
            .await
            .last()
            .map(String::as_str),
        Some("REVERSAL -11000")
    );
    assert!(unrecovered(&h, purchase.id).await.is_empty());
    h.check().await;

    if let Some(rev) = h.revenue(customer, today).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred()),
            (9_000, 9_000, 0),
            "deferred = the top-up at cost, now 0"
        );
    }
}

/// Refunding 15000 when 11000 is left: a refund takes back only purchased credit still unspent,
/// so the note is capped at the 11000 left and so is the cash refunded on it (DECISIONS, default:
/// Lago). Nothing is unrecovered: spent credit was never refunded.
#[tokio::test]
async fn s13_refund_above_the_credit_left_is_capped_at_the_unspent_credit() {
    let h = Halyard::new().await;
    let (customer, purchase, _) = through_the_credit_note(&h).await;

    let note = h
        .store()
        .create_and_finalize_credit_note(
            Actor::System,
            h.tenant,
            CreateCreditNoteParams {
                invoice_id: purchase.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: purchase.line_items[0].local_id.clone(),
                    amount_cents: 15_000,
                }],
                reason: Some("Refund".into()),
                memo: None,
                credit_type: CreditType::Refund,
                custom_properties: None,
            },
        )
        .await
        .expect("CN-2");
    assert_eq!(
        (note.subtotal, note.refunded_amount_cents),
        (-11_000, 11_000),
        "capped at the unspent 11000"
    );
    let payment = settled_payment(&h, purchase.id).await;
    let refund = |amount| OfflineRefundCommand {
        parent_transaction_id: payment,
        amount,
        credit_note_id: Some(note.id),
        refund_date: at(10, 6).naive_utc(),
        reference: Some("card refund".to_owned()),
        reason: None,
    };
    let refused = h
        .services()
        .record_offline_refund(Actor::System, h.tenant, refund(15_000))
        .await;
    assert!(
        format!("{refused:?}").contains("exceeds what this credit note still allows (11000)"),
        "{refused:?}"
    );
    h.services()
        .record_offline_refund(Actor::System, h.tenant, refund(11_000))
        .await
        .expect("card refunded");
    let grants = h.grants(customer).await;
    let top_up_credit = grant(&grants, "CREDIT_PURCHASE", "PAID", 20_000);
    assert_eq!(top_up_credit.remaining, 0);
    assert_eq!(
        h.movements(top_up_credit.id)
            .await
            .last()
            .map(String::as_str),
        Some("REVERSAL -11000")
    );
    assert!(unrecovered(&h, purchase.id).await.is_empty());
    h.check().await;
}
