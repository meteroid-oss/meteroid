//! A subscription's period invoice from its facts: evaluate at the invoice's
//! instant, reconcile the charges its slot holds against what the subscription's other documents
//! billed, and render what is left to bill as lines (R-080, R-075). Coupons, the minimum's
//! true-up, tax and credit follow as for any document.

use std::collections::{HashMap, HashSet};

use billing_core::charge::{ChargeKey, ChargeKind, Denomination, Fee};
use billing_core::evaluate::{Charge, DocumentSlot, Evaluation, evaluate};
use billing_core::facts::{Billing, Ending, EndingKind, FactKind, Facts, FeeFact};
use billing_core::issued::{IssuedKind, IssuedLine};
use billing_core::pricing;
use billing_core::proration::DayFraction;
use billing_core::reconcile::{Adjustment, Expected, TakeBack, Term, reconcile};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use common_domain::ids::{BaseId, SubscriptionId, TenantId};
use diesel_models::invoices::InvoiceRow;
use error_stack::Report;
use rust_decimal::Decimal;

use super::issued::{issued_line_of, load_billed, noted_from};
use super::load::{LoadedFacts, Overlay, load_facts, load_facts_with};
use super::pricer::{StoreFee, StorePricer};
use super::usage::{BreakdownRead, attach_breakdowns, read_usage};
use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::enums::InvoiceType;
use crate::domain::{
    Invoice, LineCreditUnit, LineItem, LineItemKind, SubscriptionDetails, SubscriptionFee,
};
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::credit::forecast::CreditBooks;
use crate::services::invoice_lines::charges::{Lineages, disambiguate, identified};
use crate::services::invoice_lines::component::{ExistingLineKey, is_usage_based_line};
use crate::services::invoice_lines::fees::pricing_error;
use crate::services::invoice_lines::invoice_lines::{
    BilledSpan, PricedInvoice, PricedLines, apply_temporal_date_range_to_names,
};
use crate::store::PgConn;

/// Issued lines past the watermark that bill no charge the facts evaluate (or more than one):
/// they bill nothing, and each is counted for support to look at.
static RECONCILE_UNMATCHED: std::sync::LazyLock<opentelemetry::metrics::Counter<u64>> =
    std::sync::LazyLock::new(|| {
        common_logging::GLOBAL_METER
            .u64_counter("billing_reconcile_unmatched_lines")
            .with_description(
                "Issued lines past the reconcile watermark that bill no charge of the facts",
            )
            .build()
    });

/// What the facts put on a document written before them that no document billed: never added to
/// it (R-077), counted so a charge left unbilled shows.
static RECONCILE_UNBILLED: std::sync::LazyLock<opentelemetry::metrics::Counter<u64>> =
    std::sync::LazyLock::new(|| {
        common_logging::GLOBAL_METER
            .u64_counter("billing_reconcile_unbilled_minor")
            .with_description(
                "What the facts put on a written document that no document billed, in minor units",
            )
            .build()
    });

/// What the documents billed of a charge beyond what the facts owe it, found by a period invoice:
/// given back by a change's notes only, counted so a charge billed too much shows.
static RECONCILE_OVERBILLED: std::sync::LazyLock<opentelemetry::metrics::Counter<u64>> =
    std::sync::LazyLock::new(|| {
        common_logging::GLOBAL_METER
            .u64_counter("billing_reconcile_overbilled_minor")
            .with_description(
                "What the documents billed of a charge beyond what the facts owe it, in minor units",
            )
            .build()
    });

fn count(counter: &opentelemetry::metrics::Counter<u64>, tenant: TenantId, amount: u64) {
    counter.add(
        amount,
        &[common_logging::KeyValue::new("tenant", tenant.to_string())],
    );
}

pub(crate) fn midnight(date: NaiveDate) -> DateTime<Utc> {
    date.and_time(NaiveTime::MIN).and_utc()
}

/// Whether a period invoice of `invoice_type` is billed from its subscription's facts.
pub(in crate::services) fn bills_from_facts(invoice_type: &InvoiceType) -> bool {
    matches!(invoice_type, InvoiceType::Recurring | InvoiceType::Final)
}

/// The document a recurring invoice of `invoice_date` (or a final one) is, and those it carries.
#[derive(Clone, Debug)]
struct Slot {
    slot: DocumentSlot,
    /// The ending's day while final invoices are off: its last charges ride that day's invoice.
    final_on: Option<NaiveDate>,
    /// Earlier periods no recurring invoice was written for, caught up by this one (R-084).
    caught_up: Vec<NaiveDate>,
    /// The period of the latest recurring invoice before this one.
    last: Option<NaiveDate>,
}

impl Slot {
    fn carries(&self, document: DocumentSlot) -> bool {
        document == self.slot
            || (document == DocumentSlot::Final && self.final_on.is_some())
            || matches!(document, DocumentSlot::Recurring { period_start } if self.caught_up.contains(&period_start))
    }
}

/// What a period invoice bills from the facts, before coupons, tax and credit.
pub(in crate::services) struct FromFacts {
    pub priced: PricedInvoice,
    /// The state the facts derive now, checked against the stored row.
    #[cfg(feature = "facts-shadow")]
    pub state: billing_core::evaluate::DerivedState,
    /// The instant the subscription ends at, as its ending fact records it.
    #[cfg(feature = "facts-shadow")]
    pub ending: Option<DateTime<Utc>>,
    /// The reconcile watermark.
    #[cfg(feature = "facts-shadow")]
    pub watermark: Option<NaiveDate>,
    /// The fee rows, as the comparison reads them.
    #[cfg(feature = "facts-shadow")]
    pub fees: Vec<crate::services::facts_shadow::ShadowFee>,
    /// Where a free trial ends, fees billing from it.
    #[cfg(feature = "facts-shadow")]
    pub trial_end: Option<NaiveDate>,
}

#[cfg(feature = "facts-shadow")]
fn shadow_fees(facts: &Facts<StoreFee>) -> Vec<crate::services::facts_shadow::ShadowFee> {
    use crate::domain::SubscriptionFee;
    facts
        .fees
        .iter()
        .map(|fee| {
            let store = &fee.fee;
            let minor = Decimal::from(10_i64.pow(u32::from(store.precision)));
            let price = match &store.fee {
                SubscriptionFee::Rate { rate } | SubscriptionFee::Capacity { rate, .. } => {
                    Some(*rate)
                }
                SubscriptionFee::Recurring { rate, quantity, .. }
                | SubscriptionFee::OneTime { rate, quantity } => {
                    Some(*rate * Decimal::from(*quantity))
                }
                _ => None,
            };
            let included = match &store.fee {
                SubscriptionFee::Capacity { included, .. } => {
                    Some(Decimal::from(*included) * store.instances)
                }
                _ => None,
            };
            crate::services::facts_shadow::ShadowFee {
                row: fee.id,
                arrears: fee.billing == Billing::Arrears,
                price: price.map(|price| price * store.instances * minor),
                included,
                metric: store.metric(),
            }
        })
        .collect()
}

/// The facts as a document written at `now` reads them: a cancellation still to come gives its
/// period back at its date (R-077), so until then the period bills whole.
pub(super) fn as_written(facts: &mut Facts<StoreFee>, now: DateTime<Utc>) {
    // A start settled by the time the invoice is computed is settled for it, whatever its date.
    if let Some(at) = facts.start.settled
        && at <= common_clock::now()
    {
        facts.start.settled = Some(at.min(now));
    }
    if facts
        .ending
        .is_some_and(|ending| ending.kind == EndingKind::Cancel && ending.at > now)
    {
        facts.ending = None;
    }
}

