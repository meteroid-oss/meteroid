mod account_credit;
pub mod credit_traffic;
mod credits;
mod history_usage;
mod preconditions;
mod replay;
mod runner;
mod scenarios;
mod usage_wait;

pub use runner::{run_preset, run_preset_at};
pub use scenarios::pulse::STAGED_USAGE_CUSTOMERS;

/// The single default sandbox business seeded for every new sandbox tenant:
/// "Pulse", a Slack-style team-collaboration SaaS. Kept as one coherent business
/// (see `scenarios::pulse`) rather than a set of randomly-picked archetypes, so
/// demo data stays easy to read and reason about.
pub fn default_scenario() -> scenarios::domain::Scenario {
    // Resolve the scenario's relative dates against today, so the demo always spans
    // the trailing 12 months whenever the sandbox is created.
    let now = chrono::Utc::now().naive_utc().date();
    scenarios::pulse::pulse_scenario(now)
}

/// "Halyard AI", an LLM API provider billed from prepaid credit, with live usage. A separate
/// tenant from Pulse: nothing in it is shared with the default sandbox.
pub fn ai_credits_scenario() -> scenarios::domain::Scenario {
    scenarios::ai_credits::ai_credits_scenario()
}

/// "Fjordline Inference", an EU inference cloud reselling models per 1M tokens through LiteLLM:
/// the AI operator template, with a price card. The `AI_OPERATOR` sandbox preset; an edition's
/// modules may add to it (a cost book, the AI module on).
pub fn ai_operator_scenario() -> scenarios::domain::Scenario {
    let now = chrono::Utc::now().naive_utc().date();
    scenarios::ai_operator::ai_operator_scenario(now)
}

/// "Acme Analytics", a product-analytics SaaS selling one ladder of plans in USD, EUR and BRL,
/// with two coupons, an LLM rate card and a sales-led Enterprise plan; its customers subscribe
/// in their own currency. The `CURRENCIES_AND_COUPONS` sandbox preset.
pub fn currencies_scenario() -> scenarios::domain::Scenario {
    let now = chrono::Utc::now().naive_utc().date();
    scenarios::currencies::currencies_scenario(now)
}
