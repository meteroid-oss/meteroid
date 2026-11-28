//! Offers (ADR 0019): reads, a plan's own start offer and the plan's or no offer at creation are
//! Core; the catalog, a catalog offer anywhere, add-on grants and mid-life application are behind
//! `subscription_phases`.
use std::sync::Arc;

use common_domain::ids::OrganizationId;
use meteroid_app::features::{Entitlements, gates};
use meteroid_grpc::meteroid::api::offers::v1::offers_service_server::OffersServiceServer;
use meteroid_store::Services;
use tonic::Status;

pub(crate) mod mapping;
mod service;

pub const OFFERS_NOT_IN_PLAN: &str = "Offers are not available for your plan";

pub struct OfferServiceComponents {
    pub services: Services,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    services: Services,
    feature_resolver: Arc<dyn Entitlements>,
) -> OffersServiceServer<OfferServiceComponents> {
    OffersServiceServer::new(OfferServiceComponents {
        services,
        feature_resolver,
    })
}

pub(crate) async fn ensure_offers_allowed(
    resolver: &dyn Entitlements,
    organization_id: OrganizationId,
    changes: bool,
) -> Result<(), Status> {
    meteroid_app::features::ensure_change_allowed(
        resolver,
        organization_id,
        gates::SUBSCRIPTION_PHASES,
        changes,
        OFFERS_NOT_IN_PLAN,
    )
    .await
}