/// What `key` owes by now that no line bills, the facts read as they stand (`unwritten`: the
/// start's settlement and the ending `as_written` set aside), with `watermark` the notes dated
/// from it took back from earlier lines; `None` when nothing.
fn unbilled_by_now(
    facts: &Facts<StoreFee>,
    (settled, ending): (Option<DateTime<Utc>>, Option<Ending>),
    key: &ChargeKey,
    issued: &[IssuedLine],
    watermark: Option<(NaiveDate, i64)>,
) -> StoreResult<Option<i64>> {
    let facts = Facts {
        start: billing_core::facts::Start {
            settled,
            ..facts.start
        },
        ending,
        ..facts.clone()
    };
    let evaluation =
        evaluate(&facts, common_clock::now(), None, &StorePricer).map_err(pricing_error)?;
    let owed_from = |from: DateTime<Utc>| -> i64 {
        evaluation
            .charges
            .iter()
            .filter(|charge| charge.key == *key && !charge.is_commitment() && charge.due_at >= from)
            .map(|charge| charge.amount)
            .sum()
    };
    let expected = Expected {
        key: key.clone(),
        amount: owed_from(DateTime::<Utc>::MIN_UTC),
    };
    let since = watermark.map(|(w, noted)| (w, owed_from(midnight(w)) + noted));
    Ok(match decide(&expected, issued, since, true) {
        Decision::Unbilled(owed) if owed > 0 => Some(owed),
        _ => None,
    })
}

/// The instant an invoice of `invoice_date` is evaluated at, however late it is computed: its date,
/// or the ending it settles that day. Every charge it carries is due by then.
fn evaluated_at(facts: &Facts<StoreFee>, invoice_date: NaiveDate) -> DateTime<Utc> {
    let now = midnight(invoice_date);
    match facts.effective_ending() {
        Some(ending) if ending.on() == invoice_date => now.max(ending.at),
        _ => now,
    }
}

