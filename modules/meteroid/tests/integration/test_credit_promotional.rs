//! Promotional credit is taken off the charges before tax, like Stripe's billing credits; paid
//! credit still pays the taxed total. Each test states its invoice in whole figures at 20% tax.
use crate::data::ids::{CUST_UBER_ID, INVOICING_ENTITY_ID, TENANT_ID};
use crate::helpers::credit::{Grant, assert_ledger_balances, fund_grant, grant_remaining};
use crate::meteroid_it::container::MeteroidSetup;
use crate::test_customer_credit::{finalize, line, pay_amount};
use chrono::{Duration, Utc};
use common_domain::actor::Actor;
use common_domain::ids::CreditGrantId;
use meteroid_store::domain::enums::InvoicePaymentStatus;
use meteroid_store::domain::{Invoice, LineItem, TaxDetail, TaxInclusiveQuote};
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::repositories::{CreditNoteInterface, InvoiceInterface};
use rust_decimal::Decimal;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(crate) fn test_error(error: impl std::fmt::Debug) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

pub(crate) use crate::helpers::invoices::{promotional, setup, taxed, taxed_draft};

/// A line quoted `quoted` tax included at 20%, stored net.
pub(crate) fn quoted(name: &str, quoted: i64) -> LineItem {
    let net = quoted * 5 / 6;
    LineItem {
        amount_subtotal: net,
        taxable_amount: net,
        tax_inclusive_quote: Some(TaxInclusiveQuote {
            amount_subtotal: quoted,
            unit_price: None,
            sub_lines: Vec::new(),
        }),
        ..taxed(name, net)
    }
}

pub(crate) async fn paid(setup: &MeteroidSetup, grant: Grant) -> CreditGrantId {
    fund_grant(&setup.store, TENANT_ID, CUST_UBER_ID, None, grant).await
}

pub(crate) async fn remaining(setup: &MeteroidSetup, grant: CreditGrantId) -> i64 {
    grant_remaining(&setup.store, TENANT_ID, CUST_UBER_ID, grant).await
}

pub(crate) fn assert_reconciles(invoice: &Invoice) {
    assert_eq!(
        invoice.subtotal - invoice.discount - invoice.promotional_credits + invoice.tax_amount,
        invoice.total,
        "subtotal - discount - promotional credit + tax = total"
    );
    let lines_total: i64 = invoice.line_items.iter().map(|l| l.amount_total).sum();
    assert_eq!(lines_total, invoice.total, "the lines add up to the total");
}

