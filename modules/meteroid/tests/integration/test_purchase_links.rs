//! Purchase links: public links that create a customer and a SelfServe checkout session,
//! and the abuse controls around them and around checkout.

use chrono::{Duration, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, CheckoutSessionId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId,
    PaymentTransactionId, SubscriptionId,
};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use license_protocol::protocol::{InstanceKey, PurchaseRequest};
use meteroid::services::proof_of_work::{self, Challenge, ProofOfWork};
use meteroid::services::purchase_links::{
    BillingAddressInput, Buyer, PurchaseError, PurchaseLinks, StartPurchase, StartedPurchase,
};
use meteroid_store::domain::InvoicingEntityPatch;
use meteroid_store::domain::checkout_sessions::{
    CheckoutCompletionResult, CheckoutSessionStatus, CheckoutType, CreateCheckoutSession,
};
use meteroid_store::domain::enums::{CustomerType, SubscriptionFeeBillingPeriod};
use meteroid_store::domain::purchase_links::{
    AllowedBuyerTypes, InstanceId, PurchaseLinkNew, PurchaseLinkUpdate, PurchaseLinkWithStats,
    TaxIdCollection,
};
use meteroid_store::domain::subscription_components::SubscriptionFee;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::purchase_links::PurchaseLinksInterface;
use rate_limiter::RateLimiter;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::data::ids::{
    CUST_UBER_PAYMENT_METHOD_ID, INVOICING_ENTITY_ID, MOCK_CONNECTOR_ID, PLAN_LEETCODE_ID,
    PLAN_VERSION_1_LEETCODE_ID, TENANT_ID,
};
use crate::harness::{TestEnv, test_env};

/// LeetCode: €35/month, no trial.
const PRICE: u64 = 3500;

fn purchases(env: &TestEnv) -> PurchaseLinks {
    PurchaseLinks::new(
        env.store().clone(),
        env.setup.config.jwt_secret.clone(),
        env.setup.config.public_url.clone(),
        &RateLimiter::default(),
    )
}

/// Solves a fresh challenge like the page's widget does.
fn pow_solution(env: &TestEnv, token: &str) -> String {
    let challenge = ProofOfWork::new(&env.setup.config.jwt_secret).create_challenge(token);
    proof_of_work::solve(&challenge).expect("solvable challenge")
}

async fn create_link(env: &TestEnv, allow_coupon_codes: bool) -> PurchaseLinkWithStats {
    env.store()
        .create_purchase_link(PurchaseLinkNew {
            tenant_id: TENANT_ID,
            plan_id: PLAN_LEETCODE_ID,
            name: "LeetCode monthly".to_string(),
            allow_coupon_codes,
            listed: false,
            success_url: Some("https://example.com/thanks".to_string()),
            cancel_url: None,
            require_billing_address: true,
            tax_id_collection: TaxIdCollection::Optional,
            allowed_buyer_types: AllowedBuyerTypes::All,
        })
        .await
        .expect("create purchase link")
}

/// A listed link: its plan must issue a license.
async fn create_listed_link(env: &TestEnv) -> PurchaseLinkWithStats {
    set_issues_license(env, true).await;
    env.store()
        .create_purchase_link(PurchaseLinkNew {
            tenant_id: TENANT_ID,
            plan_id: PLAN_LEETCODE_ID,
            name: "LeetCode for installations".to_string(),
            allow_coupon_codes: false,
            listed: true,
            success_url: None,
            cancel_url: None,
            require_billing_address: true,
            tax_id_collection: TaxIdCollection::Optional,
            allowed_buyer_types: AllowedBuyerTypes::All,
        })
        .await
        .expect("create listed purchase link")
}

async fn set_issues_license(env: &TestEnv, issues_license: bool) {
    use diesel_models::schema::plan::dsl as p;
    let mut conn = env.conn().await;
    diesel::update(p::plan.filter(p::id.eq(PLAN_LEETCODE_ID)))
        .set(p::issues_license.eq(issues_license))
        .execute(&mut conn)
        .await
        .unwrap();
}

fn attempt() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn company_purchase(env: &TestEnv, token: &str, email: &str, attempt_id: &str) -> StartPurchase {
    StartPurchase {
        token: token.to_string(),
        email: email.to_string(),
        buyer: Buyer::Company {
            name: "Acme".to_string(),
        },
        country: "FR".to_string(),
        address: BillingAddressInput {
            line1: Some("1 rue de Rivoli".to_string()),
            line2: None,
            city: Some("Paris".to_string()),
            state: None,
            zip_code: Some("75001".to_string()),
        },
        vat_number: Some("FR40303265045".to_string()),
        legal_number: Some("303265045".to_string()),
        pow_solution: pow_solution(env, token),
        attempt_id: attempt_id.to_string(),
        license_request: None,
        client_ip: Some("10.0.0.1".to_string()),
    }
}

async fn start(
    env: &TestEnv,
    purchases: &PurchaseLinks,
    token: &str,
    email: &str,
) -> Result<StartedPurchase, PurchaseError> {
    purchases
        .start(company_purchase(env, token, email, &attempt()))
        .await
}

