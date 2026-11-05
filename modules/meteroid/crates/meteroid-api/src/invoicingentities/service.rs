use bytes::Bytes;
use common_domain::ids::{BankAccountId, ConnectorId, InvoicingEntityId};
use common_grpc::middleware::server::auth::RequestExt;
use image::ImageFormat;
use meteroid_app::features::{
    EINVOICING_NOT_IN_PLAN, PORTAL_THEMING_NOT_IN_PLAN, ensure_change_allowed,
    ensure_credit_pricing_allowed, gates,
};
use meteroid_grpc::meteroid::api::invoicingentities::v1::{
    GetInvoicingEntityProvidersRequest, GetInvoicingEntityProvidersResponse,
    GetInvoicingEntityRequest, GetInvoicingEntityResponse, ListInvoicingEntitiesRequest,
    ListInvoicingEntitiesResponse, UpdateInvoicingEntityProvidersRequest,
    UpdateInvoicingEntityProvidersResponse, UpdateInvoicingEntityRequest,
    UpdateInvoicingEntityResponse, UploadInvoicingEntityLogoRequest,
    UploadInvoicingEntityLogoResponse, invoicing_entities_service_server::InvoicingEntitiesService,
};
use meteroid_store::domain::{InvoicingEntityPatch, InvoicingEntityProvidersPatch};
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use std::io::Cursor;
use tonic::{Request, Response, Status};

use crate::invoicingentities::error::InvoicingEntitiesApiError;
use meteroid_app::services::storage::{self, ObjectKind};

use super::{InvoicingEntitiesServiceComponents, mapping};

