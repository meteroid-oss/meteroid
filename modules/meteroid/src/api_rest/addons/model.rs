use crate::api_rest::model::{PaginatedRequest, PaginationResponse};
use crate::api_rest::products::model::ProductFeeTypeEnum;
use chrono::NaiveDateTime;
use common_domain::ids::{AddOnId, PriceId, ProductId};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

// ── Response ───────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AddOn {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: AddOnId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub product_id: ProductId,
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub price_id: PriceId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_type: Option<ProductFeeTypeEnum>,
    pub self_serviceable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_instances_per_subscription: Option<i32>,
    #[serde(serialize_with = "crate::api_rest::model::serialize_datetime")]
    pub created_at: NaiveDateTime,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::api_rest::model::serialize_datetime_opt"
    )]
    pub archived_at: Option<NaiveDateTime>,
}

// ── Requests ───────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct CreateAddOnRequest {
    #[validate(length(min = 1))]
    pub name: String,
    pub product_id: ProductId,
    pub price_id: PriceId,
    pub description: Option<String>,
    #[serde(default)]
    pub self_serviceable: bool,
    pub max_instances_per_subscription: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateAddOnRequest {
    #[validate(length(min = 1))]
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub price_id: Option<PriceId>,
    pub self_serviceable: Option<bool>,
    pub max_instances_per_subscription: Option<Option<i32>>,
}

// ── List ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AddOnListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    pub search: Option<String>,
    pub currency: Option<String>,
    /// Include archived add-ons in the results (default: false)
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AddOnListResponse {
    pub data: Vec<AddOn>,
    pub pagination_meta: PaginationResponse,
}
