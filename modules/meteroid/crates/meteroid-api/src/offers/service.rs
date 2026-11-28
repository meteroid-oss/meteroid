use common_domain::ids::{OfferId, OrganizationId, PlanId, SubscriptionId, TenantId};
use common_grpc::middleware::server::auth::RequestExt;
use error_stack::Report;
use meteroid_grpc::meteroid::api::offers::v1 as proto;
use meteroid_grpc::meteroid::api::offers::v1::offers_service_server::OffersService;
use meteroid_grpc::meteroid::api::offers::v1::set_plan_start_offer_request::Change;
use meteroid_store::domain::offers::{
    GrantDefinition, Offer, OfferGrantNew, OfferNew, OfferPatch, OfferSource, OfferTrial,
    PlanStartOffer, StartOfferChange,
};
use meteroid_store::domain::trials::TrialEligibility;
use meteroid_store::errors::{OfferRefusal, StoreError};
use tonic::{Request, Response, Status};

use super::{OfferServiceComponents, ensure_offers_allowed, mapping};
use crate::plans::mapping::plans::trial_terms_from_proto;

const MAX_PAGE: u32 = 500;

fn store_status(report: Report<StoreError>) -> Status {
    if let Some(refusal) = OfferRefusal::of(&report) {
        let message = match report.current_context() {
            StoreError::InvalidArgument(message) => message.clone(),
            other => other.to_string(),
        };
        return match refusal {
            OfferRefusal::Unavailable => Status::not_found(message),
            OfferRefusal::AlreadyApplied => {
                Status::failed_precondition(format!("offer_already_applied: {message}"))
            }
            OfferRefusal::AlreadyGiven | OfferRefusal::TrialNotApplicable => {
                Status::failed_precondition(message)
            }
        };
    }
    match report.current_context() {
        StoreError::InvalidArgument(message) => Status::invalid_argument(message.clone()),
        StoreError::ValueNotFound(message) => Status::not_found(message.clone()),
        StoreError::Conflict(message) => Status::failed_precondition(message.clone()),
        StoreError::Forbidden(message) => Status::permission_denied(message.clone()),
        StoreError::DuplicateValue { entity, .. } => {
            Status::already_exists(format!("{entity} already exists"))
        }
        _ => {
            log::error!("offers: {report:?}");
            Status::internal("The offer request failed")
        }
    }
}

fn description(value: Option<String>) -> Option<String> {
    value.filter(|description| !description.trim().is_empty())
}

/// An unchanged trial keeps how it bills, as offers stored before trials v2 do.
fn kept_trial(current: Option<OfferTrial>, trial: OfferTrial) -> OfferTrial {
    let Some(current) = current else {
        return trial;
    };
    let unchanged = OfferTrial {
        free: current.free,
        ..trial
    } == current;
    if unchanged { current } else { trial }
}

impl OfferServiceComponents {
    async fn ensure_entitled(
        &self,
        organization_id: OrganizationId,
        changes: bool,
    ) -> Result<(), Status> {
        ensure_offers_allowed(self.feature_resolver.as_ref(), organization_id, changes).await
    }

    async fn offers_to_proto(
        &self,
        tenant_id: TenantId,
        offers: Vec<Offer>,
    ) -> Result<Vec<proto::Offer>, Status> {
        let ids: Vec<OfferId> = offers.iter().map(|offer| offer.id).collect();
        let mut used = self
            .services
            .offers_used_by(tenant_id, &ids)
            .await
            .map_err(store_status)?;
        Ok(offers
            .into_iter()
            .map(|offer| {
                let used_by = used.remove(&offer.id).unwrap_or_default();
                mapping::offer_to_proto(offer, used_by)
            })
            .collect())
    }

    async fn offer_to_proto(
        &self,
        tenant_id: TenantId,
        offer: Offer,
    ) -> Result<proto::Offer, Status> {
        let used_by = self
            .services
            .offers_used_by(tenant_id, &[offer.id])
            .await
            .map_err(store_status)?
            .remove(&offer.id)
            .unwrap_or_default();
        Ok(mapping::offer_to_proto(offer, used_by))
    }

    async fn start_offer_to_proto(
        &self,
        tenant_id: TenantId,
        start: Option<PlanStartOffer>,
    ) -> Result<Option<proto::PlanStartOffer>, Status> {
        let Some(start) = start else {
            return Ok(None);
        };
        Ok(Some(proto::PlanStartOffer {
            plan_ids: start.plans.iter().map(|id| id.as_proto()).collect(),
            offer: Some(self.offer_to_proto(tenant_id, start.offer).await?),
        }))
    }

