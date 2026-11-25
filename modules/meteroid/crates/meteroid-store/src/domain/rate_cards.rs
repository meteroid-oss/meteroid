//! Rate cards: a metric's usage rated by its dimensions, independent of the plans that price it.
use billing_core::rate_card::{Entry, Pattern};
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    BaseId, BillableMetricId, CreditUnitId, RateCardEntryId, RateCardId, TenantId,
};
use diesel_models::enums::RateCardUnmatchedEnum;
use diesel_models::rate_cards::{RateCardEntryRow, RateCardRow};
use o2o::o2o;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::errors::StoreError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, o2o, Serialize, Deserialize)]
#[map_owned(RateCardUnmatchedEnum)]
pub enum RateCardUnmatched {
    /// The invoice waits until the usage is rated.
    #[default]
    Hold,
    /// The usage is left unbilled, on purpose.
    Ignore,
}

/// What a card's rates count: money, a rate per currency on each entry, or one credit unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RateCardDenomination {
    Money,
    CreditUnit(CreditUnitId),
}

#[derive(Clone, Debug)]
pub struct RateCard {
    pub id: RateCardId,
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
    pub metric_id: BillableMetricId,
    pub dimensions: Vec<String>,
    pub denomination: RateCardDenomination,
    /// Rates are per this many units of the metric (1000000: per 1M tokens).
    pub unit_size: Decimal,
    pub on_unmatched: RateCardUnmatched,
    pub created_at: DateTime<Utc>,
    /// Off the catalog for new prices; the prices rated by it keep rating.
    pub archived_at: Option<DateTime<Utc>>,
}

impl TryFrom<RateCardRow> for RateCard {
    type Error = StoreError;

    fn try_from(row: RateCardRow) -> Result<Self, Self::Error> {
        if row.unit_size <= Decimal::ZERO {
            return Err(StoreError::InvalidArgument(
                "a rate card's unit size is positive".to_owned(),
            ));
        }
        Ok(RateCard {
            id: row.id,
            tenant_id: row.tenant_id,
            code: row.code,
            name: row.name,
            metric_id: row.metric_id,
            dimensions: row.dimensions.into_iter().flatten().collect(),
            denomination: match row.credit_unit_id {
                Some(unit) => RateCardDenomination::CreditUnit(unit),
                None => RateCardDenomination::Money,
            },
            unit_size: row.unit_size,
            on_unmatched: row.on_unmatched.into(),
            created_at: row.created_at,
            archived_at: row.archived_at,
        })
    }
}

/// A supported currency code, upper-cased.
pub fn currency_code(code: &str) -> Result<String, StoreError> {
    crate::constants::Currencies::resolve_currency(&code.trim().to_ascii_uppercase())
        .map(|currency| currency.code.to_owned())
        .ok_or_else(|| StoreError::InvalidArgument(format!("unknown currency {code}")))
}

impl RateCard {
    /// The currency a rate of this card is written in: one for money, none for a credit unit.
    pub fn rate_currency(&self, currency: Option<&str>) -> Result<Option<String>, StoreError> {
        let currency = currency.map(str::trim).filter(|code| !code.is_empty());
        match (&self.denomination, currency) {
            (RateCardDenomination::Money, Some(code)) => currency_code(code).map(Some),
            (RateCardDenomination::Money, None) => Err(StoreError::InvalidArgument(
                "a rate of a money card has a currency".to_owned(),
            )),
            (RateCardDenomination::CreditUnit(_), None) => Ok(None),
            (RateCardDenomination::CreditUnit(_), Some(_)) => Err(StoreError::InvalidArgument(
                "a rate of a credit card is in its unit, never a currency".to_owned(),
            )),
        }
    }