impl Services {
    /// The lines of a subscription period's invoice from its facts, priced but neither covered,
    /// discounted nor taxed; the stored content when the invoice keeps its lines as written.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn price_from_facts(
        &self,
        conn: &mut PgConn,
        invoice_date: &NaiveDate,
        details: &SubscriptionDetails,
        prepaid_amount: Option<u64>,
        invoice: Option<&Invoice>,
        invoice_type: InvoiceType,
        book: Option<&CreditBooks>,
    ) -> StoreResult<PricedInvoice> {
        self.price_from_facts_with(
            conn,
            invoice_date,
            details,
            prepaid_amount,
            invoice,
            invoice_type,
            book,
            None,
        )
        .await
    }

    /// [`Self::price_from_facts`] with a change not yet written laid over the facts.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn price_from_facts_with(
        &self,
        conn: &mut PgConn,
        invoice_date: &NaiveDate,
        details: &SubscriptionDetails,
        prepaid_amount: Option<u64>,
        invoice: Option<&Invoice>,
        invoice_type: InvoiceType,
        book: Option<&CreditBooks>,
        overlay: Option<&Overlay>,
    ) -> StoreResult<PricedInvoice> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            if !bills_from_facts(&invoice_type) {
                return self
                    .price_invoice_lines(
                        conn,
                        invoice_date,
                        details,
                        prepaid_amount,
                        invoice,
                        invoice_type,
                        book,
                    )
                    .await;
            }
            let billed = self
                .bill_from_facts(
                    conn,
                    *invoice_date,
                    details,
                    prepaid_amount,
                    invoice,
                    invoice_type.clone(),
                    book,
                    overlay,
                )
                .await?;
            #[cfg(feature = "facts-shadow")]
            if overlay.is_none() {
                self.compare_with_legacy(
                    conn,
                    crate::services::facts_shadow::Compared {
                        invoice_date: *invoice_date,
                        details,
                        prepaid_amount,
                        invoice,
                        invoice_type,
                    },
                    &billed,
                )
                .await;
            }
            Ok(billed.priced)
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn bill_from_facts(
        &self,
        conn: &mut PgConn,
        invoice_date: NaiveDate,
        details: &SubscriptionDetails,
        prepaid_amount: Option<u64>,
        invoice: Option<&Invoice>,
        invoice_type: InvoiceType,
        book: Option<&CreditBooks>,
        overlay: Option<&Overlay>,
    ) -> StoreResult<FromFacts> {
        let sub = &details.subscription;
        let currency = Currencies::resolve_currency(&sub.currency).ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(format!(
                "Currency {} not found",
                sub.currency
            )))
        })?;
        // An invoice keeps the basis it was first computed on; a new one takes the subscription's.
        let tax_inclusive = invoice.map_or(sub.tax_inclusive, |invoice| invoice.tax_inclusive);
        // Stored lines are net; they re-enter in the figures they were quoted in.
        let written: Option<Vec<LineItem>> = invoice.map(|invoice| {
            invoice
                .line_items
                .iter()
                .cloned()
                .map(|line| {
                    if tax_inclusive {
                        line.into_tax_inclusive_quote()
                    } else {
                        line
                    }
                })
                .collect()
        });

        let mut loaded = load_facts_with(
            conn,
            sub.tenant_id,
            sub.id,
            invoice_date,
            written.as_deref(),
            overlay,
        )
        .await?;
        // A draft of a period a backdated cancellation ended before keeps nothing: closed empty.
        let past_the_ending = invoice.is_some_and(|invoice| {
            invoice.status == crate::domain::enums::InvoiceStatusEnum::Draft
                && invoice.invoice_type == InvoiceType::Recurring
        }) && loaded.facts.ending.is_some_and(|ending| {
            ending.kind == EndingKind::Cancel
                && midnight(invoice_date) >= ending.at
                && loaded
                    .facts
                    .ending_recorded
                    .is_some_and(|recorded| recorded > ending.on())
        });
        let cancelled_on = loaded
            .facts
            .ending
            .filter(|ending| ending.kind == EndingKind::Cancel)
            .map(|ending| ending.on());
        #[cfg(feature = "facts-shadow")]
        let ending = loaded.facts.effective_ending().map(|ending| ending.at);
        #[cfg(feature = "facts-shadow")]
        let state = evaluate(&loaded.facts, common_clock::now(), None, &StorePricer)
            .map_err(pricing_error)?
            .state;
        let now = evaluated_at(&loaded.facts, invoice_date);
        let cancelled_after = loaded
            .facts
            .ending
            .is_some_and(|ending| ending.kind == EndingKind::Cancel && ending.at > now);
        let unwritten = (loaded.facts.start.settled, loaded.facts.ending);
        as_written(&mut loaded.facts, now);

        let planned = evaluate(&loaded.facts, now, None, &StorePricer).map_err(pricing_error)?;
        let slot = self
            .slot_of(conn, &loaded, &planned, invoice_date, &invoice_type)
            .await?;

        // A written invoice billing no usage, whose period accrues none, keeps its lines.
        if let Some(invoice) = invoice.filter(|_| !past_the_ending) {
            let accrues = details.can_accrue_arrears()
                || planned
                    .usage_needed
                    .iter()
                    .any(|n| slot.carries(n.document))
                || planned
                    .charges
                    .iter()
                    .any(|c| slot.carries(c.document) && is_arrears(c, &loaded.facts));
            if !invoice.line_items.iter().any(is_usage_based_line)
                && !accrues
                && crate::services::invoice_lines::discount::shows_owed_coupons(
                    conn, details, invoice,
                )
                .await?
            {
                let already_paid = prepaid_amount.unwrap_or(0) as i64;
                let content = self
                    .estimate_stored_credit(conn, invoice, already_paid, common_clock::now(), book)
                    .await?;
                return Ok(FromFacts {
                    priced: PricedInvoice::Stored(content),
                    #[cfg(feature = "facts-shadow")]
                    state,
                    #[cfg(feature = "facts-shadow")]
                    ending,
                    #[cfg(feature = "facts-shadow")]
                    watermark: loaded.facts.reconcile_from,
                    #[cfg(feature = "facts-shadow")]
                    fees: shadow_fees(&loaded.facts),
                    #[cfg(feature = "facts-shadow")]
                    trial_end: loaded
                        .facts
                        .free_trial()
                        .map(|_| loaded.facts.billing_start()),
                });
            }
        }

        let needs: Vec<_> = planned
            .usage_needed
            .iter()
            .filter(|need| slot.carries(need.document) && need.read.window.until <= now)
            .cloned()
            .collect();
        let reads = read_usage(self.usage_client.as_ref(), &loaded, &needs).await?;
        let evaluation =
            evaluate(&loaded.facts, now, Some(&reads), &StorePricer).map_err(pricing_error)?;

        let lineages = lineages_of(&loaded.facts);
        let charges: Vec<&Charge> = evaluation
            .charges
            .iter()
            .filter(|charge| slot.carries(charge.document) && !charge.is_commitment())
            .collect();
        let from = charges
            .iter()
            .map(|charge| charge.key.from)
            .min()
            .unwrap_or(invoice_date)
            .min(invoice_date);
        let reissuing = match slot.slot {
            DocumentSlot::Recurring { period_start } => {
                Some((InvoiceType::Recurring, Some(period_start)))
            }
            DocumentSlot::Final => Some((InvoiceType::Final, None)),
            _ => None,
        };
        let billed = load_billed(
            conn,
            sub.tenant_id,
            sub.id,
            from,
            invoice.map(|invoice| invoice.id),
            reissuing,
        )
        .await?;

        // A written invoice keeps its fee lines (R-077): what a change makes of its period is the
        // change's own documents'. Its usage is read again (R-050).
        let kept: Vec<LineItem> = match (&written, invoice) {
            (Some(written), Some(_)) if !past_the_ending => written
                .iter()
                .filter(|line| !is_usage_based_line(line))
                .filter(|line| {
                    !crate::services::invoice_lines::minimum::is_true_up(line, invoice_date, None)
                })
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        let kept_issued: Vec<IssuedLine> = match invoice {
            Some(invoice) => kept
                .iter()
                .map(|line| {
                    issued_line_of(
                        line,
                        invoice,
                        sub.id,
                        &lineages,
                        &HashMap::new(),
                        &HashMap::new(),
                        &HashMap::new(),
                    )
                })
                .collect(),
            None => Vec::new(),
        };
        let others: Vec<IssuedLine> = billed.lines.iter().map(|on| on.line.clone()).collect();
        let drafted: HashSet<&str> = billed
            .lines
            .iter()
            .filter(|on| on.draft)
            .map(|on| on.line.id.as_str())
            .collect();
        let watermark = loaded.facts.reconcile_from;

        count_unmatched(
            sub.tenant_id,
            sub.id,
            &evaluation,
            billed
                .lines
                .iter()
                .chain(&billed.unattached)
                .map(|on| &on.line)
                .chain(&kept_issued),
            watermark,
            invoice_date,
        );

        let mut to_bill: Vec<(ChargeKey, i64)> = Vec::new();
        for key in keys_of(&charges) {
            // What the charge owes by now against all its lines; this document bills up to its
            // own share, as a charge renewing inside another cadence's period has several.
            let here: i64 = charges
                .iter()
                .filter(|charge| charge.key == key)
                .map(|charge| charge.amount)
                .sum();
            let expected = Expected {
                key: key.clone(),
                amount: evaluation
                    .charges
                    .iter()
                    .filter(|charge| charge.key == key && !charge.is_commitment())
                    .map(|charge| charge.amount)
                    .sum(),
            };
            let usage = matches!(key.kind, ChargeKind::Usage { .. });
            // A fixed line bills one charge it falls in: a month's line under a year's charge of
            // the same fee is the month's (R-080).
            let narrowest = |line: &IssuedLine| {
                usage
                    || billing_core::evaluate::owner(
                        line,
                        evaluation.charges.iter().map(|charge| &charge.key),
                        &evaluation.charges,
                    )
                    .is_none_or(|owner| *owner == key)
            };
            // A usage line issued over a longer period holding this one billed its usage: a
            // cancellation backdated past that line's invoice reads it again (R-080). A draft's
            // line is billed again when it is refreshed; an ending's own day starts past the line.
            let covers = |line: &IssuedLine| {
                usage
                    && line.key.subscription == key.subscription
                    && line.key.fee == key.fee
                    && line.key.kind == key.kind
                    && line.key.denomination == key.denomination
                    && line.from <= key.from
                    && key.from < line.until
                    && key.until <= line.until
                    && !drafted.contains(line.id.as_str())
            };
            let issued: Vec<IssuedLine> = others
                .iter()
                .chain(kept_issued.iter().filter(|_| !usage))
                .filter(|line| (line.bills(&key) || covers(line)) && narrowest(line))
                .map(|line| {
                    let mut line = line.clone();
                    if !line.bills(&key) {
                        (line.from, line.until) = (key.from, key.until);
                        line.key = key.clone();
                    }
                    line
                })
                .collect();
            let since: i64 = watermark.map_or(0, |w| {
                evaluation
                    .charges
                    .iter()
                    .filter(|charge| {
                        charge.key == key && !charge.is_commitment() && charge.due_at >= midnight(w)
                    })
                    .map(|charge| charge.amount)
                    .sum::<i64>()
                    + noted_from(&billed.noted, &issued, w)
            });
            let difference = match decide(
                &expected,
                &issued,
                watermark.map(|w| (w, since)),
                invoice.is_some(),
            ) {
                Decision::Bill(amount) if here < 0 && amount > 0 => continue,
                Decision::Bill(amount) => amount.min(here),
                Decision::Billed => continue,
                Decision::Overbilled(over) => {
                    log::warn!(
                        "subscription {}: {key:?} billed {over} more than the facts owe, nothing billed",
                        sub.id
                    );
                    count(
                        &RECONCILE_OVERBILLED,
                        sub.tenant_id,
                        u64::try_from(over).unwrap_or(0),
                    );
                    continue;
                }
                Decision::Legacy => {
                    log::info!(
                        "subscription {}: {key:?} billed by a line before the reconcile watermark, nothing more billed",
                        sub.id
                    );
                    continue;
                }
                // A cancellation after the document gave back, on its own note, what the
                // document still reads as owed: nothing is unbilled (R-077).
                Decision::Unbilled(owed)
                    if cancelled_after
                        && owed <= issued.iter().map(|line| line.corrected).sum::<i64>() =>
                {
                    continue;
                }
                Decision::Unbilled(as_written) => {
                    // The document reads the facts at its date; whether a charge is left unbilled
                    // is what the facts owe by now against every line, a later ending's notes too.
                    let Some(owed) = unbilled_by_now(
                        &loaded.facts,
                        unwritten,
                        &key,
                        &issued,
                        watermark.map(|w| (w, noted_from(&billed.noted, &issued, w))),
                    )?
                    else {
                        continue;
                    };
                    log::debug!(
                        "subscription {}: {key:?} owes {as_written} at the document's date",
                        sub.id
                    );
                    log::warn!(
                        "subscription {}: {key:?} owes {owed} no document billed; the written invoice keeps its lines",
                        sub.id
                    );
                    #[cfg(feature = "facts-shadow")]
                    crate::services::facts_shadow::record_unbilled(
                        sub.id,
                        &format!("{invoice_date}: {key:?} owes {owed} no document billed"),
                    );
                    count(
                        &RECONCILE_UNBILLED,
                        sub.tenant_id,
                        u64::try_from(owed).unwrap_or(0),
                    );
                    continue;
                }
            };
            if let Some(charge) = charges
                .iter()
                .find(|charge| charge.key == key && charge.once_per_product.is_some())
                && !self
                    .once_credit_due_on(conn, details, charge, &loaded.facts, &slot, invoice)
                    .await?
            {
                continue;
            }
            to_bill.push((key, difference));
        }

        let existing: HashMap<ExistingLineKey, &LineItem> = written
            .iter()
            .flatten()
            .filter(|line| is_usage_based_line(line))
            .filter_map(|line| ExistingLineKey::from_line_item(line).map(|key| (key, line)))
            .collect();
        let order = Order::of(details, &loaded.facts);
        let mut rendered: Vec<(OrderKey, LineItem, Option<BreakdownRead>)> = Vec::new();
        for (key, amount) in to_bill {
            let of_key: Vec<&Charge> = charges
                .iter()
                .copied()
                .filter(|charge| charge.key == key)
                .collect();
            let Some(row) = row_of(&loaded.facts, of_key[0]) else {
                continue;
            };
            if let [charge] = of_key[..]
                && amount == charge.amount
                && let Some(parts) = by_row(&loaded.facts, charge, currency.precision)
            {
                for (part, part_row) in parts {
                    let line = render(
                        &[&part],
                        part.amount,
                        part_row,
                        currency.precision,
                        sub.id,
                        &existing,
                        &lineages,
                    )?;
                    let read = BreakdownRead::of(&[&part], &part_row.fee);
                    rendered.push((order.key(part_row, &part), line, read));
                }
                continue;
            }
            let line = render(
                &of_key,
                amount,
                row,
                currency.precision,
                sub.id,
                &existing,
                &lineages,
            )?;
            let read = BreakdownRead::of(&of_key, &row.fee);
            rendered.push((order.key(row, of_key[0]), line, read));
        }
        rendered.sort_by(|a, b| a.0.cmp(&b.0));
        let mut with_reads: Vec<(LineItem, Option<BreakdownRead>)> = rendered
            .into_iter()
            .map(|(_, line, read)| (line, read))
            .collect();
        attach_breakdowns(self.usage_client.as_ref(), &loaded, &mut with_reads).await;
        let mut lines: Vec<LineItem> = with_reads.into_iter().map(|(line, _)| line).collect();
        lines.extend(kept);
        apply_temporal_date_range_to_names(&mut lines);
        disambiguate(&mut lines, &Lineages::of(details));

        let cycle_index = invoice
            .and_then(|invoice| invoice.cycle_index)
            .or(sub.cycle_index)
            .unwrap_or(0);
        let (read_at, span) = crate::services::invoice_lines::invoice_lines::billed_at(
            conn,
            sub.tenant_id,
            sub.id,
            BilledSpan::of(&invoice_type),
            invoice_date,
        )
        .await?;
        // A backdated cancellation's final invoice trues up the period its ending cut, as the
        // recurring invoice of the ending's day would; a draft past that ending trues up nothing.
        let trues_up = match invoice_type {
            InvoiceType::Recurring => !past_the_ending,
            InvoiceType::Final => span == BilledSpan::Period,
            _ => false,
        };
        let nominals = nominals(details, &loaded.facts, &planned, currency.precision);
        let true_up = if trues_up {
            crate::services::invoice_lines::minimum::load_period_true_up_priced(
                conn,
                details,
                read_at,
                cycle_index,
                written
                    .as_ref()
                    .zip(invoice)
                    .map(|(lines, invoice)| Invoice {
                        line_items: lines.clone(),
                        ..invoice.clone()
                    })
                    .as_ref(),
                &nominals,
            )
            .await?
        } else {
            None
        };
        // The ending's own document trues up again the earlier periods it gives spend back in.
        let ends_here = match invoice_type {
            InvoiceType::Final => true,
            InvoiceType::Recurring => !past_the_ending && cancelled_on == Some(read_at),
            _ => false,
        };
        let reopened = match cancelled_on.filter(|on| ends_here && *on <= read_at) {
            Some(on) => {
                let before = true_up
                    .as_ref()
                    .map_or(sub.current_period_start.min(on), |true_up| true_up.start);
                crate::services::invoice_lines::minimum::load_reopened_true_ups(
                    conn, details, before, on, &nominals,
                )
                .await?
                .into_boxed_slice()
            }
            None => Box::default(),
        };

        Ok(FromFacts {
            priced: PricedInvoice::Lines(PricedLines {
                lines,
                invoice_date: read_at,
                currency_precision: currency.precision,
                tax_inclusive,
                span,
                invoice_type,
                prepaid_amount,
                invoice_id: invoice.map(|invoice| invoice.id),
                true_up,
                reopened,
            }),
            #[cfg(feature = "facts-shadow")]
            state,
            #[cfg(feature = "facts-shadow")]
            ending,
            #[cfg(feature = "facts-shadow")]
            watermark: loaded.facts.reconcile_from,
            #[cfg(feature = "facts-shadow")]
            fees: shadow_fees(&loaded.facts),
            #[cfg(feature = "facts-shadow")]
            trial_end: loaded
                .facts
                .free_trial()
                .map(|_| loaded.facts.billing_start()),
        })
    }

    /// The slot of the invoice of `invoice_date`, with the periods it catches up: those since the
    /// last recurring invoice before it that none was written for.
    async fn slot_of(
        &self,
        conn: &mut PgConn,
        loaded: &LoadedFacts,
        planned: &Evaluation,
        invoice_date: NaiveDate,
        invoice_type: &InvoiceType,
    ) -> StoreResult<Slot> {
        if *invoice_type == InvoiceType::Final {
            return Ok(Slot {
                slot: DocumentSlot::Final,
                final_on: None,
                caught_up: Vec::new(),
                last: None,
            });
        }
        let final_on = loaded
            .facts
            .effective_ending()
            .map(|ending| ending.on())
            .filter(|on| {
                *on == invoice_date
                    && !crate::services::subscriptions::terminate::issues_final_invoices()
            });
        let subscription = loaded.facts.subscription;
        let last = InvoiceRow::latest_recurring_period_before(
            conn,
            loaded.tenant_id,
            subscription,
            invoice_date,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let Some(last) = last else {
            return Ok(Slot {
                slot: DocumentSlot::Recurring {
                    period_start: invoice_date,
                },
                final_on,
                caught_up: Vec::new(),
                last: None,
            });
        };
        let mut caught_up: Vec<NaiveDate> = planned
            .charges
            .iter()
            .map(|charge| charge.document)
            .chain(planned.usage_needed.iter().map(|need| need.document))
            .filter_map(|document| match document {
                DocumentSlot::Recurring { period_start }
                    if period_start < invoice_date && period_start > last =>
                {
                    Some(period_start)
                }
                _ => None,
            })
            .collect();
        caught_up.sort();
        caught_up.dedup();
        Ok(Slot {
            slot: DocumentSlot::Recurring {
                period_start: invoice_date,
            },
            final_on,
            caught_up,
            last: Some(last),
        })
    }

    /// Whether the invoice of `invoice_date` reads usage of an elapsed window: an empty one still
    /// anchors the usage its grace may bring.
    pub(in crate::services) async fn reads_usage_on(
        &self,
        conn: &mut PgConn,
        details: &SubscriptionDetails,
        invoice_date: NaiveDate,
    ) -> StoreResult<bool> {
        let sub = &details.subscription;
        let mut loaded = load_facts(conn, sub.tenant_id, sub.id, invoice_date, None).await?;
        let now = evaluated_at(&loaded.facts, invoice_date);
        as_written(&mut loaded.facts, now);
        let planned = evaluate(&loaded.facts, now, None, &StorePricer).map_err(pricing_error)?;
        let slot = self
            .slot_of(
                conn,
                &loaded,
                &planned,
                invoice_date,
                &InvoiceType::Recurring,
            )
            .await?;
        Ok(planned
            .usage_needed
            .iter()
            .any(|need| slot.carries(need.document) && need.read.window.until <= now))
    }

    /// Whether the document of a cancellation ending on `on` would true up again an earlier period
    /// it gives spend back in (R-022): written for that alone when the ending bills nothing else.
    pub(in crate::services) async fn ending_reopens_true_ups(
        &self,
        conn: &mut PgConn,
        details: &SubscriptionDetails,
        on: NaiveDate,
    ) -> StoreResult<bool> {
        let sub = &details.subscription;
        let currency = Currencies::resolve_currency(&sub.currency).ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(format!(
                "Currency {} not found",
                sub.currency
            )))
        })?;
        let loaded = load_facts(conn, sub.tenant_id, sub.id, on, None).await?;
        let planned =
            evaluate(&loaded.facts, midnight(on), None, &StorePricer).map_err(pricing_error)?;
        let nominals = nominals(details, &loaded.facts, &planned, currency.precision);
        let reopened = crate::services::invoice_lines::minimum::load_reopened_true_ups(
            conn,
            details,
            sub.current_period_start.min(on),
            on,
            &nominals,
        )
        .await?;
        Ok(reopened.iter().any(|true_up| {
            let prior = true_up.prior_as_issued(&details.applied_coupons, &sub.currency, &[]);
            !true_up.true_up.lines(&[], &prior).is_empty()
        }))
    }
}

