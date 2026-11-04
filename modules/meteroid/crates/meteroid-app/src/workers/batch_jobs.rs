use chrono::NaiveDate;
use common_domain::ids::InvoicingEntityId;
use serde::{Deserialize, Serialize};

/// Parameters carried in the job's `input_params` (JSONB).
#[derive(Debug, Serialize, Deserialize)]
pub struct TaxReportExportParams {
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    #[serde(default)]
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    /// Optional override for the per-chunk date-range width (days).
    #[serde(default)]
    pub bucket_days: Option<i64>,
}
