use common_domain::ids::UserId;
use common_grpc::middleware::common::auth::API_KEY_HEADER;
use diesel_models::enums::TenantStatusEnum;
use diesel_models::organizations::OrganizationRow;
use diesel_models::tenants::TenantRow;
use diesel_models::users::UserRow;
use http::{HeaderName, HeaderValue};
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::customers::v1::customers_service_client::CustomersServiceClient;
use meteroid_store::domain::org_security_audit_log::SecurityRequestContext;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::user_deletion::UserDeletionInterface;
use std::str::FromStr;
use tonic::Code;
use tonic::transport::Channel;
use tower_http::set_header::{SetRequestHeader, SetRequestHeaderLayer};

use crate::data::ids::{ORGANIZATION_ID, TENANT_ID};
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::SeedLevel;
use crate::meteroid_it::svc_auth::{SEED_PASSWORD, SEED_USERNAME};
use crate::{helpers, meteroid_it};

const SECOND_EMAIL: &str = "second@meteroid.dev";
const SECOND_PASSWORD: &str = "super-secret";

async fn register_via_invite(clients: &AllClients, email: &str) -> (UserId, String) {
    let invite_key = clients
        .users
        .clone()
        .invite_member(api::users::v1::InviteMemberRequest {
            email: email.to_string(),
            role: api::users::v1::OrganizationUserRole::Member as i32,
            custom_role_id: None,
        })
        .await
        .unwrap()
        .into_inner()
        .invite_id;

    let resp = clients
        .users
        .clone()
        .complete_registration(api::users::v1::CompleteRegistrationRequest {
            email: email.to_string(),
            password: SECOND_PASSWORD.to_string(),
            invite_key: Some(invite_key),
            validation_token: None,
        })
        .await
        .unwrap()
        .into_inner();

    (
        UserId::from_str(&resp.user.unwrap().id).unwrap(),
        resp.token,
    )
}

async fn login(clients: &AllClients, email: &str, password: &str) -> Result<(), Code> {
    clients
        .users
        .clone()
        .login(api::users::v1::LoginRequest {
            email: email.to_string(),
            password: password.to_string(),
        })
        .await
        .map(|_| ())
        .map_err(|e| e.code())
}