/// Gives a purchase customer a card on the mock provider, as the checkout page would.
async fn add_card(env: &TestEnv, customer_id: CustomerId) -> CustomerPaymentMethodId {
    let connection_id = crate::data::payment::get_or_create_customer_connection(
        env.pool(),
        customer_id,
        CustomerConnectionId::new(),
        MOCK_CONNECTOR_ID,
        &format!("mock_cus_{}", customer_id.as_base62()),
    )
    .await;
    let method_id = CustomerPaymentMethodId::new();
    crate::data::payment::create_customer_payment_method(
        env.pool(),
        customer_id,
        connection_id,
        method_id,
        &format!("mock_pm_{}", customer_id.as_base62()),
    )
    .await;
    method_id
}

async fn pay(env: &TestEnv, started: &StartedPurchase) -> SubscriptionId {
    let card = add_card(env, started.customer_id).await;
    match env
        .services()
        .complete_checkout(
            TENANT_ID,
            started.checkout_session_id,
            card,
            PRICE,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("checkout completes")
    {
        CheckoutCompletionResult::Completed {
            subscription_id, ..
        } => subscription_id,
        other => panic!("unexpected checkout result: {other:?}"),
    }
}

async fn set_billing_zip_code(env: &TestEnv, customer_id: CustomerId, zip_code: Option<&str>) {
    use diesel_models::schema::customer::dsl as c;
    let address = serde_json::json!({
        "line1": "1 rue de Rivoli",
        "city": "Paris",
        "country": "FR",
        "zip_code": zip_code,
    });
    let mut conn = env.conn().await;
    diesel::update(c::customer.filter(c::id.eq(customer_id)))
        .set(c::billing_address.eq(Some(address)))
        .execute(&mut conn)
        .await
        .unwrap();
}

async fn started_count(env: &TestEnv) -> u32 {
    env.store()
        .list_purchase_links(TENANT_ID, None, true)
        .await
        .unwrap()
        .iter()
        .map(|l| l.started_count)
        .sum()
}

#[rstest]
#[tokio::test]
async fn test_start_purchase_creates_a_fresh_customer_and_checkout(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let link = create_link(&env, false).await;
    assert!(
        link.link.token.len() == 32 && link.link.token.chars().all(|c| c.is_ascii_alphanumeric())
    );
    let purchases = purchases(&env);

    let offer = purchases
        .get_offer(&link.link.token, None)
        .await
        .expect("offer");
    assert_eq!(offer.link.plan.name, "LeetCode");
    assert_eq!(offer.link.version.currency, "EUR");
    assert!(offer.installation_id.is_none());
    assert_eq!(offer.components.len(), 1);
    assert_eq!(
        offer.components[0].period,
        SubscriptionFeeBillingPeriod::Monthly
    );
    assert!(matches!(
        offer.components[0].fee,
        SubscriptionFee::Rate { rate } if rate == Decimal::from(35)
    ));

    // The offer's challenge is what the page solves.
    let challenge: Challenge = serde_json::from_str(&offer.pow_challenge).unwrap();
    assert_eq!(challenge.algorithm, "SHA-256");
    let mut request = company_purchase(&env, &link.link.token, "buyer@acme.com", &attempt());
    request.pow_solution = proof_of_work::solve(&challenge).unwrap();
    let started = purchases.start(request).await.expect("start purchase");
    assert!(
        started
            .checkout_url
            .starts_with(&format!("{}/checkout?token=", env.setup.config.public_url))
    );

    let customer = env
        .store()
        .find_customer_by_id(started.customer_id, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(customer.billing_email.as_deref(), Some("buyer@acme.com"));
    assert_eq!(customer.name, "Acme");
    assert_eq!(customer.customer_type, CustomerType::Company);
    assert_eq!(customer.currency, "EUR");
    assert_eq!(customer.vat_number.as_deref(), Some("FR40303265045"));
    assert_eq!(customer.legal_number.as_deref(), Some("303265045"));
    assert!(customer.vat_number_format_valid);
    let address = customer.billing_address.clone().expect("billing address");
    assert_eq!(address.line1.as_deref(), Some("1 rue de Rivoli"));
    assert_eq!(address.city.as_deref(), Some("Paris"));
    assert_eq!(address.zip_code.as_deref(), Some("75001"));
    assert_eq!(address.country.map(|c| c.code), Some("FR".to_string()));
    assert!(customer.archived_at.is_none());

    let session = env
        .store()
        .get_checkout_session(TENANT_ID, started.checkout_session_id)
        .await
        .unwrap();
    assert_eq!(session.customer_id, started.customer_id);
    assert_eq!(session.plan_version_id, Some(PLAN_VERSION_1_LEETCODE_ID));
    assert_eq!(session.checkout_type, CheckoutType::SelfServe);
    assert_eq!(session.status, CheckoutSessionStatus::Created);
    assert!(
        session
            .expires_at
            .is_some_and(|e| e <= Utc::now() + Duration::hours(1))
    );
    assert_eq!(
        session.success_url.as_deref(),
        Some("https://example.com/thanks")
    );
    assert_eq!(
        session
            .metadata
            .as_ref()
            .and_then(|m| m["purchase_link_id"].as_str()),
        Some(link.link.id.as_base62().as_str())
    );

    // Another visit with the same email never reuses the customer.
    let again = start(&env, &purchases, &link.link.token, "buyer@acme.com")
        .await
        .expect("second purchase");
    assert_ne!(again.customer_id, started.customer_id);

    let listed = env
        .store()
        .list_purchase_links(TENANT_ID, Some(PLAN_LEETCODE_ID), false)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].started_count, 2);
    assert_eq!(listed[0].completed_count, 0);
}

