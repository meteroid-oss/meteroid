//! Pure derivation of a customer's `lifecycle_state` from native billing signals.
//!
//! The taxonomy and precedence mirror `REVENUE_SEGMENTS.md` §6.2: a customer
//! resolves to exactly one primary state by first-match precedence (top wins).
//! The result string is written into `dim_customer.attributes['lifecycle_state']`
//! so the existing engine can segment (`DimensionMap`) and break down
//! (`CustomerAttr`) on it with no new compiler work.

/// Per-customer signals aggregated from quotes, subscriptions and AR.
///
/// "Active sub" = status in `SubscriptionStatusEnum::not_terminal()`; "paid plan"
/// = `plan_type != Free`. Each field is a presence flag computed by the projection
/// source query so this rule stays pure and DB-free.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LifecycleSignals {
    /// Active sub (`Active`/`PendingCharge`) on a non-free plan, not overdue.
    pub has_active_paid_sub: bool,
    /// A live trial (`TrialActive`).
    pub is_trialing: bool,
    /// An active (`not_terminal`) sub on a `Free` plan.
    pub has_active_free_sub: bool,
    /// Any active (`not_terminal`) sub of any kind.
    pub has_active_sub: bool,
    /// A `Suspended` sub (non-payment) — the native past-due signal.
    pub has_suspended_sub: bool,
    /// A `Paused` sub — alive but inactive.
    pub has_paused_sub: bool,
    /// Ever held a paid (non-free) sub that reached `Cancelled`/`Completed`.
    pub had_paid_sub_ended: bool,
    /// Ever held a paid (non-free) sub in any state — separates churned from churned-trial.
    pub ever_paid: bool,
    /// A `TrialExpired` sub (trial ended on a paid plan without conversion).
    pub trial_expired: bool,
    /// A `{Declined, Expired, Cancelled}` quote that never converted to a subscription.
    pub has_closed_lost_quote: bool,
    /// An open (`{Draft, Pending}`) quote.
    pub has_open_quote: bool,
    /// A prior paid sub ended before a currently-active sub began (reactivation refinement).
    pub reactivated: bool,
}

/// A customer's primary lifecycle state. The string repr is the stable attribute
/// value written to ClickHouse; keep it snake_case and stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleState {
    PastDue,
    ActivePaying,
    Trialing,
    Free,
    Churned,
    ChurnedTrial,
    ClosedLost,
    Lead,
    Dormant,
    Unknown,
}

impl LifecycleState {
    /// Stable snake_case attribute value.
    pub fn as_str(&self) -> &'static str {
        match self {
            LifecycleState::PastDue => "past_due",
            LifecycleState::ActivePaying => "active_paying",
            LifecycleState::Trialing => "trialing",
            LifecycleState::Free => "free",
            LifecycleState::Churned => "churned",
            LifecycleState::ChurnedTrial => "churned_trial",
            LifecycleState::ClosedLost => "closed_lost",
            LifecycleState::Lead => "lead",
            LifecycleState::Dormant => "dormant",
            LifecycleState::Unknown => "unknown",
        }
    }
}

/// First-match precedence (top wins): a live positive subscription state beats a
/// historical/negative one; among historical states "was a real paying customer"
/// beats "only trialed" beats "only quoted".
pub fn derive_lifecycle_state(s: &LifecycleSignals) -> LifecycleState {
    // 1. Past-due: a paying customer that is overdue (Suspended is the native signal).
    if s.has_suspended_sub && (s.has_active_paid_sub || s.ever_paid) {
        return LifecycleState::PastDue;
    }
    // 2. Active / Paying (Reactivated is a refinement flag on this state, not a slot).
    if s.has_active_paid_sub {
        return LifecycleState::ActivePaying;
    }
    // 3. Trialing — a live trial, when there is no active paid sub.
    if s.is_trialing {
        return LifecycleState::Trialing;
    }
    // 4. Free — an active free user, no active paid/trial sub.
    if s.has_active_free_sub {
        return LifecycleState::Free;
    }
    // 10 (early): Dormant — alive but paused; ranked above terminal-negative states.
    if s.has_paused_sub {
        return LifecycleState::Dormant;
    }
    // 6. Churned — was paying, no active sub now.
    if s.had_paid_sub_ended && !s.has_active_sub {
        return LifecycleState::Churned;
    }
    // 7. Churned-trial — trial that never converted, never paid, no active sub.
    if s.trial_expired && !s.ever_paid && !s.has_active_sub {
        return LifecycleState::ChurnedTrial;
    }
    // 8. Closed-lost — a rejected/expired quote with no sub and never paid.
    if s.has_closed_lost_quote && !s.has_active_sub && !s.ever_paid {
        return LifecycleState::ClosedLost;
    }
    // 9. Lead / Prospect — only open quotes, no sub.
    if s.has_open_quote && !s.has_active_sub {
        return LifecycleState::Lead;
    }
    // Fall-through.
    LifecycleState::Unknown
}

/// `reactivated` rides Active/Paying as a refinement flag; surfaced separately so a
/// future attribute key can carry it without a competing primary state.
pub fn is_reactivated(state: LifecycleState, s: &LifecycleSignals) -> bool {
    matches!(state, LifecycleState::ActivePaying | LifecycleState::Free) && s.reactivated
}

