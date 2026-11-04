use common_domain::ids::{BatchJobChunkId, BatchJobId, string_serde};
use meteroid_rest_common::datetime::ApiDateTime;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use meteroid_rest_common::model::{PaginatedRequest, PaginationResponse};

// --- Query params ---

#[derive(ToSchema, IntoParams, Serialize, Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct BatchJobListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    #[param(inline)]
    pub job_type: Option<BatchJobType>,
    #[param(inline)]
    #[serde(default)]
    pub status: Option<Vec<BatchJobStatus>>,
}

fn default_limit() -> u32 {
    25
}

// --- Enums (REST representation) ---

#[derive(o2o, Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[map_owned(meteroid_store::domain::enums::BatchJobTypeEnum)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schema(title = "BatchJobType")]
pub enum BatchJobType {
    EventCsvImport,
    CustomerCsvImport,
    SubscriptionCsvImport,
    SubscriptionPlanMigration,
    TaxReportExport,
}

#[derive(o2o, Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[map_owned(meteroid_store::domain::enums::BatchJobStatusEnum)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schema(title = "BatchJobStatus")]
pub enum BatchJobStatus {
    Pending,
    Chunking,
    Processing,
    Completed,
    CompletedWithErrors,
    Failed,
    Cancelled,
}

// --- Response types ---

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BatchJobResponse {
    #[serde(with = "string_serde")]
    pub id: BatchJobId,
    pub job_type: BatchJobType,
    pub status: BatchJobStatus,
    pub total_items: Option<i32>,
    pub processed_items: i32,
    pub failed_items: i32,
    pub created_by: Uuid,
    pub created_at: ApiDateTime,
    pub completed_at: Option<ApiDateTime>,
    pub input_file_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BatchJobDetailResponse {
    #[serde(with = "string_serde")]
    pub id: BatchJobId,
    pub job_type: BatchJobType,
    pub status: BatchJobStatus,
    pub total_items: Option<i32>,
    pub processed_items: i32,
    pub failed_items: i32,
    pub created_by: Uuid,
    pub created_at: ApiDateTime,
    pub completed_at: Option<ApiDateTime>,
    pub failure_count: i64,
    pub has_error_csv: bool,
    pub error_csv_url: Option<String>,
    pub input_file_name: Option<String>,
    pub input_file_url: Option<String>,
    pub has_output: bool,
    pub output_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BatchJobItemFailureResponse {
    pub id: Uuid,
    #[serde(with = "string_serde")]
    pub chunk_id: BatchJobChunkId,
    pub item_index: i32,
    pub item_identifier: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BatchJobListResponse {
    pub data: Vec<BatchJobResponse>,
    pub pagination_meta: PaginationResponse,
}

#[derive(ToSchema, IntoParams, Serialize, Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct BatchJobFailuresRequest {
    pub chunk_id: Option<BatchJobChunkId>,
    #[serde(default = "default_limit")]
    #[validate(range(min = 1, max = 100))]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BatchJobFailuresResponse {
    pub data: Vec<BatchJobItemFailureResponse>,
    pub total_count: i64,
}