    /// The entries rating a price in `currency`: those of the currency, or all of a credit card.
    pub fn entries_in<'a>(
        &self,
        entries: &'a [RateCardEntry],
        currency: &str,
    ) -> Vec<&'a RateCardEntry> {
        let money = self.denomination == RateCardDenomination::Money;
        entries
            .iter()
            .filter(|entry| {
                !money
                    || entry
                        .currency
                        .as_deref()
                        .is_some_and(|own| own.eq_ignore_ascii_case(currency.trim()))
            })
            .collect()
    }

    /// The entry rating usage carrying `dimensions` on `on` in `currency` (none for a credit
    /// card), as billing resolves it (R-131).
    pub fn entry_for<'a>(
        &self,
        entries: &'a [RateCardEntry],
        dimensions: &std::collections::BTreeMap<String, String>,
        on: NaiveDate,
        currency: Option<&str>,
    ) -> Option<&'a RateCardEntry> {
        let in_currency: Vec<&RateCardEntry> = entries
            .iter()
            .filter(|entry| match currency {
                Some(code) => entry
                    .currency
                    .as_deref()
                    .is_some_and(|own| own.eq_ignore_ascii_case(code.trim())),
                None => entry.currency.is_none(),
            })
            .collect();
        let rated: Vec<Entry<usize>> = in_currency
            .iter()
            .enumerate()
            .map(|(index, entry)| Entry {
                key: entry.key.clone(),
                effective_from: entry.effective_from,
                rate: entry.rate.map(|_| index),
            })
            .collect();
        billing_core::rate_card::resolve(&self.dimensions, &rated, dimensions, on)
            .map(|index| in_currency[*index])
    }
}

/// The rate of each key and currency in force on `on`, removed ones left out, by key then
/// currency.
pub fn rates_at(entries: &[RateCardEntry], on: NaiveDate) -> Vec<&RateCardEntry> {
    let mut latest: std::collections::BTreeMap<(Vec<String>, Option<String>), &RateCardEntry> =
        std::collections::BTreeMap::new();
    for entry in entries.iter().filter(|entry| entry.effective_from <= on) {
        let slot = (entry.key_text(), entry.currency.clone());
        if latest
            .get(&slot)
            .is_none_or(|known| known.effective_from < entry.effective_from)
        {
            latest.insert(slot, entry);
        }
    }
    latest
        .into_values()
        .filter(|entry| entry.rate.is_some())
        .collect()
}

#[derive(Clone, Debug)]
pub struct RateCardNew {
    pub tenant_id: TenantId,
    pub code: String,
    pub name: String,
    pub metric_id: BillableMetricId,
    pub dimensions: Vec<String>,
    pub denomination: RateCardDenomination,
    pub unit_size: Decimal,
    pub on_unmatched: RateCardUnmatched,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RateCardEntry {
    pub id: RateCardEntryId,
    pub key: Vec<Pattern>,
    /// None on a credit card's entries.
    pub currency: Option<String>,
    pub effective_from: NaiveDate,
    /// None: the key has no rate in the currency from this day (a removal).
    pub rate: Option<Decimal>,
    pub label: Option<String>,
}

impl RateCardEntry {
    pub fn key_text(&self) -> Vec<String> {
        self.key.iter().map(Pattern::as_text).collect()
    }

    /// A key of `*` on every dimension: the "Any other" row, rating what no other key does.
    pub fn is_any_other(&self) -> bool {
        self.key.iter().all(|pattern| *pattern == Pattern::Any)
    }
}

impl From<RateCardEntryRow> for RateCardEntry {
    fn from(row: RateCardEntryRow) -> Self {
        RateCardEntry {
            id: row.id,
            key: row
                .key
                .iter()
                .map(|pattern| Pattern::parse(pattern.as_deref().unwrap_or("*")))
                .collect(),
            currency: row.currency,
            effective_from: row.effective_from,
            rate: row.rate,
            label: row.label,
        }
    }
}

/// A key's rate in a currency (none for a credit card).
#[derive(Clone, Debug)]
pub struct RateCardRateInput {
    pub key: Vec<String>,
    pub currency: Option<String>,
    pub rate: Decimal,
    pub label: Option<String>,
}

/// A key no longer rated in a currency (none for a credit card).
#[derive(Clone, Debug)]
pub struct RateCardKeyRemoval {
    pub key: Vec<String>,
    pub currency: Option<String>,
}

/// Rates set and keys removed together, all from one day: today when unset.
#[derive(Clone, Debug, Default)]
pub struct RateCardRatesSet {
    pub effective_from: Option<NaiveDate>,
    pub rates: Vec<RateCardRateInput>,
    pub remove: Vec<RateCardKeyRemoval>,
}

impl RateCardRatesSet {
    /// The entries this batch writes on `card`, each currency checked against its denomination.
    pub fn inputs(
        self,
        card: &RateCard,
        today: NaiveDate,
    ) -> Result<Vec<RateCardEntryInput>, StoreError> {
        let effective_from = self.effective_from.unwrap_or(today);
        let rates = self.rates.into_iter().map(|rate| -> Result<_, StoreError> {
            Ok(RateCardEntryInput {
                currency: card.rate_currency(rate.currency.as_deref())?,
                key: rate.key,
                effective_from,
                rate: Some(rate.rate),
                label: rate.label,
            })
        });
        let removals = self
            .remove
            .into_iter()
            .map(|removal| -> Result<_, StoreError> {
                Ok(RateCardEntryInput {
                    currency: card.rate_currency(removal.currency.as_deref())?,
                    key: removal.key,
                    effective_from,
                    rate: None,
                    label: None,
                })
            });
        rates.chain(removals).collect()
    }
}

/// An entry to write: a key's rate in a currency from a day, or its removal (`rate` None).
#[derive(Clone, Debug)]
pub struct RateCardEntryInput {
    pub key: Vec<String>,
    pub currency: Option<String>,
    pub effective_from: NaiveDate,
    pub rate: Option<Decimal>,
    pub label: Option<String>,
}

/// What a bulk write did: rates added, keys removed, the same already there, scheduled entries
/// replaced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RateCardUpsert {
    pub inserted: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub replaced: usize,
}

