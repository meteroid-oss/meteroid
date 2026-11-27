//! A change's draft adjustment is never deleted, only issued and voided (R-124, R-166): its notes
//! give back what it replaces, at the change or once the period's draft is issued.
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};
use chrono::NaiveDate;
use common_domain::actor::Actor;
use common_domain::ids::{InvoiceId, SubscriptionId};
use meteroid_store::domain::PaginationRequest;
use meteroid_store::domain::enums::{InvoiceStatusEnum, InvoiceType};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::InvoiceInterface;
use rstest::rstest;

async fn started(env: &TestEnv) -> SubscriptionId {
    subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await
}

async fn invoice_of(env: &TestEnv, sub: SubscriptionId, kind: InvoiceType) -> InvoiceId {
    env.store()
        .list_invoices(
            TENANT_ID,
            None,
            Some(sub),
            None,
            None,
            None,
            false,
            None,
            vec![],
            None,
            PaginationRequest {
                per_page: Some(100),
                page: 0,
            },
        )
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|row| row.invoice)
        .find(|invoice| invoice.invoice_type == kind)
        .unwrap_or_else(|| panic!("no {kind:?} invoice"))
        .id
}

async fn upgrade(env: &TestEnv, sub: SubscriptionId) -> InvoiceId {
    env.services()
        .apply_plan_change_immediate_at(
            Actor::System,
            sub,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
        )
        .await
        .expect("upgrade")
        .adjustment_invoice_id
        .expect("the upgrade bills the new plan")
}

#[rstest]
#[tokio::test]
async fn test_a_change_draft_whose_notes_were_issued_is_voided_not_deleted(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sub = started(&env).await;
    let january = invoice_of(&env, sub, InvoiceType::Recurring).await;
    env.services()
        .finalize_invoice(Actor::System, january, TENANT_ID)
        .await
        .expect("January issued");
    let adjustment = upgrade(&env, sub).await;
    let notes = env
        .store()
        .list_credit_notes_by_invoice_id(TENANT_ID, january)
        .await
        .unwrap();
    assert_eq!(
        notes.len(),
        1,
        "the upgrade gives back January's unused days"
    );

    let refused = env
        .store()
        .delete_invoice(Actor::System, adjustment, TENANT_ID)
        .await
        .expect_err("its notes were issued");
    assert!(
        format!("{refused:?}").contains("already in effect"),
        "{refused:?}"
    );

    env.services()
        .finalize_invoice(Actor::System, adjustment, TENANT_ID)
        .await
        .expect("issued");
    let voided = env
        .store()
        .void_invoice(Actor::System, adjustment, TENANT_ID)
        .await
        .expect("voided");
    assert_eq!(voided.invoice.status, InvoiceStatusEnum::Void);
}

#[rstest]
#[tokio::test]
async fn test_a_change_draft_is_refused_before_its_notes_are_issued(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sub = started(&env).await;
    let adjustment = upgrade(&env, sub).await;
    let refused = env
        .store()
        .delete_invoice(Actor::System, adjustment, TENANT_ID)
        .await
        .expect_err("January's draft gives back A's unused days once issued");
    assert!(
        format!("{refused:?}").contains("already in effect"),
        "{refused:?}"
    );
}
