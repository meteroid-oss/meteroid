use bytes::Bytes;
use image::ImageFormat;
use std::io::Cursor;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::invoicingentities::v1::{
    invoicing_entities_service_server::InvoicingEntitiesService, CreateInvoicingEntityRequest,
    CreateInvoicingEntityResponse, GetInvoicingEntityRequest, GetInvoicingEntityResponse,
    ListInvoicingEntitiesRequest, ListInvoicingEntitiesResponse, UpdateInvoicingEntityRequest,
    UpdateInvoicingEntityResponse, UploadInvoicingEntityLogoRequest,
    UploadInvoicingEntityLogoResponse,
};
use meteroid_store::domain::{Identity, InvoicingEntityPatch};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;

use crate::api::invoicingentities::error::InvoicingEntitiesApiError;
use crate::api::shared::conversions::{FromProtoOpt, ProtoConv};
use crate::services::storage::Prefix;

use super::{mapping, InvoicingEntitiesServiceComponents};

#[tonic::async_trait]
impl InvoicingEntitiesService for InvoicingEntitiesServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_invoicing_entity(
        &self,
        request: Request<GetInvoicingEntityRequest>,
    ) -> Result<Response<GetInvoicingEntityResponse>, Status> {
        let tenant = request.tenant()?;
        let id = Uuid::from_proto_opt(request.into_inner().id)?.map(Identity::UUID);

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant, id)
            .await
            .map_err(Into::<InvoicingEntitiesApiError>::into)?;

        Ok(Response::new(GetInvoicingEntityResponse {
            entity: Some(mapping::invoicing_entities::domain_to_proto(
                invoicing_entity,
            )),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_invoicing_entities(
        &self,
        request: Request<ListInvoicingEntitiesRequest>,
    ) -> Result<Response<ListInvoicingEntitiesResponse>, Status> {
        let tenant = request.tenant()?;

        let invoicing_entities = self
            .store
            .list_invoicing_entities(tenant)
            .await
            .map_err(Into::<InvoicingEntitiesApiError>::into)?
            .into_iter()
            .map(mapping::invoicing_entities::domain_to_proto)
            .collect();

        let response = ListInvoicingEntitiesResponse {
            entities: invoicing_entities,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn create_invoicing_entity(
        &self,
        request: Request<CreateInvoicingEntityRequest>,
    ) -> Result<Response<CreateInvoicingEntityResponse>, Status> {
        let tenant = request.tenant()?;
        let organization = request.organization()?;

        let data = request
            .into_inner()
            .data
            .ok_or_else(|| Status::invalid_argument("Missing data"))?;

        let res = self
            .store
            .create_invoicing_entity(
                mapping::invoicing_entities::proto_to_domain(data),
                tenant,
                organization,
            )
            .await
            .map_err(Into::<InvoicingEntitiesApiError>::into)?;

        Ok(Response::new(CreateInvoicingEntityResponse {
            entity: Some(mapping::invoicing_entities::domain_to_proto(res)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_invoicing_entity(
        &self,
        request: Request<UpdateInvoicingEntityRequest>,
    ) -> Result<Response<UpdateInvoicingEntityResponse>, Status> {
        let tenant = request.tenant()?;
        let req = request.into_inner();

        let data = req
            .data
            .ok_or_else(|| Status::invalid_argument("Missing data"))?;

        let res = self
            .store
            .patch_invoicing_entity(
                mapping::invoicing_entities::proto_to_patch_domain(data, Uuid::from_proto(req.id)?),
                tenant,
            )
            .await
            .map_err(Into::<InvoicingEntitiesApiError>::into)?;

        Ok(Response::new(UpdateInvoicingEntityResponse {
            entity: Some(mapping::invoicing_entities::domain_to_proto(res)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn upload_invoicing_entity_logo(
        &self,
        request: Request<UploadInvoicingEntityLogoRequest>,
    ) -> Result<Response<UploadInvoicingEntityLogoResponse>, Status> {
        let tenant = request.tenant()?;
        let req = request.into_inner();

        let logo_attachment_id = match req.file {
            None => None,
            Some(file) => {
                let logo_bytes = file.data;
                if logo_bytes.len() > MAX_IMAGE_SIZE {
                    return Err(Status::invalid_argument(
                        "Image size exceeds maximum allowed",
                    ));
                }
                let logo_bytes = process_image(&logo_bytes)
                    .map_err(InvoicingEntitiesApiError::InvalidArgument)?;

                let res = self
                    .object_store
                    .store(Bytes::from(logo_bytes), Prefix::ImageLogo)
                    .await
                    .map_err(Into::<InvoicingEntitiesApiError>::into)?;

                Some(res.to_string())
            }
        };

        self.store
            .patch_invoicing_entity(
                InvoicingEntityPatch {
                    id: Uuid::from_proto(req.id)?,
                    logo_attachment_id: Some(logo_attachment_id.clone()), // Option<Option<Uuid>> as we need to set it to None if no logo is uploaded
                    ..InvoicingEntityPatch::default()
                },
                tenant,
            )
            .await
            .map_err(Into::<InvoicingEntitiesApiError>::into)?;

        Ok(Response::new(UploadInvoicingEntityLogoResponse {
            logo_uid: logo_attachment_id,
        }))
    }
}
const MAX_IMAGE_SIZE: usize = 2 * 1024 * 1024; // 2 MB
const MAX_H: u32 = 160;
const MAX_W: u32 = 1024;

fn process_image(bytes: &[u8]) -> Result<Vec<u8>, String> {
    // Validate image format
    let format = image::guess_format(bytes).map_err(|_| "Unable to determine image format")?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err("Unsupported image format. Only PNG, JPEG and WebP are allowed.".to_string());
    }

    let img = image::load_from_memory(bytes).map_err(|e| format!("Failed to load image: {}", e))?;

    // Resize if necessary
    let img = if img.width() > MAX_W || img.height() > MAX_H {
        img.resize(MAX_W, MAX_H, image::imageops::FilterType::Nearest)
    } else {
        img
    };

    // Convert to PNG
    let mut buffer = Cursor::new(Vec::new());
    img.write_to(&mut buffer, ImageFormat::Png)
        .map_err(|e| format!("Failed to encode image: {}", e))?;

    Ok(buffer.into_inner())
}