/// What a bulk write of `inputs` does to `card` holding `existing` on `today`, each currency on
/// its own: an entry already there, or one repeating the rate in force, is unchanged; a key's
/// first rate may start on a past day, unless it takes over usage another key rated then; any
/// later change starts today or later, and replaces a scheduled entry of its day. A removal ends
/// the key from its day, its scheduled entries from then on withdrawn. Refused as a whole when
/// one input is (R-131).
pub fn plan_upsert(
    card: &RateCard,
    existing: &[RateCardEntry],
    inputs: &[RateCardEntryInput],
    today: NaiveDate,
) -> Result<(Vec<RateCardEntry>, Vec<RateCardEntryId>, RateCardUpsert), StoreError> {
    let dimensions = &card.dimensions;
    let invalid = |message: String| Err(StoreError::InvalidArgument(message));
    let mut inserts: Vec<RateCardEntry> = Vec::new();
    let mut replaced: Vec<RateCardEntryId> = Vec::new();
    let mut summary = RateCardUpsert::default();
    for input in inputs {
        if input.key.len() != dimensions.len() {
            return invalid(format!(
                "a key has a pattern per dimension ({}), got {}",
                dimensions.join(", "),
                input.key.len()
            ));
        }
        if input.rate.is_some_and(|rate| rate < Decimal::ZERO) {
            return invalid("a rate is never negative".to_owned());
        }
        let key: Vec<Pattern> = input.key.iter().map(|p| Pattern::parse(p)).collect();
        let named = match &input.currency {
            Some(currency) => format!("{} in {currency}", input.key.join(", ")),
            None => input.key.join(", "),
        };
        if inserts.iter().any(|e| {
            e.key == key && e.currency == input.currency && e.effective_from == input.effective_from
        }) {
            return invalid(format!(
                "{named} is written twice for {}",
                input.effective_from
            ));
        }
        let of_key: Vec<&RateCardEntry> = existing
            .iter()
            .filter(|e| !replaced.contains(&e.id))
            .chain(&inserts)
            .filter(|e| e.key == key && e.currency == input.currency)
            .collect();
        let before = of_key
            .iter()
            .filter(|e| e.effective_from < input.effective_from)
            .max_by_key(|e| e.effective_from)
            .and_then(|e| e.rate);
        if input.rate.is_none() {
            let from_then: Vec<&RateCardEntry> = of_key
                .iter()
                .copied()
                .filter(|e| e.effective_from >= input.effective_from)
                .collect();
            let kept = from_then
                .iter()
                .find(|e| e.rate.is_none() && e.effective_from == input.effective_from);
            let withdrawn: Vec<&RateCardEntry> = from_then
                .iter()
                .copied()
                .filter(|e| kept.is_none_or(|kept| kept.id != e.id))
                .collect();
            let insert = before.is_some() && kept.is_none();
            if withdrawn.is_empty() && !insert {
                summary.unchanged += 1;
                continue;
            }
            if input.effective_from < today || withdrawn.iter().any(|e| e.effective_from <= today) {
                return invalid(format!(
                    "a removal of {named} starts today or later, after its rates in force"
                ));
            }
            replaced.extend(withdrawn.iter().map(|e| e.id));
            summary.replaced += withdrawn.len();
            if !insert {
                continue;
            }
            summary.removed += 1;
        } else {
            match of_key
                .iter()
                .find(|e| e.effective_from == input.effective_from)
            {
                Some(entry) if entry.rate == input.rate => {
                    summary.unchanged += 1;
                    continue;
                }
                Some(entry) if entry.effective_from > today => {
                    replaced.push(entry.id);
                    summary.replaced += 1;
                    if before == input.rate {
                        continue;
                    }
                }
                Some(_) => {
                    return invalid(format!(
                        "the rate of {named} in force since {} cannot change: add one from today or later",
                        input.effective_from
                    ));
                }
                None => {
                    if before == input.rate {
                        summary.unchanged += 1;
                        continue;
                    }
                    if input.effective_from < today {
                        if before.is_some() {
                            return invalid(format!(
                                "a change of {named} starts today or later, not {}",
                                input.effective_from
                            ));
                        }
                        let stored: Vec<&RateCardEntry> = existing
                            .iter()
                            .filter(|e| !replaced.contains(&e.id) && e.currency == input.currency)
                            .collect();
                        if let Some(rated) = rerated(&stored, &key, input.effective_from, today) {
                            return invalid(format!(
                                "{} rated usage of {named} after {}: its rate starts today or later",
                                rated.key_text().join(", "),
                                input.effective_from
                            ));
                        }
                    }
                    summary.inserted += 1;
                }
            }
        }
        inserts.push(RateCardEntry {
            id: RateCardEntryId::new(),
            key,
            currency: input.currency.clone(),
            effective_from: input.effective_from,
            rate: input.rate,
            label: input.label.clone(),
        });
    }
    Ok((inserts, replaced, summary))
}