#[tokio::test]
async fn promotional_credit_comes_off_before_tax() -> TestResult {
    let setup = setup().await;
    let promo = promotional(&setup, Grant::invoice(3_000)).await;
    let invoice = taxed_draft(&setup, vec![taxed("Seats", 10_000)], false).await?;

    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(finalized.promotional_credits, 3_000);
    assert_eq!(finalized.tax_amount, 1_400, "20% of what the credit left");
    assert_eq!(finalized.total, 8_400);
    assert_eq!(finalized.applied_credits, 0);
    assert_eq!(finalized.amount_due, 8_400);
    assert_eq!(finalized.line_items[0].promotional_credit, Some(3_000));
    assert_eq!(finalized.line_items[0].taxable_amount, 7_000);
    assert_reconciles(&finalized);
    assert_eq!(remaining(&setup, promo).await, 0);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// Promotional credit always pays first, before tax; paid credit then pays the taxed remainder,
/// whichever of the two expires first.
#[tokio::test]
async fn promotional_then_paid_credit_whatever_expires_first() -> TestResult {
    for promotional_expires_first in [true, false] {
        let setup = setup().await;
        let (soon, later) = (
            Some(Utc::now() + Duration::days(5)),
            Some(Utc::now() + Duration::days(50)),
        );
        let (promo_expiry, paid_expiry) = if promotional_expires_first {
            (soon, later)
        } else {
            (later, soon)
        };
        let promo = promotional(
            &setup,
            Grant {
                expires_at: promo_expiry,
                ..Grant::invoice(3_000)
            },
        )
        .await;
        let cash = paid(
            &setup,
            Grant {
                expires_at: paid_expiry,
                priority: 1,
                ..Grant::invoice(5_000)
            },
        )
        .await;
        let invoice = taxed_draft(&setup, vec![taxed("Seats", 10_000)], false).await?;
        let finalized = finalize(&setup, invoice.id).await?.invoice;
        assert_eq!(finalized.promotional_credits, 3_000);
        assert_eq!(finalized.tax_amount, 1_400);
        assert_eq!(finalized.total, 8_400);
        assert_eq!(finalized.applied_credits, 5_000);
        assert_eq!(finalized.amount_due, 3_400);
        assert_reconciles(&finalized);
        assert_eq!(remaining(&setup, promo).await, 0);
        assert_eq!(remaining(&setup, cash).await, 0);
        assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    }
    Ok(())
}

/// On a tax-inclusive invoice the credit comes off the quoted price and the tax is carved out of
/// what is left: 120.00 quoted less 50.00 of credit is 70.00, of which 11.67 is tax.
#[tokio::test]
async fn tax_inclusive_promotional_credit_comes_off_the_quoted_price() -> TestResult {
    let setup = setup().await;
    let promo = promotional(&setup, Grant::invoice(5_000)).await;
    let invoice = taxed_draft(&setup, vec![quoted("Seats", 12_000)], true).await?;

    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(finalized.total, 7_000);
    assert_eq!(finalized.tax_amount, 1_167);
    assert_eq!(finalized.line_items[0].promotional_credit, Some(5_000));
    assert_eq!(finalized.line_items[0].taxable_amount, 5_833);
    assert_eq!(
        finalized.promotional_credits, 4_166,
        "the net share of the quoted 50.00"
    );
    assert_eq!(finalized.amount_due, 7_000);
    assert_reconciles(&finalized);
    assert_eq!(
        remaining(&setup, promo).await,
        0,
        "the quoted 50.00 is drawn"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A note crediting a tax-inclusive line states the net share of the promotional credit, as the
/// invoice does: the line nets to 99.99 (in proportion to its 58.33 taxable of 70.00 quoted), so
/// its reduction is 41.66, all of it the net share of the quoted 50.00 the note gives back.
#[tokio::test]
async fn a_tax_inclusive_note_states_the_net_promotional_credit() -> TestResult {
    let setup = setup().await;
    let promo = promotional(&setup, Grant::invoice(5_000)).await;
    let invoice = taxed_draft(&setup, vec![quoted("Seats", 12_000)], true).await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(finalized.line_items[0].promotional_credit, Some(5_000));
    assert_eq!(finalized.line_items[0].amount_subtotal, 9_999);
    assert_eq!(finalized.line_items[0].net_promotional_credit(), 4_166);
    assert_eq!(finalized.promotional_credits, 4_166);

    let note = setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: finalized.id,
                line_items: Vec::new(),
                reason: None,
                memo: None,
                credit_type: CreditType::DebtCancellation,
                custom_properties: None,
            },
        )
        .await
        .map_err(test_error)?;
    assert_eq!(note.line_items[0].promotional_credit, Some(-5_000));
    assert_eq!(
        remaining(&setup, promo).await,
        5_000,
        "the quoted 50.00 goes back"
    );

    let entity = setup
        .store
        .get_invoicing_entity(TENANT_ID, Some(INVOICING_ENTITY_ID))
        .await
        .map_err(test_error)?;
    // The finalized note's figure is the ledger's.
    let restored = setup
        .store
        .promotional_credit_restored(&note)
        .await
        .map_err(test_error)?;
    assert_eq!(restored, 5_000);
    let document = meteroid::services::credit_note_rendering::mapper::map_credit_note_to_invoicing(
        note,
        &entity,
        None,
        None,
        &finalized,
        &std::collections::HashMap::new(),
        restored,
    )
    .map_err(test_error)?;
    let line = &document.lines[0];
    assert_eq!(
        line.promotional_credit.map(|credit| *credit.amount()),
        Some(Decimal::new(-4_166, 2))
    );
    assert_eq!(
        *line.subtotal.amount() - *line.taxable_amount.amount(),
        Decimal::new(-4_166, 2),
        "the whole reduction is promotional credit"
    );
    assert_eq!(
        *document.metadata.promotional_credit_restored.amount(),
        Decimal::new(5_000, 2),
        "the PDF states the quoted 50.00 given back to the grant"
    );
    assert_eq!(
        *document.metadata.debt_cancelled_amount.amount(),
        Decimal::new(7_000, 2)
    );
    // The PDF summary foots: subtotal, the promotional row, tax, total.
    let content =
        meteroid_invoicing::credit_note_render::TypstCreditNoteContent::try_from(&document)
            .map_err(test_error)?;
    let shown = |value: &dyn std::fmt::Display| value.to_string().replace('\u{2212}', "-");
    assert_eq!(shown(&content.subtotal), "-99.99");
    assert_eq!(shown(&content.discount), "0.00");
    assert_eq!(shown(&content.promotional_credit), "41.66");
    assert_eq!(shown(&content.tax_amount), "-11.67");
    assert_eq!(shown(&content.total_amount), "-70.00");
    Ok(())
}

#[tokio::test]
async fn the_draft_previews_exactly_what_finalization_draws() -> TestResult {
    let setup = setup().await;
    promotional(&setup, Grant::invoice(2_000)).await;
    paid(&setup, Grant::invoice(4_000)).await;
    let invoice = taxed_draft(
        &setup,
        vec![taxed("Seats", 6_000), taxed("Support", 4_000)],
        false,
    )
    .await?;
    // An operator-edited draft keeps its lines on refresh; only the credit it would draw moves.
    {
        use diesel_async::SimpleAsyncConnection;
        setup
            .store
            .get_conn()
            .await
            .map_err(test_error)?
            .batch_execute(&format!(
                "UPDATE invoice SET manually_edited_at = now() WHERE id = '{}'",
                common_domain::ids::BaseId::as_uuid(&invoice.id)
            ))
            .await
            .map_err(test_error)?;
    }
    let preview = setup
        .services
        .refresh_invoice_data(invoice.id, TENANT_ID)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(preview.promotional_credits, 2_000);
    assert_eq!(preview.tax_amount, 1_600);
    assert_eq!(preview.total, 9_600);
    assert_eq!(preview.applied_credits, 4_000);
    assert_eq!(preview.amount_due, 5_600);
    assert_reconciles(&preview);

    let finalized = finalize(&setup, invoice.id).await?.invoice;
    for (field, previewed, issued) in [
        (
            "promotional",
            preview.promotional_credits,
            finalized.promotional_credits,
        ),
        ("tax", preview.tax_amount, finalized.tax_amount),
        ("total", preview.total, finalized.total),
        (
            "applied",
            preview.applied_credits,
            finalized.applied_credits,
        ),
        ("due", preview.amount_due, finalized.amount_due),
    ] {
        assert_eq!(previewed, issued, "{field}");
    }
    let shares = |invoice: &Invoice| {
        invoice
            .line_items
            .iter()
            .map(|line| line.promotional_credit)
            .collect::<Vec<_>>()
    };
    assert_eq!(shares(&preview), shares(&finalized));
    assert_eq!(shares(&finalized), vec![Some(1_200), Some(800)]);
    Ok(())
}

/// A note crediting the line gives the promotional credit it took back to its grant, and reverses
/// only the tax the invoice charged.
#[tokio::test]
async fn a_credit_note_returns_the_promotional_credit_and_the_tax_charged() -> TestResult {
    let setup = setup().await;
    let promo = promotional(&setup, Grant::invoice(3_000)).await;
    let invoice = taxed_draft(&setup, vec![taxed("Seats", 10_000)], false).await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    pay_amount(&setup, finalized.id, finalized.amount_due).await?;

    let half = setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: finalized.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: finalized.line_items[0].local_id.clone(),
                    amount_cents: 5_000,
                }],
                reason: Some("Half".into()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
                custom_properties: None,
            },
        )
        .await
        .map_err(test_error)?;
    assert_eq!(half.tax_amount, -700);
    assert_eq!(half.total, -4_200);
    assert_eq!(half.line_items[0].promotional_credit, Some(-1_500));
    assert_eq!(remaining(&setup, promo).await, 1_500);

    let rest = setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: finalized.id,
                line_items: Vec::new(),
                reason: Some("The rest".into()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
                custom_properties: None,
            },
        )
        .await
        .map_err(test_error)?;
    assert_eq!(half.tax_amount + rest.tax_amount, -1_400);
    assert_eq!(half.total + rest.total, -8_400);
    assert_eq!(
        remaining(&setup, promo).await,
        3_000,
        "all of it is back with its grant"
    );
    assert_eq!(
        half.credited_amount_cents + rest.credited_amount_cents,
        8_400,
        "the cash collected becomes credit"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A line taxed 0.67 was half forgiven by a note of the previous release, which returned 0.33 of
/// its tax (half-even, not cumulative). The note forgiving the rest returns the 0.34 left, so the
/// notes return the line's whole tax and cancel the invoice's whole debt.
#[tokio::test]
async fn the_note_completing_a_line_returns_the_tax_an_earlier_release_left() -> TestResult {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::credit_note::dsl as cn;
    use diesel_models::schema::invoice::dsl as inv;

    let setup = setup().await;
    let rate = Decimal::new(67, 3);
    let seats = LineItem {
        tax_rate: rate,
        tax_amount: 67,
        amount_total: 1_067,
        tax_details: vec![TaxDetail {
            tax_rate: rate,
            tax_name: "VAT".to_owned(),
            tax_amount: 67,
        }],
        ..line("Seats", 1_000, false)
    };
    let invoice = taxed_draft(&setup, vec![seats], false).await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!((finalized.tax_amount, finalized.amount_due), (67, 1_067));
    let local_id = finalized.line_items[0].local_id.clone();
    let note = |line_items: Vec<CreditLineItem>| CreateCreditNoteParams {
        invoice_id: finalized.id,
        line_items,
        reason: None,
        memo: None,
        credit_type: CreditType::DebtCancellation,
        custom_properties: None,
    };

    let half = setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            note(vec![CreditLineItem::Amount {
                local_id,
                amount_cents: 500,
            }]),
        )
        .await
        .map_err(test_error)?;
    assert_eq!(half.tax_amount, -34, "the cumulative half-up share");
    // As the previous release stored it: 0.33 of tax, 5.33 forgiven, 5.34 left due.
    let mut lines = half.line_items.clone();
    lines[0].tax_amount = -33;
    lines[0].amount_total = -533;
    lines[0].tax_details[0].tax_amount = -33;
    let mut breakdown = serde_json::to_value(&half.tax_breakdown).map_err(test_error)?;
    breakdown[0]["tax_amount"] = serde_json::json!(33);
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    diesel::update(cn::credit_note.filter(cn::id.eq(half.id)))
        .set((
            cn::line_items.eq(serde_json::to_value(&lines).map_err(test_error)?),
            cn::tax_breakdown.eq(breakdown),
            cn::tax_amount.eq(-33),
            cn::total.eq(-533),
        ))
        .execute(&mut conn)
        .await
        .map_err(test_error)?;
    diesel::update(inv::invoice.filter(inv::id.eq(finalized.id)))
        .set(inv::amount_due.eq(534))
        .execute(&mut conn)
        .await
        .map_err(test_error)?;
    drop(conn);

    let rest = setup
        .store
        .create_and_finalize_credit_note(Actor::System, TENANT_ID, note(Vec::new()))
        .await
        .map_err(test_error)?;
    assert_eq!(rest.subtotal, -500);
    assert_eq!(rest.tax_amount, -34, "0.67 returned in all");
    assert_eq!(rest.total, -534);
    assert_eq!(rest.line_items[0].tax_details[0].tax_amount, -34);
    let paid = setup
        .store
        .get_invoice_by_id(TENANT_ID, finalized.id)
        .await
        .map_err(test_error)?;
    assert_eq!(
        (paid.amount_due, paid.payment_status),
        (0, InvoicePaymentStatus::Paid),
        "the whole debt is cancelled"
    );
    Ok(())
}