impl Services {
    /// A product's `once` credit is due on the first invoice that carries its fee, unless an
    /// invoice of the subscription already billed one of that product and no void took it back
    /// (R16, OQ6): it rides each renewal's advance until then.
    async fn once_credit_due_on(
        &self,
        conn: &mut PgConn,
        details: &SubscriptionDetails,
        charge: &Charge,
        facts: &Facts<StoreFee>,
        slot: &Slot,
        invoice: Option<&Invoice>,
    ) -> StoreResult<bool> {
        let Some(product) = charge.once_per_product else {
            return Ok(true);
        };
        let sub = &details.subscription;
        let cycle_index = invoice
            .and_then(|invoice| invoice.cycle_index)
            .or(sub.cycle_index)
            .unwrap_or(0);
        // From cycle 2 on, a fee the previous renewal carried was settled there, unless a void
        // took it back.
        if cycle_index >= 2
            && let (Some(row), Some(previous)) = (row_of(facts, charge), slot.last)
            && row.from <= previous
            && !InvoiceRow::subscription_voided_once_credit(conn, sub.tenant_id, sub.id, product)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        {
            return Ok(false);
        }
        let billed = InvoiceRow::subscription_billed_once_credit(
            conn,
            sub.tenant_id,
            sub.id,
            product,
            invoice.map(|invoice| invoice.id),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        Ok(!billed)
    }
}

/// What the document being computed bills of a charge, from the lines of the subscription's other
/// documents (and its own kept lines) that billed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decision {
    /// Billed on this document; a charge of nothing no line billed is shown at nothing.
    Bill(i64),
    /// Billed whole elsewhere: nothing more to bill.
    Billed,
    /// Billed beyond what it owes by this much: given back by a change's documents (R-080), never
    /// here; counted.
    Overbilled(i64),
    /// A line from before the watermark billed it, and nothing is owed since: never billed
    /// against (R-094).
    Legacy,
    /// Owed, but this document was written before the facts that owe it and keeps its fee lines
    /// (R-077): counted, never added.
    Unbilled(i64),
}

