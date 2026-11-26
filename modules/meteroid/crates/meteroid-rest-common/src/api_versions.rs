//! The dated versions of API v2 (ADR-0013 §2, §15), shared by the REST API (which resolves a
//! request's `Meteroid-Version` among them), the dashboard's API (which pins the webhook endpoints
//! it creates to the latest) and the webhook worker (which renders each endpoint's payloads at the
//! version it holds).
use crate::datetime::ApiDateTime;
use chrono::NaiveDate;
use common_domain::ids::EventId;
use serde_json::{Value, json};

/// Every published version, oldest first. The first is a request's default.
pub const VERSIONS: &[&str] = &["2026-12-01", "2026-12-15"];

/// The first version whose webhook payloads are v2's (`{id, type, created_at, api_version,
/// data}`, `data` the resource as its v2 GET reads it). An endpoint holding an earlier version, or
/// none, receives v1's payloads.
pub const V2_WEBHOOK_PAYLOADS: &str = "2026-12-15";

/// The latest published version.
pub fn latest() -> &'static str {
    VERSIONS.last().expect("v2 has a version")
}

fn date(version: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(version, "%Y-%m-%d").ok()
}

/// Whether an endpoint holding `api_version` receives v2's payloads: a version on or after
/// [`V2_WEBHOOK_PAYLOADS`]. None (an endpoint created through v1, the dashboard before it pinned
/// versions, or before versions were recorded) and what does not read as a date are v1's.
pub fn receives_v2_payloads(api_version: Option<&str>) -> bool {
    let first = date(V2_WEBHOOK_PAYLOADS).expect("a published version is a date");
    api_version.and_then(date).is_some_and(|at| at >= first)
}

/// An event's payload in v2's shape at `api_version`: `{id, type, created_at, api_version,
/// data}`. `id` is the event's (the `webhook-id` header), `created_at` when it was recorded,
/// `type` its event type, named as in v1's payloads.
pub fn webhook_envelope(
    id: EventId,
    event_type: &str,
    created_at: ApiDateTime,
    api_version: &str,
    data: &Value,
) -> Value {
    json!({
        "id": id.to_string(),
        "type": event_type,
        "created_at": created_at,
        "api_version": api_version,
        "data": data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_dates_in_order_and_payloads_start_at_one() {
        let dates: Vec<NaiveDate> = VERSIONS.iter().map(|v| date(v).unwrap()).collect();
        assert!(
            dates.windows(2).all(|pair| pair[0] < pair[1]),
            "{VERSIONS:?}"
        );
        assert!(VERSIONS.contains(&V2_WEBHOOK_PAYLOADS));
        assert_eq!(latest(), "2026-12-15");
    }

    #[test]
    fn only_a_version_from_v2_payloads_on_receives_them() {
        assert!(!receives_v2_payloads(None));
        assert!(!receives_v2_payloads(Some("2026-12-01")));
        assert!(!receives_v2_payloads(Some("not a date")));
        assert!(receives_v2_payloads(Some("2026-12-15")));
        assert!(receives_v2_payloads(Some("2027-06-01")));
    }
}
