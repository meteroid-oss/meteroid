//! Plan options: what a buyer picks when buying a plan version ("Annual", "Startup"). Each prices
//! every charge in its own cells (`price_cells`); the first by position is the default. The alias
//! is the API handle, like a customer alias, kept when a version is drafted from another.
use super::enums::BillingPeriodEnum;
use super::price_cells::cadence_free;
use super::prices::Price;
use common_domain::ids::{PlanOptionId, PlanVersionId};
use diesel_models::plan_version_options::PlanVersionOptionRow;

/// The option a version starts with.
pub const DEFAULT_OPTION_NAME: &str = "Standard";
pub const DEFAULT_OPTION_ALIAS: &str = "standard";
const MAX_ALIAS_LEN: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanOption {
    pub id: PlanOptionId,
    pub plan_version_id: PlanVersionId,
    pub name: String,
    pub alias: String,
    pub position: i32,
    /// Sold when named (API, quotes, links), never advertised: off the pricing page and the
    /// portal. The default option never is.
    pub hidden: bool,
}

impl From<PlanVersionOptionRow> for PlanOption {
    fn from(row: PlanVersionOptionRow) -> Self {
        PlanOption {
            id: row.id,
            plan_version_id: row.plan_version_id,
            name: row.name,
            alias: row.alias,
            position: row.position,
            hidden: row.hidden,
        }
    }
}

/// An option as an integrator names it: by id, or by alias (valid across versions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanOptionRef {
    Id(PlanOptionId),
    Alias(String),
}

impl PlanOptionRef {
    pub fn matches(&self, option: &PlanOption) -> bool {
        match self {
            PlanOptionRef::Id(id) => option.id == *id,
            PlanOptionRef::Alias(alias) => option.alias.eq_ignore_ascii_case(alias.trim()),
        }
    }
}

impl std::fmt::Display for PlanOptionRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanOptionRef::Id(id) => write!(f, "{id}"),
            PlanOptionRef::Alias(alias) => write!(f, "{alias}"),
        }
    }
}

/// A lowercase slug: `^[a-z0-9][a-z0-9_-]{0,39}$`.
pub fn valid_alias(alias: &str) -> bool {
    let mut chars = alias.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    alias.len() <= MAX_ALIAS_LEN
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// The alias an option named `name` gets: its slug ("Annual 2" → "annual-2"), suffixed `-2`,
/// `-3`… when `taken`; "option" for a name without a letter or digit.
pub fn alias_from_name(name: &str, taken: &[String]) -> String {
    let mut slug = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let mut base = slug.trim_end_matches('-').to_owned();
    if base.is_empty() {
        base = "option".to_owned();
    }
    let free = |alias: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(alias));
    let fit = |base: &str, suffix: &str| {
        let room = MAX_ALIAS_LEN - suffix.len();
        let cut: String = base.chars().take(room).collect();
        format!("{}{suffix}", cut.trim_end_matches(['-', '_']))
    };
    let first = fit(&base, "");
    if free(&first) {
        return first;
    }
    (2..)
        .map(|n| fit(&base, &format!("-{n}")))
        .find(|alias| free(alias))
        .expect("an unused suffix")
}

/// The options of a version written whole from prices (API create or replace), as the options
/// migration derived them: one per cadence of the fixed prices of a charge priced on several,
/// by period length. Empty: the default option only.
pub fn cadence_options(components: &[Vec<&Price>]) -> Vec<BillingPeriodEnum> {
    let mut cadences: Vec<BillingPeriodEnum> = components
        .iter()
        .filter(|prices| switching(prices))
        .flat_map(|prices| fixed_cadences(prices))
        .collect();
    cadences.sort();
    cadences.dedup();
    cadences
}

/// Whether `price` of a charge priced `prices` goes in the option of `cadence`: all do, but the
/// fixed prices of a charge priced on several cadences, in that of theirs only.
pub fn in_cadence_option(prices: &[&Price], price: &Price, cadence: BillingPeriodEnum) -> bool {
    cadence_free(&price.pricing) || !switching(prices) || price.cadence == cadence
}

/// The name and alias of the option of `cadence`.
pub fn cadence_option_name(cadence: BillingPeriodEnum) -> (&'static str, &'static str) {
    match cadence {
        BillingPeriodEnum::Monthly => ("Monthly", "monthly"),
        BillingPeriodEnum::Quarterly => ("Quarterly", "quarterly"),
        BillingPeriodEnum::Semiannual => ("Semiannual", "semiannual"),
        BillingPeriodEnum::Annual => ("Annual", "annual"),
    }
}

fn fixed_cadences(prices: &[&Price]) -> Vec<BillingPeriodEnum> {
    let mut cadences: Vec<BillingPeriodEnum> = prices
        .iter()
        .filter(|p| !cadence_free(&p.pricing))
        .map(|p| p.cadence)
        .collect();
    cadences.sort();
    cadences.dedup();
    cadences
}

fn switching(prices: &[&Price]) -> bool {
    fixed_cadences(prices).len() > 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_are_lowercase_slugs() {
        assert!(valid_alias("annual"));
        assert!(valid_alias("0-startup_2"));
        assert!(!valid_alias(""));
        assert!(!valid_alias("-annual"));
        assert!(!valid_alias("Annual"));
        assert!(!valid_alias("an nual"));
        assert!(!valid_alias(&"a".repeat(41)));
        assert!(valid_alias(&"a".repeat(40)));
    }
}
