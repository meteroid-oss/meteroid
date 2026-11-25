use crate::ratecards::model as rest;
use chrono::NaiveDate;
use common_domain::ids::TenantId;
use meteroid_app::errors::RestApiError;
use meteroid_store::domain::rate_cards as domain;
use rust_decimal::Decimal;

pub fn card_to_rest(card: domain::RateCard) -> rest::RateCard {
    let (denomination, credit_unit_id) = match card.denomination {
        domain::RateCardDenomination::Money => (rest::RateCardDenomination::Money, None),
        domain::RateCardDenomination::CreditUnit(unit) => {
            (rest::RateCardDenomination::CreditUnit, Some(unit))
        }
    };
    rest::RateCard {
        id: card.id,
        code: card.code,
        name: card.name,
        metric_id: card.metric_id,
        dimensions: card.dimensions,
        denomination,
        credit_unit_id,
        unit_size: card.unit_size.normalize(),
        on_unmatched: match card.on_unmatched {
            domain::RateCardUnmatched::Hold => rest::RateCardUnmatched::Hold,
            domain::RateCardUnmatched::Ignore => rest::RateCardUnmatched::Ignore,
        },
        created_at: card.created_at.into(),
    }
}

/// The card with its rates in force on `at` and the entries after it.
pub fn card_with_rates(
    card: domain::RateCard,
    entries: &[domain::RateCardEntry],
    at: NaiveDate,
) -> rest::RateCardWithRates {
    let rates = domain::rates_at(entries, at)
        .into_iter()
        .filter_map(|entry| {
            entry.rate.map(|rate| rest::RateCardRate {
                key: entry.key_text(),
                any_other: entry.is_any_other(),
                currency: entry.currency.clone(),
                rate: rate.normalize(),
                effective_from: entry.effective_from,
                label: entry.label.clone(),
            })
        })
        .collect();
    let mut scheduled: Vec<rest::RateCardScheduledChange> = entries
        .iter()
        .filter(|entry| entry.effective_from > at)
        .map(|entry| rest::RateCardScheduledChange {
            key: entry.key_text(),
            any_other: entry.is_any_other(),
            currency: entry.currency.clone(),
            rate: entry.rate.map(|rate| rate.normalize()),
            effective_from: entry.effective_from,
            label: entry.label.clone(),
        })
        .collect();
    scheduled.sort_by(|a, b| {
        (a.effective_from, &a.key, &a.currency).cmp(&(b.effective_from, &b.key, &b.currency))
    });
    rest::RateCardWithRates {
        card: card_to_rest(card),
        at,
        rates,
        scheduled,
    }
}

pub fn create_req_to_domain(
    tenant_id: TenantId,
    req: rest::RateCardCreateRequest,
) -> Result<domain::RateCardNew, RestApiError> {
    let denomination = match (req.denomination, req.credit_unit_id) {
        (rest::RateCardDenomination::Money, None) => domain::RateCardDenomination::Money,
        (rest::RateCardDenomination::CreditUnit, Some(unit)) => {
            domain::RateCardDenomination::CreditUnit(unit)
        }
        (rest::RateCardDenomination::Money, Some(_)) => {
            return Err(RestApiError::InvalidInput(
                "a MONEY card has no credit_unit_id".to_owned(),
            ));
        }
        (rest::RateCardDenomination::CreditUnit, None) => {
            return Err(RestApiError::InvalidInput(
                "a CREDIT_UNIT card names its credit_unit_id".to_owned(),
            ));
        }
    };
    Ok(domain::RateCardNew {
        tenant_id,
        code: req.code,
        name: req.name,
        metric_id: req.metric_id,
        dimensions: req.dimensions,
        denomination,
        unit_size: req.unit_size.unwrap_or(Decimal::ONE),
        on_unmatched: match req.on_unmatched {
            rest::RateCardUnmatched::Hold => domain::RateCardUnmatched::Hold,
            rest::RateCardUnmatched::Ignore => domain::RateCardUnmatched::Ignore,
        },
    })
}

pub fn rates_req_to_domain(req: rest::RateCardRatesSetRequest) -> domain::RateCardRatesSet {
    domain::RateCardRatesSet {
        effective_from: req.effective_from,
        rates: req
            .rates
            .into_iter()
            .map(|rate| domain::RateCardRateInput {
                key: rate.key,
                currency: rate.currency,
                rate: rate.rate,
                label: rate.label,
            })
            .collect(),
        remove: req
            .remove
            .into_iter()
            .map(|removal| domain::RateCardKeyRemoval {
                key: removal.key,
                currency: removal.currency,
            })
            .collect(),
    }
}