/// A key's rate in a currency as a write changed it, from the day of its batch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RateCardRateChange {
    pub key: Vec<String>,
    /// None on a credit card.
    pub currency: Option<String>,
    /// The key's rate the day before; None when it had none (a first rate, or after a removal).
    pub from: Option<Decimal>,
    /// None: the key is removed from the day.
    pub to: Option<Decimal>,
}

/// A scheduled entry a write deleted before its day: replaced by a rate of the batch, or withdrawn.
/// Kept in the audit only, the entry being gone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RateCardWithdrawnEntry {
    pub id: RateCardEntryId,
    pub key: Vec<String>,
    pub currency: Option<String>,
    pub effective_from: NaiveDate,
    /// None: a scheduled removal.
    pub rate: Option<Decimal>,
    pub label: Option<String>,
}

/// What a bulk write changed, the metadata of its `rate_card.rates_changed` activity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RateCardRatesChanged {
    /// The day the batch takes effect.
    pub effective_from: Option<NaiveDate>,
    pub changes: Vec<RateCardRateChange>,
    pub withdrawn: Vec<RateCardWithdrawnEntry>,
}

impl RateCardRatesChanged {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.withdrawn.is_empty()
    }
}

/// What writing `inserts` and deleting `replaced` over `existing` changes, as [`plan_upsert`]
/// planned them: each entry written with the rate its key had in its currency the day before,
/// and each scheduled entry deleted.
pub fn rates_changed(
    existing: &[RateCardEntry],
    inserts: &[RateCardEntry],
    replaced: &[RateCardEntryId],
    effective_from: NaiveDate,
) -> RateCardRatesChanged {
    let kept: Vec<&RateCardEntry> = existing
        .iter()
        .filter(|e| !replaced.contains(&e.id))
        .chain(inserts)
        .collect();
    let changes = inserts
        .iter()
        .map(|entry| RateCardRateChange {
            key: entry.key_text(),
            currency: entry.currency.clone(),
            from: kept
                .iter()
                .filter(|e| {
                    e.key == entry.key
                        && e.currency == entry.currency
                        && e.effective_from < entry.effective_from
                })
                .max_by_key(|e| e.effective_from)
                .and_then(|e| e.rate),
            to: entry.rate,
        })
        .collect();
    let withdrawn = existing
        .iter()
        .filter(|e| replaced.contains(&e.id))
        .map(|e| RateCardWithdrawnEntry {
            id: e.id,
            key: e.key_text(),
            currency: e.currency.clone(),
            effective_from: e.effective_from,
            rate: e.rate,
            label: e.label.clone(),
        })
        .collect();
    RateCardRatesChanged {
        effective_from: Some(effective_from),
        changes,
        withdrawn,
    }
}

