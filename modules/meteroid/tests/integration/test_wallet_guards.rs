//! Promo-code tries never fail open, and the portal's top-up paths end where the customer can act.
use crate::data::ids::{CUST_UBER_ID, CUST_UBER_PAYMENT_METHOD_ID, TENANT_ID};
use crate::data::payment::{
    run_customer_payment_methods_seed, run_mock_payment_provider_seed, set_mock_charge_behavior,
};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use common_domain::ids::{CustomerId, WalletId};
use meteroid_store::adapters::payment::error::CustomerFacingMessage;
use meteroid_store::domain::connectors::MockChargeBehavior;
use meteroid_store::domain::credit::TopUpWallet;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::services::promo_code_attempts::{
    PROMO_CODE_ATTEMPTS, PROMO_CODE_WINDOW_MINUTES, PromoCodeTry,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn test_error(error: impl std::fmt::Debug) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

async fn setup() -> MeteroidSetup {
    crate::helpers::init::logging();
    let database =
        crate::meteroid_it::container::create_seeded_test_database(SeedLevel::PRODUCT).await;
    let setup = crate::meteroid_it::container::start_meteroid(database, SeedLevel::PRODUCT).await;
    run_mock_payment_provider_seed(&setup.store.pool, false).await;
    run_customer_payment_methods_seed(&setup.store.pool).await;
    setup
}

async fn wallet(setup: &MeteroidSetup, customer: CustomerId) -> TestResult<WalletId> {
    let currency = setup
        .store
        .find_customer_by_id(customer, TENANT_ID)
        .await
        .map_err(test_error)?
        .currency;
    Ok(setup
        .store
        .create_customer_credit_wallet(TENANT_ID, customer, &currency)
        .await
        .map_err(test_error)?
        .id)
}

/// RB-14: without Redis the code tries are counted in Postgres: a burst of five failed tries, then
/// refused until the oldest leaves the window; a usable code gives its try back.
#[tokio::test]
async fn promo_code_tries_are_counted_in_postgres() -> TestResult {
    let setup = setup().await;
    let mut taken = Vec::new();
    for _ in 0..PROMO_CODE_ATTEMPTS {
        match setup
            .services
            .take_promo_code_attempt(TENANT_ID, CUST_UBER_ID)
            .await
            .map_err(test_error)?
        {
            PromoCodeTry::Taken(id) => taken.push(id),
            PromoCodeTry::Limited { .. } => return Err("refused within the budget".into()),
        }
    }
    let PromoCodeTry::Limited {
        retry_after_seconds,
    } = setup
        .services
        .take_promo_code_attempt(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
    else {
        return Err("a sixth try passed".into());
    };
    assert!((1..=PROMO_CODE_WINDOW_MINUTES * 60).contains(&retry_after_seconds));
    setup
        .services
        .release_promo_code_attempt(TENANT_ID, CUST_UBER_ID, taken[0])
        .await
        .map_err(test_error)?;
    assert!(matches!(
        setup
            .services
            .take_promo_code_attempt(TENANT_ID, CUST_UBER_ID)
            .await
            .map_err(test_error)?,
        PromoCodeTry::Taken(_)
    ));
    Ok(())
}

/// RB-7: a declined portal top-up tells the customer only the decline class, while the operator
/// and REST keep the provider's reason; the hosted checkout returns to the portal.
#[tokio::test]
async fn a_declined_top_up_reads_safely_and_checkout_returns_to_the_portal() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Failed).await;
    let declined = setup
        .services
        .top_up_wallet(
            TopUpWallet {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id: wallet,
                amount_minor: 5_000,
                payment_method_id: CUST_UBER_PAYMENT_METHOD_ID,
                idempotency_key: "declined".to_owned(),
                expires_at: None,
                unit_price: None,
                expected_unit_price: None,
            },
            None,
        )
        .await
        .expect_err("declined");
    assert!(matches!(
        declined.current_context(),
        StoreError::FailedPrecondition(_)
    ));
    let customer_message = declined
        .frames()
        .find_map(|frame| frame.downcast_ref::<CustomerFacingMessage>())
        .ok_or("a customer-facing message")?;
    assert!(
        customer_message.0.starts_with("Your "),
        "{}",
        customer_message.0
    );

    // The portal's own page, stored without the token a checkout session's readers would see.
    let portal = setup
        .store
        .settings
        .public_url
        .trim_end_matches('/')
        .to_owned();
    let session = setup
        .services
        .create_portal_top_up_checkout(
            TENANT_ID,
            CUST_UBER_ID,
            wallet,
            5_000,
            "new-card".to_owned(),
            None,
            Some(format!("{portal}/portal?token=abc&page=balance")),
        )
        .await
        .map_err(test_error)?;
    assert_eq!(
        session.success_url.as_deref(),
        Some(format!("{portal}/portal?page=balance").as_str())
    );
    assert_eq!(session.cancel_url, session.success_url);
    for (key, refused) in [
        ("bad-url", "javascript:alert(1)"),
        ("other-host", "https://evil.example.com/portal"),
    ] {
        assert!(
            setup
                .services
                .create_portal_top_up_checkout(
                    TENANT_ID,
                    CUST_UBER_ID,
                    wallet,
                    5_000,
                    key.to_owned(),
                    None,
                    Some(refused.to_owned()),
                )
                .await
                .is_err(),
            "{refused}"
        );
    }
    Ok(())
}