#[tonic::async_trait]
impl InvoicingEntitiesService for InvoicingEntitiesServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_invoicing_entity(
        &self,
        request: Request<GetInvoicingEntityRequest>,
    ) -> Result<Response<GetInvoicingEntityResponse>, Status> {
        let tenant = request.tenant()?;
        let id = InvoicingEntityId::from_proto_opt(request.into_inner().id)?;

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
    async fn update_invoicing_entity(
        &self,
        request: Request<UpdateInvoicingEntityRequest>,
    ) -> Result<Response<UpdateInvoicingEntityResponse>, Status> {
        let tenant = request.tenant()?;
        let organization = request.organization()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let data = req
            .data
            .ok_or_else(|| Status::invalid_argument("Missing data"))?;
        let id = InvoicingEntityId::from_proto(req.id)?;
        let patch = mapping::invoicing_entities::proto_to_patch_domain(data, id)?;

        let sets_format = patch.einvoicing_format.flatten();
        let enables_top_ups = patch.portal_credit_top_up_enabled == Some(true);
        let sets_theme = patch.portal_theme_mode.is_some() || patch.portal_roundness.is_some();
        if sets_format.is_some() || enables_top_ups || sets_theme {
            let stored = self
                .store
                .get_invoicing_entity(tenant, Some(id))
                .await
                .map_err(Into::<InvoicingEntitiesApiError>::into)?;
            ensure_change_allowed(
                self.feature_resolver.as_ref(),
                organization,
                gates::EINVOICING,
                sets_format.is_some_and(|format| stored.einvoicing_format != Some(format)),
                EINVOICING_NOT_IN_PLAN,
            )
            .await?;
            ensure_credit_pricing_allowed(
                self.feature_resolver.as_ref(),
                organization,
                enables_top_ups && !stored.portal_credit_top_up_enabled,
            )
            .await?;
            // The settings form fills an unset value with the portal's built-in default.
            let changes = |new: &Option<Option<String>>, stored: &Option<String>, default: &str| {
                new.as_ref().is_some_and(|new| {
                    new.as_deref()
                        .is_some_and(|new| new != stored.as_deref().unwrap_or(default))
                })
            };
            let changes_theme =
                changes(&patch.portal_theme_mode, &stored.portal_theme_mode, "dark")
                    || changes(&patch.portal_roundness, &stored.portal_roundness, "Modern");
            ensure_change_allowed(
                self.feature_resolver.as_ref(),
                organization,
                gates::PORTAL_THEMING,
                changes_theme,
                PORTAL_THEMING_NOT_IN_PLAN,
            )
            .await?;
        }

        let res = self
            .store
            .patch_invoicing_entity(actor, patch, tenant)
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
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let uploaded = match req.file {
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

                let stored = storage::put(
                    self.object_store.as_ref(),
                    &self.store,
                    ObjectKind::ImageLogo,
                    tenant,
                    Bytes::from(logo_bytes),
                )
                .await
                .map_err(Into::<InvoicingEntitiesApiError>::into)?;

                Some(stored)
            }
        };

        self.store
            .patch_invoicing_entity(
                actor,
                InvoicingEntityPatch {
                    id: InvoicingEntityId::from_proto(req.id)?,
                    // outer Some always patches the field; inner None clears it when no logo
                    logo_attachment_id: Some(uploaded.as_ref().map(|s| s.id)),
                    ..InvoicingEntityPatch::default()
                },
                tenant,
            )
            .await
            .map_err(Into::<InvoicingEntitiesApiError>::into)?;

        Ok(Response::new(UploadInvoicingEntityLogoResponse {
            logo_uid: uploaded.map(|s| s.id.to_string()),
        }))
    }

    async fn get_invoicing_entity_providers(
        &self,
        request: Request<GetInvoicingEntityProvidersRequest>,
    ) -> Result<Response<GetInvoicingEntityProvidersResponse>, Status> {
        let tenant = request.tenant()?;

        let req = request.into_inner();

        let res = self
            .store
            .resolve_providers_by_id(tenant, InvoicingEntityId::from_proto(req.id)?)
            .await
            .map_err(Into::<InvoicingEntitiesApiError>::into)?;

        Ok(Response::new(GetInvoicingEntityProvidersResponse {
            card_provider: res.card_provider.and_then(|c| {
                super::super::connectors::mapping::connectors::connector_meta_to_server(&c)
            }),
            direct_debit_provider: res.direct_debit_provider.and_then(|c| {
                super::super::connectors::mapping::connectors::connector_meta_to_server(&c)
            }),
            bank_account: res
                .bank_account
                .map(super::super::bankaccounts::mapping::bank_accounts::domain_to_proto),
        }))
    }

    async fn update_invoicing_entity_providers(
        &self,
        request: Request<UpdateInvoicingEntityProvidersRequest>,
    ) -> Result<Response<UpdateInvoicingEntityProvidersResponse>, Status> {
        let tenant = request.tenant()?;
        let actor = request.actor_typed()?;

        let req = request.into_inner();

        let res = self
            .store
            .patch_invoicing_entity_providers(
                actor,
                InvoicingEntityProvidersPatch {
                    bank_account_id: Some(BankAccountId::from_proto_opt(req.bank_account_id)?),
                    card_provider_id: Some(ConnectorId::from_proto_opt(req.card_provider_id)?),
                    direct_debit_provider_id: Some(ConnectorId::from_proto_opt(
                        req.direct_debit_provider_id,
                    )?),
                    id: InvoicingEntityId::from_proto(req.id)?,
                },
                tenant,
            )
            .await
            .map_err(Into::<InvoicingEntitiesApiError>::into)?;

        Ok(Response::new(UpdateInvoicingEntityProvidersResponse {
            card_provider: res.card_provider.and_then(|c| {
                super::super::connectors::mapping::connectors::connector_meta_to_server(&c)
            }),
            direct_debit_provider: res.direct_debit_provider.and_then(|c| {
                super::super::connectors::mapping::connectors::connector_meta_to_server(&c)
            }),
            bank_account: res
                .bank_account
                .map(super::super::bankaccounts::mapping::bank_accounts::domain_to_proto),
        }))
    }
}
const MAX_IMAGE_SIZE: usize = 1024 * 1024; // 1 MB
const MAX_H: u32 = 160;
const MAX_W: u32 = 2048;

fn process_image(bytes: &[u8]) -> Result<Vec<u8>, String> {
    // Validate image format
    let format = image::guess_format(bytes).map_err(|_| "Unable to determine image format")?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err("Unsupported image format. Only PNG, JPEG and WebP are allowed.".to_string());
    }

    let img = image::load_from_memory(bytes).map_err(|e| format!("Failed to load image: {e}"))?;

    // Resize if necessary
    let img = if img.width() > MAX_W || img.height() > MAX_H {
        img.resize(MAX_W, MAX_H, image::imageops::FilterType::Nearest)
    } else {
        img
    };

    // Convert to PNG
    let mut buffer = Cursor::new(Vec::new());
    img.write_to(&mut buffer, ImageFormat::Png)
        .map_err(|e| format!("Failed to encode image: {e}"))?;

    Ok(buffer.into_inner())
}
