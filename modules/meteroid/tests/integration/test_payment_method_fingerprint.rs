//! Payment-method fingerprint dedup: re-adding the same card/mandate under a new provider id
//! updates the existing row instead of creating a duplicate.

use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{TestEnv, test_env};
use common_domain::ids::{BaseId, CustomerPaymentMethodId};
use meteroid_store::domain::{CustomerPaymentMethodNew, PaymentMethodTypeEnum};
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;

fn method(
    external_id: &str,
    fingerprint: Option<&str>,
    payment_method_type: PaymentMethodTypeEnum,
    exp_year: i32,
) -> CustomerPaymentMethodNew {
    CustomerPaymentMethodNew {
        id: CustomerPaymentMethodId::new(),
        tenant_id: TENANT_ID,
        customer_id: CUST_UBER_ID,
        connection_id: CUST_UBER_CONNECTION_ID,
        external_payment_method_id: external_id.to_string(),
        payment_method_type,
        account_number_hint: None,
        card_brand: Some("visa".to_string()),
        card_last4: Some("4242".to_string()),
        card_exp_month: Some(12),
        card_exp_year: Some(exp_year),
        fingerprint: fingerprint.map(str::to_string),
    }
}

#[rstest]
#[tokio::test]
async fn test_same_fingerprint_reuses_row_and_adopts_new_provider_id(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let store = env.store();

    let first = store
        .upsert_payment_method(method(
            "pm_a",
            Some("fp_1"),
            PaymentMethodTypeEnum::Card,
            2030,
        ))
        .await
        .unwrap();
    assert_ne!(first.id, CUST_UBER_PAYMENT_METHOD_ID);

    // Same card re-added: new provider id, same fingerprint.
    let readded = store
        .upsert_payment_method(method(
            "pm_b",
            Some("fp_1"),
            PaymentMethodTypeEnum::Card,
            2032,
        ))
        .await
        .unwrap();
    assert_eq!(readded.id, first.id);
    assert_eq!(readded.external_payment_method_id, "pm_b");
    assert_eq!(readded.card_exp_year, Some(2032));

    // Redelivery of the same provider id is a plain refresh.
    let redelivered = store
        .upsert_payment_method(method(
            "pm_b",
            Some("fp_1"),
            PaymentMethodTypeEnum::Card,
            2033,
        ))
        .await
        .unwrap();
    assert_eq!(redelivered.id, first.id);
    assert_eq!(redelivered.card_exp_year, Some(2033));

    // insert-if-not-exist keeps what is stored for a known provider id...
    let kept = store
        .insert_payment_method_if_not_exist(method(
            "pm_b",
            Some("fp_1"),
            PaymentMethodTypeEnum::Card,
            2040,
        ))
        .await
        .unwrap();
    assert_eq!(kept.id, first.id);
    assert_eq!(kept.card_exp_year, Some(2033));

    // ...but still folds a new provider id with a known fingerprint.
    let folded = store
        .insert_payment_method_if_not_exist(method(
            "pm_c",
            Some("fp_1"),
            PaymentMethodTypeEnum::Card,
            2041,
        ))
        .await
        .unwrap();
    assert_eq!(folded.id, first.id);
    assert_eq!(folded.external_payment_method_id, "pm_c");

    let methods = store
        .list_payment_methods_by_customer(&TENANT_ID, &CUST_UBER_ID)
        .await
        .unwrap();
    // Seeded legacy card (no fingerprint) + the single deduplicated row.
    assert_eq!(methods.len(), 2);
}

#[rstest]
#[tokio::test]
async fn test_fingerprint_dedup_is_scoped_by_type_and_skips_archived(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let store = env.store();

    let card = store
        .upsert_payment_method(method(
            "pm_a",
            Some("fp_1"),
            PaymentMethodTypeEnum::Card,
            2030,
        ))
        .await
        .unwrap();

    let other_card = store
        .upsert_payment_method(method(
            "pm_b",
            Some("fp_2"),
            PaymentMethodTypeEnum::Card,
            2030,
        ))
        .await
        .unwrap();
    assert_ne!(other_card.id, card.id);

    let sepa = store
        .upsert_payment_method(method(
            "pm_c",
            Some("fp_1"),
            PaymentMethodTypeEnum::DirectDebitSepa,
            2030,
        ))
        .await
        .unwrap();
    assert_ne!(sepa.id, card.id);

    // Methods without a fingerprint (GoCardless, Stancer, legacy rows) never fold.
    let no_fp_1 = store
        .upsert_payment_method(method("pm_d", None, PaymentMethodTypeEnum::Card, 2030))
        .await
        .unwrap();
    let no_fp_2 = store
        .upsert_payment_method(method("pm_e", None, PaymentMethodTypeEnum::Card, 2030))
        .await
        .unwrap();
    assert_ne!(no_fp_1.id, no_fp_2.id);

    // A revoked method stays archived; re-adding the same card starts a fresh row.
    let detached = store
        .detach_payment_method_by_external_id(TENANT_ID, "pm_a")
        .await
        .unwrap();
    assert_eq!(detached, Some(card.id));

    let readded = store
        .upsert_payment_method(method(
            "pm_f",
            Some("fp_1"),
            PaymentMethodTypeEnum::Card,
            2030,
        ))
        .await
        .unwrap();
    assert_ne!(readded.id, card.id);
    assert_eq!(readded.external_payment_method_id, "pm_f");

    let archived = store
        .get_payment_method_by_id(&TENANT_ID, &card.id)
        .await
        .unwrap();
    assert!(archived.archived_at.is_some());
    assert_eq!(archived.external_payment_method_id, "pm_a");
}

/// Portal add and provider webhook race for one method, plus re-adds under new provider ids:
/// every writer succeeds and ends on one row.
#[rstest]
#[tokio::test]
async fn test_concurrent_writers_converge_on_one_row(#[future] test_env: TestEnv) {
    use std::collections::HashSet;

    let env = test_env.await;
    env.seed_payments().await;
    let store = env.store().clone();

    let same_provider_id = (0..8).map(|_| {
        let store = store.clone();
        async move {
            store
                .upsert_payment_method(method(
                    "pm_race",
                    Some("fp_race"),
                    PaymentMethodTypeEnum::Card,
                    2030,
                ))
                .await
        }
    });
    let ids: HashSet<_> = futures::future::join_all(same_provider_id)
        .await
        .into_iter()
        .map(|r| {
            r.expect("concurrent upsert of the same provider id must not fail")
                .id
        })
        .collect();
    assert_eq!(ids.len(), 1);
    let row_id = *ids.iter().next().unwrap();

    let distinct_provider_ids = (0..8).map(|i| {
        let store = store.clone();
        async move {
            store
                .upsert_payment_method(method(
                    &format!("pm_race_{i}"),
                    Some("fp_race"),
                    PaymentMethodTypeEnum::Card,
                    2031,
                ))
                .await
        }
    });
    let ids: HashSet<_> = futures::future::join_all(distinct_provider_ids)
        .await
        .into_iter()
        .map(|r| {
            r.expect("concurrent re-adds of the same card must not fail")
                .id
        })
        .collect();
    assert_eq!(ids, HashSet::from([row_id]));

    let methods = store
        .list_payment_methods_by_customer(&TENANT_ID, &CUST_UBER_ID)
        .await
        .unwrap();
    assert_eq!(methods.len(), 2);
}
