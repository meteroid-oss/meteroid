//! A quote's contract value (R-165): the subscription it converts to is written and projected
//! by the billing core in a transaction rolled back, so the value is what billing will bill.

use chrono::{Months, NaiveDate};
use common_domain::ids::{QuoteId, TenantId};
use diesel_models::quote_contract_values::QuoteContractValueRow;
use error_stack::Report;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;
use tracing::log;

use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::entity_activity::Actor;
use crate::domain::enums::SubscriptionActivationCondition;
use crate::domain::quotes::{DetailedQuote, QuoteContractValue};
use crate::errors::StoreError;
use crate::services::ServicesEdge;
use crate::services::quotes::build_subscription_from_quote;
use crate::services::timeline::Projection;
use crate::store::PgConn;

const MAX_MONTHS: u32 = 60;

impl ServicesEdge {
    /// Values the quote as stored and keeps the value, after each of its writes: what a
    /// published quote reads, and a revision records, is the value it was published with.
    pub async fn refresh_quote_contract_value(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<()> {
        use crate::repositories::QuotesInterface;
        let quote = self
            .store
            .get_detailed_quote_by_id(tenant_id, quote_id)
            .await?;
        let valued = self.quote_contract_value(tenant_id, &quote).await;
        let mut conn = self.store.get_conn().await?;
        let stored = match valued {
            Ok(value) => {
                QuoteContractValueRow {
                    quote_id,
                    tenant_id,
                    value: serde_json::to_value(&value).map_err(|error| {
                        StoreError::SerdeError("quote contract value".into(), error)
                    })?,
                    computed_at: common_clock::now().naive_utc(),
                }
                .upsert(&mut conn)
                .await
            }
            Err(error) => {
                log::warn!("no contract value for quote {quote_id:?}: {error:?}");
                QuoteContractValueRow::delete(&mut conn, tenant_id, quote_id).await
            }
        };
        stored.map_err(Into::into)
    }

    pub async fn quote_contract_value(
        &self,
        tenant_id: TenantId,
        quote: &DetailedQuote,
    ) -> StoreResult<QuoteContractValue> {
        let mut params = build_subscription_from_quote(quote)?;
        // Valued as from its start, whatever activates it.
        params.subscription.activation_condition = SubscriptionActivationCondition::OnStart;
        let start = params.subscription.start_date;
        let term_months = params
            .subscription
            .end_date
            .map(|end| months_between(start, end));
        let last_phase = quote
            .phases
            .iter()
            .map(|phase| phase.offset_months)
            .max()
            .unwrap_or(0);
        let months = term_months
            .unwrap_or_else(|| (last_phase + 12).max(12))
            .clamp(1, MAX_MONTHS);
        let until = start
            .checked_add_months(Months::new(months))
            .unwrap_or(start);
        let mut conn = self.store.get_conn().await?;
        let mut processed = self
            .processed_from_quote(&mut conn, &params, quote.phases.clone(), tenant_id)
            .await?;
        processed.dry_run();
        let precision =
            Currencies::resolve_currency_precision(processed.currency()).ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(format!(
                    "Unknown currency {}",
                    processed.currency()
                )))
            })?;

        let mut valued = None;
        let rolled_back = self
            .store
            .transaction_with(&mut conn, |conn| {
                let valued = &mut valued;
                let processed = std::slice::from_ref(&processed);
                async move {
                    let created = self
                        .services
                        .persist_subscriptions_skip_coupon_validation(
                            conn,
                            &Actor::System,
                            processed,
                            tenant_id,
                            &self.store.settings.public_url,
                        )
                        .await?;
                    let subscription_id = created
                        .first()
                        .map(|sub| sub.id)
                        .ok_or_else(|| Report::new(StoreError::InsertError))?;
                    let today = common_clock::now().date_naive();
                    let projection = crate::services::timeline::project_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                        months_between(today, until).max(1),
                    )
                    .await?;
                    let phases = crate::services::subscriptions::phases::list_phases_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                    )
                    .await?
                    .into_iter()
                    .map(|phase| phase.start_date)
                    .filter(|day| *day > start)
                    .collect::<Vec<_>>();
                    *valued = Some((projection, phases));
                    Err::<(), _>(Report::new(StoreError::FailedPrecondition(
                        "a quote's value is rolled back".into(),
                    )))
                }
                .scope_boxed()
            })
            .await;
        let (projection, phase_starts) = match (valued, rolled_back) {
            (Some(valued), _) => valued,
            (None, Err(error)) => return Err(error),
            (None, Ok(())) => {
                return Err(Report::new(StoreError::FailedPrecondition(
                    "a quote's value was not rolled back".into(),
                )));
            }
        };
        let mut written = phase_starts.into_iter();
        let step_starts: Vec<NaiveDate> = quote
            .phases
            .iter()
            .map(|phase| match phase.offset_months {
                0 => start,
                offset => written.next().unwrap_or_else(|| {
                    start
                        .checked_add_months(Months::new(offset))
                        .unwrap_or(start)
                }),
            })
            .collect();
        Ok(contract_value(
            &projection,
            start,
            until,
            term_months,
            months,
            &step_starts,
            precision,
        ))
    }
}