    async fn start_offer(
        &self,
        tenant_id: TenantId,
        plan_id: PlanId,
    ) -> Result<Option<PlanStartOffer>, Status> {
        Ok(self
            .services
            .plan_start_offers(tenant_id, &[plan_id])
            .await
            .map_err(store_status)?
            .remove(&plan_id))
    }

    /// A catalog offer named, an add-on grant added or a trial switching to a paid plan is
    /// Enterprise; keeping what the plan has is not.
    async fn ensure_start_offer_allowed(
        &self,
        tenant_id: TenantId,
        organization_id: OrganizationId,
        plan_id: PlanId,
        change: &StartOfferChange,
    ) -> Result<(), Status> {
        match change {
            StartOfferChange::None => Ok(()),
            StartOfferChange::Catalog(offer_id) => {
                let current = self.start_offer(tenant_id, plan_id).await?;
                self.ensure_entitled(
                    organization_id,
                    current.is_none_or(|start| start.offer.id != *offer_id),
                )
                .await
            }
            StartOfferChange::Inline {
                trial,
                grants,
                eligibility,
            } => {
                if let Some(eligibility) = eligibility {
                    let current = self.start_offer(tenant_id, plan_id).await?;
                    let had = current.map_or(TrialEligibility::ProductFamily, |start| {
                        start.offer.eligibility
                    });
                    self.ensure_entitled(organization_id, *eligibility != had)
                        .await?;
                }
                let add_ons: Vec<&OfferGrantNew> = grants
                    .iter()
                    .filter(|grant| matches!(grant.definition, GrantDefinition::AddOn(_)))
                    .collect();
                if !add_ons.is_empty() {
                    let current = self.start_offer(tenant_id, plan_id).await?;
                    let kept = |grant: &OfferGrantNew| {
                        current.as_ref().is_some_and(|start| {
                            start.offer.grants.iter().any(|had| {
                                had.window == grant.window && had.definition == grant.definition
                            })
                        })
                    };
                    self.ensure_entitled(organization_id, !add_ons.iter().all(|grant| kept(grant)))
                        .await?;
                }
                if let Some(terms) = trial {
                    let switches_to_paid = self
                        .services
                        .store()
                        .trial_switches_to_paid(tenant_id, plan_id, terms)
                        .await
                        .map_err(store_status)?;
                    meteroid_app::features::ensure_change_allowed(
                        self.feature_resolver.as_ref(),
                        organization_id,
                        meteroid_app::features::gates::SUBSCRIPTION_PHASES,
                        switches_to_paid,
                        meteroid_app::features::TRIAL_PAID_SWITCH_NOT_IN_PLAN,
                    )
                    .await?;
                }
                Ok(())
            }
        }
    }
}