#[rstest]
#[tokio::test]
async fn test_unavailable_links_are_refused(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let purchases = purchases(&env);

    // Regenerating disables the previous URL.
    let link = create_link(&env, false).await;
    let regenerated = env
        .store()
        .regenerate_purchase_link_token(TENANT_ID, link.link.id)
        .await
        .unwrap();
    assert_ne!(regenerated.link.token, link.link.token);
    assert!(matches!(
        purchases.get_offer(&link.link.token, None).await,
        Err(PurchaseError::NotFound)
    ));
    assert!(
        purchases
            .get_offer(&regenerated.link.token, None)
            .await
            .is_ok()
    );

    // Archived links.
    env.store()
        .archive_purchase_link(TENANT_ID, link.link.id)
        .await
        .unwrap();
    assert!(matches!(
        start(&env, &purchases, &regenerated.link.token, "a@acme.com").await,
        Err(PurchaseError::NotFound)
    ));

    // Links of a plan that is no longer active.
    let other = create_link(&env, false).await;
    {
        use diesel_models::enums::PlanStatusEnum;
        use diesel_models::schema::plan::dsl as p;
        let mut conn = env.conn().await;
        diesel::update(p::plan.filter(p::id.eq(PLAN_LEETCODE_ID)))
            .set(p::status.eq(PlanStatusEnum::Inactive))
            .execute(&mut conn)
            .await
            .unwrap();
    }
    assert!(matches!(
        start(&env, &purchases, &other.link.token, "a@acme.com").await,
        Err(PurchaseError::NotFound)
    ));
    assert!(
        env.store()
            .list_listed_purchase_links(TENANT_ID)
            .await
            .unwrap()
            .is_empty()
    );

    // Garbage tokens never reach the database.
    assert!(matches!(
        purchases.get_offer("../../etc", None).await,
        Err(PurchaseError::NotFound)
    ));
}

#[rstest]
#[tokio::test]
async fn test_proof_of_work_is_required_and_single_use(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let link = create_link(&env, false).await;
    let purchases = purchases(&env);
    let token = link.link.token.as_str();

    let mut unsolved = company_purchase(&env, token, "a@acme.com", &attempt());
    unsolved.pow_solution = String::new();
    assert!(matches!(
        purchases.start(unsolved).await,
        Err(PurchaseError::ProofOfWorkFailed)
    ));

    let mut foreign = company_purchase(&env, token, "a@acme.com", &attempt());
    foreign.pow_solution = proof_of_work::solve(
        &ProofOfWork::new(&"another-secret".to_string().into()).create_challenge(token),
    )
    .unwrap();
    assert!(matches!(
        purchases.start(foreign).await,
        Err(PurchaseError::ProofOfWorkFailed)
    ));

    // Solved for another link.
    let other = create_link(&env, false).await;
    let mut misplaced = company_purchase(&env, token, "a@acme.com", &attempt());
    misplaced.pow_solution = pow_solution(&env, &other.link.token);
    assert!(matches!(
        purchases.start(misplaced).await,
        Err(PurchaseError::ProofOfWorkFailed)
    ));

    let first = company_purchase(&env, token, "a@acme.com", &attempt());
    let replayed_solution = first.pow_solution.clone();
    purchases.start(first).await.expect("solved purchase");

    // A solution is spent once used, whatever the attempt.
    let mut replay = company_purchase(&env, token, "b@acme.com", &attempt());
    replay.pow_solution = replayed_solution;
    assert!(matches!(
        purchases.start(replay).await,
        Err(PurchaseError::ProofOfWorkFailed)
    ));
    assert_eq!(started_count(&env).await, 1, "refused starts leave nothing");
}

