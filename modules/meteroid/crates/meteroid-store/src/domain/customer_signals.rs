/// The built-in signal whose thresholds the customer summary marks on usage bars.
pub const LIMIT_BURNDOWN_CODE: &str = "limit_burndown";
/// Built-in signals measured against the subscription's plan (limits, minimum): a plan change
/// resolves them.
pub const PLAN_DEPENDENT_SIGNAL_CODES: [&str; 2] = ["limit_burndown", "minimum_shortfall"];

/// How a signal moves revenue: the worklist tabs.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    strum::EnumIter,
    strum::EnumString,
    strum::Display,
    strum::IntoStaticStr,
)]
#[strum(serialize_all = "snake_case")]
pub enum SignalCategory {
    /// Outgrowing the plan: limits, overage.
    Expansion,
    /// Usage falling away.
    Risk,
    /// Commitments and renewals.
    Contracts,
    /// Getting paid.
    Billing,
}

impl SignalCategory {
    pub fn as_str(&self) -> &'static str {
        self.into()
    }

    /// Whether this category's signals put the customer's MRR at stake: every one but
    /// expansion, whose upside is not priced.
    pub fn is_downside(&self) -> bool {
        !matches!(self, SignalCategory::Expansion)
    }
}

/// A customer's open signals, for a list row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenSignals {
    pub count: i64,
    pub top_category: SignalCategory,
    pub critical: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    #[test]
    fn categories_parse_and_tell_downside_apart() {
        assert_eq!(
            "contracts".parse::<SignalCategory>().unwrap(),
            SignalCategory::Contracts
        );
        assert_eq!(SignalCategory::iter().count(), 4);
        assert!(!SignalCategory::Expansion.is_downside());
        assert!(SignalCategory::Billing.is_downside());
    }
}
