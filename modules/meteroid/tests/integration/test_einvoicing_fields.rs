//! The fields XRechnung asks of us — the seller contact and the buyer reference — are
//! stored, cleared when blank, and frozen onto a document with the rest of its parties.

use chrono::NaiveDate;
use common_domain::actor::Actor;
use common_domain::ids::CustomerId;
use rstest::rstest;

use meteroid_store::domain::{CustomerPatch, EInvoicingFormat, InvoicingEntityPatch};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::customers::CustomersInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

fn buyer_reference_patch(id: CustomerId, value: &str) -> CustomerPatch {
    CustomerPatch {
        id,
        name: None,
        alias: None,
        billing_email: None,
        invoicing_emails: None,
        phone: None,
        currency: None,
        billing_address: None,
        shipping_address: None,
        invoicing_entity_id: None,
        vat_number: None,
        legal_number: None,
        buyer_reference: Some(Some(value.to_string())),
        custom_taxes: None,
        current_payment_method_id: None,
        tax_status: None,
        exemption_reason: None,
        connected_account_id: None,
        custom_properties: None,
        preferred_locales: None,
        customer_type: None,
        first_name: None,
        last_name: None,
    }
}

#[rstest]
#[tokio::test]
async fn the_xrechnung_fields_are_snapshotted_onto_the_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;

    let entity = env
        .store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                einvoicing_format: Some(Some(EInvoicingFormat::XRechnung)),
                contact_name: Some(Some("Buchhaltung".to_string())),
                contact_phone: Some(Some("+49 30 1234567".to_string())),
                contact_email: Some(Some("billing@example.com".to_string())),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("patch invoicing entity");
    assert_eq!(entity.einvoicing_format, Some(EInvoicingFormat::XRechnung));
    assert_eq!(entity.contact_phone.as_deref(), Some("+49 30 1234567"));

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let customer_id = env.get_subscription(sub_id).await.customer_id;
    env.store()
        .patch_customer(
            Actor::System,
            TENANT_ID,
            buyer_reference_patch(customer_id, "991-01234-56"),
        )
        .await
        .expect("patch customer");

    let draft = env
        .insert_draft_invoice(sub_id, NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
        .await;
    let stored = env
        .store()
        .get_invoice_by_id(TENANT_ID, draft.id)
        .await
        .expect("get invoice");

    assert_eq!(
        stored.customer_details.buyer_reference.as_deref(),
        Some("991-01234-56")
    );
    let seller = &stored.seller_details;
    assert_eq!(seller.contact_name.as_deref(), Some("Buchhaltung"));
    assert_eq!(seller.contact_phone.as_deref(), Some("+49 30 1234567"));
    assert_eq!(seller.contact_email.as_deref(), Some("billing@example.com"));
}

#[rstest]
#[tokio::test]
async fn a_blank_contact_clears_it(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let patch = |email: &str| InvoicingEntityPatch {
        id: INVOICING_ENTITY_ID,
        contact_email: Some(Some(email.to_string())),
        ..Default::default()
    };

    env.store()
        .patch_invoicing_entity(Actor::System, patch("billing@example.com"), TENANT_ID)
        .await
        .expect("set");
    let cleared = env
        .store()
        .patch_invoicing_entity(Actor::System, patch("  "), TENANT_ID)
        .await
        .expect("clear");

    assert_eq!(cleared.contact_email, None);
}
