use bytes::Bytes;
use common_domain::ids::{BankAccountId, ConnectorId, InvoicingEntityId};
use common_grpc::middleware::server::auth::RequestExt;
use image::ImageFormat;
use meteroid_app::features::{
    MINIMUM_COMMITMENTS_NOT_IN_PLAN, PORTAL_THEMING_NOT_IN_PLAN, ensure_change_allowed, gates,
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

        let sets_theme = patch.portal_theme_mode.is_some() || patch.portal_roundness.is_some();
        if sets_theme {
            let stored = self
                .store
                .get_invoicing_entity(tenant, Some(id))
                .await
                .map_err(Into::<InvoicingEntitiesApiError>::into)?;
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

        // A minimum charge is set or changed under the commitments gate; clearing it stays open.
        if let Some(Some(minimum)) = patch.min_charge_amount {
            let stored = self
                .store
                .get_invoicing_entity(tenant, Some(id))
                .await
                .map_err(Into::<InvoicingEntitiesApiError>::into)?;
            ensure_change_allowed(
                self.feature_resolver.as_ref(),
                organization,
                gates::MINIMUM_COMMITMENTS,
                stored.min_charge_amount != Some(minimum),
                MINIMUM_COMMITMENTS_NOT_IN_PLAN,
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
                    return Err(InvoicingEntitiesApiError::InvalidArgument(format!(
                        "Logo is {:.1} MB, the maximum is {} MB.",
                        logo_bytes.len() as f64 / (1024.0 * 1024.0),
                        MAX_IMAGE_SIZE / (1024 * 1024)
                    ))
                    .into());
                }
                let logo_bytes = tokio::task::spawn_blocking(move || process_image(&logo_bytes))
                    .await
                    .map_err(|e| Status::internal(format!("Logo processing failed: {e}")))?
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
const MAX_IMAGE_SIZE: usize = 2 * 1024 * 1024;
const MAX_H: u32 = 160;
const MAX_W: u32 = 2048;
const MAX_SOURCE_DIMENSION: u32 = 10_000;

fn process_image(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let img = match image::guess_format(bytes) {
        Ok(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP) => decode_raster(bytes)?,
        Ok(_) => return Err(UNSUPPORTED_FORMAT.to_string()),
        Err(_) if looks_like_svg(bytes) => rasterize_svg(bytes)?,
        Err(_) => return Err(UNSUPPORTED_FORMAT.to_string()),
    };

    let img = if img.width() > MAX_W || img.height() > MAX_H {
        img.resize(MAX_W, MAX_H, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    let mut buffer = Cursor::new(Vec::new());
    img.write_to(&mut buffer, ImageFormat::Png)
        .map_err(|e| format!("Failed to encode image: {e}"))?;

    Ok(buffer.into_inner())
}

const UNSUPPORTED_FORMAT: &str = "Unsupported image format. Use PNG, JPEG, WebP or SVG.";

fn decode_raster(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("Failed to read image: {e}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_DIMENSION);
    limits.max_image_height = Some(MAX_SOURCE_DIMENSION);
    reader.limits(limits);
    reader.decode().map_err(|e| match e {
        image::ImageError::Limits(_) => format!(
            "Image dimensions are too large (max {MAX_SOURCE_DIMENSION}×{MAX_SOURCE_DIMENSION} px)."
        ),
        e => format!("Failed to load image: {e}"),
    })
}

// Plain-text SVG only: gzipped SVGZ is rejected to avoid decompression bombs.
fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let trimmed = head.trim_ascii_start();
    trimmed.starts_with(b"<") && head.windows(4).any(|w| w.eq_ignore_ascii_case(b"<svg"))
}

fn rasterize_svg(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let options = resvg::usvg::Options {
        // Never resolve hrefs to local files; embedded data: URIs still work.
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: resvg::usvg::ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_data(bytes, &options)
        .map_err(|e| format!("Failed to parse SVG: {e}"))?;

    let size = tree.size();
    let scale = (MAX_W as f32 / size.width()).min(MAX_H as f32 / size.height());
    let width = (size.width() * scale).round().max(1.0) as u32;
    let height = (size.height() * scale).round().max(1.0) as u32;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| "SVG has invalid dimensions.".to_string())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    let rgba = image::RgbaImage::from_raw(width, height, demultiply(pixmap))
        .ok_or_else(|| "Failed to rasterize SVG.".to_string())?;
    Ok(image::DynamicImage::ImageRgba8(rgba))
}

fn demultiply(pixmap: resvg::tiny_skia::Pixmap) -> Vec<u8> {
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(width, height)
            .write_to(&mut buf, ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    fn decoded(bytes: &[u8]) -> image::DynamicImage {
        image::load_from_memory_with_format(bytes, ImageFormat::Png).unwrap()
    }

    #[test]
    fn large_png_is_downscaled_within_bounds() {
        let out = decoded(&process_image(&png(1000, 500)).unwrap());
        assert_eq!((out.width(), out.height()), (320, 160));
    }

    #[test]
    fn svg_is_rasterized_to_fit_height() {
        let svg = br##"<?xml version="1.0"?>
            <svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">
              <rect width="40" height="20" fill="#ff0000"/>
            </svg>"##;
        let out = decoded(&process_image(svg).unwrap()).to_rgba8();
        assert_eq!(out.dimensions(), (320, 160));
        assert_eq!(out.get_pixel(10, 10).0, [255, 0, 0, 255]);
    }

    #[test]
    fn svg_local_file_href_is_not_resolved() {
        let path = std::env::temp_dir().join(format!("logo-href-{}.png", std::process::id()));
        let opaque = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            10,
            10,
            image::Rgba([0, 0, 255, 255]),
        ));
        opaque.save(&path).unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">
              <image width="10" height="10" xlink:href="{}"/>
            </svg>"#,
            path.display()
        );
        let result = process_image(svg.as_bytes());
        std::fs::remove_file(&path).ok();
        let out = decoded(&result.unwrap()).to_rgba8();
        assert!(out.pixels().all(|p| p.0[3] == 0));
    }

    #[test]
    fn unsupported_formats_are_rejected() {
        assert_eq!(process_image(b"hello").unwrap_err(), UNSUPPORTED_FORMAT);
        let mut gif = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 2)
            .write_to(&mut gif, ImageFormat::Gif)
            .unwrap();
        assert_eq!(
            process_image(&gif.into_inner()).unwrap_err(),
            UNSUPPORTED_FORMAT
        );
    }
}