#[rstest]
#[tokio::test]
async fn test_resubmitting_an_attempt_reuses_the_purchase(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let link = create_link(&env, false).await;
    let purchases = purchases(&env);
    let token = link.link.token.as_str();
    let attempt_id = attempt();

    let first = purchases
        .start(company_purchase(&env, token, "a@acme.com", &attempt_id))
        .await
        .expect("first submit");

    // Corrected details while the checkout is open: same customer, same checkout.
    let mut corrected = company_purchase(&env, token, "billing@acme.com", &attempt_id);
    corrected.buyer = Buyer::Company {
        name: "Acme Corp".to_string(),
    };
    corrected.country = "DE".to_string();
    corrected.address.city = Some("Berlin".to_string());
    corrected.vat_number = None;
    let second = purchases.start(corrected).await.expect("resubmit");
    assert_eq!(second.customer_id, first.customer_id);
    assert_eq!(second.checkout_session_id, first.checkout_session_id);
    let customer = env
        .store()
        .find_customer_by_id(first.customer_id, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(customer.name, "Acme Corp");
    assert_eq!(customer.billing_email.as_deref(), Some("billing@acme.com"));
    assert_eq!(customer.vat_number, None, "a cleared tax ID is removed");
    let address = customer.billing_address.expect("billing address");
    assert_eq!(address.city.as_deref(), Some("Berlin"));
    assert_eq!(address.country.map(|c| c.code), Some("DE".to_string()));

    // Once the checkout expired unpaid: a new checkout for the same customer.
    expire_checkout(&env, first.checkout_session_id).await;
    let third = purchases
        .start(company_purchase(
            &env,
            token,
            "billing@acme.com",
            &attempt_id,
        ))
        .await
        .expect("restart after expiry");
    assert_eq!(third.customer_id, first.customer_id);
    assert_ne!(third.checkout_session_id, first.checkout_session_id);
    assert_eq!(started_count(&env).await, 1, "one attempt, one purchase");

    // A paid attempt can't be reused.
    pay(&env, &third).await;
    assert!(matches!(
        purchases
            .start(company_purchase(
                &env,
                token,
                "billing@acme.com",
                &attempt_id
            ))
            .await,
        Err(PurchaseError::InvalidInput(_))
    ));
}

#[rstest]
#[tokio::test]
async fn test_buyer_input_and_installation_requests_are_validated(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let link = create_link(&env, false).await;
    let purchases = purchases(&env);
    let token = link.link.token.as_str();

    assert!(matches!(
        start(&env, &purchases, token, "nope").await,
        Err(PurchaseError::InvalidInput(_))
    ));

    let mut bad_attempt = company_purchase(&env, token, "a@acme.com", "x");
    bad_attempt.attempt_id = "../x".to_string();
    assert!(matches!(
        purchases.start(bad_attempt).await,
        Err(PurchaseError::InvalidInput(_))
    ));

    let mut forged = company_purchase(&env, token, "a@acme.com", &attempt());
    forged.license_request = Some("e30.c2ln".to_string());
    assert!(matches!(
        purchases.start(forged).await,
        Err(PurchaseError::InvalidInput(_))
    ));
    assert!(matches!(
        purchases.get_offer(token, Some("e30.c2ln")).await,
        Err(PurchaseError::InvalidInput(_))
    ));
}

fn individual_purchase(env: &TestEnv, token: &str) -> StartPurchase {
    let mut purchase = company_purchase(env, token, "ada@example.com", &attempt());
    purchase.buyer = Buyer::Individual {
        first_name: "Ada".to_string(),
        last_name: "Lovelace".to_string(),
    };
    purchase.vat_number = None;
    purchase.legal_number = None;
    purchase
}

#[rstest]
#[tokio::test]
async fn test_link_buyer_options_are_enforced(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let purchases = purchases(&env);

    let companies = create_link(&env, false).await;
    let updated = env
        .store()
        .update_purchase_link(
            TENANT_ID,
            companies.link.id,
            PurchaseLinkUpdate {
                name: companies.link.name.clone(),
                allow_coupon_codes: false,
                listed: false,
                success_url: None,
                cancel_url: None,
                require_billing_address: true,
                tax_id_collection: TaxIdCollection::Optional,
                allowed_buyer_types: AllowedBuyerTypes::CompaniesOnly,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        updated.link.allowed_buyer_types,
        AllowedBuyerTypes::CompaniesOnly
    );
    let token = companies.link.token.as_str();
    assert!(matches!(
        purchases.start(individual_purchase(&env, token)).await,
        Err(PurchaseError::InvalidInput(_))
    ));

    // The full address is required by default.
    let mut no_street = company_purchase(&env, token, "a@acme.com", &attempt());
    no_street.address.line1 = Some("  ".to_string());
    assert!(matches!(
        purchases.start(no_street).await,
        Err(PurchaseError::InvalidInput(_))
    ));
    purchases
        .start(company_purchase(&env, token, "a@acme.com", &attempt()))
        .await
        .expect("a company with an address");

    let individuals = env
        .store()
        .create_purchase_link(PurchaseLinkNew {
            tenant_id: TENANT_ID,
            plan_id: PLAN_LEETCODE_ID,
            name: "Individuals".to_string(),
            allow_coupon_codes: false,
            listed: false,
            success_url: None,
            cancel_url: None,
            require_billing_address: false,
            tax_id_collection: TaxIdCollection::Off,
            allowed_buyer_types: AllowedBuyerTypes::IndividualsOnly,
        })
        .await
        .unwrap();
    let token = individuals.link.token.as_str();
    let offer = purchases.get_offer(token, None).await.unwrap();
    assert!(!offer.link.link.require_billing_address);
    assert_eq!(offer.link.link.tax_id_collection, TaxIdCollection::Off);
    assert!(matches!(
        purchases
            .start(company_purchase(&env, token, "a@acme.com", &attempt()))
            .await,
        Err(PurchaseError::InvalidInput(_))
    ));

    // Only the country is needed.
    let mut country_only = individual_purchase(&env, token);
    country_only.address = BillingAddressInput::default();
    let started = purchases
        .start(country_only)
        .await
        .expect("an individual with a country");
    let customer = env
        .store()
        .find_customer_by_id(started.customer_id, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(customer.customer_type, CustomerType::Individual);
    assert_eq!(customer.name, "Ada Lovelace");
    let address = customer.billing_address.expect("billing address");
    assert_eq!(address.country.map(|c| c.code), Some("FR".to_string()));
    assert_eq!(address.line1, None);

    assert_eq!(started_count(&env).await, 2, "refused starts leave nothing");
}

#[rstest]
#[tokio::test]
async fn test_tax_ids_follow_the_link_policy(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let purchases = purchases(&env);
    let link = env
        .store()
        .create_purchase_link(PurchaseLinkNew {
            tenant_id: TENANT_ID,
            plan_id: PLAN_LEETCODE_ID,
            name: "No tax ID".to_string(),
            allow_coupon_codes: false,
            listed: false,
            success_url: None,
            cancel_url: None,
            require_billing_address: true,
            tax_id_collection: TaxIdCollection::Off,
            allowed_buyer_types: AllowedBuyerTypes::All,
        })
        .await
        .unwrap();
    let token = link.link.token.as_str();

    assert!(matches!(
        purchases
            .start(company_purchase(&env, token, "a@acme.com", &attempt()))
            .await,
        Err(PurchaseError::InvalidInput(_))
    ));
    let mut without = company_purchase(&env, token, "a@acme.com", &attempt());
    without.vat_number = None;
    purchases.start(without).await.expect("no tax ID given");

    // Individuals never give one, whatever the link.
    let mut individual = individual_purchase(&env, token);
    individual.vat_number = Some("FR40303265045".to_string());
    assert!(matches!(
        purchases.start(individual).await,
        Err(PurchaseError::InvalidInput(_))
    ));

    let required = env
        .store()
        .create_purchase_link(PurchaseLinkNew {
            tenant_id: TENANT_ID,
            plan_id: PLAN_LEETCODE_ID,
            name: "Tax ID required".to_string(),
            allow_coupon_codes: false,
            listed: false,
            success_url: None,
            cancel_url: None,
            require_billing_address: true,
            tax_id_collection: TaxIdCollection::Required,
            allowed_buyer_types: AllowedBuyerTypes::All,
        })
        .await
        .unwrap();
    let token = required.link.token.as_str();
    let mut without = company_purchase(&env, token, "b@acme.com", &attempt());
    without.vat_number = None;
    assert!(matches!(
        purchases.start(without).await,
        Err(PurchaseError::InvalidInput(_))
    ));
    purchases
        .start(company_purchase(&env, token, "b@acme.com", &attempt()))
        .await
        .expect("a company with its tax ID");
    purchases
        .start(individual_purchase(&env, token))
        .await
        .expect("individuals are not asked for one");
}

#[rstest]
#[tokio::test]
async fn test_checkout_refuses_another_customers_payment_method(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let link = create_link(&env, false).await;
    let started = start(&env, &purchases(&env), &link.link.token, "a@acme.com")
        .await
        .unwrap();

    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            started.checkout_session_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            PRICE,
            "EUR".to_string(),
            None,
        )
        .await;
    let err = result.expect_err("a foreign payment method must be refused");
    assert!(format!("{err:?}").contains("Payment method not found"));
    assert!(
        env.get_transactions_by_checkout_session(started.checkout_session_id)
            .await
            .is_empty(),
        "nothing was charged"
    );
}

#[rstest]
#[tokio::test]
async fn test_invoicing_entity_requiring_billing_information(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                require_billing_information: Some(true),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("require billing information");
    let link = create_link(&env, false).await;
    let purchases = purchases(&env);

    // The link asks for a street and city only; the entity adds the postal code.
    let mut without_zip = company_purchase(&env, &link.link.token, "a@acme.com", &attempt());
    without_zip.address.zip_code = None;
    let err = purchases
        .start(without_zip)
        .await
        .expect_err("the postal code is required");
    assert!(matches!(&err, PurchaseError::InvalidInput(m) if m.contains("postal code")));

    let started = start(&env, &purchases, &link.link.token, "b@acme.com")
        .await
        .unwrap();
    let card = add_card(&env, started.customer_id).await;

    // Checkout checks the customer as it is when completed, not as the purchase page left it.
    set_billing_zip_code(&env, started.customer_id, None).await;
    let err = env
        .services()
        .complete_checkout(
            TENANT_ID,
            started.checkout_session_id,
            card,
            PRICE,
            "EUR".to_string(),
            None,
        )
        .await
        .expect_err("incomplete billing information is refused");
    assert!(format!("{err:?}").contains("Billing information is required"));
    assert!(
        env.get_transactions_by_checkout_session(started.checkout_session_id)
            .await
            .is_empty(),
        "nothing was charged"
    );

    set_billing_zip_code(&env, started.customer_id, Some("75001")).await;
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            started.checkout_session_id,
            card,
            PRICE,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("complete billing information");
    assert!(matches!(result, CheckoutCompletionResult::Completed { .. }));
}

#[rstest]
#[tokio::test]
async fn test_coupon_codes_follow_the_link_policy(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.create_percentage_coupon("SAVE10", 10).await;
    let purchases = purchases(&env);

    let closed = create_link(&env, false).await;
    let started = start(&env, &purchases, &closed.link.token, "a@acme.com")
        .await
        .unwrap();
    let card = add_card(&env, started.customer_id).await;
    let err = env
        .services()
        .complete_checkout(
            TENANT_ID,
            started.checkout_session_id,
            card,
            PRICE * 9 / 10,
            "EUR".to_string(),
            Some("SAVE10".to_string()),
        )
        .await
        .expect_err("coupon must be refused");
    assert!(format!("{err:?}").contains("Coupon codes are not accepted"));

    let open = create_link(&env, true).await;
    let started = start(&env, &purchases, &open.link.token, "b@acme.com")
        .await
        .unwrap();
    let card = add_card(&env, started.customer_id).await;
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            started.checkout_session_id,
            card,
            PRICE * 9 / 10,
            "EUR".to_string(),
            Some("SAVE10".to_string()),
        )
        .await
        .expect("coupon accepted on a link that allows it");
    assert!(matches!(result, CheckoutCompletionResult::Completed { .. }));
}

#[rstest]
#[tokio::test]
async fn test_checkout_is_cancelled_after_repeated_declines(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.set_mock_payment_failure(true).await;
    let link = create_link(&env, false).await;
    let attempt_id = attempt();
    let started = purchases(&env)
        .start(company_purchase(
            &env,
            &link.link.token,
            "a@acme.com",
            &attempt_id,
        ))
        .await
        .unwrap();
    let card = add_card(&env, started.customer_id).await;

    for attempt in 1..=5 {
        let result = env
            .services()
            .complete_checkout(
                TENANT_ID,
                started.checkout_session_id,
                card,
                PRICE,
                "EUR".to_string(),
                None,
            )
            .await;
        assert!(result.is_err(), "attempt {attempt} is declined");
    }

    let session = env
        .store()
        .get_checkout_session(TENANT_ID, started.checkout_session_id)
        .await
        .unwrap();
    assert_eq!(session.status, CheckoutSessionStatus::Cancelled);

    env.set_mock_payment_failure(false).await;
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            started.checkout_session_id,
            card,
            PRICE,
            "EUR".to_string(),
            None,
        )
        .await;
    assert!(result.is_err(), "a cancelled checkout can't be paid");
    assert_eq!(
        env.get_transactions_by_checkout_session(started.checkout_session_id)
            .await
            .len(),
        5
    );

    // No more cards can be set up on it...
    let status = setup_intent(&env, started.checkout_session_id)
        .await
        .expect_err("closed checkout");
    assert_eq!(status.code(), tonic::Code::FailedPrecondition);

    // ...and the attempt can't start over with a fresh checkout.
    assert!(matches!(
        purchases(&env)
            .start(company_purchase(
                &env,
                &link.link.token,
                "a@acme.com",
                &attempt_id
            ))
            .await,
        Err(PurchaseError::InvalidInput(_))
    ));
}

