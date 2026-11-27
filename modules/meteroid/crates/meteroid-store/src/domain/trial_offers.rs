//! Plan phase templates (R-154) and their generators (R-155). Trials: `domain::trials`.

/// A phase of a plan version's template, at `offset_months` from a subscription's billing start
/// (R-154). Its fees are named by plan component.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanPhaseTemplate {
    pub offset_months: u32,
    pub label: Option<String>,
    pub terms: crate::domain::subscription_phases::PhaseTerms,
}

fn priced(
    prices: &[(common_domain::ids::PriceComponentId, rust_decimal::Decimal)],
    factor: rust_decimal::Decimal,
) -> crate::domain::subscription_phases::PhaseTerms {
    crate::domain::subscription_phases::PhaseTerms {
        components: prices
            .iter()
            .map(
                |(component, price)| crate::domain::subscription_phases::PhaseComponent {
                    lineage: None,
                    plan_component: Some(*component),
                    price: (price * factor).round_dp(8),
                    list_rate: None,
                    name: None,
                },
            )
            .collect(),
        ..Default::default()
    }
}

/// An escalator: `prices` from the start, then `percent` more, compounded, every `every_months`
/// for `steps` steps (R-155).
pub fn escalator(
    prices: &[(common_domain::ids::PriceComponentId, rust_decimal::Decimal)],
    percent: rust_decimal::Decimal,
    every_months: u32,
    steps: u32,
) -> Vec<PlanPhaseTemplate> {
    let step = (rust_decimal::Decimal::ONE_HUNDRED + percent) / rust_decimal::Decimal::ONE_HUNDRED;
    let mut factor = rust_decimal::Decimal::ONE;
    (0..=steps)
        .map(|n| {
            let phase = PlanPhaseTemplate {
                offset_months: n * every_months,
                label: Some(if n == 0 {
                    "Start".to_string()
                } else {
                    format!("+{percent}% ({n})")
                }),
                terms: priced(prices, factor),
            };
            factor *= step;
            phase
        })
        .collect()
}

/// `months` free: every listed fee at zero from the start, then at its price (R-155).
pub fn free_months(
    prices: &[(common_domain::ids::PriceComponentId, rust_decimal::Decimal)],
    months: u32,
) -> Vec<PlanPhaseTemplate> {
    vec![
        PlanPhaseTemplate {
            offset_months: 0,
            label: Some(format!("{months} months free")),
            terms: priced(prices, rust_decimal::Decimal::ZERO),
        },
        PlanPhaseTemplate {
            offset_months: months,
            label: Some("Paid".to_string()),
            terms: priced(prices, rust_decimal::Decimal::ONE),
        },
    ]
}
