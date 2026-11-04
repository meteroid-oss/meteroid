use common_domain::ids::{PlanId, PurchaseLinkId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::purchaselinks::v1::purchase_links_service_server::PurchaseLinksService;
use meteroid_grpc::meteroid::api::purchaselinks::v1::{
    ArchivePurchaseLinkRequest, ArchivePurchaseLinkResponse, CreatePurchaseLinkRequest,
    CreatePurchaseLinkResponse, ListPurchaseLinksRequest, ListPurchaseLinksResponse,
    RegeneratePurchaseLinkUrlRequest, RegeneratePurchaseLinkUrlResponse, UpdatePurchaseLinkRequest,
    UpdatePurchaseLinkResponse,
};
use meteroid_store::domain::purchase_links::{PurchaseLinkNew, PurchaseLinkUpdate};
use meteroid_store::repositories::purchase_links::PurchaseLinksInterface;
use tonic::{Request, Response, Status};

use super::PurchaseLinksServiceComponents;
use super::error::PurchaseLinkApiError;
use super::mapping::{
    allowed_buyer_types_from_server, domain_to_server, tax_id_collection_from_server,
};

/// Empty strings from the UI mean "no URL".
fn optional_url(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[tonic::async_trait]
impl PurchaseLinksService for PurchaseLinksServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_purchase_links(
        &self,
        request: Request<ListPurchaseLinksRequest>,
    ) -> Result<Response<ListPurchaseLinksResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let plan_id = PlanId::from_proto_opt(req.plan_id)?;

        let links = self
            .store
            .list_purchase_links(tenant_id, plan_id, req.include_archived)
            .await
            .map_err(Into::<PurchaseLinkApiError>::into)?;

        Ok(Response::new(ListPurchaseLinksResponse {
            links: links
                .into_iter()
                .map(|l| domain_to_server(l, &self.public_url))
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_purchase_link(
        &self,
        request: Request<CreatePurchaseLinkRequest>,
    ) -> Result<Response<CreatePurchaseLinkResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let allowed_buyer_types = allowed_buyer_types_from_server(req.allowed_buyer_types());
        let tax_id_collection = tax_id_collection_from_server(req.tax_id_collection());

        let link = self
            .store
            .create_purchase_link(PurchaseLinkNew {
                tenant_id,
                plan_id: PlanId::from_proto(req.plan_id)?,
                name: req.name,
                allow_coupon_codes: req.allow_coupon_codes,
                listed: req.listed,
                success_url: optional_url(req.success_url),
                cancel_url: optional_url(req.cancel_url),
                require_billing_address: req.require_billing_address.unwrap_or(true),
                tax_id_collection,
                allowed_buyer_types,
            })
            .await
            .map_err(Into::<PurchaseLinkApiError>::into)?;

        Ok(Response::new(CreatePurchaseLinkResponse {
            link: Some(domain_to_server(link, &self.public_url)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_purchase_link(
        &self,
        request: Request<UpdatePurchaseLinkRequest>,
    ) -> Result<Response<UpdatePurchaseLinkResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let allowed_buyer_types = allowed_buyer_types_from_server(req.allowed_buyer_types());
        let tax_id_collection = tax_id_collection_from_server(req.tax_id_collection());

        let link = self
            .store
            .update_purchase_link(
                tenant_id,
                PurchaseLinkId::from_proto(req.id)?,
                PurchaseLinkUpdate {
                    name: req.name,
                    allow_coupon_codes: req.allow_coupon_codes,
                    listed: req.listed,
                    success_url: optional_url(req.success_url),
                    cancel_url: optional_url(req.cancel_url),
                    require_billing_address: req.require_billing_address.unwrap_or(true),
                    tax_id_collection,
                    allowed_buyer_types,
                },
            )
            .await
            .map_err(Into::<PurchaseLinkApiError>::into)?;

        Ok(Response::new(UpdatePurchaseLinkResponse {
            link: Some(domain_to_server(link, &self.public_url)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn regenerate_purchase_link_url(
        &self,
        request: Request<RegeneratePurchaseLinkUrlRequest>,
    ) -> Result<Response<RegeneratePurchaseLinkUrlResponse>, Status> {
        let tenant_id = request.tenant()?;
        let id = PurchaseLinkId::from_proto(request.into_inner().id)?;

        let link = self
            .store
            .regenerate_purchase_link_token(tenant_id, id)
            .await
            .map_err(Into::<PurchaseLinkApiError>::into)?;

        Ok(Response::new(RegeneratePurchaseLinkUrlResponse {
            link: Some(domain_to_server(link, &self.public_url)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn archive_purchase_link(
        &self,
        request: Request<ArchivePurchaseLinkRequest>,
    ) -> Result<Response<ArchivePurchaseLinkResponse>, Status> {
        let tenant_id = request.tenant()?;
        let id = PurchaseLinkId::from_proto(request.into_inner().id)?;

        let link = self
            .store
            .archive_purchase_link(tenant_id, id)
            .await
            .map_err(Into::<PurchaseLinkApiError>::into)?;

        Ok(Response::new(ArchivePurchaseLinkResponse {
            link: Some(domain_to_server(link, &self.public_url)),
        }))
    }
}