/// Calls the portal's SetupIntent with the checkout token of `checkout_session_id`.
async fn setup_intent(
    env: &TestEnv,
    checkout_session_id: CheckoutSessionId,
) -> Result<(), tonic::Status> {
    use common_grpc::middleware::server::AuthorizedState;
    use common_grpc::middleware::server::auth::{AuthorizedAsPortalUser, ResourceAccess};
    use meteroid::api::portal::shared::PortalSharedServiceComponents;
    use meteroid_grpc::meteroid::portal::shared::v1::SetupIntentRequest;
    use meteroid_grpc::meteroid::portal::shared::v1::portal_shared_service_server::PortalSharedService;

    let components = PortalSharedServiceComponents {
        store: env.store().clone(),
        services: env.services().clone(),
        object_store: env.setup.object_store.clone(),
        jwt_secret: env.setup.config.jwt_secret.clone(),
        checkout_setup_limiter: RateLimiter::default(),
    };
    let mut request = tonic::Request::new(SetupIntentRequest {
        connection_id: Some(CustomerConnectionId::new().as_proto()),
        ..Default::default()
    });
    request
        .extensions_mut()
        .insert(AuthorizedState::Shared(AuthorizedAsPortalUser {
            tenant_id: TENANT_ID,
            resource_access: ResourceAccess::CheckoutSession(checkout_session_id),
            scopes: None,
            delegated: false,
        }));
    components.setup_intent(request).await.map(drop)
}