/// The positive difference between `expected` and what `issued`, the lines billing its key, still
/// bill after their notes (R-080). With `watermark`, the watermark and what the key's charges due
/// on or after it owe plus what notes from it took back from earlier lines: a line before it
/// settles the charges due before it, so only those are billed against the lines from it (R-094).
pub(super) fn decide(
    expected: &Expected,
    issued: &[IssuedLine],
    watermark: Option<(NaiveDate, i64)>,
    written: bool,
) -> Decision {
    let adjustments = reconcile(
        std::slice::from_ref(expected),
        issued,
        TakeBack::LatestFirst,
    );
    let (difference, over) =
        adjustments
            .iter()
            .fold((0, 0), |(bill, over), adjustment| match adjustment {
                Adjustment::Bill { amount, .. } => (bill + amount, over),
                Adjustment::Credit { amount, .. } | Adjustment::Short { amount, .. } => {
                    (bill, over + amount)
                }
            });
    let legacy = watermark.filter(|(w, _)| issued.iter().any(|line| line.from < *w));
    if over > 0 && legacy.is_none() {
        return Decision::Overbilled(over);
    }
    let difference = match legacy {
        Some((w, since)) => {
            let after: Vec<IssuedLine> = issued
                .iter()
                .filter(|line| line.from >= w)
                .cloned()
                .collect();
            let owed: i64 = reconcile(
                &[Expected {
                    key: expected.key.clone(),
                    amount: since,
                }],
                &after,
                TakeBack::LatestFirst,
            )
            .iter()
            .map(|adjustment| match adjustment {
                Adjustment::Bill { amount, .. } => *amount,
                _ => 0,
            })
            .sum();
            if owed <= 0 {
                return Decision::Legacy;
            }
            owed
        }
        None if difference == 0 && !issued.is_empty() => return Decision::Billed,
        None => difference,
    };
    let usage = matches!(expected.key.kind, ChargeKind::Usage { .. });
    match (written && !usage, difference) {
        (true, 0) => Decision::Billed,
        (true, owed) => Decision::Unbilled(owed),
        (false, amount) => Decision::Bill(amount),
    }
}

/// Whether `charge` is billed at its window's end: usage, or a fee in arrears.
fn is_arrears(charge: &Charge, facts: &Facts<StoreFee>) -> bool {
    charge.usage.is_some()
        || row_of(facts, charge).is_some_and(|row| row.billing == Billing::Arrears)
}

/// The lineage root of each fee row, closed rows' included.
pub(super) fn lineages_of(facts: &Facts<StoreFee>) -> Lineages {
    Lineages::of_roots(facts.fees.iter().filter_map(|row| match row.lineage {
        Fee::Component(root) => Some((row.id, *root)),
        Fee::AddOn(root) => Some((row.id, *root)),
        _ => None,
    }))
}

/// The keys of `charges`, in their order.
fn keys_of(charges: &[&Charge]) -> Vec<ChargeKey> {
    let mut keys: Vec<ChargeKey> = Vec::new();
    for charge in charges {
        if !keys.contains(&charge.key) {
            keys.push(charge.key.clone());
        }
    }
    keys
}

/// A fixed fee in arrears billed by several rows of its lineage over its window, one part per
/// row, each what is left of the row's price at its first day less at its end (R-079), when the
/// parts add up to the charge: the lines show what each row billed.
fn by_row<'a>(
    facts: &'a Facts<StoreFee>,
    charge: &Charge,
    precision: u8,
) -> Option<Vec<(Charge, &'a FeeFact<StoreFee>)>> {
    if charge.key.kind != ChargeKind::Fee || charge.usage.is_some() {
        return None;
    }
    let (start, end) = charge.window;
    let mut rows: Vec<&FeeFact<StoreFee>> = facts
        .fees
        .iter()
        .filter(|row| {
            row.lineage == charge.key.fee
                && row.billing == Billing::Arrears
                && row.from < end
                && row.to.is_none_or(|to| to > start)
        })
        .collect();
    if rows.len() < 2 {
        return None;
    }
    rows.sort_by_key(|row| row.from);
    let mut parts = Vec::with_capacity(rows.len());
    for row in rows {
        let from = row.from.max(charge.window.0);
        let until = row.to.map_or(charge.window.1, |to| to.min(charge.window.1));
        if from >= until {
            continue;
        }
        let share = share_of((from, until), charge.nominal);
        let line = fixed_shown(&row.fee.fee, row.fee.instances, None, share, precision)?;
        let full = fixed_shown(&row.fee.fee, row.fee.instances, None, None, precision)?;
        let amount = Term {
            fee: charge.key.fee.clone(),
            full: i64::try_from(full.total).ok()?,
            period: charge.nominal,
            from,
            until,
        }
        .amount();
        parts.push((
            Charge {
                window: (from, until),
                amount,
                quantity: line.quantity,
                share,
                ..charge.clone()
            },
            row,
        ));
    }
    (parts.iter().map(|(part, _)| part.amount).sum::<i64>() == charge.amount).then_some(parts)
}

