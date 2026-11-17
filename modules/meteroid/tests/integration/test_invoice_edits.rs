//! Edits and voids of invoices: the bounds a draft edit keeps, and what a finalized invoice refuses.

use crate::data::ids::TENANT_ID;
use crate::test_credit_note::{insert_plain_draft, setup_unpaid_invoice};
use chrono::TimeZone;
use common_domain::actor::Actor;
use diesel_models::invoices::{InvoiceRow, InvoiceRowLinesPatch};
use meteroid_store::domain::{Invoice, InvoiceLinesPatch, UpdateInvoiceParams};
use meteroid_store::repositories::InvoiceInterface;
use rust_decimal::Decimal;

fn discount_of(discount: String) -> UpdateInvoiceParams {
    UpdateInvoiceParams {
        memo: None,
        reference: None,
        purchase_order: None,
        due_date: None,
        line_items: None,
        discount: Some(discount),
        customer_details: None,
        invoicing_entity_id: None,
        annexes: None,
        custom_properties: None,
    }
}

fn lines_patch(invoice: &Invoice) -> InvoiceRowLinesPatch {
    InvoiceLinesPatch {
        line_items: invoice.line_items.clone(),
        amount_due: invoice.amount_due,
        subtotal: invoice.subtotal,
        subtotal_recurring: invoice.subtotal_recurring,
        total: invoice.total,
        tax_amount: invoice.tax_amount,
        applied_credits: invoice.applied_credits,
        promotional_credits: invoice.promotional_credits,
        applied_coupons: vec![],
        tax_breakdown: invoice.tax_breakdown.clone(),
        annexes: invoice.annexes.clone(),
    }
    .try_into()
    .unwrap()
}

/// I25, R-087: a discount below zero or above the lines' subtotal is refused; the whole subtotal
/// leaves only nothing to tax.
#[tokio::test]
async fn a_draft_discount_outside_its_subtotal_is_refused() {
    let (_setup, services, store, invoice) = setup_unpaid_invoice().await;
    let draft = insert_plain_draft(&store, &invoice).await;
    let major = |minor: i64| Decimal::new(minor, 2).to_string();

    for refused in [major(-1), major(draft.subtotal + 1)] {
        assert!(
            services
                .update_draft_invoice(draft.id, TENANT_ID, discount_of(refused.clone()))
                .await
                .is_err(),
            "a discount of {refused} on a subtotal of {} is refused",
            draft.subtotal
        );
    }

    let edited = services
        .update_draft_invoice(draft.id, TENANT_ID, discount_of(major(draft.subtotal)))
        .await
        .unwrap()
        .invoice;
    assert_eq!(edited.discount, draft.subtotal);
    assert!(
        edited
            .line_items
            .iter()
            .all(|line| line.taxable_amount == 0)
    );
    assert_eq!(edited.total, 0);
}

/// R-087: an edit counts as paid only the cash collected, never what a negative total left of
/// the amount due.
#[tokio::test]
async fn a_draft_edit_counts_as_paid_only_the_cash_collected() {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::invoice;

    let (_setup, services, store, invoice) = setup_unpaid_invoice().await;
    let draft = insert_plain_draft(&store, &invoice).await;
    let mut conn = store.pool.get().await.unwrap();
    // A negative draft (R7): nothing is due on it.
    diesel::update(invoice::table.filter(invoice::id.eq(draft.id)))
        .set((invoice::total.eq(-1000_i64), invoice::amount_due.eq(0_i64)))
        .execute(&mut conn)
        .await
        .unwrap();

    let edited = services
        .update_draft_invoice(draft.id, TENANT_ID, discount_of("0".to_string()))
        .await
        .unwrap()
        .invoice;
    assert!(edited.total > 0);
    assert_eq!(edited.amount_due, edited.total);
}

/// R-050: a finalized invoice's lines are never written, whoever calls.
#[tokio::test]
async fn a_finalized_invoice_refuses_a_lines_write() {
    let (_setup, _services, store, invoice) = setup_unpaid_invoice().await;
    let mut conn = store.pool.get().await.unwrap();

    let written = lines_patch(&invoice)
        .update_lines(invoice.id, TENANT_ID, &mut conn)
        .await;
    assert!(written.is_err(), "a finalized invoice's lines were written");
}

/// R-088: a void is stamped at the instant it is made at, as its credit note and ledger are.
#[tokio::test]
async fn a_void_is_stamped_at_its_business_instant() {
    let (_setup, _services, store, invoice) = setup_unpaid_invoice().await;
    let at = chrono::Utc.with_ymd_and_hms(2025, 3, 4, 5, 6, 7).unwrap();

    store
        .void_invoice_at(Actor::System, invoice.id, TENANT_ID, at)
        .await
        .unwrap();

    let mut conn = store.pool.get().await.unwrap();
    let voided = InvoiceRow::find_by_id(&mut conn, TENANT_ID, invoice.id)
        .await
        .unwrap();
    assert_eq!(voided.voided_at, Some(at.naive_utc()));
}
