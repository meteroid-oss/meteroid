//! Each period invoice billed from the facts, checked against what the per-operation code would
//! have computed for it, and the state the facts derive against the stored row, in tests only
//! (feature `facts-shadow`): each difference not in [`SKIPS`] fails the test, as `timeline_shadow`
//! does.

use std::sync::Mutex;

use billing_core::evaluate::{DerivedState, Status};
use chrono::NaiveDate;
use common_domain::ids::{BaseId, SubscriptionId};
use diesel_models::invoices::InvoiceRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use crate::StoreResult;
use crate::domain::enums::{InvoiceType, SubscriptionStatusEnum};
use crate::domain::{Invoice, LineItem, Subscription, SubscriptionDetails};
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::evaluate::bill::{FromFacts, midnight};
use crate::services::invoice_lines::invoice_lines::PricedInvoice;
use crate::services::lifecycle::derived::stored_as;
use crate::store::PgConn;

static MISMATCHES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What the comparison found different since the last call.
pub fn take_mismatches() -> Vec<String> {
    MISMATCHES
        .lock()
        .map(|mut found| std::mem::take(&mut *found))
        .unwrap_or_default()
}

/// A known divergence: a difference of `class` whose text holds `contains`.
struct Skip {
    class: &'static str,
    contains: &'static str,
    reason: &'static str,
}

/// Known, explained divergences; each entry says in one line why it is expected.
const SKIPS: &[Skip] = &[
    Skip {
        class: "id",
        contains: "",
        reason: "R-075: a line of a closed row is keyed by its lineage's root; the legacy path keyed it by the row",
    },
    Skip {
        class: "watermark",
        contains: "",
        reason: "R-080: a period ending by the reconcile watermark was settled before; the legacy path, blind to it, bills it again",
    },
    Skip {
        class: "dated price",
        contains: "",
        reason: "R-097: a price succeeded inside the document's span bills each part at its own rates; the legacy path prices the span at today's",
    },
    Skip {
        class: "legacy",
        contains: "kept reading metering it had not read",
        reason: "the legacy path asks for usage the facts never read; under the pass's row locks it may read no more, so the invoice cannot be compared",
    },
    Skip {
        class: "late draft",
        contains: "",
        reason: "R-011: a draft finalized after later renewals; the legacy path drops a closed row's arrears once a later invoice billed it up to its end (retain_unsettled), its own period's too",
    },
    Skip {
        class: "legacy one-time",
        contains: "",
        reason: "R-075: a one-time fee bills once, on its day's document; the legacy path bills it again on another invoice of that day, an ending's",
    },
    Skip {
        class: "amendment cadence",
        contains: "",
        reason: "R-084: an amendment's cadence is no plan switch, its grid kept; the legacy path reads grids from plan switches alone",
    },
    Skip {
        class: "rider",
        contains: "",
        reason: "R-011: a row's arrears ended off its grid ride the next renewal, after the row of its fee that replaced it; the legacy path drops them",
    },
    Skip {
        class: "late rider",
        contains: "",
        reason: "R-011: arrears of a row ended off its grid bill on the renewal ending the period that holds them; the legacy path bills them one renewal later",
    },
    Skip {
        class: "name range",
        contains: "",
        reason: "a fee billed by several lines on one invoice is named with each line's dates; the other path billed it by one",
    },
    Skip {
        class: "term end",
        contains: "",
        reason: "R-084, R-002: a fixed term's end cuts its last period as a cancellation that day would, a capacity's allowance with its fee; the legacy path billed that period whole",
    },
    Skip {
        class: "capacity allowance",
        contains: "",
        reason: "R-002: a capacity's allowance is prorated with its fee where the fee is, its overage over the same days and rate higher; the legacy path counted the allowance whole",
    },
    Skip {
        class: "first partial capacity",
        contains: "",
        reason: "R-002: a capacity's fee is prorated in a first partial period, as any fixed fee is; the legacy path billed it whole",
    },
    Skip {
        class: "backdated",
        contains: "legacy only",
        reason: "R-077: a cancellation backdated into periods already billed takes their unused days back from their lines; the legacy path bills the days it kept again on the cancellation's own invoice",
    },
    Skip {
        class: "tiling",
        contains: "",
        reason: "R-079: a prorated part is what is left at its start less what is left at its end, each rounded once; the legacy path rounds the share once, a minor unit apart at a half",
    },
];

