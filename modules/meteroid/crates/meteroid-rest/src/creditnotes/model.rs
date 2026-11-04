use crate::currencies::model::Currency;
use crate::invoices::model::{InvoiceLineItem, TaxBreakdownItem};
use common_domain::ids::{
    CreditNoteId, CustomerId, InvoiceId, PlanVersionId, SubscriptionId, string_serde,
    string_serde_opt,
};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::model::{PaginatedRequest, PaginationResponse};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

#[derive(o2o, Debug, Clone, Serialize, Deserialize, ToSchema)]
#[map_owned(meteroid_store::domain::enums::CreditNoteStatus)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schema(title = "CreditNoteStatus")]
pub enum CreditNoteStatus {
    Draft,
    Finalized,
    Voided,
}

#[derive(o2o, Debug, Clone, Serialize, Deserialize, ToSchema)]
#[map_owned(meteroid_store::domain::enums::CreditType)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schema(title = "CreditType")]
pub enum CreditType {
    CreditToBalance,
    Refund,
    DebtCancellation,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreditNote {
    #[serde(with = "string_serde")]
    pub id: CreditNoteId,
    pub credit_note_number: String,
    pub status: CreditNoteStatus,
    pub credit_type: CreditType,
    #[serde(with = "string_serde")]
    pub customer_id: CustomerId,
    #[serde(with = "string_serde")]
    pub invoice_id: InvoiceId,
    pub invoice_number: String,
    #[serde(default, with = "string_serde_opt")]
    pub subscription_id: Option<SubscriptionId>,
    #[serde(default, with = "string_serde_opt")]
    pub plan_version_id: Option<PlanVersionId>,
    pub currency: Currency,
    pub subtotal: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub refunded_amount_cents: i64,
    pub credited_amount_cents: i64,
    pub line_items: Vec<InvoiceLineItem>,
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    pub reason: Option<String>,
    pub memo: Option<String>,
    pub created_at: ApiDateTime,
    pub updated_at: Option<ApiDateTime>,
    pub finalized_at: Option<ApiDateTime>,
    pub voided_at: Option<ApiDateTime>,
    /// User-defined custom property values, keyed by definition `key`.
    #[schema(value_type = serde_json::Value)]
    pub custom_properties: serde_json::Value,
}

/// Merge update of a credit note's custom property values (send a key with `null` to remove it).
/// Allowed at any status — custom properties stay editable after the credit note is finalized.
#[derive(Debug, Serialize, Deserialize, ToSchema, Validate)]
pub struct CreditNoteCustomPropertiesRequest {
    #[schema(value_type = serde_json::Value)]
    pub custom_properties: serde_json::Value,
}

#[derive(Deserialize, Validate, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct CreditNoteListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    /// Filter by customer ID
    #[serde(default, with = "string_serde_opt")]
    pub customer_id: Option<CustomerId>,
    /// Filter by invoice ID
    #[serde(default, with = "string_serde_opt")]
    pub invoice_id: Option<InvoiceId>,
    #[param(inline)]
    #[serde(default)]
    pub status: Option<CreditNoteStatus>,
    /// Free-text search over credit note number.
    pub search: Option<String>,
    /// Sort order. Format: `column.direction`. Allowed columns: `created_at`, `credit_note_number`, `total`, `status`. Direction: `asc` or `desc`. Default: `created_at.desc`.
    pub order_by: Option<String>,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct CreditNoteListResponse {
    pub data: Vec<CreditNote>,
    pub pagination_meta: PaginationResponse,
}