#[rstest]
#[tokio::test]
async fn test_installation_binding_is_kept_through_checkout(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let link = create_listed_link(&env).await;
    let purchases = purchases(&env);

    let key = InstanceKey::from_bytes(&[7; 32]);
    let instance_id = InstanceId::parse("vm-acme-prod:4f2a").unwrap();
    let license_request = PurchaseRequest {
        instance_id: instance_id.to_string(),
        instance_public_key: key.public_key(),
        issued_at: Utc::now().timestamp(),
    }
    .sign(&key);

    let offer = purchases
        .get_offer(&link.link.token, Some(&license_request))
        .await
        .unwrap();
    assert_eq!(offer.installation_id, Some(instance_id.clone()));

    let mut request = company_purchase(&env, &link.link.token, "ops@acme.com", &attempt());
    request.license_request = Some(license_request);
    let started = purchases.start(request).await.unwrap();

    assert!(
        env.store()
            .find_purchase_binding_by_instance(TENANT_ID, &instance_id)
            .await
            .unwrap()
            .is_empty(),
        "no binding before the checkout completes"
    );

    let subscription_id = pay(&env, &started).await;

    let binding = env
        .store()
        .find_purchase_binding_by_subscription(TENANT_ID, subscription_id)
        .await
        .unwrap()
        .expect("binding by subscription");
    assert_eq!(binding.subscription_id, subscription_id);
    assert_eq!(binding.instance_id, instance_id);
    assert_eq!(binding.instance_public_key, key.public_key());

    assert_eq!(
        env.store()
            .find_purchase_binding_by_instance(TENANT_ID, &instance_id)
            .await
            .unwrap(),
        vec![binding]
    );
    assert!(
        env.store()
            .find_purchase_binding_by_subscription(TENANT_ID, SubscriptionId::new())
            .await
            .unwrap()
            .is_none()
    );

    let listed = env
        .store()
        .list_listed_purchase_links(TENANT_ID)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].token, link.link.token);

    let stats = env
        .store()
        .get_purchase_link(TENANT_ID, link.link.id)
        .await
        .unwrap();
    assert_eq!((stats.started_count, stats.completed_count), (1, 1));
}

