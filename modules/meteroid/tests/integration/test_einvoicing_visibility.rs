//! Where a failed e-invoice can be found once it has happened: the invoices list and the
//! documents webhook. The pre-finalization check has its own coverage.

use chrono::NaiveDate;
use common_domain::ids::{BaseId, StoredDocumentId};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use rstest::rstest;

use meteroid_store::domain::PaginationRequest;
use meteroid_store::domain::einvoicing::{EInvoicingFinding, EInvoicingOutcome, EInvoicingStatus};
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::domain::stored_object::StoredObjectRef;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

async fn failed_invoices(env: &TestEnv, status: Option<EInvoicingStatus>) -> Vec<String> {
    env.store()
        .list_invoices(
            TENANT_ID,
            None,
            None,
            None,
            None,
            status,
            false,
            None,
            vec![],
            None,
            PaginationRequest {
                page: 0,
                per_page: None,
            },
        )
        .await
        .expect("list invoices")
        .items
        .into_iter()
        .map(|i| i.invoice.id.to_string())
        .collect()
}

/// A failed e-invoice is something an operator has to go looking for, so it must be
/// reachable from the list without opening each invoice, and pushed to integrators
/// on the documents webhook rather than left in the database.
#[rstest]
#[tokio::test]
async fn failed_einvoicing_is_listable_and_reported_on_the_documents_webhook(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let draft = env
        .insert_draft_invoice(sub_id, NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
        .await;

    let pdf_id = StoredDocumentId::new();
    let outcome = EInvoicingOutcome::failed(
        "EN 16931",
        vec![EInvoicingFinding {
            rule: "BR-11".to_string(),
            term: "BG-8/BT-55".to_string(),
            message: "The Buyer postal address shall contain a Buyer country code.".to_string(),
            hint: None,
        }],
        None,
    );
    env.store()
        .save_invoice_documents(
            draft.id,
            TENANT_ID,
            draft.customer_id,
            StoredObjectRef {
                id: pdf_id,
                key: format!("{}/invoice_pdf/{}", TENANT_ID, pdf_id.as_uuid()),
            },
            None,
            Some(outcome),
        )
        .await
        .expect("save invoice documents");

    let failed = failed_invoices(&env, Some(EInvoicingStatus::Failed)).await;
    assert_eq!(
        failed,
        vec![draft.id.to_string()],
        "the failed filter finds it"
    );
    let generated = failed_invoices(&env, Some(EInvoicingStatus::Generated)).await;
    assert!(
        generated.is_empty(),
        "the generated filter does not, got {generated:?}"
    );

    // The REST list runs its own query, so its filter is covered separately.
    for (status, expected) in [
        (EInvoicingStatus::Failed, vec![draft.id]),
        (EInvoicingStatus::Generated, vec![]),
    ] {
        let listed: Vec<_> = env
            .store()
            .list_full_invoices(
                TENANT_ID,
                None,
                None,
                None,
                Some(status),
                None,
                None,
                PaginationRequest {
                    page: 0,
                    per_page: None,
                },
            )
            .await
            .expect("list full invoices")
            .items
            .into_iter()
            .map(|(i, _)| i.invoice.id)
            .collect();
        assert_eq!(listed, expected, "REST list filtered on {status:?}");
    }

    let msgs = env
        .store()
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(200),
            MessageReadVtSec(0),
        )
        .await
        .expect("read webhook_out queue");
    let event = msgs
        .into_iter()
        .filter_map(|m| m.message)
        .filter_map(|m| serde_json::from_value::<OutboxEvent>(m.0).ok())
        .find_map(|e| match e {
            OutboxEvent::InvoiceAccountingPdfGenerated(e) if e.invoice_id == draft.id => Some(e),
            _ => None,
        })
        .expect("the documents event reaches the webhook queue");
    assert_eq!(event.pdf_id, pdf_id);
    assert_eq!(event.einvoicing_status, Some(EInvoicingStatus::Failed));
    let report = event
        .einvoicing_report
        .expect("the report travels with the event");
    assert_eq!(report.profile, "EN 16931");
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].rule, "BR-11");
}
