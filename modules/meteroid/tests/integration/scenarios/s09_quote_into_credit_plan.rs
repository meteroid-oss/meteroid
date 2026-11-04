//! S9. A quote's terms on Scale: the pack sold at 45000 for 55000 of credit, sent as an invoice.
//! The pack mints when it is paid (P 45000 + B 10000) and pays its whole period's usage (OQ10);
//! paid only after its period was invoiced, it is credited back to the balance instead (OQ11).
//! Quotes convert at the present only, so the converted subscription is built with the quote's
//! override directly (see FOLLOWUPS "Scenario tests").
use common_domain::ids::{CreditGrantId, CustomerId, PriceComponentId, SubscriptionId};
use diesel_models::customer_credit::CustomerCreditEntryRow;
use meteroid_store::domain::Invoice;
use meteroid_store::domain::enums::BillingPeriodEnum;
use meteroid_store::domain::price_components::{CreditExpiry, PriceEntry, PriceInput};
use meteroid_store::domain::prices::Pricing;
use meteroid_store::domain::subscription_components::{
    ComponentOverride, CreateSubscriptionComponents,
};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit_notes::CreditType;
use rust_decimal_macros::dec;

use super::kit::{
    Halyard, LITE, M, STANDARD, Tax, at, at_time, day, expiring, grant, lines, money, summary,
};

/// Movements as "KIND amount", with the reason of reversals and expiries.
async fn ledger(h: &Halyard, grant: CreditGrantId) -> Vec<String> {
    let mut conn = h.env.conn().await;
    CustomerCreditEntryRow::list_by_grant(&mut conn, h.tenant, grant)
        .await
        .expect("movements")
        .into_iter()
        .map(|entry| match entry.reason {
            Some(reason) if entry.kind == "REVERSAL" || entry.kind == "EXPIRY" => {
                format!("{} {} {reason}", entry.kind, entry.amount)
            }
            _ => format!("{} {}", entry.kind, entry.amount),
        })
        .collect()
}

async fn scale_pack(h: &Halyard) -> PriceComponentId {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::price_component::dsl as c;
    c::price_component
        .filter(c::plan_version_id.eq(h.scale))
        .filter(c::name.eq("Monthly credit pack"))
        .select(c::id)
        .first::<PriceComponentId>(&mut h.env.conn().await)
        .await
        .expect("Scale's pack")
}

/// The quote converted: Scale from Sep 1, pack at 45000 for 55000, net terms omitted, invoices
/// sent. INV-1 is issued unpaid; only the welcome credit is granted.
async fn converted_quote(h: &Halyard) -> (CustomerId, SubscriptionId, Invoice) {
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let mut request = h.request(customer, h.scale, day(9, 1));
    request.price_components = Some(CreateSubscriptionComponents {
        parameterized_components: vec![],
        overridden_components: vec![ComponentOverride {
            component_id: scale_pack(h).await,
            name: "Monthly credit pack".to_owned(),
            price_entry: PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "USD".to_owned(),
                pricing: Pricing::Credit {
                    unit_price: dec!(450),
                    quantity: 1,
                    credit_amount: dec!(550),
                    expiry: Some(CreditExpiry::BillingPeriod),
                    credit_unit_id: None,
                    rollover: None,
                    once: false,
                },
                credit_unit: None,
            }),
        }],
        extra_components: vec![],
        remove_components: vec![],
    });
    let (sub, first) = h.start(request).await;
    let first = first.expect("INV-1");
    assert_eq!(first.net_terms, 0, "a credit plan's net terms default to 0");
    assert_eq!(
        money(&first),
        (45_000, 0, 0, 0, 45_000, 0, 45_000),
        "{:?}",
        lines(&first)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        summary(&grants),
        vec!["INCLUDED_CREDIT PROMOTIONAL 5000 5000"],
        "the welcome credit is granted with the invoice; the pack waits for its payment"
    );
    assert_eq!(
        expiring(&grants, "INCLUDED_CREDIT", at(11, 30)).subscription_id,
        Some(sub)
    );
    h.check().await;
    (customer, sub, first)
}