#[tonic::async_trait]
impl OffersService for OfferServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_offers(
        &self,
        request: Request<proto::ListOffersRequest>,
    ) -> Result<Response<proto::ListOffersResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let limit = req
            .limit
            .map_or(i64::MAX, |limit| i64::from(limit.clamp(1, MAX_PAGE)));
        let offers = self
            .services
            .list_offers(
                tenant_id,
                OfferId::from_proto_opt(req.after)?,
                req.include_archived,
                limit,
            )
            .await
            .map_err(store_status)?;
        Ok(Response::new(proto::ListOffersResponse {
            offers: self.offers_to_proto(tenant_id, offers).await?,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_offer(
        &self,
        request: Request<proto::GetOfferRequest>,
    ) -> Result<Response<proto::GetOfferResponse>, Status> {
        let tenant_id = request.tenant()?;
        let id = OfferId::from_proto(request.into_inner().id)?;
        let offer = self
            .services
            .find_offer(tenant_id, id)
            .await
            .map_err(store_status)?;
        Ok(Response::new(proto::GetOfferResponse {
            offer: Some(self.offer_to_proto(tenant_id, offer).await?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_offer(
        &self,
        request: Request<proto::CreateOfferRequest>,
    ) -> Result<Response<proto::CreateOfferResponse>, Status> {
        self.ensure_entitled(request.organization()?, true).await?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let offer = OfferNew {
            name: req.name,
            description: description(req.description),
            trial: mapping::trial_from_proto(req.trial)?,
            grants: mapping::grants_from_proto(req.grants)?,
            eligibility: mapping::eligibility_from_proto(req.eligibility)?,
        };
        let created = self
            .services
            .create_offer(tenant_id, offer)
            .await
            .map_err(store_status)?;
        Ok(Response::new(proto::CreateOfferResponse {
            offer: Some(self.offer_to_proto(tenant_id, created).await?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_offer(
        &self,
        request: Request<proto::UpdateOfferRequest>,
    ) -> Result<Response<proto::UpdateOfferResponse>, Status> {
        self.ensure_entitled(request.organization()?, true).await?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let id = OfferId::from_proto(&req.id)?;
        let current = self
            .services
            .find_offer(tenant_id, id)
            .await
            .map_err(store_status)?;
        let trial =
            mapping::trial_from_proto(req.trial)?.map(|trial| kept_trial(current.trial, trial));
        let updated = self
            .services
            .update_offer(
                tenant_id,
                id,
                OfferPatch {
                    name: Some(req.name),
                    description: Some(description(req.description)),
                    trial: Some(trial),
                    grants: Some(mapping::grants_from_proto(req.grants)?),
                    eligibility: Some(mapping::eligibility_from_proto(req.eligibility)?),
                },
            )
            .await
            .map_err(store_status)?;
        Ok(Response::new(proto::UpdateOfferResponse {
            offer: Some(self.offer_to_proto(tenant_id, updated).await?),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn archive_offer(
        &self,
        request: Request<proto::ArchiveOfferRequest>,
    ) -> Result<Response<proto::ArchiveOfferResponse>, Status> {
        self.ensure_entitled(request.organization()?, true).await?;
        let tenant_id = request.tenant()?;
        let id = OfferId::from_proto(request.into_inner().id)?;
        let plans = self
            .services
            .archive_offer(tenant_id, id)
            .await
            .map_err(store_status)?;
        Ok(Response::new(proto::ArchiveOfferResponse {
            plan_ids: plans.iter().map(|id| id.as_proto()).collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn set_plan_start_offer(
        &self,
        request: Request<proto::SetPlanStartOfferRequest>,
    ) -> Result<Response<proto::SetPlanStartOfferResponse>, Status> {
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();
        let plan_id = PlanId::from_proto(&req.plan_id)?;
        let change = match req.change.ok_or_else(|| {
            Status::invalid_argument("a start offer is the plan's own, a catalog offer or none")
        })? {
            Change::Inline(inline) => StartOfferChange::Inline {
                trial: inline.trial.map(trial_terms_from_proto).transpose()?,
                grants: mapping::grants_from_proto(inline.grants)?,
                eligibility: inline
                    .eligibility
                    .map(mapping::eligibility_from_proto)
                    .transpose()?,
            },
            Change::CatalogOfferId(id) => StartOfferChange::Catalog(OfferId::from_proto(id)?),
            Change::None(_) => StartOfferChange::None,
        };
        self.ensure_start_offer_allowed(tenant_id, organization_id, plan_id, &change)
            .await?;
        let start = self
            .services
            .set_plan_start_offer(actor, tenant_id, plan_id, change)
            .await
            .map_err(store_status)?;
        Ok(Response::new(proto::SetPlanStartOfferResponse {
            start_offer: self.start_offer_to_proto(tenant_id, start).await?,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_plan_start_offer(
        &self,
        request: Request<proto::GetPlanStartOfferRequest>,
    ) -> Result<Response<proto::GetPlanStartOfferResponse>, Status> {
        let tenant_id = request.tenant()?;
        let plan_id = PlanId::from_proto(request.into_inner().plan_id)?;
        let start = self.start_offer(tenant_id, plan_id).await?;
        Ok(Response::new(proto::GetPlanStartOfferResponse {
            start_offer: self.start_offer_to_proto(tenant_id, start).await?,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn apply_offer(
        &self,
        request: Request<proto::ApplyOfferRequest>,
    ) -> Result<Response<proto::ApplyOfferResponse>, Status> {
        self.ensure_entitled(request.organization()?, true).await?;
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();
        let source = OfferSource::of_actor(&actor);
        let grants = self
            .services
            .apply_offer(
                actor,
                tenant_id,
                SubscriptionId::from_proto(&req.subscription_id)?,
                OfferId::from_proto(&req.offer_id)?,
                source,
            )
            .await
            .map_err(store_status)?;
        Ok(Response::new(proto::ApplyOfferResponse {
            grants: grants
                .iter()
                .map(mapping::subscription_grant_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_grants(
        &self,
        request: Request<proto::ListGrantsRequest>,
    ) -> Result<Response<proto::ListGrantsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let subscription_id = SubscriptionId::from_proto(request.into_inner().subscription_id)?;
        let grants = self
            .services
            .subscription_grants(tenant_id, subscription_id)
            .await
            .map_err(store_status)?;
        Ok(Response::new(proto::ListGrantsResponse {
            grants: grants
                .iter()
                .map(mapping::subscription_grant_to_proto)
                .collect(),
        }))
    }
}
