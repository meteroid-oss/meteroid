use crate::tenants::error::TenantApiError;
use common_domain::ids::TenantId;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::features::{
    PORTAL_EMBEDDING_NOT_IN_PLAN, ResolvedEntitlements, ensure_change_allowed,
    ensure_production_tenant_allowed, gates,
};
use meteroid_grpc::meteroid::api::tenants::v1::{
    ActiveTenantRequest, ActiveTenantResponse, CopiedRows, CreateTenantRequest,
    CreateTenantResponse, DeleteTenantRequest, DeleteTenantResponse, GetTenantByIdRequest,
    GetTenantByIdResponse, GetTenantSeedingStatusRequest, GetTenantSeedingStatusResponse,
    ListTenantsCurrenciesRequest, ListTenantsCurrenciesResponse,
    ListTenantsCurrenciesWithCustomerCountRequest, ListTenantsCurrenciesWithCustomerCountResponse,
    ListTenantsRequest, ListTenantsResponse, ResumeTenantRequest, ResumeTenantResponse,
    SandboxPreset, UpdateTenantAvailableCurrenciesRequest, UpdateTenantAvailableCurrenciesResponse,
    UpdateTenantRequest, UpdateTenantResponse,
    list_tenants_currencies_with_customer_count_response::ListCurrency,
    tenants_service_server::TenantsService,
};
use meteroid_store::domain::TenantEnvironmentEnum;
use meteroid_store::domain::sandbox_seeds::SandboxSeedStatus;
use meteroid_store::domain::tenants::normalize_embed_origins;
use meteroid_store::repositories::sandbox::SandboxInterface;
use meteroid_store::repositories::tenant_copy::TenantCopyScope;
use meteroid_store::repositories::{OrganizationsInterface, TenantCopyInterface, TenantInterface};
use tonic::{Request, Response, Status};

use super::{TenantServiceComponents, mapping};

const SEEDING_GRACE: chrono::Duration = chrono::Duration::minutes(3);
const SEEDING_MAX_DURATION: chrono::Duration = chrono::Duration::minutes(30);

fn ensure_sandbox_preset_entitled(
    preset: SandboxPreset,
    resolved: &ResolvedEntitlements,
) -> Result<(), Status> {
    match preset {
        SandboxPreset::Default | SandboxPreset::CurrenciesAndCoupons => Ok(()),
        SandboxPreset::AiCredits if resolved.is_enabled(gates::ADMIN_SANDBOXES) => Ok(()),
        SandboxPreset::AiCredits => Err(Status::permission_denied(
            "This sandbox example is not enabled for this organization.",
        )),
    }
}