#[tokio::test]
async fn s09_quote_pack_paid_late_in_its_period_pays_the_whole_period() {
    let h = Halyard::new().await;
    let (customer, sub, first) = converted_quote(&h).await;

    // Sep 1 - Sep 4 12:00: standard 8M (9600) + lite 2M (400) = 10000.
    h.tokens(customer, at_time(9, 2, 10, 0, 0), STANDARD, 8 * M);
    h.tokens(customer, at_time(9, 3, 10, 0, 0), LITE, 2 * M);

    h.pay(first.id, at_time(9, 4, 12, 0, 0)).await;
    let grants = h.grants(customer).await;
    let paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 45_000);
    let bonus = grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 10_000);
    assert_eq!(
        (paid.cost_minor, paid.expires_at, bonus.expires_at),
        (Some(45_000), Some(at(10, 1)), Some(at(10, 1))),
        "P 45000 + B 55000 − 45000, ending with their period"
    );
    h.check().await;

    // Sep 4 - 30: standard 25M = 30000.
    h.tokens(customer, at_time(9, 10, 10, 0, 0), STANDARD, 25 * M);
    let renewal = h.renew(sub, at(10, 1)).await.expect("INV-2");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let renewal = h.invoice(renewal.id).await;
    assert_eq!(
        money(&renewal),
        (85_000, 0, 10_000, 0, 75_000, 30_000, 45_000),
        "October's pack 45000 and September's usage 40000, Sep 1-4 included: {:?}",
        lines(&renewal)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        h.movements(grant(&grants, "CREDIT_PURCHASE", "PAID", 45_000).id)
            .await,
        vec!["FUND 45000", "DRAW -30000", "EXPIRY -15000"]
    );
    assert_eq!(
        h.movements(grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 10_000).id)
            .await,
        vec!["FUND 10000", "DRAW -10000"]
    );
    assert_eq!(
        expiring(&grants, "INCLUDED_CREDIT", at(11, 30)).remaining,
        5_000
    );
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred(), rev.breakage),
            (90_000, 45_000, 45_000, 15_000)
        );
    }
}

/// S9-late (OQ11): INV-1 is still unpaid when INV-2 bills September's usage. Paid on Oct 3, the
/// pack is credited back for what it would have paid of September on time: its bonus 10000 first,
/// then 30000 of its paid credit; 40000 of paid credit is reversed with the note, 5000 and the
/// bonus lapse.
#[tokio::test]
async fn s09_late_pack_paid_after_its_period_was_invoiced_credits_back_what_it_would_have_paid() {
    let h = Halyard::new().await;
    let (customer, sub, first) = converted_quote(&h).await;
    h.tokens(customer, at_time(9, 2, 10, 0, 0), STANDARD, 8 * M);
    h.tokens(customer, at_time(9, 3, 10, 0, 0), LITE, 2 * M);
    h.tokens(customer, at_time(9, 10, 10, 0, 0), STANDARD, 25 * M);

    let renewal = h.renew(sub, at(10, 1)).await.expect("INV-2");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let renewal = h.invoice(renewal.id).await;
    let billed = money(&renewal);
    assert_eq!(
        billed,
        (85_000, 0, 5_000, 0, 80_000, 0, 80_000),
        "no pack credit yet: the welcome credit pays 5000 of the usage: {:?}",
        lines(&renewal)
    );
    h.check().await;

    h.pay(first.id, at(10, 3)).await;
    h.sweep(customer, at(10, 3) + Halyard::hours(1)).await;
    let notes = h
        .store()
        .list_credit_notes_by_invoice_id(h.tenant, first.id)
        .await
        .expect("notes");
    assert_eq!(notes.len(), 1, "CN-2 on INV-1");
    assert_eq!(notes[0].credit_type, CreditType::CreditToBalance);
    assert_eq!((notes[0].subtotal, notes[0].total), (-40_000, -40_000));
    let grants = h.grants(customer).await;
    assert_eq!(
        ledger(&h, grant(&grants, "CREDIT_PURCHASE", "PAID", 45_000).id).await,
        vec![
            "FUND 45000",
            "REVERSAL -40000 Removed: paid after its period was invoiced",
            "EXPIRY -5000 Removed: paid after its period was invoiced",
        ]
    );
    assert_eq!(
        h.movements(grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 10_000).id)
            .await,
        vec!["FUND 10000", "EXPIRY -10000"]
    );
    let balance = grant(&grants, "CREDIT_NOTE", "PAID", 40_000);
    assert_eq!(
        (
            balance.subscription_id,
            balance.expires_at,
            balance.remaining
        ),
        (None, None, 40_000)
    );
    assert_eq!(
        money(&h.invoice(renewal.id).await),
        billed,
        "INV-2 unchanged"
    );
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(10, 3)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred(), rev.breakage),
            (125_000, 40_000, 85_000, 5_000),
            "the 5000 of fee beyond September's usage is breakage; deferred = G-cn 40000 + \
             October's pack, billed and not yet funded"
        );
    }
}