/// Voiding gives every draw back, promotional and paid, and checks the invoice drew exactly what
/// it shows.
#[tokio::test]
async fn a_void_returns_promotional_and_paid_credit() -> TestResult {
    let setup = setup().await;
    let promo = promotional(&setup, Grant::invoice(3_000)).await;
    let cash = paid(&setup, Grant::invoice(2_000)).await;
    let invoice = taxed_draft(&setup, vec![taxed("Seats", 10_000)], false).await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(
        (finalized.promotional_credits, finalized.applied_credits),
        (3_000, 2_000)
    );
    setup
        .store
        .void_invoice(Actor::System, finalized.id, TENANT_ID)
        .await
        .map_err(test_error)?;
    assert_eq!(remaining(&setup, promo).await, 3_000);
    assert_eq!(remaining(&setup, cash).await, 2_000);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// QA M13: an invoice a debt-cancellation note cancelled whole is Paid (as Stripe marks it) but
/// settled by credit notes, while one paid 120.00 in money is not, as GetInvoice and ListInvoices
/// state it.
#[tokio::test]
async fn an_invoice_cancelled_by_a_note_is_settled_by_credit_notes() -> TestResult {
    use meteroid_grpc::meteroid::api::invoices::v1::{GetInvoiceRequest, ListInvoicesRequest};
    let setup = setup().await;
    let cancelled = finalize(
        &setup,
        taxed_draft(&setup, vec![taxed("Seats", 10_000)], false)
            .await?
            .id,
    )
    .await?
    .invoice;
    setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: cancelled.id,
                line_items: Vec::new(),
                reason: None,
                memo: None,
                credit_type: CreditType::DebtCancellation,
                custom_properties: None,
            },
        )
        .await
        .map_err(test_error)?;
    let paid = finalize(
        &setup,
        taxed_draft(&setup, vec![taxed("Seats", 10_000)], false)
            .await?
            .id,
    )
    .await?
    .invoice;
    assert_eq!(paid.total, 12_000);
    pay_amount(&setup, paid.id, paid.total).await?;

    let auth = crate::meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let mut clients = crate::meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let listed = clients
        .invoices
        .list_invoices(ListInvoicesRequest {
            customer_id: Some(CUST_UBER_ID.to_string()),
            pagination: Some(common_grpc::meteroid::common::v1::Pagination {
                per_page: Some(100),
                page: 0,
            }),
            ..Default::default()
        })
        .await
        .map_err(test_error)?
        .into_inner()
        .invoices;
    for (id, settled_by_notes) in [(cancelled.id, true), (paid.id, false)] {
        let invoice = clients
            .invoices
            .get_invoice(GetInvoiceRequest { id: id.to_string() })
            .await
            .map_err(test_error)?
            .into_inner()
            .invoice
            .ok_or("no invoice")?;
        assert_eq!(
            invoice.payment_status,
            meteroid_grpc::meteroid::api::invoices::v1::InvoicePaymentStatus::Paid as i32
        );
        assert_eq!(
            invoice.settled_by_credit_notes, settled_by_notes,
            "{}",
            invoice.invoice_number
        );
        let row = listed
            .iter()
            .find(|row| row.id == id.to_string())
            .ok_or("not listed")?;
        assert_eq!(row.settled_by_credit_notes, settled_by_notes);
    }

    // The webhook says so too: `invoice.paid` of the cancelled invoice is a settlement by notes.
    use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
    use meteroid_store::domain::outbox_event::OutboxEvent;
    use meteroid_store::domain::pgmq::PgmqQueue;
    use meteroid_store::repositories::pgmq::PgmqInterface;
    let paid_events: Vec<(common_domain::ids::InvoiceId, bool)> = setup
        .store
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(1000),
            MessageReadVtSec(0),
        )
        .await
        .map_err(test_error)?
        .into_iter()
        .filter_map(|message| message.message)
        .filter_map(|message| serde_json::from_value::<OutboxEvent>(message.0).ok())
        .filter_map(|event| match event {
            OutboxEvent::InvoicePaid(e) => Some((e.invoice_id, e.settled_by_credit_notes)),
            _ => None,
        })
        .collect();
    assert!(
        paid_events.contains(&(cancelled.id, true)),
        "{paid_events:?}"
    );
    assert!(!paid_events.contains(&(paid.id, true)), "{paid_events:?}");
    Ok(())
}