#[tonic::async_trait]
impl TenantsService for TenantServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn update_tenant(
        &self,
        request: Request<UpdateTenantRequest>,
    ) -> Result<Response<UpdateTenantResponse>, Status> {
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;
        let actor = request.actor_typed()?;

        let inner = request
            .into_inner()
            .data
            .ok_or(TenantApiError::MissingArgument(
                "No data provided".to_string(),
            ))?;

        let req = mapping::tenants::update_req_to_domain(inner)?;

        if req.environment == Some(TenantEnvironmentEnum::Production) {
            let current = self
                .store
                .find_tenant_by_id(tenant_id)
                .await
                .map_err(Into::<TenantApiError>::into)?;
            if current.tenant.environment != TenantEnvironmentEnum::Production {
                ensure_production_tenant_allowed(
                    self.feature_resolver.as_ref(),
                    &self.store,
                    organization_id,
                )
                .await?;
            }
        }
        // No longer offered: a tenant that has it keeps it, or turns it off for good.
        if req.keep_last_events == Some(true) {
            let current = self
                .store
                .find_tenant_by_id(tenant_id)
                .await
                .map_err(Into::<TenantApiError>::into)?;
            if !current.tenant.keep_last_events {
                return Err(Status::failed_precondition(
                    "Keeping the last copy of re-sent events is no longer offered",
                ));
            }
        }
        if let Some(origins) = &req.portal_embed_origins {
            let current = self
                .store
                .get_portal_embed_origins(tenant_id)
                .await
                .map_err(Into::<TenantApiError>::into)?;
            let origins = normalize_embed_origins(origins.clone())
                .map_err(|e| Status::invalid_argument(e.to_string()))?;
            let adds = origins.iter().any(|o| !current.contains(o));
            ensure_change_allowed(
                self.feature_resolver.as_ref(),
                organization_id,
                gates::PORTAL_EMBEDDING,
                adds,
                PORTAL_EMBEDDING_NOT_IN_PLAN,
            )
            .await?;
        }

        let organization = self
            .store
            .get_organization_by_id(organization_id)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        let res = self
            .store
            .update_tenant(actor, req, organization_id, tenant_id)
            .await
            .map(mapping::tenants::domain_to_server)
            .map_err(Into::<TenantApiError>::into)?;

        self.store.slug_ids.remove(&organization.slug, &res.slug);

        Ok(Response::new(UpdateTenantResponse { tenant: Some(res) }))
    }

    #[tracing::instrument(skip_all)]
    async fn active_tenant(
        &self,
        request: Request<ActiveTenantRequest>,
    ) -> Result<Response<ActiveTenantResponse>, Status> {
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;

        let tenant = self
            .store
            .find_tenant_by_id_and_organization(tenant_id, organization_id)
            .await
            .map(mapping::tenants::domain_to_server)
            .map_err(Into::<TenantApiError>::into)?;

        // Primary activity signal (throttled): the app loads this on every tenant
        // page. Powers sandbox freshness and unused-tenant detection; never fail on it.
        let _ = self.store.touch_tenant_activity(tenant_id).await;

        let organization = self
            .store
            .get_organization_by_id(organization_id)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(ActiveTenantResponse {
            tenant: Some(tenant),
            trade_name: organization.trade_name,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_tenants(
        &self,
        request: Request<ListTenantsRequest>,
    ) -> Result<Response<ListTenantsResponse>, Status> {
        let organization = request.organization()?;

        let result = self
            .store
            .list_tenants_by_organization_id(organization)
            .await
            .map(|x| {
                x.into_iter()
                    .map(mapping::tenants::domain_to_server)
                    .collect()
            })
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(ListTenantsResponse { tenants: result }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_tenant_by_id(
        &self,
        request: Request<GetTenantByIdRequest>,
    ) -> Result<Response<GetTenantByIdResponse>, Status> {
        let organization_id = request.organization()?;

        let req = request.into_inner();
        let tenant_id = TenantId::from_proto(req.tenant_id)?;

        let domain_tenant = self
            .store
            .find_tenant_by_id_and_organization(tenant_id, organization_id)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        // Best-effort activity signal (throttled): powers sandbox freshness and
        // unused-tenant detection. Never fail the request on it.
        let _ = self.store.touch_tenant_activity(tenant_id).await;

        Ok(Response::new(GetTenantByIdResponse {
            tenant: Some(mapping::tenants::domain_to_server(domain_tenant)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_tenant_seeding_status(
        &self,
        request: Request<GetTenantSeedingStatusRequest>,
    ) -> Result<Response<GetTenantSeedingStatusResponse>, Status> {
        let organization_id = request.organization()?;
        let tenant_id = TenantId::from_proto(request.into_inner().tenant_id)?;

        // Scope check: the tenant must belong to the caller's organization.
        let tenant = self
            .store
            .find_tenant_by_id_and_organization(tenant_id, organization_id)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        let seed = self
            .store
            .get_sandbox_seed(tenant_id)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        // Past this window seeding is over: a sandbox still lagging behind is stuck, and a failure is old news.
        let now = common_clock::wall().naive_utc();
        let started_at = seed.as_ref().map_or(tenant.created_at, |s| s.started_at);
        if now - started_at > SEEDING_MAX_DURATION {
            return Ok(Response::new(GetTenantSeedingStatusResponse {
                in_progress: false,
                pending_subscriptions: 0,
                failed: false,
                failure_reason: None,
            }));
        }

        if let Some(seed) = &seed
            && seed.status == SandboxSeedStatus::Failed
        {
            return Ok(Response::new(GetTenantSeedingStatusResponse {
                in_progress: false,
                pending_subscriptions: 0,
                failed: true,
                failure_reason: seed.failure_reason.clone(),
            }));
        }

        let pending = self
            .store
            .count_subscriptions_pending_catchup(tenant_id)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        // Before the subscriptions exist `pending` is 0 although seeding isn't finished.
        let in_progress = match seed.map(|s| s.status) {
            Some(SandboxSeedStatus::InProgress) => true,
            Some(_) => pending > 0,
            // Seeded before seeds were tracked.
            None => pending > 0 || now - started_at < SEEDING_GRACE,
        };

        Ok(Response::new(GetTenantSeedingStatusResponse {
            in_progress,
            pending_subscriptions: pending as u32,
            failed: false,
            failure_reason: None,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn resume_tenant(
        &self,
        request: Request<ResumeTenantRequest>,
    ) -> Result<Response<ResumeTenantResponse>, Status> {
        let organization_id = request.organization()?;
        let tenant_id = TenantId::from_proto(request.into_inner().tenant_id)?;

        let mut domain_tenant = self
            .store
            .find_tenant_by_id_and_organization(tenant_id, organization_id)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        self.store
            .resume_tenant(tenant_id)
            .await
            .map_err(Into::<TenantApiError>::into)?;
        domain_tenant.status = meteroid_store::domain::enums::TenantStatusEnum::Active;

        Ok(Response::new(ResumeTenantResponse {
            tenant: Some(mapping::tenants::domain_to_server(domain_tenant)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_tenant(
        &self,
        request: Request<CreateTenantRequest>,
    ) -> Result<Response<CreateTenantResponse>, Status> {
        let organization_id = request.organization()?;

        let actor = request.actor_typed()?;
        let request = request.into_inner();
        let preset = request.sandbox_preset();
        let copy_from = request.copy_from.clone();
        let req = mapping::tenants::create_req_to_domain(request)?;

        if req.environment == TenantEnvironmentEnum::Production {
            ensure_production_tenant_allowed(
                self.feature_resolver.as_ref(),
                &self.store,
                organization_id,
            )
            .await?;
        }

        let mut copied = Vec::new();
        let res = if let Some(copy) = copy_from {
            let source = TenantId::from_proto(copy.tenant_id)?;
            let created = self
                .store
                .insert_tenant_copy(
                    actor,
                    source,
                    req,
                    organization_id,
                    TenantCopyScope {
                        catalog: copy.catalog,
                        invoicing: copy.invoicing,
                    },
                )
                .await
                .map_err(Into::<TenantApiError>::into)?;
            copied = created
                .copied
                .into_iter()
                .map(|(table, rows)| CopiedRows {
                    table,
                    rows: rows as u64,
                })
                .collect();
            created.tenant
        } else if req.environment == TenantEnvironmentEnum::Sandbox {
            ensure_sandbox_preset_entitled(
                preset,
                &self.feature_resolver.resolve(organization_id).await,
            )?;
            self.sandboxes
                .create(organization_id, req.name, req.email_delivery_mode, preset)
                .await
                .map_err(Into::<TenantApiError>::into)?
        } else {
            self.store
                .insert_tenant(req, organization_id)
                .await
                .map_err(Into::<TenantApiError>::into)?
        };

        Ok(Response::new(CreateTenantResponse {
            tenant: Some(mapping::tenants::domain_to_server(res)),
            copied,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn delete_tenant(
        &self,
        request: Request<DeleteTenantRequest>,
    ) -> Result<Response<DeleteTenantResponse>, Status> {
        request.require_admin()?;
        let actor = request.actor_user()?;
        let organization_id = request.organization()?;
        let request_context = meteroid_app::security_context::from_request(&request);
        let tenant_id = TenantId::from_proto(request.into_inner().tenant_id)?;

        let organization_deleted = self
            .store
            .schedule_tenant_deletion(actor, organization_id, tenant_id, request_context)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(DeleteTenantResponse {
            organization_deleted,
        }))
    }

    async fn list_tenant_currencies(
        &self,
        request: Request<ListTenantsCurrenciesRequest>,
    ) -> Result<Response<ListTenantsCurrenciesResponse>, Status> {
        let tenant = request.tenant()?;

        let res = self
            .store
            .list_tenant_currencies(tenant)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(ListTenantsCurrenciesResponse {
            currencies: res,
        }))
    }

    async fn list_tenant_currencies_with_customer_count(
        &self,
        request: Request<ListTenantsCurrenciesWithCustomerCountRequest>,
    ) -> Result<Response<ListTenantsCurrenciesWithCustomerCountResponse>, Status> {
        let tenant = request.tenant()?;

        let res = self
            .store
            .list_tenant_currencies_with_customer_count(tenant)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(
            ListTenantsCurrenciesWithCustomerCountResponse {
                currencies: res
                    .into_iter()
                    .map(|(c, cc)| ListCurrency {
                        currency: c,
                        customer_count: cc,
                    })
                    .collect(),
            },
        ))
    }

    async fn update_tenant_available_currencies(
        &self,
        request: Request<UpdateTenantAvailableCurrenciesRequest>,
    ) -> Result<Response<UpdateTenantAvailableCurrenciesResponse>, Status> {
        let tenant = request.tenant()?;
        let res = self
            .store
            .update_tenant_available_currencies(tenant, request.into_inner().currencies)
            .await
            .map_err(Into::<TenantApiError>::into)?;

        Ok(Response::new(UpdateTenantAvailableCurrenciesResponse {
            currencies: res,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exploratory_sandboxes_need_the_gate() {
        let unflagged = ResolvedEntitlements::with_enabled(&[]);
        assert!(ensure_sandbox_preset_entitled(SandboxPreset::Default, &unflagged).is_ok());
        assert!(
            ensure_sandbox_preset_entitled(SandboxPreset::CurrenciesAndCoupons, &unflagged).is_ok()
        );
        assert!(ensure_sandbox_preset_entitled(SandboxPreset::AiCredits, &unflagged).is_err());
        assert!(
            ensure_sandbox_preset_entitled(
                SandboxPreset::AiCredits,
                &ResolvedEntitlements::outage()
            )
            .is_err()
        );
        let flagged = ResolvedEntitlements::with_enabled(&[gates::ADMIN_SANDBOXES]);
        assert!(ensure_sandbox_preset_entitled(SandboxPreset::AiCredits, &flagged).is_ok());
    }
}
