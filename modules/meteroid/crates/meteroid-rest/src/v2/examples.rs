//! The examples the v2 document shows (ADR-0013 §3): the first integration told as one story,
//! each operation's request and response in `examples.json` as the API answered them. The tenant
//! sells an AI API: the customer `northwind` (`Northwind Labs GmbH`, in EUR) subscribes to the
//! plan `growth` (a platform fee, `seats`, and `tokens` sold in usage levels, the add-on
//! `support`), adds seats, a level and the add-on, moves to `scale` at the period's end, is
//! invoiced and pays; its usage is ingested as `completion` events; the tenant's webhook endpoint
//! receives the billing events. Every id is the one the API gave, in its prefixed form.
use serde_json::Value;
use std::sync::LazyLock;

/// The examples, by operation id: each `{request?, response}`.
static EXAMPLES: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("examples.json")).expect("examples.json is JSON")
});

/// The operations shown by example: a JSON body sent has its request, a JSON success its
/// response (`document::tests::the_first_integration_is_shown_by_example`).
pub const SHOWN: &[&str] = &[
    "list_customers",
    "create_customer",
    "get_customer",
    "update_customer",
    "archive_customer",
    "unarchive_customer",
    "list_plans",
    "create_plan",
    "get_plan",
    "update_plan",
    "put_plan_draft",
    "publish_plan_draft",
    "discard_plan_draft",
    "list_plan_versions",
    "archive_plan",
    "unarchive_plan",
    "list_subscriptions",
    "create_subscription",
    "get_subscription",
    "get_subscription_summary",
    "update_subscription",
    "get_subscription_schedule",
    "preview_plan_change",
    "change_plan",
    "preview_quantity_change",
    "change_quantity",
    "preview_level_change",
    "change_level",
    "preview_amendment",
    "amend_subscription",
    "preview_cancellation",
    "cancel_subscription",
    "revoke_cancellation",
    "extend_trial",
    "end_trial",
    "preview_trial_end",
    "list_invoices",
    "create_invoice",
    "get_invoice",
    "update_invoice",
    "refresh_invoice",
    "finalize_invoice",
    "void_invoice",
    "mark_invoice_uncollectible",
    "mark_invoice_paid",
    "record_invoice_payment",
    "reverse_invoice_payment",
    "download_invoice_pdf",
    "download_invoice_xml",
    "ingest_events",
    "ingest_segment_events",
    "list_webhook_endpoints",
    "create_webhook_endpoint",
    "get_webhook_endpoint",
    "update_webhook_endpoint",
    "archive_webhook_endpoint",
    "get_webhook_endpoint_secret",
    "rotate_webhook_endpoint_secret",
    "list_webhook_deliveries",
];

fn part(operation: &str, part: &str) -> Value {
    EXAMPLES
        .get(operation)
        .and_then(|example| example.get(part))
        .cloned()
        .unwrap_or_else(|| panic!("examples.json has no {part} of {operation}"))
}

/// The request body `operation` is shown with.
pub fn request(operation: &str) -> Value {
    part(operation, "request")
}

/// The response `operation` is shown with.
pub fn response(operation: &str) -> Value {
    part(operation, "response")
}
