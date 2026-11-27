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

/// A sign-up plan backfill's `input_params`; its input file holds the customer ids, one a line
/// under a `customer_id` header.
#[derive(Debug, Serialize, Deserialize)]
pub struct SignupPlanBackfillParams {
    pub product_family_id: common_domain::ids::ProductFamilyId,
    /// The sign-up plan's version previewed: the job stops starting customers once it changed.
    pub plan_version_id: common_domain::ids::PlanVersionId,
}

pub const SIGNUP_PLAN_BACKFILL_HEADER: &str = "customer_id";