#[tokio::test]
async fn test_delete_user() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;
    let store = setup.store.clone();
    let ctx = SecurityRequestContext::default;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let admin_id = UserId::from_str(&auth.user.unwrap().id).unwrap();
    let admin = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let anonymous = AllClients::from_channel(setup.channel.clone(), "", "", "");

    let (second_id, second_token) = register_via_invite(&admin, SECOND_EMAIL).await;
    let second = AllClients::from_channel(
        setup.channel.clone(),
        second_token.as_str(),
        "TESTORG",
        "testslug",
    );

    // The only admin of an org that still has members cannot be deleted, and nothing changes.
    let err = store
        .delete_user(second_id, admin_id, ctx())
        .await
        .unwrap_err();
    assert!(matches!(
        err.current_context(),
        StoreError::FailedPrecondition(_)
    ));
    assert_eq!(
        login(&anonymous, SEED_USERNAME, SEED_PASSWORD).await,
        Ok(())
    );

    let deleted = store.delete_user(admin_id, second_id, ctx()).await.unwrap();
    assert_eq!(deleted.organizations_left, vec![ORGANIZATION_ID]);
    assert!(deleted.deleted_organizations.is_empty());

    // Blocked: no login, and the session it already holds is refused.
    assert_eq!(
        login(&anonymous, SECOND_EMAIL, SECOND_PASSWORD).await,
        Err(Code::Unauthenticated)
    );
    let me = second.users.clone().me(api::users::v1::MeRequest {}).await;
    assert_eq!(me.unwrap_err().code(), Code::Unauthenticated);
    let orgs = second
        .organizations
        .clone()
        .list_organizations(api::organizations::v1::ListOrganizationsRequest {})
        .await;
    assert_eq!(orgs.unwrap_err().code(), Code::Unauthenticated);
    let tenants = second
        .tenants
        .clone()
        .list_tenants(api::tenants::v1::ListTenantsRequest {})
        .await;
    assert_eq!(tenants.unwrap_err().code(), Code::Unauthenticated);

    // Anonymised in place: the id survives, the personal data does not.
    let mut conn = store.get_conn().await.unwrap();
    let row = UserRow::find_by_id(&mut conn, *second_id).await.unwrap();
    assert_eq!(row.email, format!("deleted+{second_id}@deleted.invalid"));
    assert!(row.archived_at.is_some());
    assert!(row.password_hash.is_none());
    assert!(row.first_name.is_none() && row.last_name.is_none());
    assert!(!row.totp_enabled && row.totp_secret.is_none());
    assert!(
        UserRow::find_by_email(&mut conn, SECOND_EMAIL.to_string())
            .await
            .unwrap()
            .is_none()
    );

    let users = admin
        .users
        .clone()
        .list_users(api::users::v1::ListUsersRequest {})
        .await
        .unwrap()
        .into_inner()
        .users;
    assert_eq!(users.len(), 1);

    let again = store
        .delete_user(admin_id, second_id, ctx())
        .await
        .unwrap_err();
    assert!(matches!(
        again.current_context(),
        StoreError::FailedPrecondition(_)
    ));

    // The email is free again: the same person can come back as a new user.
    let (returning_id, _) = register_via_invite(&admin, SECOND_EMAIL).await;
    assert_ne!(returning_id, second_id);
    assert_eq!(
        login(&anonymous, SECOND_EMAIL, SECOND_PASSWORD).await,
        Ok(())
    );

    let api_key = admin
        .api_tokens
        .clone()
        .create_api_token(api::apitokens::v1::CreateApiTokenRequest {
            name: "doomed".to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .api_key;

    // Once the returning user is gone too, the admin is the sole member: their org goes with them.
    store
        .delete_user(admin_id, returning_id, ctx())
        .await
        .unwrap();
    let deleted = store.delete_user(admin_id, admin_id, ctx()).await.unwrap();
    assert_eq!(deleted.deleted_organizations, vec![ORGANIZATION_ID]);
    assert_eq!(
        login(&anonymous, SEED_USERNAME, SEED_PASSWORD).await,
        Err(Code::Unauthenticated)
    );
    let org = OrganizationRow::get_by_id(&mut conn, ORGANIZATION_ID)
        .await
        .unwrap();
    assert!(org.archived_at.is_some());
    let tenant = TenantRow::find_by_id(&mut conn, TENANT_ID).await.unwrap();
    assert_eq!(tenant.status, TenantStatusEnum::PendingDeletion);

    // API keys of a tenant pending deletion stop working.
    let customers = CustomersServiceClient::new(api_key_svc(&setup.channel, &api_key))
        .list_customers(api::customers::v1::ListCustomerRequest::default())
        .await;
    assert_eq!(customers.unwrap_err().code(), Code::Unauthenticated);
}

fn api_key_svc(channel: &Channel, api_key: &str) -> SetRequestHeader<Channel, HeaderValue> {
    tower::ServiceBuilder::new()
        .layer(SetRequestHeaderLayer::if_not_present(
            HeaderName::from_str(API_KEY_HEADER).unwrap(),
            HeaderValue::from_str(api_key).unwrap(),
        ))
        .service(channel.clone())
}

#[tokio::test]
async fn test_delete_sole_member_of_org_with_production_tenant() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;
    let store = setup.store.clone();

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let admin_id = UserId::from_str(&auth.user.unwrap().id).unwrap();
    let admin = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let anonymous = AllClients::from_channel(setup.channel.clone(), "", "", "");

    // 0 == PRODUCTION
    admin
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: "Production".to_string(),
            environment: 0,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: 0,
        })
        .await
        .unwrap();

    let err = store
        .delete_user(admin_id, admin_id, SecurityRequestContext::default())
        .await
        .unwrap_err();
    assert!(matches!(
        err.current_context(),
        StoreError::FailedPrecondition(_)
    ));

    // Nothing changed: the user can still log in and the org is not archived.
    assert_eq!(
        login(&anonymous, SEED_USERNAME, SEED_PASSWORD).await,
        Ok(())
    );
    let mut conn = store.get_conn().await.unwrap();
    let org = OrganizationRow::get_by_id(&mut conn, ORGANIZATION_ID)
        .await
        .unwrap();
    assert!(org.archived_at.is_none());
    let tenant = TenantRow::find_by_id(&mut conn, TENANT_ID).await.unwrap();
    assert_ne!(tenant.status, TenantStatusEnum::PendingDeletion);
}
