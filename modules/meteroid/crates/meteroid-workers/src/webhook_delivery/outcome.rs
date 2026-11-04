use chrono::NaiveDateTime;
use common_domain::ids::{WebhookDeliveryId, WebhookEndpointId};
use meteroid_store::domain::enums::{WebhookDeliveryStatus, WebhookEndpointDisabledReason};
use meteroid_store::domain::webhooks_out::WebhookAttemptOutcome;
use meteroid_store::services::webhooks::out::schedule::{
    EndpointHealth, is_endpoint_wide_failure, next_attempt_at, pause_after, should_auto_disable,
};

const HTTP_GONE: u16 = 410;

/// What one HTTP attempt produced: a response we read, or no response at all
/// (SSRF rejection, DNS failure, timeout, transport error).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptResult {
    Response { status: u16, body_prefix: String },
    Error(String),
}

/// Everything the decision needs besides the attempt itself. `health` holds the endpoint
/// counters as they were *before* this attempt.
#[derive(Clone, Copy, Debug)]
pub struct AttemptContext {
    pub delivery_id: WebhookDeliveryId,
    pub endpoint_id: WebhookEndpointId,
    pub attempt_number: i32,
    pub lease_expires_at: NaiveDateTime,
    pub duration_ms: i32,
    pub health: EndpointHealth,
    pub now: NaiveDateTime,
    pub jitter: f64,
    pub auto_disable_after_hours: i64,
}

pub fn decide_outcome(ctx: AttemptContext, result: AttemptResult) -> WebhookAttemptOutcome {
    let (response_status, response_body, error, succeeded) = match &result {
        AttemptResult::Response {
            status,
            body_prefix,
        } => {
            let ok = (200..300).contains(status);
            (
                Some(*status as i16),
                Some(body_prefix.clone()),
                (!ok).then(|| format!("endpoint responded with HTTP {status}")),
                ok,
            )
        }
        AttemptResult::Error(message) => (None, None, Some(message.clone()), false),
    };

    let gone = matches!(&result, AttemptResult::Response { status, .. } if *status == HTTP_GONE);
    let endpoint_wide_failure =
        !succeeded && is_endpoint_wide_failure(response_status.map(|s| s as u16));

    let mut pause_endpoint_until = None;
    let mut refund_attempt = false;

    let (final_status, next_attempt, disable_endpoint) = if succeeded {
        (WebhookDeliveryStatus::Succeeded, None, None)
    } else if gone {
        (
            WebhookDeliveryStatus::Failed,
            None,
            Some(WebhookEndpointDisabledReason::Gone),
        )
    } else {
        let health = EndpointHealth {
            consecutive_failures: ctx.health.consecutive_failures.saturating_add(1),
            failing_since: ctx.health.failing_since.or(Some(ctx.now)),
            unreachable_streak: if endpoint_wide_failure {
                ctx.health.unreachable_streak.saturating_add(1)
            } else {
                0
            },
        };
        let disable = should_auto_disable(&health, ctx.now, ctx.auto_disable_after_hours)
            .then_some(WebhookEndpointDisabledReason::AutoFailures);

        let failing_for = health
            .failing_since
            .map_or_else(chrono::Duration::zero, |since| ctx.now - since);
        let pause = endpoint_wide_failure
            .then(|| pause_after(health.unreachable_streak, failing_for))
            .flatten();

        match pause {
            // The endpoint as a whole is down: pause it, and keep this delivery at the
            // head of its queue without spending budget, so the next probe sends it and
            // a long outage never runs an event out of retries.
            Some(pause) => {
                let until = ctx.now + pause;
                pause_endpoint_until = Some(until);
                refund_attempt = true;
                (WebhookDeliveryStatus::Pending, Some(until), disable)
            }
            None => {
                let next = next_attempt_at(ctx.now, ctx.attempt_number, ctx.jitter);
                let status = if next.is_some() {
                    WebhookDeliveryStatus::Pending
                } else {
                    WebhookDeliveryStatus::Failed
                };
                (status, next, disable)
            }
        }
    };

    WebhookAttemptOutcome {
        delivery_id: ctx.delivery_id,
        endpoint_id: ctx.endpoint_id,
        attempt_number: ctx.attempt_number,
        lease_expires_at: ctx.lease_expires_at,
        duration_ms: ctx.duration_ms,
        response_status,
        response_body,
        error,
        succeeded,
        next_attempt_at: next_attempt,
        final_status,
        disable_endpoint,
        endpoint_wide_failure,
        pause_endpoint_until,
        refund_attempt,
    }
}

