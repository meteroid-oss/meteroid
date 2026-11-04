//! Payment connector lifecycle: drafts, activation, disable/enable, soft delete, credential
//! rotation and routing rules. Uses the mock provider, whose credential check reports a
//! configurable account without calling out.

use crate::data::ids::{
    CUST_SPOTIFY_ID, INVOICING_ENTITY_ID, PLAN_VERSION_1_LEETCODE_ID, TENANT_ID,
};
use crate::data::payment::get_or_create_customer_connection;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, CheckoutSessionId, ConnectorId, CustomerConnectionId, CustomerPaymentMethodId,
    PaymentTransactionId,
};
use diesel_models::checkout_sessions::CheckoutSessionRowNew;
use diesel_models::enums::{CheckoutTypeEnum, PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::payments::PaymentTransactionRowNew;
use meteroid_store::Store;
use meteroid_store::domain::InvoicingEntityProvidersPatch;
use meteroid_store::domain::connectors::{MethodState, MockPublicData, PaymentCredentialsUpdate};
use meteroid_store::domain::enums::{ConnectorStatusEnum, PaymentMethodTypeEnum, PaymentProvider};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;

fn mock_credentials(
    account_id: &str,
    pending: &[PaymentMethodTypeEnum],
) -> PaymentCredentialsUpdate {
    PaymentCredentialsUpdate::Mock(MockPublicData {
        account_id: Some(account_id.to_string()),
        pending_methods: pending.to_vec(),
        ..Default::default()
    })
}

/// Every direct-debit rail pending: the mock connector can take cards only.
const DEBITS: [PaymentMethodTypeEnum; 3] = [
    PaymentMethodTypeEnum::DirectDebitSepa,
    PaymentMethodTypeEnum::DirectDebitAch,
    PaymentMethodTypeEnum::DirectDebitBacs,
];

async fn active_mock_connector(store: &Store, alias: &str, account_id: &str) -> ConnectorId {
    let draft = store
        .create_payment_provider_draft(TENANT_ID, alias.to_string(), PaymentProvider::Mock)
        .await
        .unwrap();
    store
        .update_payment_provider_credentials(
            Actor::System,
            TENANT_ID,
            draft.id,
            mock_credentials(account_id, &DEBITS),
        )
        .await
        .unwrap();
    store
        .activate_connector(Actor::System, TENANT_ID, draft.id)
        .await
        .unwrap();
    draft.id
}

fn assert_precondition<T: std::fmt::Debug>(result: meteroid_store::StoreResult<T>, contains: &str) {
    let err = result.expect_err("expected a failed precondition");
    match err.current_context() {
        StoreError::FailedPrecondition(msg) => {
            assert!(msg.contains(contains), "unexpected message: {msg}")
        }
        other => panic!("expected FailedPrecondition, got {other:?}"),
    }
}

#[tokio::test]
async fn test_connector_lifecycle_from_draft_to_soft_delete() {
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;
    let store = &setup.store;

    let draft = store
        .create_payment_provider_draft(TENANT_ID, "mock-eu".to_string(), PaymentProvider::Mock)
        .await
        .unwrap();
    assert_eq!(draft.status, ConnectorStatusEnum::Draft);
    assert_eq!(
        draft.webhook_route,
        Some(meteroid_store::domain::enums::ConnectorWebhookRouteEnum::ConnectorId)
    );

    // Nothing verified yet.
    assert_precondition(
        store
            .activate_connector(Actor::System, TENANT_ID, draft.id)
            .await,
        "Verify the credentials",
    );

    let with_credentials = store
        .update_payment_provider_credentials(
            Actor::System,
            TENANT_ID,
            draft.id,
            mock_credentials("acct_a", &DEBITS),
        )
        .await
        .unwrap();
    assert_eq!(with_credentials.status, ConnectorStatusEnum::Draft);
    let account = with_credentials.account().expect("account detected");
    assert_eq!(account.account_id.as_deref(), Some("acct_a"));
    assert_eq!(
        with_credentials.method_state(&PaymentMethodTypeEnum::DirectDebitSepa),
        MethodState::Pending
    );
    assert!(with_credentials.supports_card());
    assert!(!with_credentials.supports_direct_debit());

    // The name can change while setting up, not once live.
    let renamed = store
        .rename_draft_connector(TENANT_ID, draft.id, "mock-eu-2".to_string())
        .await
        .unwrap();
    assert_eq!(renamed.alias, "mock-eu-2");

    let active = store
        .activate_connector(Actor::System, TENANT_ID, draft.id)
        .await
        .unwrap();
    assert_eq!(active.status, ConnectorStatusEnum::Active);
    assert_precondition(
        store
            .rename_draft_connector(TENANT_ID, draft.id, "mock-eu-3".to_string())
            .await,
        "while it's being set up",
    );

    let disabled = store
        .disable_connector(Actor::System, TENANT_ID, draft.id)
        .await
        .unwrap();
    assert_eq!(disabled.status, ConnectorStatusEnum::Disabled);
    assert!(disabled.ensure_active().is_err());

    let enabled = store
        .enable_connector(Actor::System, TENANT_ID, draft.id)
        .await
        .unwrap();
    assert_eq!(enabled.status, ConnectorStatusEnum::Active);

    store
        .delete_connector(Actor::System, draft.id, TENANT_ID)
        .await
        .unwrap();

    let listed = store.list_connectors(None, TENANT_ID).await.unwrap();
    assert!(
        listed.iter().all(|c| c.id != draft.id),
        "deleted connectors are hidden"
    );

    // The row stays for payment history, without its secrets.
    let deleted = store
        .get_connector_with_data(draft.id, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(deleted.status, ConnectorStatusEnum::Deleted);
    assert!(deleted.payment_credentials().is_none());

    // Deleting again is a no-op; the alias is free again.
    store
        .delete_connector(Actor::System, draft.id, TENANT_ID)
        .await
        .unwrap();
    store
        .create_payment_provider_draft(TENANT_ID, "mock-eu".to_string(), PaymentProvider::Mock)
        .await
        .expect("a deleted connector's alias can be reused");
}

#[tokio::test]
async fn test_discarding_a_draft_removes_it() {
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;
    let store = &setup.store;

    let draft = store
        .create_payment_provider_draft(TENANT_ID, "mock-tmp".to_string(), PaymentProvider::Mock)
        .await
        .unwrap();
    store
        .delete_connector(Actor::System, draft.id, TENANT_ID)
        .await
        .unwrap();

    assert!(
        store
            .get_connector_with_data(draft.id, TENANT_ID)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn test_delete_is_refused_while_a_payment_is_in_flight() {
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;
    let store = &setup.store;

    let connector_id = active_mock_connector(store, "mock-busy", "acct_a").await;

    let connection_id = get_or_create_customer_connection(
        &store.pool,
        CUST_SPOTIFY_ID,
        CustomerConnectionId::new(),
        connector_id,
        "mock_cus_spotify",
    )
    .await;
    let payment_method_id = CustomerPaymentMethodId::new();
    crate::data::payment::create_customer_payment_method(
        &store.pool,
        CUST_SPOTIFY_ID,
        connection_id,
        payment_method_id,
        "mock_pm_busy",
    )
    .await;

    {
        let mut conn = store.pool.get().await.unwrap();
        let checkout_session_id = CheckoutSessionId::new();
        CheckoutSessionRowNew {
            id: checkout_session_id,
            tenant_id: TENANT_ID,
            customer_id: CUST_SPOTIFY_ID,
            plan_version_id: Some(PLAN_VERSION_1_LEETCODE_ID),
            billing_start_date: None,
            billing_day_anchor: None,
            net_terms: None,
            trial_duration_days: None,
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            payment_methods_config: None,
            components: None,
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_at: None,
            metadata: None,
            checkout_type: CheckoutTypeEnum::SelfServe,
            subscription_id: None,
            change_date: None,
            success_url: None,
            cancel_url: None,
        }
        .insert(&mut conn)
        .await
        .unwrap();

        PaymentTransactionRowNew {
            id: PaymentTransactionId::new(),
            tenant_id: TENANT_ID,
            invoice_id: None,
            provider_transaction_id: Some("mock_pi_busy".to_string()),
            amount: 10_000,
            currency: "usd".to_string(),
            payment_method_id: Some(payment_method_id),
            status: PaymentStatusEnum::Pending,
            payment_type: PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: None,
            checkout_session_id: Some(checkout_session_id),
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        }
        .insert(&mut conn)
        .await
        .unwrap();
    }

    let usage = store
        .connector_usage(TENANT_ID, connector_id)
        .await
        .unwrap();
    assert_eq!(usage.in_flight_transactions, 1);
    assert_eq!(usage.customers_with_saved_methods, 1);

    assert_precondition(
        store
            .delete_connector(Actor::System, connector_id, TENANT_ID)
            .await,
        "still in progress",
    );

    // Disabling is the way out, and the in-flight payment keeps the connector alive.
    store
        .disable_connector(Actor::System, TENANT_ID, connector_id)
        .await
        .unwrap();
    assert_precondition(
        store
            .delete_connector(Actor::System, connector_id, TENANT_ID)
            .await,
        "still in progress",
    );
}

/// Rotation isn't refused when the keys belong to another account; the new account is recorded
/// so the credentials modal can show it.
#[tokio::test]
async fn test_rotating_credentials_records_the_new_account() {
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PRODUCT).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PRODUCT)
            .await;
    let store = &setup.store;

    let connector_id = active_mock_connector(store, "mock-rotate", "acct_a").await;
    get_or_create_customer_connection(
        &store.pool,
        CUST_SPOTIFY_ID,
        CustomerConnectionId::new(),
        connector_id,
        "mock_cus_spotify",
    )
    .await;

    let rotated = store
        .update_payment_provider_credentials(
            Actor::System,
            TENANT_ID,
            connector_id,
            mock_credentials("acct_b", &[]),
        )
        .await
        .unwrap();
    assert_eq!(rotated.status, ConnectorStatusEnum::Active);
    let account = rotated.account().unwrap();
    assert_eq!(account.account_id.as_deref(), Some("acct_b"));
    // The rotation also refreshed the enabled methods.
    assert!(rotated_supports_debit(account));
}

fn rotated_supports_debit(account: &meteroid_store::domain::connectors::ProviderAccount) -> bool {
    account.methods.iter().any(|m| {
        m.method == PaymentMethodTypeEnum::DirectDebitSepa && m.state == MethodState::Enabled
    })
}

#[tokio::test]
async fn test_routing_requires_an_active_connector_with_the_rail_enabled() {
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;
    let store = &setup.store;

    let connector_id = active_mock_connector(store, "mock-route", "acct_a").await;
    let patch = |card: Option<ConnectorId>, direct_debit: Option<ConnectorId>| {
        InvoicingEntityProvidersPatch {
            id: INVOICING_ENTITY_ID,
            card_provider_id: Some(card),
            direct_debit_provider_id: Some(direct_debit),
            bank_account_id: Some(None),
        }
    };

    // Direct debit is pending on the account.
    let err = store
        .patch_invoicing_entity_providers(Actor::System, patch(None, Some(connector_id)), TENANT_ID)
        .await
        .expect_err("a pending rail can't be routed");
    assert!(matches!(
        err.current_context(),
        StoreError::InvalidArgument(_)
    ));

    store
        .patch_invoicing_entity_providers(Actor::System, patch(Some(connector_id), None), TENANT_ID)
        .await
        .unwrap();

    store
        .disable_connector(Actor::System, TENANT_ID, connector_id)
        .await
        .unwrap();

    // Saving with the disabled connector left in its slot still works.
    store
        .patch_invoicing_entity_providers(Actor::System, patch(Some(connector_id), None), TENANT_ID)
        .await
        .unwrap();

    // Assigning it anew does not.
    store
        .patch_invoicing_entity_providers(Actor::System, patch(None, None), TENANT_ID)
        .await
        .unwrap();
    let err = store
        .patch_invoicing_entity_providers(Actor::System, patch(Some(connector_id), None), TENANT_ID)
        .await
        .expect_err("a disabled connector can't be newly routed");
    assert!(matches!(
        err.current_context(),
        StoreError::InvalidArgument(_)
    ));

    // Deleting clears any routing to it.
    store
        .enable_connector(Actor::System, TENANT_ID, connector_id)
        .await
        .unwrap();
    store
        .patch_invoicing_entity_providers(Actor::System, patch(Some(connector_id), None), TENANT_ID)
        .await
        .unwrap();
    store
        .delete_connector(Actor::System, connector_id, TENANT_ID)
        .await
        .unwrap();
    let providers = store
        .resolve_providers_by_id(TENANT_ID, INVOICING_ENTITY_ID)
        .await
        .unwrap();
    assert!(providers.card_provider.is_none());
}

/// Hosted checkouts (GoCardless) record neither a payment method nor a connection until
/// fulfilled; delete must still see them.
#[tokio::test]
async fn test_delete_sees_a_pending_hosted_checkout() {
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;
    let store = &setup.store;

    let connector_id = active_mock_connector(store, "mock-hosted", "acct_a").await;
    get_or_create_customer_connection(
        &store.pool,
        CUST_SPOTIFY_ID,
        CustomerConnectionId::new(),
        connector_id,
        "mock_cus_spotify",
    )
    .await;

    {
        let mut conn = store.pool.get().await.unwrap();
        let checkout_session_id = CheckoutSessionId::new();
        CheckoutSessionRowNew {
            id: checkout_session_id,
            tenant_id: TENANT_ID,
            customer_id: CUST_SPOTIFY_ID,
            plan_version_id: Some(PLAN_VERSION_1_LEETCODE_ID),
            billing_start_date: None,
            billing_day_anchor: None,
            net_terms: None,
            trial_duration_days: None,
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            payment_methods_config: None,
            components: None,
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_at: None,
            metadata: None,
            checkout_type: CheckoutTypeEnum::SelfServe,
            subscription_id: None,
            change_date: None,
            success_url: None,
            cancel_url: None,
        }
        .insert(&mut conn)
        .await
        .unwrap();

        PaymentTransactionRowNew {
            id: PaymentTransactionId::new(),
            tenant_id: TENANT_ID,
            invoice_id: None,
            provider_transaction_id: None,
            amount: 10_000,
            currency: "usd".to_string(),
            payment_method_id: None,
            status: PaymentStatusEnum::Pending,
            payment_type: PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: None,
            checkout_session_id: Some(checkout_session_id),
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: Some(CUST_SPOTIFY_ID),
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        }
        .insert(&mut conn)
        .await
        .unwrap();
    }

    assert_precondition(
        store
            .delete_connector(Actor::System, connector_id, TENANT_ID)
            .await,
        "still in progress",
    );
}

/// A key the provider refuses flags the connection without changing its status; credentials the
/// provider accepts clear the flag.
#[tokio::test]
async fn test_rejected_credentials_are_flagged_until_fixed() {
    use diesel_models::connectors::ConnectorRowPatch;

    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;
    let store = &setup.store;

    let connector_id = active_mock_connector(store, "mock-revoked", "acct_a").await;

    // The key is revoked at the provider.
    {
        let mut conn = store.pool.get().await.unwrap();
        ConnectorRowPatch {
            id: connector_id,
            alias: None,
            sensitive: None,
            account: None,
            updated_at: None,
            data: Some(Some(serde_json::json!({
                "Mock": { "account_id": "acct_a", "reject_credentials": true }
            }))),
        }
        .patch(&mut conn, TENANT_ID)
        .await
        .unwrap();
    }

    let _ = store
        .check_connector(TENANT_ID, connector_id)
        .await
        .expect_err("the provider refuses the stored key");
    let flagged = store
        .get_connector_with_data(connector_id, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(
        flagged.status,
        ConnectorStatusEnum::Active,
        "status is untouched"
    );
    assert!(
        flagged
            .credentials_error()
            .as_ref()
            .is_some_and(|e| e.message.contains("rejected")),
        "the rejection is recorded: {:?}",
        flagged.credentials_error()
    );

    // New credentials that the provider accepts clear it.
    let fixed = store
        .update_payment_provider_credentials(
            Actor::System,
            TENANT_ID,
            connector_id,
            mock_credentials("acct_a", &DEBITS),
        )
        .await
        .unwrap();
    assert!(fixed.credentials_error().is_none());
}