/// Each fixed fee charge's whole period and its price there, as its notes prorate it (R-079).
fn nominals(
    details: &SubscriptionDetails,
    facts: &Facts<StoreFee>,
    evaluation: &Evaluation,
    precision: u8,
) -> crate::services::invoice_lines::minimum::Nominals {
    let periods = evaluation
        .charges
        .iter()
        .filter(|charge| {
            charge.key.kind == ChargeKind::Fee
                && charge.key.denomination == Denomination::Money
                && charge.usage.is_none()
        })
        .filter_map(|charge| {
            let row = row_of(facts, charge)?;
            let full = fixed_shown(
                &row.fee.fee,
                row.fee.instances,
                charge.quantity,
                None,
                precision,
            )?;
            let seat = match &row.fee.fee {
                SubscriptionFee::Slot { .. } => fixed_shown(
                    &row.fee.fee,
                    row.fee.instances,
                    Some(Decimal::ONE),
                    None,
                    precision,
                )
                .and_then(|seat| i64::try_from(seat.total).ok()),
                _ => None,
            };
            Some((
                charge.key.fee.clone(),
                (charge.key.from, charge.key.until),
                billing_core::minimum::Nominal {
                    period: charge.nominal,
                    full: i64::try_from(full.total).ok()?,
                    base: 0,
                },
                seat,
                charge.window.0,
            ))
        })
        .collect();
    crate::services::invoice_lines::minimum::Nominals::new(Lineages::of(details), periods)
}

/// The days of `window` over those of `nominal`, when fewer (R-002).
fn share_of(
    window: (NaiveDate, NaiveDate),
    nominal: (NaiveDate, NaiveDate),
) -> Option<DayFraction> {
    let days = (window.1 - window.0).num_days();
    let whole = (nominal.1 - nominal.0).num_days();
    (whole > 0 && days < whole).then(|| DayFraction::new(days, whole))
}

/// The fee row a charge was priced from: the latest its trace names.
pub(super) fn row_of<'a>(
    facts: &'a Facts<StoreFee>,
    charge: &Charge,
) -> Option<&'a FeeFact<StoreFee>> {
    let ids: HashSet<uuid::Uuid> = charge
        .trace
        .iter()
        .filter(|fact| fact.kind == FactKind::Fee)
        .map(|fact| fact.id)
        .collect();
    facts
        .fees
        .iter()
        .filter(|row| ids.contains(&row.id))
        .max_by_key(|row| row.from)
}

/// Counts and logs the issued lines past the watermark, in periods up to `until`, that bill no
/// fee charge of the evaluation, or more than one.
fn count_unmatched<'a>(
    tenant: TenantId,
    subscription: SubscriptionId,
    evaluation: &Evaluation,
    lines: impl Iterator<Item = &'a IssuedLine>,
    watermark: Option<NaiveDate>,
    until: NaiveDate,
) {
    let mut keys: Vec<&ChargeKey> = evaluation
        .charges
        .iter()
        .filter(|charge| matches!(charge.key.kind, ChargeKind::Fee | ChargeKind::Credit))
        .map(|charge| &charge.key)
        .collect();
    keys.sort();
    keys.dedup();
    for line in lines {
        let fee_line = matches!(line.kind, IssuedKind::Fee { .. });
        if !fee_line
            || line.voided
            || line.key.denomination != Denomination::Money
            || line.from >= until
            || watermark.is_some_and(|w| line.from < w)
        {
            continue;
        }
        // A line counts for one charge holding it: a run of another cadence nests in a period.
        let owner = billing_core::evaluate::owner(line, keys.iter().copied(), &evaluation.charges);
        let matched = keys
            .iter()
            .filter(|key| line.bills(key))
            .filter(|key| {
                owner.is_some_and(|owner| (key.from, key.until) == (owner.from, owner.until))
            })
            .count();
        if matched != 1 {
            log::warn!(
                "subscription {subscription}: issued line {} ({:?} {}..{}) bills {matched} charges of the facts, nothing billed against it",
                line.id,
                line.key.fee,
                line.from,
                line.until
            );
            count(&RECONCILE_UNMATCHED, tenant, 1);
        }
    }
}

/// Where a line goes among a document's lines: components, then add-ons, then rows no longer
/// listed; recurring fees before one-time ones; in each row its arrears before its advance.
pub(super) type OrderKey = (u8, u8, usize, u8, NaiveDate, u8, String);

pub(super) struct Order {
    components: Vec<uuid::Uuid>,
    add_ons: Vec<uuid::Uuid>,
    closed: Vec<uuid::Uuid>,
}

impl Order {
    pub(super) fn of(details: &SubscriptionDetails, facts: &Facts<StoreFee>) -> Self {
        let components: Vec<uuid::Uuid> = details
            .price_components
            .iter()
            .map(|c| c.id.as_uuid())
            .collect();
        let add_ons: Vec<uuid::Uuid> = details.add_ons.iter().map(|a| a.id.as_uuid()).collect();
        let mut closed: Vec<&FeeFact<StoreFee>> = facts
            .fees
            .iter()
            .filter(|row| !components.contains(&row.id) && !add_ons.contains(&row.id))
            .collect();
        closed.sort_by_key(|row| (row.from, row.id));
        Self {
            components,
            add_ons,
            closed: closed.into_iter().map(|row| row.id).collect(),
        }
    }

    pub(super) fn key(&self, row: &FeeFact<StoreFee>, charge: &Charge) -> OrderKey {
        let add_on = matches!(row.lineage, Fee::AddOn(_));
        let (group, index) = if let Some(i) = self.components.iter().position(|id| *id == row.id) {
            (0, i)
        } else if let Some(i) = self.add_ons.iter().position(|id| *id == row.id) {
            (1, i)
        } else {
            let i = self.closed.iter().position(|id| *id == row.id).unwrap_or(0);
            (if add_on { 3 } else { 2 }, i)
        };
        let one_time = u8::from(row.billing == Billing::OneTime);
        let advance = u8::from(charge.usage.is_none() && row.billing != Billing::Arrears);
        let kind = u8::from(charge.usage.is_some());
        (
            group,
            one_time,
            index,
            advance,
            charge.window.0,
            kind,
            format!("{:?}", charge.key.kind),
        )
    }
}

