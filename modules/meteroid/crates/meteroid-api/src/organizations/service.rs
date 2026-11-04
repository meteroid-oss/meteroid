use common_domain::auth::OrgMemberRole;
use common_domain::country::CountryCode;
use common_domain::ids::{OrganizationId, UserId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::organizations::v1::{
    CreateOrganizationRequest, CreateOrganizationResponse, DeleteOrganizationRequest,
    DeleteOrganizationResponse, GetCurrentOrganizationRequest, GetCurrentOrganizationResponse,
    ListOrganizationsRequest, ListOrganizationsResponse, Organization,
    organizations_service_server::OrganizationsService,
};
use meteroid_middleware::server::auth::SupportGrantMode;
use meteroid_store::Store;
use meteroid_store::domain::{
    InvoicingEntityNew, OrganizationNew, Tenant, TenantEnvironmentEnum, TenantNew,
};
use meteroid_store::repositories::RolesInterface;
use meteroid_store::repositories::organizations::OrganizationsInterface;
use meteroid_store::repositories::tenants::TenantInterface;
use tonic::{Request, Response, Status};

use crate::organizations::error::OrganizationApiError;
use meteroid_app::features::{Entitlements, ensure_production_tenant_allowed};

use super::{OrganizationsServiceComponents, mapping};

/// A new organization starts with a Production tenant next to its Development one, where the
/// edition allows it (cloud requires a subscription first). Never fails the signup.
pub async fn signup_production_tenant(
    store: &Store,
    entitlements: &dyn Entitlements,
    organization_id: OrganizationId,
    invoicing_entity: Option<InvoicingEntityNew>,
) -> Option<Tenant> {
    if store
        .organization_hooks
        .ensure_production_tenant_allowed(organization_id)
        .await
        .is_err()
        || ensure_production_tenant_allowed(entitlements, store, organization_id)
            .await
            .is_err()
    {
        return None;
    }
    let tenant = TenantNew {
        name: "Production".to_string(),
        environment: TenantEnvironmentEnum::Production,
        email_delivery_mode: None,
        invoicing_entity,
    };
    match store.insert_tenant(tenant, organization_id).await {
        Ok(tenant) => Some(tenant),
        Err(e) => {
            log::error!("signup: Production tenant for {organization_id} not created: {e:?}");
            None
        }
    }
}

impl OrganizationsServiceComponents {
    /// The support-session access mode when the caller isn't a member of
    /// `organization_id` but is a platform admin holding a live grant for it —
    /// `None` otherwise. Cheap and fail-closed; only reached on a membership miss.
    async fn support_session_mode(
        &self,
        user_id: UserId,
        organization_id: OrganizationId,
    ) -> Option<SupportGrantMode> {
        let support_grants = self.support_grants.as_ref()?;
        let admin_org = self.store.settings.admin_organization?;
        if self
            .store
            .resolve_member_access(user_id, admin_org)
            .await
            .is_err()
        {
            return None;
        }
        support_grants.grant_mode(user_id, organization_id).await
    }
}

#[tonic::async_trait]
impl OrganizationsService for OrganizationsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_organizations(
        &self,
        request: Request<ListOrganizationsRequest>,
    ) -> Result<Response<ListOrganizationsResponse>, Status> {
        let user = request.actor_user()?;

        let organizations: Vec<Organization> = self
            .store
            .list_organizations_for_user(user)
            .await
            .map_err(Into::<OrganizationApiError>::into)?
            .into_iter()
            .map(mapping::organization::domain_to_proto)
            .collect();

        let response = ListOrganizationsResponse { organizations };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn get_current_organizations(
        &self,
        request: Request<GetCurrentOrganizationRequest>,
    ) -> Result<Response<GetCurrentOrganizationResponse>, Status> {
        let user_id = request.actor_user()?;
        let organization_id = request.organization()?;
        let role = request.actor_role()?;
        let organization = self
            .store
            .get_organizations_with_tenants_by_id(organization_id)
            .await
            .map_err(Into::<OrganizationApiError>::into)?;

        let (permissions, support_session) = match self
            .store
            .resolve_member_access(user_id, organization_id)
            .await
        {
            Ok(access) => (access.permissions.to_effective_codes(), false),
            // Not a member: a platform admin holding a live support grant loads the
            // bootstrap at the granted level. Any other failure keeps the error.
            Err(err) => match self.support_session_mode(user_id, organization_id).await {
                Some(mode) => (mode.member_access().permissions.to_effective_codes(), true),
                None => return Err(OrganizationApiError::from(err).into()),
            },
        };

        let is_platform_admin = self
            .store
            .settings
            .admin_organization
            .is_some_and(|admin_org| admin_org == organization_id)
            && role == OrgMemberRole::Admin;

        let response = GetCurrentOrganizationResponse {
            organization: Some(mapping::organization::domain_with_tenants_to_proto(
                organization,
                None,
            )),
            is_platform_admin,
            role: mapping::role::member_role_to_proto(role).into(),
            permissions,
            support_session,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn create_organization(
        &self,
        request: Request<CreateOrganizationRequest>,
    ) -> Result<Response<CreateOrganizationResponse>, Status> {
        let user = request.actor_user()?;
        let request = request.into_inner();

        // Seed the default invoicing entity's language when the caller supplied one.
        // Onboarding does so where the country alone is a poor guess — Belgium binds the
        // language by seat of establishment, Switzerland has four official languages —
        // rather than letting the country default silently decide. Absent leaves the
        // column NULL, which resolves from the country as before.
        let invoicing_language =
            meteroid_invoicing::lang::reject_unsupported(request.invoicing_language.as_deref())
                .map_err(|tag| {
                    Status::invalid_argument(format!(
                        "Unsupported invoicing_language '{tag}'; supported: {:?}",
                        meteroid_invoicing::lang::SUPPORTED
                    ))
                })
                .map(|()| request.invoicing_language.clone())?;

        let organization_new = OrganizationNew {
            trade_name: request.trade_name.clone(),
            country: CountryCode::from_proto(request.country.clone())?,
            invoicing_entity: invoicing_language.map(|language| {
                meteroid_store::domain::InvoicingEntityNew {
                    invoicing_language: Some(language),
                    ..Default::default()
                }
            }),
        };

        let invoicing_entity = organization_new.invoicing_entity.clone();
        let mut organization = self
            .store
            .insert_organization(organization_new, user)
            .await
            .map_err(Into::<OrganizationApiError>::into)?;

        if let Some(production) = signup_production_tenant(
            &self.store,
            self.feature_resolver.as_ref(),
            organization.organization.id,
            invoicing_entity,
        )
        .await
        {
            organization.tenants.push(production);
        }

        let sandbox_tenant = self
            .sandboxes
            .provision_for_signup(organization.organization.id)
            .await
            .map_err(Into::<OrganizationApiError>::into)?;

        let response = CreateOrganizationResponse {
            organization: Some(mapping::organization::domain_with_tenants_to_proto(
                organization,
                sandbox_tenant,
            )),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn delete_organization(
        &self,
        request: Request<DeleteOrganizationRequest>,
    ) -> Result<Response<DeleteOrganizationResponse>, Status> {
        request.require_admin()?;
        let actor = request.actor_user()?;
        let organization_id = request.organization()?;
        let request_context = meteroid_app::security_context::from_request(&request);

        self.store
            .schedule_organization_deletion(actor, organization_id, request_context)
            .await
            .map_err(Into::<OrganizationApiError>::into)?;

        Ok(Response::new(DeleteOrganizationResponse {}))
    }
}