/// Terminal outcome for a delivery whose endpoint disappeared or stopped being
/// deliverable between the claim and the HTTP call.
pub fn cancelled_outcome(
    delivery_id: WebhookDeliveryId,
    endpoint_id: WebhookEndpointId,
    attempt_number: i32,
    lease_expires_at: NaiveDateTime,
    reason: &str,
) -> WebhookAttemptOutcome {
    WebhookAttemptOutcome {
        delivery_id,
        endpoint_id,
        attempt_number,
        lease_expires_at,
        duration_ms: 0,
        response_status: None,
        response_body: None,
        error: Some(reason.to_string()),
        succeeded: false,
        next_attempt_at: None,
        final_status: WebhookDeliveryStatus::Cancelled,
        disable_endpoint: None,
        endpoint_wide_failure: false,
        pause_endpoint_until: None,
        refund_attempt: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, NaiveDate};
    use common_domain::ids::BaseId;
    use meteroid_store::services::webhooks::out::schedule::{
        AUTO_DISABLE_CONSECUTIVE_FAILURES, MAX_ATTEMPTS, PROBE_THRESHOLD,
    };

    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, 19)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
    }

    fn ctx(attempt_number: i32, consecutive_failures: i32) -> AttemptContext {
        AttemptContext {
            delivery_id: WebhookDeliveryId::new(),
            endpoint_id: WebhookEndpointId::new(),
            attempt_number,
            lease_expires_at: now() + Duration::seconds(90),
            duration_ms: 42,
            health: EndpointHealth {
                consecutive_failures,
                failing_since: (consecutive_failures > 0).then(|| now() - Duration::minutes(1)),
                unreachable_streak: 0,
            },
            now: now(),
            jitter: 0.0,
            auto_disable_after_hours: 120,
        }
    }

    fn response(status: u16) -> AttemptResult {
        AttemptResult::Response {
            status,
            body_prefix: "ok".to_string(),
        }
    }

    #[test]
    fn two_hundreds_succeed_and_clear_the_schedule() {
        for status in [200u16, 201, 204, 299] {
            let outcome = decide_outcome(ctx(1, 3), response(status));
            assert!(outcome.succeeded, "status {status}");
            assert_eq!(outcome.final_status, WebhookDeliveryStatus::Succeeded);
            assert!(outcome.next_attempt_at.is_none());
            assert!(outcome.disable_endpoint.is_none());
            assert!(outcome.error.is_none());
        }
    }

    #[test]
    fn redirects_and_server_errors_reschedule() {
        for status in [301u16, 302, 400, 404, 429, 500, 503] {
            let outcome = decide_outcome(ctx(1, 0), response(status));
            assert!(!outcome.succeeded, "status {status}");
            assert_eq!(outcome.final_status, WebhookDeliveryStatus::Pending);
            assert_eq!(
                outcome.next_attempt_at,
                Some(now() + Duration::seconds(5)),
                "status {status}"
            );
            assert_eq!(outcome.response_status, Some(status as i16));
        }
    }

    #[test]
    fn gone_fails_the_delivery_and_disables_the_endpoint() {
        let outcome = decide_outcome(ctx(1, 0), response(410));
        assert_eq!(outcome.final_status, WebhookDeliveryStatus::Failed);
        assert!(outcome.next_attempt_at.is_none());
        assert_eq!(
            outcome.disable_endpoint,
            Some(WebhookEndpointDisabledReason::Gone)
        );
    }

    #[test]
    fn last_attempt_is_terminal() {
        let outcome = decide_outcome(ctx(MAX_ATTEMPTS, 0), response(500));
        assert_eq!(outcome.final_status, WebhookDeliveryStatus::Failed);
        assert!(outcome.next_attempt_at.is_none());
    }

    #[test]
    fn transport_errors_have_no_status_and_keep_the_error_text() {
        let outcome = decide_outcome(
            ctx(2, 0),
            AttemptResult::Error("destination is not an allowed address".to_string()),
        );
        assert!(outcome.response_status.is_none());
        assert!(outcome.response_body.is_none());
        assert_eq!(
            outcome.error.as_deref(),
            Some("destination is not an allowed address")
        );
        assert_eq!(outcome.final_status, WebhookDeliveryStatus::Pending);
        assert_eq!(
            outcome.next_attempt_at,
            Some(now() + Duration::seconds(300))
        );
    }

    #[test]
    fn auto_disable_triggers_on_a_long_dead_endpoint() {
        let mut c = ctx(1, AUTO_DISABLE_CONSECUTIVE_FAILURES - 1);
        c.health.failing_since = Some(now() - Duration::hours(121));

        let outcome = decide_outcome(c, response(500));
        assert_eq!(
            outcome.disable_endpoint,
            Some(WebhookEndpointDisabledReason::AutoFailures)
        );
    }

    #[test]
    fn a_short_failure_streak_keeps_the_endpoint_enabled() {
        let mut c = ctx(1, AUTO_DISABLE_CONSECUTIVE_FAILURES * 2);
        c.health.failing_since = Some(now() - Duration::hours(10));

        let outcome = decide_outcome(c, response(500));
        assert!(outcome.disable_endpoint.is_none());
    }

    #[test]
    fn a_success_never_disables_even_with_a_dead_history() {
        let mut c = ctx(1, AUTO_DISABLE_CONSECUTIVE_FAILURES * 2);
        c.health.failing_since = Some(now() - Duration::days(30));

        let outcome = decide_outcome(c, response(200));
        assert!(outcome.disable_endpoint.is_none());
    }

    /// An endpoint that went down a few seconds ago.
    fn unreachable(streak: i32) -> AttemptContext {
        let mut c = ctx(3, streak);
        c.health.unreachable_streak = streak;
        c.health.failing_since = Some(now() - Duration::seconds(10));
        c
    }

    #[test]
    fn unreachable_attempts_pause_the_endpoint_past_the_threshold() {
        let below = decide_outcome(
            unreachable(PROBE_THRESHOLD - 2),
            AttemptResult::Error("dns".to_string()),
        );
        assert!(below.endpoint_wide_failure);
        assert!(below.pause_endpoint_until.is_none(), "not yet");
        assert!(!below.refund_attempt);

        let at = decide_outcome(
            unreachable(PROBE_THRESHOLD - 1),
            AttemptResult::Error("dns".to_string()),
        );
        assert_eq!(at.pause_endpoint_until, Some(now() + Duration::seconds(30)));
        assert!(
            at.refund_attempt,
            "a paused endpoint's probe spends no budget"
        );
        assert_eq!(at.final_status, WebhookDeliveryStatus::Pending);
        assert_eq!(at.next_attempt_at, at.pause_endpoint_until);
    }

    #[test]
    fn a_long_outage_is_never_probed_faster_than_it_has_lasted() {
        // A single event's own ladder (5s, 5m, 30m, 2h) reaches the threshold hours in.
        let mut c = unreachable(PROBE_THRESHOLD - 1);
        c.health.failing_since = Some(now() - Duration::minutes(155));

        let outcome = decide_outcome(c, AttemptResult::Error("refused".to_string()));
        assert_eq!(
            outcome.pause_endpoint_until,
            Some(now() + Duration::seconds(3_600))
        );
    }

    #[test]
    fn a_gateway_error_counts_as_unreachable() {
        let outcome = decide_outcome(unreachable(PROBE_THRESHOLD - 1), response(503));
        assert!(outcome.endpoint_wide_failure);
        assert!(outcome.pause_endpoint_until.is_some());
    }

    #[test]
    fn an_event_specific_rejection_never_pauses() {
        let outcome = decide_outcome(unreachable(PROBE_THRESHOLD + 10), response(400));
        assert!(!outcome.endpoint_wide_failure);
        assert!(outcome.pause_endpoint_until.is_none());
        assert!(!outcome.refund_attempt);
        assert_eq!(
            outcome.next_attempt_at,
            Some(now() + Duration::seconds(1_800)),
            "its own retry ladder: the delay after a third attempt"
        );
    }

    #[test]
    fn a_probe_of_a_paused_endpoint_never_runs_out_of_budget() {
        let mut c = unreachable(PROBE_THRESHOLD + 3);
        c.attempt_number = MAX_ATTEMPTS;
        let outcome = decide_outcome(c, AttemptResult::Error("timeout".to_string()));
        assert_eq!(outcome.final_status, WebhookDeliveryStatus::Pending);
        assert!(outcome.refund_attempt);
    }
}
