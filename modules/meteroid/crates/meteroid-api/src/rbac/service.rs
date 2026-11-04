use std::sync::Arc;

use common_domain::ids::{OrganizationId, RoleId, UserId};
use common_domain::rbac::Permission;
use common_grpc::middleware::server::auth::RequestExt;
use tonic::{Request, Response, Status};

use meteroid_grpc::meteroid::api::rbac::v1 as server;
use meteroid_grpc::meteroid::api::rbac::v1::rbac_service_server::RbacService;
use meteroid_grpc::meteroid::api::users::v1::OrganizationUserRole as GrpcOrganizationUserRole;
use meteroid_middleware::server::auth::strategies::jwt_strategy::invalidate_member_access_cache;
use meteroid_store::Store;
use meteroid_store::repositories::RolesInterface;

use crate::users::mapping::role as role_mapping;
use meteroid_app::features::{Entitlements, gates};
use meteroid_app::security_context;

use super::error::RbacApiError;
use super::mapping;

pub struct RbacServiceComponents {
    pub store: Store,
    pub feature_resolver: Arc<dyn Entitlements>,
}

impl RbacServiceComponents {
    /// Custom roles are gated behind the `rbac` feature.
    async fn ensure_rbac_enabled(&self, org_id: OrganizationId) -> Result<(), Status> {
        if self
            .feature_resolver
            .resolve(org_id)
            .await
            .is_enabled(gates::RBAC)
        {
            Ok(())
        } else {
            Err(Status::permission_denied(
                "Custom roles are not enabled for this organization.",
            ))
        }
    }
}

#[tonic::async_trait]
impl RbacService for RbacServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_permissions(
        &self,
        request: Request<server::ListPermissionsRequest>,
    ) -> Result<Response<server::ListPermissionsResponse>, Status> {
        request.organization()?;
        let permissions = Permission::ALL
            .iter()
            .map(|p| mapping::permission_to_proto(*p))
            .collect();
        Ok(Response::new(server::ListPermissionsResponse {
            permissions,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_roles(
        &self,
        request: Request<server::ListRolesRequest>,
    ) -> Result<Response<server::ListRolesResponse>, Status> {
        let org_id = request.organization()?;
        let custom = self
            .store
            .list_roles(org_id)
            .await
            .map_err(Into::<RbacApiError>::into)?;
        let mut roles = mapping::builtin_roles();
        roles.extend(custom.into_iter().map(mapping::role_to_proto));
        Ok(Response::new(server::ListRolesResponse { roles }))
    }

    #[tracing::instrument(skip_all)]
    async fn assign_role(
        &self,
        request: Request<server::AssignRoleRequest>,
    ) -> Result<Response<server::AssignRoleResponse>, Status> {
        let org_id = request.organization()?;
        let actor = request.actor_user()?;
        let request_context = security_context::from_request(&request);
        let req = request.into_inner();
        let target = UserId::from_proto(&req.user_id)?;
        let grpc_role = GrpcOrganizationUserRole::try_from(req.role)
            .map_err(|_| Status::invalid_argument("Invalid role"))?;
        let role = role_mapping::server_to_domain(grpc_role);
        let custom_role_id = RoleId::from_proto_opt(req.custom_role_id)?;
        // Assigning a custom role requires the feature; clearing to a built-in tier does not.
        if custom_role_id.is_some() {
            self.ensure_rbac_enabled(org_id).await?;
        }
        self.store
            .assign_member_role(org_id, actor, target, role, custom_role_id, request_context)
            .await
            .map_err(Into::<RbacApiError>::into)?;
        // Take effect on the member's next request, not after the 5-minute TTL.
        invalidate_member_access_cache(target, org_id).await;
        Ok(Response::new(server::AssignRoleResponse {}))
    }
}
