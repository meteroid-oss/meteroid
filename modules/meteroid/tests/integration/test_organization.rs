use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use meteroid_grpc::meteroid::api;

#[tokio::test]
async fn test_delete_organization() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
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

    // The admin user starts as a member of the seeded organization.
    let before = clients
        .organizations
        .clone()
        .list_organizations(api::organizations::v1::ListOrganizationsRequest {})
        .await
        .unwrap()
        .into_inner()
        .organizations;
    assert_eq!(before.len(), 1);

    clients
        .organizations
        .clone()
        .delete_organization(api::organizations::v1::DeleteOrganizationRequest {})
        .await
        .unwrap();

    // Everything under it is gone: the user is no longer a member of any org.
    let after = clients
        .organizations
        .clone()
        .list_organizations(api::organizations::v1::ListOrganizationsRequest {})
        .await
        .unwrap()
        .into_inner()
        .organizations;
    assert!(after.is_empty());
}

#[tokio::test]
async fn test_delete_organization_blocked_by_production_tenant() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
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

    // 0 == PRODUCTION
    clients
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

    let err = clients
        .organizations
        .clone()
        .delete_organization(api::organizations::v1::DeleteOrganizationRequest {})
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::FailedPrecondition);

    // The organization is untouched.
    let orgs = clients
        .organizations
        .clone()
        .list_organizations(api::organizations::v1::ListOrganizationsRequest {})
        .await
        .unwrap()
        .into_inner()
        .organizations;
    assert_eq!(orgs.len(), 1);
}

#[tokio::test]
async fn test_delete_tenant_and_last_drops_organization() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
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

    // 1 == STAGING (non-prod, no sandbox seeding). The seed already has one
    // Development tenant ("testslug"), so the org now has two tenants.
    clients
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: "Staging".to_string(),
            environment: 1,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: 0,
        })
        .await
        .unwrap();

    let tenants = clients
        .tenants
        .clone()
        .list_tenants(api::tenants::v1::ListTenantsRequest {})
        .await
        .unwrap()
        .into_inner()
        .tenants;
    assert_eq!(tenants.len(), 2);

    // Delete the non-active tenant first (keeps the request's tenant context valid).
    let staging = tenants.iter().find(|t| t.slug != "testslug").unwrap();
    let deleted = clients
        .tenants
        .clone()
        .delete_tenant(api::tenants::v1::DeleteTenantRequest {
            tenant_id: staging.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!deleted.organization_deleted);

    let remaining = clients
        .tenants
        .clone()
        .list_tenants(api::tenants::v1::ListTenantsRequest {})
        .await
        .unwrap()
        .into_inner()
        .tenants;
    assert_eq!(remaining.len(), 1);

    // Dropping the last tenant drops the organization.
    let deleted_last = clients
        .tenants
        .clone()
        .delete_tenant(api::tenants::v1::DeleteTenantRequest {
            tenant_id: remaining[0].id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(deleted_last.organization_deleted);

    let orgs = clients
        .organizations
        .clone()
        .list_organizations(api::organizations::v1::ListOrganizationsRequest {})
        .await
        .unwrap()
        .into_inner()
        .organizations;
    assert!(orgs.is_empty());
}

#[tokio::test]
async fn test_delete_production_tenant_blocked() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
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

    // 0 == PRODUCTION
    let prod = clients
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
        .unwrap()
        .into_inner()
        .tenant
        .unwrap();

    let err = clients
        .tenants
        .clone()
        .delete_tenant(api::tenants::v1::DeleteTenantRequest {
            tenant_id: prod.id.clone(),
        })
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
}

mod signup_production_tenant {
    use std::sync::Arc;

    use common_domain::ids::OrganizationId;
    use error_stack::Report;
    use meteroid::api::organizations::signup_production_tenant;
    use meteroid_app::features::{Entitlements, OssEntitlements, ResolvedEntitlements};
    use meteroid_store::StoreResult;
    use meteroid_store::domain::TenantEnvironmentEnum;
    use meteroid_store::errors::StoreError;
    use meteroid_store::hooks::OrganizationHooks;
    use meteroid_store::repositories::tenants::TenantInterface;

    use crate::data::ids::ORGANIZATION_ID;
    use crate::helpers;
    use crate::meteroid_it;
    use crate::meteroid_it::container::SeedLevel;

    struct NoProduction;

    #[async_trait::async_trait]
    impl OrganizationHooks for NoProduction {
        async fn ensure_production_tenant_allowed(&self, _: OrganizationId) -> StoreResult<()> {
            Err(Report::new(StoreError::FailedPrecondition(
                "subscription required".to_string(),
            )))
        }
    }

    struct OneProduction;

    #[async_trait::async_trait]
    impl Entitlements for OneProduction {
        async fn resolve(&self, _: OrganizationId) -> ResolvedEntitlements {
            ResolvedEntitlements::open()
        }
        async fn alert_definition_limit(&self, _: OrganizationId) -> Option<u32> {
            None
        }
        async fn production_tenant_limit(&self, _: OrganizationId) -> Option<u32> {
            Some(1)
        }
        async fn invoicing_entity_limit(&self, _: OrganizationId) -> Option<u32> {
            None
        }
    }

    async fn production_count(store: &meteroid_store::Store) -> usize {
        store
            .list_tenants_by_organization_id(ORGANIZATION_ID)
            .await
            .unwrap()
            .iter()
            .filter(|t| t.environment == TenantEnvironmentEnum::Production)
            .count()
    }

    #[tokio::test]
    async fn created_where_allowed_and_skipped_otherwise() {
        helpers::init::logging();
        let database =
            meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
        let setup = meteroid_it::container::start_meteroid(database, SeedLevel::MINIMAL).await;
        let store = setup.store.clone();
        let entitlements = OssEntitlements;
        let before = production_count(&store).await;

        let mut refusing = store.clone();
        refusing.organization_hooks = Arc::new(NoProduction);
        assert!(
            signup_production_tenant(&refusing, &entitlements, ORGANIZATION_ID, None)
                .await
                .is_none(),
            "a refusing edition hook skips the Production tenant"
        );
        assert_eq!(production_count(&store).await, before);

        let created = signup_production_tenant(&store, &entitlements, ORGANIZATION_ID, None)
            .await
            .expect("OSS creates the organization's Production tenant");
        assert_eq!(created.environment, TenantEnvironmentEnum::Production);
        assert_eq!(created.name, "Production");
        assert_eq!(production_count(&store).await, before + 1);

        assert!(
            signup_production_tenant(&store, &entitlements, ORGANIZATION_ID, None)
                .await
                .is_none(),
            "OSS holds one Production tenant"
        );

        assert!(
            signup_production_tenant(&store, &OneProduction, ORGANIZATION_ID, None)
                .await
                .is_none(),
            "the Production cap is respected"
        );
        assert_eq!(production_count(&store).await, before + 1);
    }
}