#[rstest]
#[tokio::test]
async fn test_abandoned_purchases_are_cleaned_up(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let link = create_link(&env, false).await;
    let purchases = purchases(&env);
    let token = link.link.token.as_str();

    let abandoned = start(&env, &purchases, token, "gone@acme.com")
        .await
        .unwrap();

    let paid = start(&env, &purchases, token, "paid@acme.com")
        .await
        .unwrap();
    pay(&env, &paid).await;

    // A debit still settling at 24h.
    let settling = start(&env, &purchases, token, "settling@acme.com")
        .await
        .unwrap();

    // Bought another way: an open checkout of its own, or a live payment on another one.
    let other_open = start(&env, &purchases, token, "open@acme.com")
        .await
        .unwrap();
    other_checkout(&env, other_open.customer_id).await;
    let other_paid = start(&env, &purchases, token, "other@acme.com")
        .await
        .unwrap();
    let other_paid_checkout = other_checkout(&env, other_paid.customer_id).await;
    pending_payment(&env, other_paid_checkout).await;
    expire_checkout(&env, other_paid_checkout).await;

    for started in [&abandoned, &paid, &settling, &other_open, &other_paid] {
        backdate(&env, started.checkout_session_id).await;
    }
    {
        use diesel_models::enums::CheckoutSessionStatusEnum;
        use diesel_models::schema::checkout_session::dsl as cs;
        let mut conn = env.conn().await;
        diesel::update(cs::checkout_session.filter(cs::id.eq(settling.checkout_session_id)))
            .set(cs::status.eq(CheckoutSessionStatusEnum::AwaitingPayment))
            .execute(&mut conn)
            .await
            .unwrap();
    }

    let cutoff = (Utc::now() - Duration::hours(24)).naive_utc();
    let cleaned = env
        .store()
        .cleanup_abandoned_purchases(cutoff, 100)
        .await
        .unwrap();
    assert_eq!(cleaned, 1);

    let archived = |customer: meteroid_store::domain::Customer| customer.archived_at.is_some();
    for (started, expected) in [
        (&abandoned, true),
        (&paid, false),
        (&settling, false),
        (&other_open, false),
        (&other_paid, false),
    ] {
        let customer = env
            .store()
            .find_customer_by_id(started.customer_id, TENANT_ID)
            .await
            .unwrap();
        assert_eq!(
            archived(customer),
            expected,
            "customer {}",
            started.customer_id
        );
    }

    assert_eq!(
        env.store()
            .cleanup_abandoned_purchases(cutoff, 100)
            .await
            .unwrap(),
        0,
        "cleaned purchases are not visited again"
    );
}

async fn expire_checkout(env: &TestEnv, id: CheckoutSessionId) {
    use diesel_models::schema::checkout_session::dsl as cs;
    let mut conn = env.conn().await;
    diesel::update(cs::checkout_session.filter(cs::id.eq(id)))
        .set(cs::expires_at.eq(Some(Utc::now() - Duration::minutes(1))))
        .execute(&mut conn)
        .await
        .unwrap();
}