/// S9-forgiven (OQ11): INV-1 is paid 20000, then its rest is forgiven after INV-2 billed
/// September. The forgiveness funds the paid share, dated at the payment; the grant only existed
/// after its period, so the late pack credits that share back to the balance once the forgiveness
/// committed, whether the payment came before or after the period ended; never twice.
#[tokio::test]
async fn s09_late_pack_paid_in_part_then_forgiven_credits_the_paid_share_back() {
    for (paid_on, via_sweep, used) in [
        (at(10, 3), false, true),
        (at(9, 20), false, true),
        (at(10, 3), true, true),
        (at(10, 3), true, false),
    ] {
        forgiven_late_pack(paid_on, via_sweep, used).await;
    }
}

/// `via_sweep`: the note's event is missed, and the credit sweep credits the pack back. `used`:
/// September's 10000 of usage would have drawn the bonus (4444) then 5556 of the paid share: all
/// 10000 is credited back.
async fn forgiven_late_pack(paid_on: chrono::DateTime<chrono::Utc>, via_sweep: bool, used: bool) {
    use common_domain::actor::Actor;
    use meteroid_store::domain::LineItemKind;
    use meteroid_store::domain::enums::InvoicePaymentStatus;
    use meteroid_store::repositories::credit_notes::{CreateCreditNoteParams, CreditLineItem};

    let h = Halyard::new().await;
    let (customer, sub, first) = converted_quote(&h).await;
    let pay_part = || async {
        h.services()
            .add_manual_payment_transaction(
                Actor::System,
                h.tenant,
                first.id,
                dec!(200),
                paid_on.fixed_offset(),
                Some("wire-part".to_owned()),
            )
            .await
            .expect("20000 paid");
        h.env.run_outbox_and_orchestration().await;
    };
    if paid_on < at(10, 1) {
        pay_part().await;
    }
    if used {
        h.tokens(customer, at_time(9, 2, 10, 0, 0), STANDARD, 8 * M);
        h.tokens(customer, at_time(9, 3, 10, 0, 0), LITE, 2 * M);
    }
    h.renew(sub, at(10, 1)).await.expect("INV-2");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    if paid_on >= at(10, 1) {
        pay_part().await;
    }

    let pack = first
        .line_items
        .iter()
        .find(|line| line.kind == LineItemKind::Credit)
        .expect("the pack line")
        .local_id
        .clone();
    let forgiven = h
        .store()
        .create_and_finalize_credit_note(
            Actor::System,
            h.tenant,
            CreateCreditNoteParams {
                invoice_id: first.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: pack,
                    amount_cents: 25_000,
                }],
                reason: Some("Rest forgiven".to_owned()),
                memo: None,
                credit_type: CreditType::DebtCancellation,
                custom_properties: None,
            },
        )
        .await
        .expect("the rest forgiven");
    assert_eq!(
        h.invoice(first.id).await.payment_status,
        InvoicePaymentStatus::Paid
    );

    if via_sweep {
        assert_eq!(
            h.services()
                .credit_back_forgiven_late_packs_due(200)
                .await
                .expect("the sweep's credit-back"),
            1
        );
    }
    for _ in 0..2 {
        h.env.run_outbox_and_orchestration().await;
        let notes = h
            .store()
            .list_credit_notes_by_invoice_id(h.tenant, first.id)
            .await
            .expect("notes");
        let back = notes
            .iter()
            .filter(|note| note.id != forgiven.id)
            .collect::<Vec<_>>();
        let credited: Vec<_> = back
            .iter()
            .map(|note| (note.subtotal, note.total))
            .collect();
        assert_eq!(
            credited,
            if used {
                vec![(-10_000, -10_000)]
            } else {
                vec![]
            },
            "paid {paid_on}: what September would have drawn, credited back once"
        );
        assert!(
            back.iter()
                .all(|note| note.credit_type == CreditType::CreditToBalance)
        );
    }
    let grants = h.grants(customer).await;
    let paid = grants
        .iter()
        .find(|g| g.source_kind == "CREDIT_PURCHASE" && g.classification == "PAID")
        .expect("the paid share funded");
    let removal = "Removed: paid after its period was invoiced";
    assert_eq!(
        ledger(&h, paid.id).await,
        if used {
            vec![
                "FUND 20000".to_owned(),
                format!("REVERSAL -10000 {removal}"),
                format!("EXPIRY -10000 {removal}"),
            ]
        } else {
            vec!["FUND 20000".to_owned(), format!("EXPIRY -20000 {removal}")]
        },
        "paid {paid_on}"
    );
    if used {
        let balance = grant(&grants, "CREDIT_NOTE", "PAID", 10_000);
        assert_eq!((balance.subscription_id, balance.remaining), (None, 10_000));
    }
    h.check().await;
}