/// The line billing `amount` of the charges of one key, all priced from `row`.
pub(super) fn render(
    charges: &[&Charge],
    amount: i64,
    row: &FeeFact<StoreFee>,
    precision: u8,
    subscription: SubscriptionId,
    existing: &HashMap<ExistingLineKey, &LineItem>,
    lineages: &Lineages,
) -> StoreResult<LineItem> {
    let charge = charges[0];
    let fee = &row.fee;
    let whole = charges.len() == 1 && amount == charge.amount;
    let window = (
        charges
            .iter()
            .map(|c| c.window.0)
            .min()
            .unwrap_or(charge.window.0),
        charges
            .iter()
            .map(|c| c.window.1)
            .max()
            .unwrap_or(charge.window.1),
    );
    let metric = match &charge.key.kind {
        ChargeKind::Usage { metric, .. } => Some(*metric),
        _ => None,
    };
    let (quantity, unit_price, is_prorated, sub_lines) = match metric {
        Some(_) => {
            let sub_lines = if whole {
                charge
                    .shown
                    .sub_lines
                    .iter()
                    .cloned()
                    .map(Into::into)
                    .collect()
            } else {
                Vec::new()
            };
            // A remainder of the charge shows no quantity: its amount is not that many units'.
            let (quantity, unit_price) = if whole {
                (charge.quantity, charge.shown.unit_price)
            } else {
                (None, None)
            };
            (quantity, unit_price, false, sub_lines)
        }
        None => {
            let shown = if whole {
                fixed_shown(
                    &fee.fee,
                    fee.instances,
                    charge.quantity,
                    charge.share,
                    precision,
                )
                .filter(|line| i64::try_from(line.total).ok() == Some(amount))
            } else {
                None
            };
            match shown {
                Some(line) => (line.quantity, line.unit_price, line.is_prorated, Vec::new()),
                None => {
                    let quantity = charge.quantity.filter(|q| !q.is_zero());
                    let unit_price = quantity.map(|q| unit_price_of(amount, precision, q));
                    (quantity, unit_price, true, Vec::new())
                }
            }
        }
    };

    let dimensions = charge.shown.dimensions.clone();
    let name = match dimensions.as_ref().filter(|d| !d.is_empty()) {
        Some(dimensions) => {
            let mut sorted: Vec<_> = dimensions.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);
            let values: Vec<String> = sorted.into_iter().map(|(_, v)| v.clone()).collect();
            format!("{} ({})", fee.name, values.join(", "))
        }
        None => fee.name.clone(),
    };
    let existing_line = metric.and_then(|metric_id| {
        existing.get(&ExistingLineKey {
            metric_id,
            sub_component_id: fee.sub_component_id,
            sub_add_on_id: fee.sub_add_on_id,
            group_by_dimensions: dimensions.clone(),
        })
    });

    let fee_unit = match &fee.fee {
        SubscriptionFee::Usage { credit_unit, .. }
        | SubscriptionFee::Capacity { credit_unit, .. } => credit_unit.as_ref(),
        _ => None,
    };
    let credit_unit = match (fee_unit, &charge.key.denomination) {
        (Some(unit), Denomination::CreditUnit(_)) if metric.is_some() => Some(LineCreditUnit {
            unit_id: unit.unit_id,
            code: unit.code.clone(),
            precision: unit.precision,
            rate: unit.rate,
            price_rate: Some(unit.rate),
            unit_price,
            sub_lines: sub_lines.clone(),
            consumed: amount,
            covered: 0,
        }),
        _ => None,
    };
    let (money, unit_price, sub_lines) = match &credit_unit {
        Some(block) => (
            crate::services::invoice_lines::credit_units::overage_money(block, precision)?,
            None,
            Vec::new(),
        ),
        None => (amount, unit_price, sub_lines),
    };

    let (kind, credit_grant_amount, credit_grant_expiry, credit_grant_included) = match &fee.fee {
        SubscriptionFee::Credit { rate, expiry, .. } => (
            LineItemKind::Credit,
            charge.credit_grant.map(|grant| {
                if whole {
                    grant.amount
                } else {
                    grant_share(
                        charges
                            .iter()
                            .filter_map(|c| c.credit_grant)
                            .map(|g| g.amount)
                            .sum(),
                        amount,
                        charges.iter().map(|c| c.amount).sum(),
                    )
                }
            }),
            *expiry,
            rate.is_zero(),
        ),
        _ => (LineItemKind::Regular, None, None, false),
    };
    let credit_grant_unit = match &fee.fee {
        SubscriptionFee::Credit { credit_unit, .. } => credit_unit.clone(),
        _ => None,
    };
    let credit_grant_rollover = match &fee.fee {
        SubscriptionFee::Credit {
            rate,
            quantity,
            rollover: Some(rollover),
            credit_unit,
            ..
        } if rate.is_zero() => Some(crate::services::invoice_lines::component::pack_rollover(
            rollover,
            Decimal::from(*quantity) * fee.instances,
            credit_unit
                .as_ref()
                .map_or(precision, |unit| unit.precision),
        )?),
        _ => None,
    };
    let usage_ends_at = charge
        .usage
        .map(|usage| usage.until)
        .filter(|until| *until != midnight(window.1));

    let line = LineItem {
        local_id: existing_line
            .map(|line| line.local_id.clone())
            .unwrap_or_default(),
        name: existing_line.map_or(name, |line| line.name.clone()),
        charge_type: Some(fee.fee.charge_type()),
        quantity,
        unit_price,
        start_date: window.0,
        end_date: window.1,
        sub_lines,
        is_prorated,
        price_component_id: fee.price_component_id,
        sub_component_id: fee.sub_component_id,
        sub_add_on_id: fee.sub_add_on_id,
        product_id: crate::services::subscriptions::utils::fee_tax_product(
            &fee.fee,
            fee.product_id,
        ),
        metric_id: metric,
        description: existing_line.and_then(|line| line.description.clone()),
        group_by_dimensions: dimensions,
        amount_subtotal: money,
        tax_rate: existing_line.map_or(Decimal::ZERO, |line| line.tax_rate),
        tax_amount: 0,
        tax_details: vec![],
        taxable_amount: money,
        amount_total: money,
        kind,
        credit_grant_amount,
        credit_grant_expiry,
        credit_grant_included,
        taxed_at_purchase: credit_grant_unit.is_some(),
        tax_inclusive_quote: None,
        credit_unit,
        credit_grant_unit,
        credit_grant_rollover,
        promotional_credit: None,
        subscription_id: Some(subscription),
        usage_ends_at,
        usage_breakdown: None,
    };
    Ok(identified(line, lineages))
}

