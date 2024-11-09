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
use error_stack::{Report, Result, ResultExt};
use fang::Deserialize;
use image::ImageFormat::Png;
use jsonwebtoken::{decode, DecodingKey, Validation};
use meteroid_store::repositories::InvoiceInterface;
use secrecy::ExposeSecret;
use utoipa::OpenApi;
use uuid::Uuid;

#[derive(OpenApi)]
#[openapi(paths(get_logo, get_invoice_pdf))]
pub struct FileApi;

//todo: switch to binary response body
//todo: switch str to uuid
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
    Path(uuid): Path<String>,
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
    image_uuid: String,
    app_state: AppState,
) -> Result<Response, errors::RestApiError> {
    let uid = Uuid::parse_str(&image_uuid).change_context(errors::RestApiError::InvalidInput)?;

    let data = app_state
        .object_store
        .retrieve(uid, Prefix::ImageLogo)
        .await
        .change_context(errors::RestApiError::ObjectStoreError)?;

    // resize
    let mut img =
        image::load_from_memory(&data).change_context(errors::RestApiError::ImageLoadingError)?;
    img = img.resize(350, 20, image::imageops::FilterType::Nearest);
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
    path = "/v1/invoice/pdf/{uuid}",
    params(
        ("uuid" = Uuid, Path, description = "Invoice database UUID"),
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
    Path(uuid): Path<String>,
    Query(params): Query<TokenParams>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    match get_invoice_pdf_handler(uuid, params, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling invoice PDF: {}", e);
            e.current_context().clone().into_response()
        }
    }
}

async fn get_invoice_pdf_handler(
    invoice_uuid: String,
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
        .find_invoice_by_id(claims.tenant_id, claims.entity_id)
        .await
        .change_context(errors::RestApiError::StoreError)?;

    if invoice.invoice.local_id != invoice_uuid {
        return Err(Report::new(errors::RestApiError::Forbidden));
    }
    match invoice.invoice.pdf_document_id {
        Some(uid) => {
            let data = app_state
                .object_store
                .retrieve(
                    Uuid::parse_str(&uid).change_context(errors::RestApiError::StoreError)?,
                    Prefix::InvoicePdf,
                )
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
