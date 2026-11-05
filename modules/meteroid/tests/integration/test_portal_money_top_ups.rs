//! Portal top-ups in money are part of every edition; promotion codes stay portal self-service.
use meteroid::module::Modules;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::portal::wallets::v1 as portal;
use meteroid_grpc::meteroid::portal::wallets::v1::portal_wallets_service_client::PortalWalletsServiceClient;
use meteroid_store::jwt_claims::{PortalScope, ResourceAccess, generate_portal_token};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use tonic::Code;

use crate::data::ids::{CUST_UBER_ID, CUST_UBER_PAYMENT_METHOD_ID, INVOICING_ENTITY_ID, TENANT_ID};
use crate::data::payment::{run_customer_payment_methods_seed, run_mock_payment_provider_seed};
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::SeedLevel;

fn portal_request<T>(token: &str, message: T) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request
        .metadata_mut()
        .insert("x-portal-token", token.parse().unwrap());
    request
}

#[tokio::test]
async fn oss_customers_top_up_a_money_wallet_from_the_portal() {
    crate::helpers::init::logging();
    let modules = Modules::core();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::PRODUCT).await;
    let instance = modules.edition.instance();
    let setup = meteroid_it::container::start_meteroid_with_modules(
        database,
        SeedLevel::PRODUCT,
        instance,
        modules,
    )
    .await;
    run_mock_payment_provider_seed(&setup.store.pool, false).await;
    run_customer_payment_methods_seed(&setup.store.pool).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );

    clients
        .invoicing_entities
        .clone()
        .update_invoicing_entity(api::invoicingentities::v1::UpdateInvoicingEntityRequest {
            id: INVOICING_ENTITY_ID.as_proto(),
            data: Some(api::invoicingentities::v1::InvoicingEntityData {
                portal_top_up_enabled: Some(true),
                ..Default::default()
            }),
        })
        .await
        .expect("the portal top-up switch is open in OSS");

    let currency = setup
        .store
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .unwrap()
        .currency;
    let wallet = setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, &currency)
        .await
        .unwrap()
        .id;
    let token = generate_portal_token(
        &setup.config.jwt_secret,
        TENANT_ID,
        ResourceAccess::Customer(CUST_UBER_ID),
        Some(PortalScope::ALL.to_vec()),
        None,
        false,
    )
    .unwrap();
    let mut client = PortalWalletsServiceClient::new(setup.channel.clone());

    let balances = client
        .get_balances(portal_request(&token, portal::GetBalancesRequest {}))
        .await
        .unwrap()
        .into_inner();
    assert!(balances.top_up_enabled);
    assert!(
        balances
            .wallets
            .iter()
            .any(|w| w.id == wallet.as_proto() && w.can_top_up)
    );

    let quote = client
        .quote_top_up(portal_request(
            &token,
            portal::QuoteTopUpRequest {
                wallet_id: wallet.as_proto(),
                amount: 5_000,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .quote
        .unwrap();
    let charged = client
        .top_up_wallet(portal_request(
            &token,
            portal::TopUpWalletRequest {
                wallet_id: wallet.as_proto(),
                amount: 5_000,
                payment_method_id: CUST_UBER_PAYMENT_METHOD_ID.as_proto(),
                idempotency_key: "oss-portal-top-up".to_owned(),
                expected_quote: Some(quote),
                return_url: None,
            },
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(charged.status(), portal::TopUpStatus::Succeeded);

    let redeem = client
        .redeem_code(portal_request(
            &token,
            portal::RedeemCodeRequest {
                code: "WELCOME".to_owned(),
                subscription_id: None,
            },
        ))
        .await
        .expect_err("promotion codes stay portal self-service");
    assert_eq!(redeem.code(), Code::PermissionDenied);
}