fn record(subscription: SubscriptionId, class: &'static str, detail: &str) {
    if let Some(skip) = SKIPS
        .iter()
        .find(|skip| skip.class == class && detail.contains(skip.contains))
    {
        log::debug!("facts shadow: skipped ({}): {detail}", skip.reason);
        if std::env::var_os("FACTS_SHADOW_SHOW_SKIPS").is_some()
            && let Ok(mut found) = MISMATCHES.lock()
        {
            found.push(format!("{subscription} [skipped {class}] {detail}"));
        }
        return;
    }
    let difference = format!("{subscription} [{class}] {detail}");
    log::warn!("facts shadow: {difference}");
    if let Ok(mut found) = MISMATCHES.lock() {
        found.push(difference);
    }
}

/// What the facts owe that a written document keeps unbilled (R-077): a difference of its own.
pub(crate) fn record_unbilled(subscription: SubscriptionId, detail: &str) {
    record(subscription, "unbilled", detail);
}

/// A period invoice billed from the facts, as the per-operation code is asked for it.
pub(crate) struct Compared<'a> {
    pub invoice_date: NaiveDate,
    pub details: &'a SubscriptionDetails,
    pub prepaid_amount: Option<u64>,
    pub invoice: Option<&'a Invoice>,
    pub invoice_type: InvoiceType,
}