#[cfg(test)]
mod tests {
    use super::*;
    fn s() -> LifecycleSignals {
        LifecycleSignals::default()
    }

    #[test]
    fn string_reprs_are_stable_snake_case() {
        assert_eq!(LifecycleState::PastDue.as_str(), "past_due");
        assert_eq!(LifecycleState::ActivePaying.as_str(), "active_paying");
        assert_eq!(LifecycleState::Trialing.as_str(), "trialing");
        assert_eq!(LifecycleState::Free.as_str(), "free");
        assert_eq!(LifecycleState::Churned.as_str(), "churned");
        assert_eq!(LifecycleState::ChurnedTrial.as_str(), "churned_trial");
        assert_eq!(LifecycleState::ClosedLost.as_str(), "closed_lost");
        assert_eq!(LifecycleState::Lead.as_str(), "lead");
        assert_eq!(LifecycleState::Dormant.as_str(), "dormant");
        assert_eq!(LifecycleState::Unknown.as_str(), "unknown");
    }

    #[test]
    fn empty_signals_fall_through_to_unknown() {
        assert_eq!(derive_lifecycle_state(&s()), LifecycleState::Unknown);
    }

    #[test]
    fn past_due_beats_active_paying() {
        let sig = LifecycleSignals {
            has_active_paid_sub: true,
            has_suspended_sub: true,
            ever_paid: true,
            has_active_sub: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::PastDue);
    }

    #[test]
    fn suspended_without_ever_paid_is_not_past_due() {
        // A Suspended free/never-paid customer is not "past-due" (nothing owed).
        let sig = LifecycleSignals {
            has_suspended_sub: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::Unknown);
    }

    #[test]
    fn active_paying_when_not_overdue() {
        let sig = LifecycleSignals {
            has_active_paid_sub: true,
            has_active_sub: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::ActivePaying);
    }

    #[test]
    fn active_paid_beats_trialing_and_free_siblings() {
        let sig = LifecycleSignals {
            has_active_paid_sub: true,
            is_trialing: true,
            has_active_free_sub: true,
            has_active_sub: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::ActivePaying);
    }

    #[test]
    fn trialing_when_no_active_paid_sub() {
        let sig = LifecycleSignals {
            is_trialing: true,
            has_active_sub: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::Trialing);
    }

    #[test]
    fn trial_beats_free_sibling() {
        let sig = LifecycleSignals {
            is_trialing: true,
            has_active_free_sub: true,
            has_active_sub: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::Trialing);
    }

    #[test]
    fn free_active_user() {
        let sig = LifecycleSignals {
            has_active_free_sub: true,
            has_active_sub: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::Free);
    }

    #[test]
    fn paused_is_dormant_and_beats_historical_negative_states() {
        let sig = LifecycleSignals {
            has_paused_sub: true,
            has_active_sub: true,
            had_paid_sub_ended: true,
            has_closed_lost_quote: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::Dormant);
    }

    #[test]
    fn churned_was_paying_no_active_sub() {
        let sig = LifecycleSignals {
            had_paid_sub_ended: true,
            ever_paid: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::Churned);
    }

    #[test]
    fn churned_beats_closed_lost() {
        let sig = LifecycleSignals {
            had_paid_sub_ended: true,
            ever_paid: true,
            has_closed_lost_quote: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::Churned);
    }

    #[test]
    fn churned_trial_when_never_paid() {
        let sig = LifecycleSignals {
            trial_expired: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::ChurnedTrial);
    }

    #[test]
    fn churned_trial_beats_closed_lost() {
        let sig = LifecycleSignals {
            trial_expired: true,
            has_closed_lost_quote: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::ChurnedTrial);
    }

    #[test]
    fn trial_expired_but_later_paid_is_churned_not_churned_trial() {
        // ever_paid disqualifies churned-trial; a paid sub that ended is churned.
        let sig = LifecycleSignals {
            trial_expired: true,
            ever_paid: true,
            had_paid_sub_ended: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::Churned);
    }

    #[test]
    fn closed_lost_when_only_a_rejected_quote() {
        let sig = LifecycleSignals {
            has_closed_lost_quote: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::ClosedLost);
    }

    #[test]
    fn closed_lost_flips_out_once_a_sub_exists() {
        // A later subscription must override closed-lost (no latching).
        let sig = LifecycleSignals {
            has_closed_lost_quote: true,
            has_active_free_sub: true,
            has_active_sub: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::Free);
    }

    #[test]
    fn lead_when_only_open_quote() {
        let sig = LifecycleSignals {
            has_open_quote: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::Lead);
    }

    #[test]
    fn closed_lost_beats_lead_when_both_present() {
        let sig = LifecycleSignals {
            has_closed_lost_quote: true,
            has_open_quote: true,
            ..s()
        };
        assert_eq!(derive_lifecycle_state(&sig), LifecycleState::ClosedLost);
    }

    #[test]
    fn reactivated_is_a_flag_on_active_paying_not_a_state() {
        let sig = LifecycleSignals {
            has_active_paid_sub: true,
            has_active_sub: true,
            reactivated: true,
            ..s()
        };
        let state = derive_lifecycle_state(&sig);
        assert_eq!(state, LifecycleState::ActivePaying);
        assert!(is_reactivated(state, &sig));
        assert!(!is_reactivated(LifecycleState::Churned, &sig));
    }
}