/// The unit price of `quantity` units billing `amount` minor units, to 8 places, half-up.
fn unit_price_of(amount: i64, precision: u8, quantity: Decimal) -> Decimal {
    (Decimal::new(amount, u32::from(precision)) / quantity)
        .round_dp_with_strategy(8, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
}

/// What a pack's charges granting `granted` grant for the `billed` part of the `owed` they bill.
fn grant_share(granted: i64, billed: i64, owed: i64) -> i64 {
    if owed == 0 || billed == owed {
        granted
    } else {
        billing_core::scale(granted, billed.clamp(0, owed.max(0)), owed)
    }
}

/// A fixed fee's line as its price shows it over a window billing `share` of its period.
fn fixed_shown(
    fee: &SubscriptionFee,
    instances: Decimal,
    quantity: Option<Decimal>,
    share: Option<DayFraction>,
    precision: u8,
) -> Option<pricing::PricedLine> {
    let line = match fee {
        SubscriptionFee::Rate { rate } => {
            pricing::simple_prorated(rate, &instances, share, precision)
        }
        SubscriptionFee::OneTime { rate, quantity } => pricing::simple_prorated(
            rate,
            &(Decimal::from(*quantity) * instances),
            None,
            precision,
        ),
        SubscriptionFee::Recurring { rate, quantity, .. } => pricing::simple_prorated(
            rate,
            &(Decimal::from(*quantity) * instances),
            share,
            precision,
        ),
        SubscriptionFee::Slot { unit_rate, .. } => {
            pricing::simple_prorated(unit_rate, &quantity?, share, precision)
        }
        SubscriptionFee::Capacity { rate, .. } => {
            pricing::simple_prorated(rate, &instances, None, precision)
        }
        SubscriptionFee::Credit { rate, quantity, .. } => {
            pricing::credit_pack(rate, Decimal::from(*quantity) * instances, share, precision)
        }
        SubscriptionFee::Usage { .. } => return None,
    };
    line.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, m, d).unwrap()
    }

    fn key(kind: ChargeKind, from: NaiveDate) -> ChargeKey {
        ChargeKey {
            subscription: Some(uuid::Uuid::from_u128(1).into()),
            fee: Fee::Component(uuid::Uuid::from_u128(2).into()),
            from,
            until: day(3, 1),
            kind,
            denomination: Denomination::Money,
        }
    }

    fn expected(kind: ChargeKind, amount: i64) -> Expected {
        Expected {
            key: key(kind, day(2, 1)),
            amount,
        }
    }

    fn line(from: NaiveDate, subtotal: i64, corrected: i64) -> IssuedLine {
        IssuedLine {
            id: format!("line-{from}"),
            key: key(ChargeKind::Fee, from),
            kind: IssuedKind::Fee {
                price_component: None,
                add_on: false,
            },
            lineage: None,
            quantity: None,
            unit_priced: true,
            from,
            until: day(3, 1),
            subtotal,
            quoted: None,
            already_credited: corrected,
            corrected,
            settled: None,
            voided: false,
        }
    }

    fn usage() -> ChargeKind {
        ChargeKind::Usage {
            metric: uuid::Uuid::from_u128(3).into(),
            dimensions: BTreeMap::new(),
        }
    }

    /// A renewal bills a charge no document billed, whole: 29.00 for February.
    /// Rules: R-080
    #[test]
    fn a_new_invoice_bills_what_no_document_billed() {
        assert_eq!(
            decide(&expected(ChargeKind::Fee, 2_900), &[], None, false),
            Decision::Bill(2_900)
        );
    }

    /// February's 29.00, of which an adjustment billed 19.00: the renewal bills the 10.00 left
    /// (29.00 − 19.00), never the whole fee again.
    /// Rules: R-080
    #[test]
    fn a_new_invoice_bills_only_what_its_charge_still_owes() {
        let elsewhere = [line(day(2, 10), 1_900, 0)];
        assert_eq!(
            decide(&expected(ChargeKind::Fee, 2_900), &elsewhere, None, false),
            Decision::Bill(1_000)
        );
    }

    /// Billed whole elsewhere, or more than owed (the give-back is the change's own note): the
    /// renewal bills nothing of it, and what was billed beyond the 14.00 owed, 15.00, is counted.
    /// Rules: R-080
    #[test]
    fn a_charge_billed_elsewhere_bills_nothing() {
        let whole = [line(day(2, 1), 2_900, 0)];
        assert_eq!(
            decide(&expected(ChargeKind::Fee, 2_900), &whole, None, false),
            Decision::Billed
        );
        assert_eq!(
            decide(&expected(ChargeKind::Fee, 1_400), &whole, None, false),
            Decision::Overbilled(1_500)
        );
    }

    /// A 29.00 line the engine corrected by 10.00 still bills 19.00: 29.00 − 19.00 = 10.00 owed.
    /// Rules: R-080, R-082
    #[test]
    fn what_a_correction_took_back_is_owed_again() {
        let corrected = [line(day(2, 1), 2_900, 1_000)];
        assert_eq!(
            decide(&expected(ChargeKind::Fee, 2_900), &corrected, None, false),
            Decision::Bill(1_000)
        );
    }

    /// A line from Jan 31 billing 19.00 of a 29.00 charge: with the watermark on Feb 1 it is the
    /// legacy path's and nothing is billed against it; with the watermark on Jan 31, the 10.00.
    /// Rules: R-094
    #[test]
    fn a_line_before_the_watermark_is_never_billed_against() {
        let before = [line(day(1, 31), 1_900, 0)];
        let charge = Expected {
            key: key(ChargeKind::Fee, day(1, 31)),
            amount: 2_900,
        };
        assert_eq!(
            decide(&charge, &before, Some((day(2, 1), 0)), false),
            Decision::Legacy
        );
        assert_eq!(
            decide(&charge, &before, Some((day(1, 31), 2_900)), false),
            Decision::Bill(1_000)
        );
    }

    /// A yearly 366.00 billed on Jan 1, before the watermark on Mar 1, raised by one unit on Mar 15
    /// (36600 × 292/366 = 29200): the year's line settles what was due before the watermark, the
    /// raise, due after it, is billed against the lines dated from it: none, so 29200. Owing
    /// nothing after the watermark, nothing is billed against the year's line.
    /// Rules: R-094
    #[test]
    fn a_charge_due_after_the_watermark_is_billed_against_the_lines_after_it() {
        let year = |from| ChargeKey {
            until: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            ..key(ChargeKind::Fee, from)
        };
        let legacy = [IssuedLine {
            key: year(day(1, 1)),
            until: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            ..line(day(1, 1), 36_600, 0)
        }];
        let charge = Expected {
            key: year(day(1, 1)),
            amount: 36_600 - 29_200 + 58_400,
        };
        assert_eq!(
            decide(&charge, &legacy, Some((day(3, 1), 29_200)), false),
            Decision::Bill(29_200)
        );
        let after = [
            legacy[0].clone(),
            IssuedLine {
                id: "raise".to_owned(),
                key: year(day(3, 15)),
                until: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                ..line(day(3, 15), 29_200, 0)
            },
        ];
        assert_eq!(
            decide(&charge, &after, Some((day(3, 1), 29_200)), false),
            Decision::Legacy
        );
    }

    /// A written invoice keeps its fee lines: 29.00 the facts owe that no document billed is
    /// counted, never added. Its usage is read again: 7.00 of it is billed.
    /// Rules: R-077, R-050
    #[test]
    fn a_written_invoice_adds_no_fee_and_reads_its_usage_again() {
        assert_eq!(
            decide(&expected(ChargeKind::Fee, 2_900), &[], None, true),
            Decision::Unbilled(2_900)
        );
        assert_eq!(
            decide(&expected(ChargeKind::Fee, 0), &[], None, true),
            Decision::Billed
        );
        assert_eq!(
            decide(&expected(usage(), 700), &[], None, true),
            Decision::Bill(700)
        );
    }

    /// Ten seats at 1.00 billed 10.00 on a January invoice since voided; five more from Jan 16
    /// add 500 × 16/31 = 258. The void forgave the 10.00: only the 258 is billed, and a charge the
    /// void covers whole, or a smaller one, bills nothing.
    /// Rules: R-096
    #[test]
    fn a_voided_line_counts_as_billed_and_forgives_nothing_more() {
        let voided = [IssuedLine {
            voided: true,
            ..line(day(2, 1), 1_000, 0)
        }];
        assert_eq!(
            decide(&expected(ChargeKind::Fee, 1_258), &voided, None, false),
            Decision::Bill(258)
        );
        assert_eq!(
            decide(&expected(ChargeKind::Fee, 1_000), &voided, None, false),
            Decision::Billed
        );
        assert_eq!(
            decide(&expected(ChargeKind::Fee, 516), &voided, None, false),
            Decision::Billed
        );
    }

    /// 10.00 over 3 units is 3.33333333 a unit, rounded to 8 places half-up; 20.00 over 3,
    /// 6.66666667.
    #[test]
    fn a_line_unit_price_has_eight_places() {
        assert_eq!(
            unit_price_of(1_000, 2, Decimal::from(3)).to_string(),
            "3.33333333"
        );
        assert_eq!(
            unit_price_of(2_000, 2, Decimal::from(3)).to_string(),
            "6.66666667"
        );
    }

    /// A pack's 50.00 charge granting 50.00, of which 20.00 is billed here: it grants 20.00.
    #[test]
    fn a_pack_billed_in_part_grants_its_share() {
        assert_eq!(grant_share(5_000, 2_000, 5_000), 2_000);
        assert_eq!(grant_share(5_000, 5_000, 5_000), 5_000);
        assert_eq!(grant_share(4_000, 1_000, 3_000), 1_333);
    }

    /// A free fee is shown once, at nothing, as its price says.
    #[test]
    fn a_charge_of_nothing_is_shown_once() {
        assert_eq!(
            decide(&expected(ChargeKind::Fee, 0), &[], None, false),
            Decision::Bill(0)
        );
        let shown = [line(day(2, 1), 0, 0)];
        assert_eq!(
            decide(&expected(ChargeKind::Fee, 0), &shown, None, false),
            Decision::Billed
        );
    }
}