type Differences = Vec<(&'static str, String)>;

impl Services {
    /// Checks `billed` against what `price_invoice_lines` computes for the same invoice, reading
    /// in a savepoint the metering the facts read; reports and never fails.
    pub(crate) async fn compare_with_legacy(
        &self,
        conn: &mut PgConn,
        compared: Compared<'_>,
        billed: &FromFacts,
    ) {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(self.compare_with_legacy_unboxed(conn, compared, billed)).await;
    }

    async fn compare_with_legacy_unboxed(
        &self,
        conn: &mut PgConn,
        compared: Compared<'_>,
        billed: &FromFacts,
    ) {
        let subscription = compared.details.subscription.id;
        let details = compared.details;
        // A served pass answers only what it read: the legacy reads go to what the facts read.
        let legacy = match &self.served {
            Some(served) => match served.peek() {
                Some(peek) => self.reading_usage(peek),
                None => return,
            },
            None => self.reading_usage(self.usage_client.clone()),
        };
        let checked = self
            .store
            .transaction_with(conn, |conn| {
                async move { compare(&legacy, conn, &compared, billed).await }.scope_boxed()
            })
            .await;
        match checked {
            Ok(differences) => {
                for (class, detail) in differences {
                    record(subscription, class, &detail);
                }
            }
            // Blind to the watermark, the legacy path reads the usage of periods it settled.
            Err(error) if billed.watermark.is_some() => {
                record(subscription, "watermark", &format!("{error:?}"))
            }
            Err(error) => {
                // Reading a period whole, the legacy path asks for metering the parts never read.
                let dated = match span_of(&billed.priced) {
                    Some(span) => succeeded_within(conn, details, span)
                        .await
                        .is_ok_and(|lineages| !lineages.is_empty()),
                    None => false,
                };
                let class = if dated { "dated price" } else { "legacy" };
                record(subscription, class, &format!("{error:?}"))
            }
        }
    }
}

async fn compare(
    legacy: &Services,
    conn: &mut PgConn,
    compared: &Compared<'_>,
    billed: &FromFacts,
) -> StoreResult<Differences> {
    // The termination instant and the ending fact are stored apart, a microsecond apart at most:
    // the legacy path reads the ending's.
    let mut details = compared.details.clone();
    if let (Some(at), Some(ending)) = (details.subscription.terminated_at, billed.ending)
        && (at.and_utc() - ending).abs() <= chrono::Duration::microseconds(1)
    {
        details.subscription.terminated_at = Some(ending.naive_utc());
    }
    let computed = legacy
        .price_invoice_lines(
            conn,
            &compared.invoice_date,
            &details,
            compared.prepaid_amount,
            compared.invoice,
            compared.invoice_type.clone(),
            None,
        )
        .await?;
    let slot = format!(
        "{:?} {}{}",
        compared.invoice_type,
        compared.invoice_date,
        compared
            .invoice
            .map(|invoice| format!(" (draft {})", invoice.id))
            .unwrap_or_default()
    );
    let (mut differences, rows) = document_differences(&slot, &computed, &billed.priced);
    if let Some(watermark) = billed.watermark {
        for difference in &mut differences {
            if difference.0 == "line"
                && difference.1.contains("legacy only")
                && difference.1.contains(&format!("..{watermark} "))
            {
                difference.0 = "watermark";
            }
        }
    }

    // After an amendment moved the cadence (a window following one of its plan version), the
    // legacy path still reads its grids from plan switches alone.
    if differences.iter().any(|(class, _)| *class == "line") {
        let windows =
            diesel_models::subscription_facts::SubscriptionPlanWindowRow::list_by_subscription(
                conn,
                compared.details.subscription.id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let amended = windows.windows(2).any(|pair| {
            pair[0].plan_version_id == pair[1].plan_version_id
                && format!("{:?}", pair[0].cadence) != format!("{:?}", pair[1].cadence)
        });
        if amended {
            for difference in &mut differences {
                if difference.0 == "line" {
                    difference.0 = "amendment cadence";
                }
            }
        }
    }

    // A draft finalized after later renewals: the legacy path reads its closed rows as settled.
    if compared.invoice.is_some() && differences.iter().any(|(class, _)| *class == "line") {
        let sub = &compared.details.subscription;
        let (later, _) = InvoiceRow::recurring_period_starts_around(
            conn,
            sub.tenant_id,
            sub.id,
            compared.invoice_date + chrono::Days::new(1),
            compared.invoice_date,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        if !later.is_empty() {
            for difference in &mut differences {
                if difference.0 == "line" {
                    difference.0 = "late draft";
                }
            }
        }
    }

    // Only the lines of a lineage whose price a successor dates inside the span are skipped.
    if differences.iter().any(|(class, _)| *class == "line")
        && let Some(span) = span_of(&billed.priced)
    {
        let dated = succeeded_within(conn, compared.details, span).await?;
        let root = lineage_roots(conn, compared.details).await?;
        for (difference, rows) in differences.iter_mut().zip(&rows) {
            if difference.0 == "line"
                && rows
                    .iter()
                    .any(|row| root.get(row).is_some_and(|root| dated.contains(root)))
            {
                difference.0 = "dated price";
            }
        }
    }

    // A line both paths bill, ending on the fixed term's end: the legacy path billed it whole.
    if let Some(end) = compared.details.subscription.end_date {
        let ends_there = format!("..{end} ");
        for difference in &mut differences {
            if difference.0 == "line"
                && !difference.1.contains(" only: ")
                && difference.1.contains(&ends_there)
            {
                difference.0 = "term end";
            }
        }
    }

    let details = compared.details;
    let clock = common_clock::now();
    if midnight(compared.invoice_date) <= clock {
        let stored: Subscription = SubscriptionRow::get_subscription_by_id(
            conn,
            &details.subscription.tenant_id,
            details.subscription.id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .try_into()?;
        // The invoice of a cancellation backdated into periods already billed: their lines give
        // back its unused days, the legacy path bills those days again on it.
        if compared.invoice_date < clock.date_naive()
            && stored.status == SubscriptionStatusEnum::Cancelled
            && stored.current_period_start == compared.invoice_date
        {
            for difference in differences.iter_mut() {
                if difference.0 == "line" && difference.1.contains("legacy only") {
                    difference.0 = "backdated";
                }
            }
        }
        differences.extend(state_differences(
            &stored,
            &billed.state,
            clock.date_naive(),
        ));
        if let Some(difference) = mrr_difference(legacy, conn, &stored, clock.date_naive()).await? {
            differences.push(("mrr", difference));
        }
    }
    Ok(differences)
}

/// The stored MRR against the MRR of the fees in force in the stored period, once the row has
/// caught up and its creation's New Business is booked (the writer's own guard).
async fn mrr_difference(
    services: &Services,
    conn: &mut PgConn,
    stored: &Subscription,
    today: NaiveDate,
) -> StoreResult<Option<String>> {
    use diesel_models::enums::SubscriptionEventType;
    use diesel_models::subscription_events::SubscriptionEventRow;

    let Some(end) = stored
        .current_period_end
        .filter(|end| *end > today && stored.status == SubscriptionStatusEnum::Active)
    else {
        return Ok(None);
    };
    let created = SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
        conn,
        stored.id,
        SubscriptionEventType::Created,
        stored.start_date,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    if created.is_some_and(|event| {
        event.mrr_delta.is_some_and(|delta| delta != 0) && event.bi_mrr_movement_log_id.is_none()
    }) {
        return Ok(None);
    }
    let day = end.pred_opt().map_or(today, |last| today.min(last));
    let details = services
        .store
        .subscription_details_as_of(conn, stored.tenant_id, stored.id, day)
        .await?;
    let precision =
        crate::constants::Currencies::resolve_currency_precision(&details.subscription.currency)
            .unwrap_or(2);
    let lines = crate::services::subscriptions::plan_change::calculate_components_mrr_with_slots(
        conn,
        stored.tenant_id,
        stored.id,
        &details.price_components,
        precision,
    )
    .await?;
    let lines =
        crate::services::subscriptions::utils::with_add_on_lines(&details, lines, precision);
    let restated = crate::services::subscriptions::utils::subscription_mrr(
        &lines,
        details
            .minimum
            .as_ref()
            .map(|minimum| (minimum.amount_subunit, &minimum.scope)),
        details.subscription.period,
        details.subscription.tax_inclusive_rate,
    );
    let stored_mrr = stored.mrr_cents as i64;
    Ok((restated != stored_mrr).then(|| format!("restated {restated} stored {stored_mrr}")))
}

/// The days a document's lines bill, `[first start, last end)`.
fn span_of(priced: &PricedInvoice) -> Option<(NaiveDate, NaiveDate)> {
    let (_, lines) = lines_of(priced);
    let from = lines.iter().map(|line| line.start_date).min()?;
    let until = lines.iter().map(|line| line.end_date).max()?;
    Some((from, until))
}

/// Whether a price the subscription's fee rows name, closed ones too, has a successor taking
/// over inside `span`.
/// Each fee row of the subscription, component or add-on, by id: its lineage's root.
async fn lineage_roots(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
) -> StoreResult<std::collections::HashMap<uuid::Uuid, uuid::Uuid>> {
    let subscription = details.subscription.id;
    let components =
        diesel_models::subscription_components::SubscriptionComponentRow::list_fact_rows(
            conn,
            subscription,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let add_ons = diesel_models::subscription_add_ons::SubscriptionAddOnRow::list_fact_rows(
        conn,
        subscription,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    Ok(components
        .iter()
        .map(|c| (c.id.as_uuid(), c.lineage_id.unwrap_or(c.id).as_uuid()))
        .chain(
            add_ons
                .iter()
                .map(|a| (a.id.as_uuid(), a.lineage_id.unwrap_or(a.id).as_uuid())),
        )
        .collect())
}

/// The lineages, by root, whose price a dated successor changes strictly inside `(from, until)`.
async fn succeeded_within(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    (from, until): (NaiveDate, NaiveDate),
) -> StoreResult<std::collections::HashSet<uuid::Uuid>> {
    let subscription = details.subscription.id;
    let components =
        diesel_models::subscription_components::SubscriptionComponentRow::list_fact_rows(
            conn,
            subscription,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let add_ons = diesel_models::subscription_add_ons::SubscriptionAddOnRow::list_fact_rows(
        conn,
        subscription,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    let priced: Vec<(common_domain::ids::PriceId, uuid::Uuid)> = components
        .iter()
        .filter_map(|c| Some((c.price_id?, c.lineage_id.unwrap_or(c.id).as_uuid())))
        .chain(
            add_ons
                .iter()
                .filter_map(|a| Some((a.price_id?, a.lineage_id.unwrap_or(a.id).as_uuid()))),
        )
        .collect();
    let roots: Vec<_> = priced.iter().map(|(price, _)| *price).collect();
    let succeeded: Vec<_> = diesel_models::prices::PriceRow::successors_of(
        conn,
        &roots,
        details.subscription.tenant_id,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?
    .into_iter()
    .filter(|row| row.effective_from.is_some_and(|at| from < at && at < until))
    .filter_map(|row| row.supersedes)
    .collect();
    Ok(priced
        .into_iter()
        .filter(|(price, _)| succeeded.contains(price))
        .map(|(_, lineage)| lineage)
        .collect())
}

/// The lines a document holds, each as its stored JSON: every field it is written with.
fn lines_of(priced: &PricedInvoice) -> (&'static str, &[LineItem]) {
    match priced {
        PricedInvoice::Stored(content) => ("stored", &content.invoice_lines),
        PricedInvoice::Lines(priced) => ("priced", &priced.lines),
    }
}

/// A line's name without the date ranges an invoice billing its fee by several lines adds.
fn base_name(name: &str) -> &str {
    let mut name = name;
    while let Some(open) = name.rfind(" (")
        && name.ends_with(')')
        && name[open + 2..name.len() - 1]
            .split(" - ")
            .all(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok())
    {
        name = &name[..open];
    }
    name
}

/// The documents' differences, each with the fee rows of the lines it compares.
fn document_differences(
    slot: &str,
    legacy: &PricedInvoice,
    core: &PricedInvoice,
) -> (Differences, Vec<Vec<uuid::Uuid>>) {
    let mut differences = Differences::new();
    let mut rows: Vec<Vec<uuid::Uuid>> = Vec::new();
    // Both keep the written lines; only the credit they would draw is estimated again.
    if let (PricedInvoice::Stored(_), PricedInvoice::Stored(_)) = (legacy, core) {
        return (differences, rows);
    }
    let rows_of = |lines: &[&LineItem]| -> Vec<uuid::Uuid> {
        lines
            .iter()
            .flat_map(|line| {
                line.sub_component_id
                    .map(|id| id.as_uuid())
                    .into_iter()
                    .chain(line.sub_add_on_id.map(|id| id.as_uuid()))
            })
            .collect()
    };
    let ((legacy_kind, legacy_lines), (core_kind, core_lines)) = (lines_of(legacy), lines_of(core));
    if legacy_kind != core_kind {
        differences.push((
            "document",
            format!("{slot}: legacy {legacy_kind} lines, facts {core_kind} lines"),
        ));
        rows.push(Vec::new());
    }
    if let (PricedInvoice::Lines(a), PricedInvoice::Lines(b)) = (legacy, core)
        && a.true_up != b.true_up
    {
        differences.push((
            "document",
            format!(
                "{slot}: true-up legacy {:?} facts {:?}",
                a.true_up, b.true_up
            ),
        ));
        rows.push(Vec::new());
    }
    let describe = |line: &LineItem| {
        format!(
            "{} {}..{} qty {:?} x {:?} = {}",
            line.name,
            line.start_date,
            line.end_date,
            line.quantity,
            line.unit_price,
            line.amount_subtotal
        )
    };
    // A one-time fee the legacy path bills again, its own document having billed it.
    let once = |line: &LineItem| {
        line.charge_type == Some(crate::domain::invoice_lines::ChargeType::OneTime)
            && !core_lines.iter().any(|core| {
                (&core.name, core.start_date, core.end_date)
                    == (&line.name, line.start_date, line.end_date)
            })
    };
    for line in legacy_lines.iter().filter(|line| once(line)) {
        differences.push((
            "legacy one-time",
            format!("{slot}: legacy only: {}", describe(line)),
        ));
        rows.push(rows_of(&[line]));
    }
    let legacy_lines: Vec<&LineItem> = legacy_lines.iter().filter(|line| !once(line)).collect();
    // Rows of one fee in turn over a period: the legacy path bills each row's part on a line of
    // its own, its share rounded once; the facts one line, each part a difference of what is left.
    let parts = |line: &LineItem| -> Vec<&LineItem> {
        let mut parts: Vec<&LineItem> = legacy_lines
            .iter()
            .copied()
            .filter(|legacy| {
                base_name(&legacy.name) == base_name(&line.name)
                    && line.start_date <= legacy.start_date
                    && legacy.end_date <= line.end_date
            })
            .collect();
        parts.sort_by_key(|part| part.start_date);
        let tiled = parts.len() > 1
            && parts
                .first()
                .is_some_and(|p| p.start_date == line.start_date)
            && parts.last().is_some_and(|p| p.end_date == line.end_date)
            && parts.windows(2).all(|w| w[0].end_date == w[1].start_date);
        let sum: i64 = parts.iter().map(|part| part.amount_subtotal).sum();
        let apart = (sum - line.amount_subtotal).unsigned_abs();
        if tiled && apart < parts.len() as u64 {
            parts
        } else {
            Vec::new()
        }
    };
    let mut split: Vec<&LineItem> = Vec::new();
    for line in core_lines.iter() {
        let parts = parts(line);
        if parts.is_empty() {
            continue;
        }
        differences.push((
            "tiling",
            format!(
                "{slot}: {} billed by {} legacy lines",
                describe(line),
                parts.len()
            ),
        ));
        let mut shown = parts.clone();
        shown.push(line);
        rows.push(rows_of(&shown));
        split.extend(parts);
    }
    let core_lines: Vec<&LineItem> = core_lines
        .iter()
        .filter(|line| parts(line).is_empty())
        .collect();
    let legacy_lines: Vec<&LineItem> = legacy_lines
        .iter()
        .copied()
        .filter(|line| !split.iter().any(|part| std::ptr::eq(*part, *line)))
        .collect();
    // Arrears of a row ended off its grid riding the first renewal from its end (R-011), after
    // the invoice's own periods began, which the legacy path never bills.
    let rider = |line: &LineItem| {
        !legacy_lines.iter().any(|legacy| {
            (base_name(&legacy.name), legacy.start_date, legacy.end_date)
                == (base_name(&line.name), line.start_date, line.end_date)
        }) && core_lines
            .iter()
            .any(|other| other.start_date >= line.end_date && other.end_date > line.end_date)
    };
    for line in core_lines.iter().filter(|line| rider(line)) {
        differences.push(("rider", format!("{slot}: facts only: {}", describe(line))));
        rows.push(rows_of(&[line]));
    }
    let core_lines: Vec<&LineItem> = core_lines
        .iter()
        .copied()
        .filter(|line| !rider(line))
        .collect();
    // A legacy line of days before every fee period the facts put on this document, which no
    // facts line of the document bills: arrears the facts billed on an earlier renewal.
    let first = core_lines
        .iter()
        .filter(|line| line.charge_type != Some(crate::domain::invoice_lines::ChargeType::Usage))
        .map(|line| line.start_date)
        .min();
    let late = |line: &LineItem| {
        first.is_some_and(|first| line.end_date <= first)
            && !core_lines.iter().any(|core| {
                (base_name(&core.name), core.start_date, core.end_date)
                    == (base_name(&line.name), line.start_date, line.end_date)
            })
    };
    for line in legacy_lines.iter().filter(|line| late(line)) {
        differences.push((
            "late rider",
            format!("{slot}: legacy only: {}", describe(line)),
        ));
        rows.push(rows_of(&[line]));
    }
    let legacy_lines: Vec<&LineItem> = legacy_lines
        .iter()
        .copied()
        .filter(|line| !late(line))
        .collect();
    for i in 0..legacy_lines.len().max(core_lines.len()) {
        match (legacy_lines.get(i).copied(), core_lines.get(i).copied()) {
            (Some(a), Some(b)) => {
                let (va, vb) = (
                    serde_json::to_value(a).unwrap_or_default(),
                    serde_json::to_value(b).unwrap_or_default(),
                );
                if va == vb {
                    continue;
                }
                let mut fields: Vec<String> = match (&va, &vb) {
                    (serde_json::Value::Object(ma), serde_json::Value::Object(mb)) => ma
                        .keys()
                        .chain(mb.keys())
                        .filter(|k| ma.get(*k) != mb.get(*k))
                        .map(|k| {
                            format!(
                                "{k}: legacy {} facts {}",
                                ma.get(k).map(ToString::to_string).unwrap_or_default(),
                                mb.get(k).map(ToString::to_string).unwrap_or_default()
                            )
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                fields.sort();
                fields.dedup();
                let unkeyed = |line: &LineItem| {
                    let mut line = line.clone();
                    line.local_id.clear();
                    for sub_line in &mut line.sub_lines {
                        sub_line.local_id.clear();
                    }
                    serde_json::to_value(line).unwrap_or_default()
                };
                // The same prorated line but for a minor unit of its amounts.
                let tiling =
                    a.is_prorated && (a.amount_subtotal - b.amount_subtotal).abs() == 1 && {
                        let mut priced = a.clone();
                        priced.amount_subtotal = b.amount_subtotal;
                        priced.taxable_amount = b.taxable_amount;
                        priced.tax_amount = b.tax_amount;
                        priced.amount_total = b.amount_total;
                        priced.unit_price = b.unit_price;
                        priced.tax_details = b.tax_details.clone();
                        unkeyed(&priced) == unkeyed(b)
                    };
                // A capacity's overage over the same days and rate, its allowance prorated by the
                // facts with its fee where the legacy path counted it whole.
                let overage = |line: &LineItem| match line.sub_lines.as_slice() {
                    [only] if only.name == "Overage" => Some(only.unit_price),
                    _ => None,
                };
                let allowance = a.metric_id.is_some()
                    && (a.start_date, a.end_date, a.metric_id)
                        == (b.start_date, b.end_date, b.metric_id)
                    && overage(a).is_some()
                    && overage(a) == overage(b)
                    && b.amount_subtotal > a.amount_subtotal;
                // A fixed fee the legacy path billed whole over the same days, the facts prorated.
                let partial_capacity = a.metric_id.is_none()
                    && b.metric_id.is_none()
                    && (a.start_date, a.end_date, a.sub_component_id, a.quantity)
                        == (b.start_date, b.end_date, b.sub_component_id, b.quantity)
                    && !a.is_prorated
                    && b.is_prorated
                    && (0..a.amount_subtotal).contains(&b.amount_subtotal);
                differences.push((
                    if unkeyed(a) == unkeyed(b) {
                        "id"
                    } else if allowance {
                        "capacity allowance"
                    } else if partial_capacity {
                        "first partial capacity"
                    } else if fields
                        .iter()
                        .all(|f| f.starts_with("name:") || f.starts_with("local_id:"))
                        && base_name(&a.name) == base_name(&b.name)
                    {
                        "name range"
                    } else if tiling {
                        "tiling"
                    } else {
                        "line"
                    },
                    format!("{slot}: line {i} {}: {}", describe(a), fields.join("; ")),
                ));
                rows.push(rows_of(&[a, b]));
            }
            (Some(a), None) => {
                differences.push(("line", format!("{slot}: legacy only: {}", describe(a))));
                rows.push(rows_of(&[a]));
            }
            (None, Some(b)) => {
                differences.push(("line", format!("{slot}: facts only: {}", describe(b))));
                rows.push(rows_of(&[b]));
            }
            (None, None) => {}
        }
    }
    (differences, rows)
}

/// The derived status, current period and plan version against the stored row, once the row
/// has caught up with `today` (a renewal still to run leaves it a period behind).
fn state_differences(stored: &Subscription, state: &DerivedState, today: NaiveDate) -> Differences {
    let mut differences = Differences::new();
    // A cancellation backdated before the stored period is settled before its row is patched:
    // the facts end it already, the row still runs the period it gives back.
    let backdated = state.status == Status::Cancelled
        && state
            .current_period
            .is_some_and(|(from, to)| to.is_none() && from < stored.current_period_start);
    if stored.current_period_end.is_some_and(|end| end <= today)
        || stored.status == SubscriptionStatusEnum::Paused
        || backdated
    {
        return differences;
    }
    if let Some(status) = stored_as(&stored.status)
        && status != state.status
    {
        differences.push((
            "status",
            format!("derived {:?} stored {:?}", state.status, stored.status),
        ));
    }
    let period = Some((stored.current_period_start, stored.current_period_end));
    if state.current_period != period {
        differences.push((
            "current_period",
            format!("derived {:?} stored {period:?}", state.current_period),
        ));
    }
    if state.plan_version != Some(stored.plan_version_id) {
        differences.push((
            "plan_version",
            format!(
                "derived {:?} stored {}",
                state.plan_version, stored.plan_version_id
            ),
        ));
    }
    differences
}
