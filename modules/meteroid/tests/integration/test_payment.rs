use super::*;
use crate::meteroid_it::container::SeedLevel;
use tokio;

// #[tokio::test]
// async fn test_registration_flow() {
//     // Test the registration flow
//     // 1. EU selects plan on OC Pricing Page, or clicks signup
//     // 2. Redirected to OC's Signup
//     // 3. OC creates account in OP
//     // 4. OC redirect to plan selection
// }

#[tokio::test]
#[ignore]
async fn test_other() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;

    let mut clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    // we create an invoicing entity
    let created_entity_response = clients
        .invoicing_entities
        .clone()
        .create_invoicing_entity(tonic::Request::new(
            meteroid_grpc::meteroid::api::invoicingentities::v1::CreateInvoicingEntityRequest {
                data: Some(
                    meteroid_grpc::meteroid::api::invoicingentities::v1::InvoicingEntityData {
                        legal_name: Some("Test Company".to_string()), // optional ?
                        country: Some("FR".to_string()),              // optional ?
                        invoice_number_pattern: None,
                        grace_period_hours: None,
                        net_terms: None,
                        invoice_footer_info: None,
                        invoice_footer_legal: None,
                        logo_attachment_id: None,
                        brand_color: None,
                        address_line1: None,
                        address_line2: None,
                        zip_code: None,
                        state: None,
                        city: None,
                        vat_number: None,
                    },
                ),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();

    // we create the bank account (+ associate it ? or we create it in the invoicing entity directly ?)
    let account = clients
        .bank_accounts
        .clone()
        .create_bank_account(tonic::Request::new(
            meteroid_grpc::meteroid::api::bankaccounts::v1::CreateBankAccountRequest {
                data: Some(
                    meteroid_grpc::meteroid::api::bankaccounts::v1::BankAccountData {
                        format:
                        Some(meteroid_grpc::meteroid::api::bankaccounts::v1::bank_account_data::Format::IbanBicSwift(
                            meteroid_grpc::meteroid::api::bankaccounts::v1::IbanBicSwift {
                                iban: "FR1420041010050500013M02606".to_string(),
                                bic_swift: Some("ABNAFRPP".to_string()),
                            },
                        )),
                        country: "FR".to_string(),
                        currency: "EUR".to_string(),
                        bank_name: "Test Bank".to_string(),
                    },
                ),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .account
        .unwrap();

    // we create a stripe connection (+ associate it ? or in invoicing entity directly ?)

    // we associate both (TODO try with & without, and document the behaviour)
    clients.invoicing_entities
        .update_invoicing_entity_payment_methods(tonic::Request::new(
            meteroid_grpc::meteroid::api::invoicingentities::v1::UpdateInvoicingEntityPaymentMethodsRequest {
                id: created_entity_response.id,
                bank_account_id: Some(account.id),
                card_pp_id: None,
            },
        ));

    // we add a payment method to customer

    // we update it
}

#[tokio::test]
async fn test_free_subscription_flow() {
    // Test the free subscription flow
    // 1. User selects Free
    // 2. OC requests OP to create a Free Subscription
    // 3. OP returns OK
}

// TODO test with rest api
#[tokio::test]
async fn test_paid_subscription_flow() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    // Test the paid subscription flow
    // 1. User selects Paid
    // 2. OC requests OP to create a Paid Subscription
    // let subscription = clients
    //     .subscriptions
    //     .clone()
    //     .create_subscription(CreateSubscriptionRequest {})
    //     .await
    //     .unwrap()
    //     .into_inner();

    // TODO that's for the first subscription. WHat happens if it's a second one ? (or downgrade/upgrade)
    // 3. OP creates the customer in Stripe
    // 4. OP creates a Pending Subscription and a Checkout Session with PP
    // 5. PP returns URL
    // 6. OP returns URL to OC, that redirects the user to this URL

    // 7. User completes payment
    // 8. PP confirms payment to OP
    // 9. OP updates Subscription to Active, notifies OC, sends paid invoice to user
    // 10. OC grants access to user
}

#[tokio::test]
async fn test_trial_subscription_flow() {
    // Test the trial subscription flow
    // Trials work like Paid or Free, depending on whether a credit card is required
    // Trial expiration is visible instantly by API and should also be polled for webhooks
}

#[tokio::test]
async fn test_sales_led_flow() {
    // Test the sales-led flow
    // 1. OC creates account in OP
    // 2. OC requests OP to create a Pending Subscription
    // 3. An invoice is created with bank info or a payment link
    // 4. When invoice is paid, subscription is activated
}

#[tokio::test]
async fn test_upgrade_downgrade_flow() {
    // Test the upgrade/downgrade flow
    // 1. Either update the subscription in-place
    // 2. Or create a new one with a parent relationship
    // 3. When the subscription becomes active, the parent ends
}

#[tokio::test]
async fn test_recurring_payment_flow() {
    // Test the recurring payment flow
    // Ensure PP integration is marked as recurring and is consistent across PP
}

// OTHER TEST CASES
// Bank only => what happens when a bank account gets deleted ? Make sure the status is clear, same for mayment methods
// can user delete its single payment method ? (though that's more on stripe side)