/// Moves a purchase (and its checkout session) two days back.
async fn backdate(env: &TestEnv, checkout_session_id: CheckoutSessionId) {
    use diesel_models::schema::checkout_session::dsl as cs;
    use diesel_models::schema::purchase_link_session::dsl as pls;

    let two_days_ago = Utc::now() - Duration::days(2);
    let mut conn = env.conn().await;
    diesel::update(
        pls::purchase_link_session.filter(pls::checkout_session_id.eq(checkout_session_id)),
    )
    .set(pls::created_at.eq(two_days_ago.naive_utc()))
    .execute(&mut conn)
    .await
    .unwrap();
    diesel::update(cs::checkout_session.filter(cs::id.eq(checkout_session_id)))
        .set(cs::expires_at.eq(Some(two_days_ago + Duration::hours(1))))
        .execute(&mut conn)
        .await
        .unwrap();
}

/// A checkout the merchant opened for the customer outside the purchase link.
async fn other_checkout(env: &TestEnv, customer_id: CustomerId) -> CheckoutSessionId {
    env.store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
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
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
            change_date: None,
            success_url: None,
            cancel_url: None,
        })
        .await
        .expect("checkout")
        .id
}

async fn pending_payment(env: &TestEnv, checkout_session_id: CheckoutSessionId) {
    use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
    use diesel_models::payments::PaymentTransactionRowNew;

    let mut conn = env.conn().await;
    PaymentTransactionRowNew {
        id: PaymentTransactionId::new(),
        tenant_id: TENANT_ID,
        invoice_id: None,
        provider_transaction_id: Some("mock_pending".to_string()),
        amount: PRICE as i64,
        currency: "EUR".to_string(),
        payment_method_id: None,
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
    .expect("pending payment");
}

#[rstest]
#[tokio::test]
async fn test_only_licensing_plans_can_be_listed(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let refused = env
        .store()
        .create_purchase_link(PurchaseLinkNew {
            tenant_id: TENANT_ID,
            plan_id: PLAN_LEETCODE_ID,
            name: "Listed".to_string(),
            allow_coupon_codes: false,
            listed: true,
            success_url: None,
            cancel_url: None,
            require_billing_address: true,
            tax_id_collection: TaxIdCollection::Optional,
            allowed_buyer_types: AllowedBuyerTypes::All,
        })
        .await;
    assert!(refused.is_err(), "the plan issues no license");

    let link = create_link(&env, false).await;
    let update = |listed| PurchaseLinkUpdate {
        name: "Listed".to_string(),
        allow_coupon_codes: false,
        listed,
        success_url: None,
        cancel_url: None,
        require_billing_address: true,
        tax_id_collection: TaxIdCollection::Optional,
        allowed_buyer_types: AllowedBuyerTypes::All,
    };
    assert!(
        env.store()
            .update_purchase_link(TENANT_ID, link.link.id, update(true))
            .await
            .is_err()
    );

    set_issues_license(&env, true).await;
    env.store()
        .update_purchase_link(TENANT_ID, link.link.id, update(true))
        .await
        .expect("listed once the plan issues licenses");
    assert_eq!(
        env.store()
            .list_listed_purchase_links(TENANT_ID)
            .await
            .unwrap()
            .len(),
        1
    );

    // A plan that stops issuing licenses drops out of the offers.
    set_issues_license(&env, false).await;
    assert!(
        env.store()
            .list_listed_purchase_links(TENANT_ID)
            .await
            .unwrap()
            .is_empty()
    );
}

/// The cleanup re-checks each candidate under lock: one resumed after the listing is kept.
#[rstest]
#[tokio::test]
async fn test_cleanup_skips_a_purchase_that_changed_after_listing(#[future] test_env: TestEnv) {
    use diesel_models::customers::CustomerRow;
    use diesel_models::purchase_links::PurchaseLinkSessionRow;

    let env = test_env.await;
    let link = create_link(&env, false).await;
    let started = start(&env, &purchases(&env), &link.link.token, "late@acme.com")
        .await
        .unwrap();
    backdate(&env, started.checkout_session_id).await;

    let cutoff = (Utc::now() - Duration::hours(24)).naive_utc();
    let mut conn = env.conn().await;
    let candidates = PurchaseLinkSessionRow::list_abandoned(&mut conn, cutoff, 10)
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);

    // Meanwhile the buyer comes back through another checkout.
    other_checkout(&env, started.customer_id).await;
    let lock = CustomerRow::lock(&mut conn, TENANT_ID, started.customer_id)
        .await
        .unwrap();
    assert!(
        PurchaseLinkSessionRow::lock_abandoned(&mut conn, &lock, &candidates[0], cutoff)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        env.store()
            .cleanup_abandoned_purchases(cutoff, 10)
            .await
            .unwrap(),
        0
    );
    let customer = env
        .store()
        .find_customer_by_id(started.customer_id, TENANT_ID)
        .await
        .unwrap();
    assert!(customer.archived_at.is_none());
}