/// The whole months from `from` to `to`.
fn full_months(from: NaiveDate, to: NaiveDate) -> u32 {
    let mut months = 0;
    while from
        .checked_add_months(Months::new(months + 1))
        .is_some_and(|day| day <= to)
    {
        months += 1;
    }
    months
}

/// The months from `from` to `to`, a part of one counting as one.
fn months_between(from: NaiveDate, to: NaiveDate) -> u32 {
    let full = full_months(from, to);
    full + u32::from(from.checked_add_months(Months::new(full)) != Some(to))
}

fn contract_value(
    projection: &Projection,
    start: NaiveDate,
    until: NaiveDate,
    term_months: Option<u32>,
    months: u32,
    phase_starts: &[NaiveDate],
    precision: u8,
) -> QuoteContractValue {
    let major = |cents: i64| Decimal::new(cents, u32::from(precision));
    let invoices: Vec<_> = projection
        .invoices
        .iter()
        .filter(|invoice| invoice.on >= start && invoice.on < until)
        .collect();
    let total: i64 = invoices.iter().map(|invoice| invoice.total).sum();
    let committed: i64 = invoices.iter().map(|invoice| invoice.committed_total).sum();

    let year_count = months.div_ceil(12) as usize;
    let mut years = vec![0i64; year_count];
    for invoice in &invoices {
        let year = (full_months(start, invoice.on) / 12) as usize;
        if let Some(slot) = years.get_mut(year.min(year_count.saturating_sub(1))) {
            *slot += invoice.total;
        }
    }

    let phases = phase_starts
        .iter()
        .enumerate()
        .map(|(i, from)| {
            let to = phase_starts.get(i + 1).copied();
            if to.is_none() && term_months.is_none() {
                return None;
            }
            let to = to.unwrap_or(until).min(until);
            Some(major(
                invoices
                    .iter()
                    .filter(|invoice| invoice.on >= *from && invoice.on < to)
                    .map(|invoice| invoice.total)
                    .sum(),
            ))
        })
        .collect();

    let annual = if months >= 12 {
        (major(total) * Decimal::from(12) / Decimal::from(months)).round_dp(u32::from(precision))
    } else {
        major(total)
    };
    QuoteContractValue {
        term_months,
        months,
        total: major(total),
        committed_total: major(committed),
        annual,
        years: years.into_iter().map(major).collect(),
        phases,
        first_invoice: invoices.first().map(|invoice| major(invoice.total)),
    }
}

/// The value kept by [`ServicesEdge::refresh_quote_contract_value`].
pub async fn stored_contract_value(
    conn: &mut PgConn,
    tenant_id: TenantId,
    quote_id: QuoteId,
) -> StoreResult<Option<QuoteContractValue>> {
    let row = QuoteContractValueRow::find(conn, tenant_id, quote_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(row.and_then(|row| serde_json::from_value(row.value).ok()))
}
