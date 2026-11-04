//! Correlation fields shared by logs and traces.
//!
//! A `tracing` span field named in [`FIELDS`] is copied onto every log record emitted inside
//! that span, so it reaches Loki as structured metadata (`| tenant_id="tnt_..."`) next to the
//! span attribute Tempo indexes (`{span.tenant_id="tnt_..."}`). Other span fields stay on the
//! trace only, which keeps log records small.
//!
//! Declare a field at the outermost span that knows its value, and not again in nested spans:
//! every ancestor's copy is added to the record, so a repeated field is a repeated attribute.
//! Values are the public base62 ids (`Display`), the form the UI, the API and webhooks show.

pub const TENANT_ID: &str = "tenant_id";
pub const ORGANIZATION_ID: &str = "organization_id";
pub const CUSTOMER_ID: &str = "customer_id";
pub const SUBSCRIPTION_ID: &str = "subscription_id";
pub const INVOICE_ID: &str = "invoice_id";
pub const BATCH_JOB_ID: &str = "batch_job_id";

pub const FIELDS: [&str; 6] = [
    TENANT_ID,
    ORGANIZATION_ID,
    CUSTOMER_ID,
    SUBSCRIPTION_ID,
    INVOICE_ID,
    BATCH_JOB_ID,
];
