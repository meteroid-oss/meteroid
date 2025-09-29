use super::AppState;
use std::io::Cursor;

use axum::extract::Query;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use hyper::StatusCode;

use crate::errors;

use crate::api::sharable::ShareableEntityClaims;
use crate::services::storage::Prefix;
use common_domain::ids::{InvoiceId, StoredDocumentId};
use error_stack::{Report, Result, ResultExt};
use image::ImageFormat::Png;
use jsonwebtoken::{DecodingKey, Validation, decode};
use meteroid_store::repositories::InvoiceInterface;
use secrecy::ExposeSecret;
use serde::Deserialize;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(paths(get_logo, get_invoice_pdf))]
pub struct FileApi;

#[utoipa::path(
    get,
    tag = "file",
    path = "/v1/logo/{uuid}",
    params(
        ("uuid" = Uuid, Path, description = "Logo database UUID")
    ),
    responses(
        (status = 200, content_type = "image/png", description = "Logo as PNG image", body = [u8]),
        (status = 400, description = "Invalid UUID"),
        (status = 500, description = "Internal error"),
    )
)]
#[axum::debug_handler]
pub async fn get_logo(
    Path(uuid): Path<StoredDocumentId>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    match get_logo_handler(uuid, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling logo: {}", e);
            e.current_context().clone().into_response()
        }
    }
}

async fn get_logo_handler(
    image_uuid: StoredDocumentId,
    app_state: AppState,
) -> Result<Response, errors::RestApiError> {
    let data = app_state
        .object_store
        .retrieve(image_uuid, Prefix::ImageLogo)
        .await
        .change_context(errors::RestApiError::ObjectStoreError)?;

    // resize
    let mut img =
        image::load_from_memory(&data).change_context(errors::RestApiError::ImageLoadingError)?;
    img = img.resize(350, 350, image::imageops::FilterType::Nearest);
    let mut buffer = Vec::new();
    img.write_to(&mut Cursor::new(&mut buffer), Png)
        .change_context(errors::RestApiError::ImageLoadingError)?;

    let data = bytes::Bytes::from(buffer);

    Ok((StatusCode::OK, [("Content-Type", "image/png")], data).into_response())
}

#[derive(Deserialize)]
pub struct TokenParams {
    token: String,
}

#[utoipa::path(
    get,
    tag = "file",
    path = "/v1/invoice/pdf/{uid}",
    params(
        ("uid" = String, Path, description = "Invoice database UID"),
        ("token" = str, Query, description = "Security token"),
    ),
    responses(
        (status = 200, content_type = "application/pdf", description = "Invoice in PDF", body = [u8]),
        (status = 400, description = "Invalid UUID or token"),
        (status = 401, description = "Unauthorized - invalid token"),
        (status = 500, description = "Internal error"),
    )
)]
#[axum::debug_handler]
pub async fn get_invoice_pdf(
    Path(uid): Path<InvoiceId>,
    Query(params): Query<TokenParams>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    match get_invoice_pdf_handler(uid, params, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling invoice PDF: {:?}", e);
            e.current_context().clone().into_response()
        }
    }
}

async fn get_invoice_pdf_handler(
    invoice_uid: InvoiceId,
    token: TokenParams,
    app_state: AppState,
) -> Result<Response, errors::RestApiError> {
    let claims = decode::<ShareableEntityClaims>(
        &token.token,
        &DecodingKey::from_secret(app_state.jwt_secret.expose_secret().as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| Report::new(errors::RestApiError::Unauthorized))?
    .claims;

    let invoice = app_state
        .store
        .get_detailed_invoice_by_id(claims.tenant_id, claims.entity_id.into())
        .await
        .change_context(errors::RestApiError::StoreError)?;

    if invoice.invoice.id != invoice_uid {
        return Err(Report::new(errors::RestApiError::Forbidden));
    }
    match invoice.invoice.pdf_document_id {
        Some(id) => {
            let data = app_state
                .object_store
                .retrieve(id, Prefix::InvoicePdf)
                .await
                .change_context(errors::RestApiError::ObjectStoreError)?;

            Ok((
                StatusCode::OK,
                [
                    ("Content-Type", "application/pdf"),
                    ("Content-Disposition", "inline"),
                ],
                data,
            )
                .into_response())
        }
        None => Ok((
            StatusCode::NOT_FOUND,
            "No attached PDF. Generation may be pending",
        )
            .into_response()),
    }
}