/// A stored entry of another key that rated, between `from` and `today`, usage a new entry of
/// `key` from `from` would take over.
fn rerated<'a>(
    known: &[&'a RateCardEntry],
    key: &[Pattern],
    from: NaiveDate,
    today: NaiveDate,
) -> Option<&'a RateCardEntry> {
    let next_of = |key: &[Pattern], after: NaiveDate| {
        known
            .iter()
            .filter(|e| e.key == key && e.effective_from > after)
            .map(|e| e.effective_from)
            .min()
    };
    let until = next_of(key, from).map_or(today, |next| next.min(today));
    known.iter().copied().find(|other| {
        other.key != key
            && other.rate.is_some()
            && billing_core::rate_card::overlaps(key, &other.key)
            && billing_core::rate_card::beats(key, &other.key)
            && from.max(other.effective_from)
                < next_of(&other.key, other.effective_from).map_or(until, |end| end.min(until))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    fn card() -> RateCard {
        RateCard {
            id: RateCardId::new(),
            tenant_id: TenantId::new(),
            code: "llm".into(),
            name: "LLM".into(),
            metric_id: BillableMetricId::new(),
            dimensions: vec!["model".into(), "token_type".into()],
            denomination: RateCardDenomination::Money,
            unit_size: Decimal::ONE,
            on_unmatched: RateCardUnmatched::Hold,
            created_at: common_clock::now(),
            archived_at: None,
        }
    }

    fn input_in(
        model: &str,
        currency: &str,
        from: u32,
        rate: Option<Decimal>,
    ) -> RateCardEntryInput {
        RateCardEntryInput {
            key: vec![model.into(), "input".into()],
            currency: Some(currency.into()),
            effective_from: day(from),
            rate,
            label: None,
        }
    }

    fn input(model: &str, from: u32, rate: Decimal) -> RateCardEntryInput {
        input_in(model, "EUR", from, Some(rate))
    }

    fn removal(model: &str, from: u32) -> RateCardEntryInput {
        input_in(model, "EUR", from, None)
    }

    fn stored_in(model: &str, currency: &str, from: u32, rate: Option<Decimal>) -> RateCardEntry {
        RateCardEntry {
            id: RateCardEntryId::new(),
            key: vec![Pattern::parse(model), Pattern::parse("input")],
            currency: Some(currency.into()),
            effective_from: day(from),
            rate,
            label: None,
        }
    }

    fn stored(model: &str, from: u32, rate: Decimal) -> RateCardEntry {
        stored_in(model, "EUR", from, Some(rate))
    }

    /// Rules: R-131
    #[test]
    fn a_keys_first_rate_may_start_in_the_past_a_change_only_from_today() {
        let existing = vec![stored("x", 1, dec!(3))];
        let (inserts, _, summary) = plan_upsert(
            &card(),
            &existing,
            &[input("new-model", 3, dec!(5))],
            day(10),
        )
        .unwrap();
        assert_eq!((inserts.len(), summary.inserted), (1, 1));
        assert!(plan_upsert(&card(), &existing, &[input("x", 5, dec!(4))], day(10)).is_err());
        assert!(plan_upsert(&card(), &existing, &[input("x", 10, dec!(4))], day(10)).is_ok());
    }

    /// Rules: R-131
    #[test]
    fn a_repeated_sync_changes_nothing_and_a_scheduled_rate_can_be_replaced() {
        let scheduled = stored("x", 20, dec!(4));
        let existing = vec![stored("x", 1, dec!(3)), scheduled.clone()];
        let (inserts, replaced, summary) = plan_upsert(
            &card(),
            &existing,
            &[input("x", 1, dec!(3)), input("x", 20, dec!(5))],
            day(10),
        )
        .unwrap();
        assert_eq!(summary.unchanged, 1);
        assert_eq!(replaced, vec![scheduled.id]);
        assert_eq!(inserts.len(), 1);
        assert!(
            plan_upsert(&card(), &existing, &[input("x", 1, dec!(9))], day(10)).is_err(),
            "a rate in force never changes"
        );
        let (inserts, replaced, _) =
            plan_upsert(&card(), &existing, &[input("x", 20, dec!(3))], day(10)).unwrap();
        assert!(
            inserts.is_empty(),
            "back to the rate in force: the change is withdrawn"
        );
        assert_eq!(replaced, vec![scheduled.id]);
    }

    /// Rules: R-131
    #[test]
    fn a_backdated_key_never_takes_over_usage_another_key_rated() {
        let mut any = stored("*", 1, dec!(1));
        any.key[1] = Pattern::Any;
        let existing = vec![any];
        assert!(
            plan_upsert(&card(), &existing, &[input("c", 3, dec!(5))], day(10)).is_err(),
            "`*` rated c's usage since the 1st"
        );
        assert!(plan_upsert(&card(), &existing, &[input("c", 10, dec!(5))], day(10)).is_ok());
        assert!(
            plan_upsert(
                &card(),
                &existing,
                &[input_in("c", "USD", 3, Some(dec!(5)))],
                day(10)
            )
            .is_ok(),
            "`*` rated no usage in USD"
        );

        let held = vec![stored("c", 10, dec!(5))];
        let (inserts, _, _) =
            plan_upsert(&card(), &held, &[input("c", 1, dec!(5))], day(12)).unwrap();
        assert_eq!(inserts.len(), 1, "a held key's days before its first entry");
        assert!(
            plan_upsert(
                &card(),
                &[],
                &[input("c", 1, dec!(5)), input("c", 5, dec!(6))],
                day(10)
            )
            .is_err(),
            "a past change in one request"
        );
    }

    #[test]
    fn a_daily_sync_of_the_rate_in_force_changes_nothing() {
        let existing = vec![stored("x", 1, dec!(3))];
        let (inserts, _, summary) =
            plan_upsert(&card(), &existing, &[input("x", 10, dec!(3))], day(10)).unwrap();
        assert!(inserts.is_empty());
        assert_eq!(summary.unchanged, 1);
    }

    #[test]
    fn a_key_has_a_pattern_per_dimension() {
        let bad = RateCardEntryInput {
            key: vec!["x".into()],
            ..input("x", 10, dec!(1))
        };
        assert!(plan_upsert(&card(), &[], &[bad], day(10)).is_err());
    }

    /// Each currency is its own: x in USD is a new key, x in EUR keeps its rate.
    /// Rules: R-131
    #[test]
    fn each_currency_rates_on_its_own() {
        let existing = vec![stored("x", 1, dec!(3))];
        let (inserts, _, summary) = plan_upsert(
            &card(),
            &existing,
            &[
                input("x", 10, dec!(3)),
                input_in("x", "USD", 10, Some(dec!(4))),
            ],
            day(10),
        )
        .unwrap();
        assert_eq!((summary.unchanged, summary.inserted), (1, 1));
        assert_eq!(inserts[0].currency.as_deref(), Some("USD"));
    }

    /// A removal is one more dated entry; it starts today or later and withdraws the key's
    /// scheduled rates from its day; removing a key without a rate changes nothing.
    /// Rules: R-131
    #[test]
    fn a_key_is_removed_from_a_day_on() {
        let scheduled = stored("x", 20, dec!(4));
        let existing = vec![stored("x", 1, dec!(3)), scheduled.clone()];
        let (inserts, replaced, summary) =
            plan_upsert(&card(), &existing, &[removal("x", 12)], day(10)).unwrap();
        assert_eq!((summary.removed, summary.replaced), (1, 1));
        assert_eq!(replaced, vec![scheduled.id]);
        assert_eq!(inserts[0].rate, None);
        assert!(
            plan_upsert(&card(), &existing, &[removal("x", 5)], day(10)).is_err(),
            "a removal never rewrites past days"
        );
        let (_, _, summary) = plan_upsert(&card(), &existing, &[removal("y", 5)], day(10)).unwrap();
        assert_eq!(summary.unchanged, 1);

        let removed = vec![stored("x", 1, dec!(3)), stored_in("x", "EUR", 12, None)];
        let (inserts, _, summary) =
            plan_upsert(&card(), &removed, &[removal("x", 12)], day(10)).unwrap();
        assert!(inserts.is_empty());
        assert_eq!(summary.unchanged, 1, "a repeated removal");
    }

    /// One day for the batch, each rate's currency checked against the card's denomination.
    #[test]
    fn a_batch_of_rates_takes_one_day_and_the_cards_denomination() {
        let set = RateCardRatesSet {
            effective_from: None,
            rates: vec![RateCardRateInput {
                key: vec!["x".into(), "input".into()],
                currency: Some("eur".into()),
                rate: dec!(3),
                label: None,
            }],
            remove: vec![RateCardKeyRemoval {
                key: vec!["y".into(), "input".into()],
                currency: Some("USD".into()),
            }],
        };
        let inputs = set.clone().inputs(&card(), day(10)).unwrap();
        assert!(inputs.iter().all(|i| i.effective_from == day(10)));
        assert_eq!(inputs[0].currency.as_deref(), Some("EUR"));
        assert_eq!(inputs[1].rate, None);

        let credits = RateCard {
            denomination: RateCardDenomination::CreditUnit(CreditUnitId::new()),
            ..card()
        };
        assert!(
            set.inputs(&credits, day(10)).is_err(),
            "a credit rate has no currency"
        );
        assert!(card().rate_currency(None).is_err());
        assert!(card().rate_currency(Some("XYZ")).is_err());
        assert_eq!(credits.rate_currency(None).unwrap(), None);
    }

    /// The audit of a batch: each rate with the one its key had the day before, a removal's
    /// `to` unset, and the scheduled entries it deleted, kept as they were.
    #[test]
    fn a_batch_records_each_rate_before_and_after_and_the_scheduled_entries_it_deleted() {
        let scheduled = stored("x", 20, dec!(4));
        let existing = vec![
            stored("x", 1, dec!(3)),
            scheduled.clone(),
            stored("y", 1, dec!(2)),
        ];
        let inputs = [
            input("x", 20, dec!(5)),
            removal("y", 12),
            input("z", 12, dec!(1)),
        ];
        let (inserts, replaced, _) = plan_upsert(&card(), &existing, &inputs, day(10)).unwrap();
        let changed = rates_changed(&existing, &inserts, &replaced, day(12));
        let changes: Vec<(String, Option<Decimal>, Option<Decimal>)> = changed
            .changes
            .iter()
            .map(|c| (c.key[0].clone(), c.from, c.to))
            .collect();
        assert_eq!(
            changes,
            vec![
                ("x".into(), Some(dec!(3)), Some(dec!(5))),
                ("y".into(), Some(dec!(2)), None),
                ("z".into(), None, Some(dec!(1))),
            ]
        );
        assert_eq!(changed.withdrawn.len(), 1);
        assert_eq!(changed.withdrawn[0].id, scheduled.id);
        assert_eq!(changed.withdrawn[0].rate, Some(dec!(4)));

        let (inserts, replaced, _) =
            plan_upsert(&card(), &existing, &[input("x", 1, dec!(3))], day(10)).unwrap();
        assert!(
            rates_changed(&existing, &inserts, &replaced, day(10)).is_empty(),
            "a repeated sync records nothing"
        );
    }

    #[test]
    fn the_rates_in_force_leave_removed_keys_out() {
        let entries = vec![
            stored("x", 1, dec!(3)),
            stored("x", 5, dec!(4)),
            stored_in("x", "USD", 1, Some(dec!(5))),
            stored("y", 1, dec!(1)),
            stored_in("y", "EUR", 3, None),
            stored("z", 20, dec!(9)),
        ];
        let rates: Vec<(String, Option<String>, Option<Decimal>)> = rates_at(&entries, day(10))
            .into_iter()
            .map(|e| (e.key_text()[0].clone(), e.currency.clone(), e.rate))
            .collect();
        assert_eq!(
            rates,
            vec![
                ("x".into(), Some("EUR".into()), Some(dec!(4))),
                ("x".into(), Some("USD".into()), Some(dec!(5))),
            ]
        );
        let usage = std::collections::BTreeMap::from([
            ("model".to_owned(), "x".to_owned()),
            ("token_type".to_owned(), "input".to_owned()),
        ]);
        let rate = |currency| {
            card()
                .entry_for(&entries, &usage, day(10), Some(currency))
                .and_then(|e| e.rate)
        };
        assert_eq!(rate("EUR"), Some(dec!(4)));
        assert_eq!(rate("usd"), Some(dec!(5)));
        assert_eq!(rate("GBP"), None, "no rate in GBP: unmatched");
    }
}
