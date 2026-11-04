//! Sandboxes are core: without any module, a Sandbox tenant is seeded from the default preset,
//! the AI credits preset stays behind its gate, and a new organization gets no sandbox.

use std::sync::Arc;
use std::time::Duration;

use common_domain::ids::TenantId;
use meteroid::module::{ApiContext, Modules};
use meteroid_app::features::OssEntitlements;
use meteroid_app::services::egress::Egress;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::tenants::v1::{SandboxPreset, TenantEnvironmentEnum};
use meteroid_store::domain::PaginationRequest;
use meteroid_store::domain::sandbox_seeds::SandboxSeedStatus;
use meteroid_store::hooks::no_analytics_hook;
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::sandbox::SandboxInterface;

use crate::data::ids::ORGANIZATION_ID;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};

/// Seeding runs in the background.
async fn await_seeded(setup: &MeteroidSetup, tenant_id: TenantId) {
    for _ in 0..240 {
        let seed = setup.store.get_sandbox_seed(tenant_id).await.unwrap();
        match seed.map(|s| s.status) {
            Some(SandboxSeedStatus::Completed) => return,
            Some(SandboxSeedStatus::Failed) => panic!("the default sandbox seed failed"),
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
    panic!("the default sandbox seed did not complete");
}

#[tokio::test]
async fn core_seeds_sandbox_tenants_but_none_at_signup() {
    helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::MINIMAL).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );

    let tenant = clients
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: "Demo".to_string(),
            environment: TenantEnvironmentEnum::Sandbox as i32,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: SandboxPreset::Default as i32,
        })
        .await
        .expect("core creates a sandbox without the sandboxes module")
        .into_inner()
        .tenant
        .unwrap();
    assert_eq!(tenant.environment(), TenantEnvironmentEnum::Sandbox);
    assert_eq!(tenant.name, "Demo");

    let tenant_id = TenantId::from_proto(tenant.id.as_str()).unwrap();
    await_seeded(&setup, tenant_id).await;
    let customers = setup
        .store
        .list_customers(
            tenant_id,
            PaginationRequest {
                page: 0,
                per_page: Some(10),
            },
            None,
            None,
            Some(false),
        )
        .await
        .unwrap();
    assert!(
        !customers.items.is_empty(),
        "the default preset seeds its customers"
    );

    let refused = clients
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: "Halyard".to_string(),
            environment: TenantEnvironmentEnum::Sandbox as i32,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: SandboxPreset::AiCredits as i32,
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), tonic::Code::PermissionDenied);

    let ctx = ApiContext {
        config: setup.config.clone(),
        store: setup.store.clone(),
        services: setup.services.clone(),
        feature_resolver: Arc::new(OssEntitlements),
        feature_gates: Arc::new([]),
        rate_limiter: Default::default(),
        redis: None,
        analytics: no_analytics_hook(),
        egress: Egress::new(setup.config.webhook_out.clone(), None),
    };
    let signup = Modules::core()
        .sandboxes(&ctx)
        .provision_for_signup(ORGANIZATION_ID)
        .await
        .unwrap();
    assert!(signup.is_none(), "core gives a new organization no sandbox");
}
