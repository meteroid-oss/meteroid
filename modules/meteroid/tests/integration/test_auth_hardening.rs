use std::sync::{Arc, Mutex};

use meteroid_grpc::meteroid::api::users::v1::{
    InitResetPasswordRequest, LoginRequest, MeRequest, ResetPasswordRequest,
};
use meteroid_store::clients::usage::MockUsageClient;
use meteroid_store::repositories::UserInterface;
use meteroid_testkit::mailer::MockMailerService;
use secrecy::ExposeSecret;
use tonic::Code;

use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::SeedLevel;
use crate::{helpers, meteroid_it};

// Its own user: the session cache is process-wide, and the seeded user id is shared by every test.
const EMAIL: &str = "reset-me@acme.test";
const PASSWORD: &str = "the-first-password";
const NEW_PASSWORD: &str = "a-brand-new-password";

fn reset_token(url: &str) -> String {
    url.split_once("token=").unwrap().1.to_string()
}

async fn login(
    channel: &tonic::transport::Channel,
    email: &str,
    password: &str,
) -> Result<String, Code> {
    AllClients::from_channel(channel.clone(), "", "", "")
        .users
        .clone()
        .login(LoginRequest {
            email: email.to_string(),
            password: password.to_string(),
        })
        .await
        .map(|r| r.into_inner().token)
        .map_err(|s| s.code())
}

async fn me(channel: &tonic::transport::Channel, token: &str) -> Result<(), Code> {
    AllClients::from_channel(channel.clone(), token, "", "")
        .users
        .clone()
        .me(MeRequest {})
        .await
        .map(|_| ())
        .map_err(|s| s.code())
}

async fn reset(channel: &tonic::transport::Channel, token: &str) -> Result<(), Code> {
    AllClients::from_channel(channel.clone(), "", "", "")
        .users
        .clone()
        .reset_password(ResetPasswordRequest {
            token: token.to_string(),
            new_password: NEW_PASSWORD.to_string(),
        })
        .await
        .map(|_| ())
        .map_err(|s| s.code())
}

#[tokio::test]
async fn password_reset_links_are_single_use_and_end_sessions() {
    helpers::init::logging();
    let sent: Arc<Mutex<Vec<String>>> = Arc::default();
    let mut mailer = MockMailerService::new();
    let captured = sent.clone();
    mailer
        .expect_send_reset_password_link()
        .returning(move |link| {
            captured
                .lock()
                .unwrap()
                .push(link.url.expose_secret().to_string());
            Ok(())
        });
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup = meteroid_it::container::start_meteroid_with_clients(
        database,
        SeedLevel::MINIMAL,
        Arc::new(MockUsageClient::noop()),
        Arc::new(mailer),
    )
    .await;
    let channel = setup.channel.clone();
    setup
        .store
        .find_or_create_instance_user(EMAIL.to_string(), PASSWORD.to_string().into())
        .await
        .unwrap();

    let shouted = EMAIL.to_uppercase();
    let session = login(&channel, &format!("  {shouted} "), PASSWORD)
        .await
        .expect("login is case-insensitive");
    me(&channel, &session).await.unwrap();

    for _ in 0..2 {
        AllClients::from_channel(channel.clone(), "", "", "")
            .users
            .clone()
            .init_reset_password(InitResetPasswordRequest {
                email: shouted.clone(),
            })
            .await
            .unwrap();
    }
    let links = sent.lock().unwrap().clone();
    assert_eq!(links.len(), 2);
    let (superseded, latest) = (reset_token(&links[0]), reset_token(&links[1]));

    assert_eq!(
        reset(&channel, &superseded).await,
        Err(Code::InvalidArgument)
    );
    reset(&channel, &latest).await.expect("latest link resets");
    assert_eq!(reset(&channel, &latest).await, Err(Code::InvalidArgument));

    assert_eq!(me(&channel, &session).await, Err(Code::Unauthenticated));
    assert_eq!(
        login(&channel, EMAIL, PASSWORD).await,
        Err(Code::Unauthenticated)
    );
    let fresh = login(&channel, EMAIL, NEW_PASSWORD).await.unwrap();
    me(&channel, &fresh).await.unwrap();
}
