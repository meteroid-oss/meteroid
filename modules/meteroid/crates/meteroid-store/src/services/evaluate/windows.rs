//! The usage windows a subscription's period invoice carries, read from its facts as billing it
//! evaluates them: at its date, on its slot (R-084, R-016, R-002).

use billing_core::evaluate::{DocumentSlot, usage_windows};
use billing_core::facts::{EndingKind, Facts};
use billing_core::pricer::UsageWindow;
use billing_core::proration::DayFraction;
use chrono::NaiveDate;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::enums::InvoiceStatusEnum;
use diesel_models::invoices::InvoiceRow;
use error_stack::Report;

use super::bill::{as_written, evaluated_at, slot_on};
use super::load::load_facts;
use super::pricer::{StoreFee, StorePricer};
use crate::StoreResult;
use crate::domain::BillableMetric;
use crate::domain::enums::InvoiceType;
use crate::errors::StoreError;
use crate::services::invoice_lines::fees::pricing_error;
use crate::services::invoice_lines::invoice_lines::BilledSpan;
use crate::store::PgConn;

/// A usage window an invoice carries: the fee row pricing it, the metric it reads, its days (an
/// ending's day alone when both are that day), the instants it reads and the share of a
/// capacity's allowance it holds.
#[derive(Clone, Debug)]
pub(crate) struct InvoiceWindow {
    pub fee: StoreFee,
    pub metric: BillableMetric,
    pub days: (NaiveDate, NaiveDate),
    pub read: UsageWindow,
    pub allowance: DayFraction,
}

/// The usage windows the invoice of `invoice_date` carries, its period's own or its final one
/// (`span`), as `billed_at` keys them. An invoice dated by the reconcile watermark was billed
/// before the facts: its periods read whole.
pub(crate) async fn invoice_windows(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    invoice_date: NaiveDate,
    span: BilledSpan,
) -> StoreResult<Vec<InvoiceWindow>> {
    let mut loaded = load_facts(conn, tenant_id, subscription_id, invoice_date, None).await?;
    if loaded
        .facts
        .reconcile_from
        .is_some_and(|watermark| invoice_date <= watermark)
    {
        loaded.facts.reconcile_from = None;
        loaded.facts.arrears_from = None;
    }
    let final_invoice = span == BilledSpan::Final
        || read_as_final(
            conn,
            tenant_id,
            subscription_id,
            &loaded.facts,
            invoice_date,
        )
        .await?;
    let invoice_type = if final_invoice {
        InvoiceType::Final
    } else {
        InvoiceType::Recurring
    };
    let now = evaluated_at(&loaded.facts, invoice_date);
    as_written(&mut loaded.facts, now);
    let windows = usage_windows(&loaded.facts, now, &StorePricer).map_err(pricing_error)?;
    let documents: Vec<DocumentSlot> = windows.iter().map(|window| window.document).collect();
    let slot = slot_on(conn, &loaded, &documents, invoice_date, &invoice_type).await?;
    let mut carried = Vec::new();
    for window in windows {
        if !slot.carries(window.document)
            || window.read.until > now
            || window.read.until <= window.read.from
        {
            continue;
        }
        let Some(metric_id) = window.row.fee.metric() else {
            continue;
        };
        let metric = loaded
            .metrics
            .iter()
            .find(|metric| metric.id == metric_id)
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(format!(
                    "metric with id {metric_id}"
                )))
            })?;
        carried.push(InvoiceWindow {
            fee: window.row.fee.clone(),
            metric: metric.clone(),
            days: window.days,
            read: window.read,
            allowance: window.allowance,
        });
    }
    Ok(carried)
}

/// Whether a period read on `on` is a final invoice's: `billed_at` reads the final invoice of a
/// backdated cancellation on the day it ended, as a period when that day's recurring invoice is
/// not live. The facts put what that ending bills on the final invoice (R-016).
async fn read_as_final<F>(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    facts: &Facts<F>,
    on: NaiveDate,
) -> StoreResult<bool> {
    let backdated_to_on = facts.ending.is_some_and(|ending| {
        ending.kind == EndingKind::Cancel
            && ending.on() == on
            && facts.ending_recorded.is_some_and(|recorded| recorded > on)
    });
    if !backdated_to_on {
        return Ok(false);
    }
    let live = Box::pin(InvoiceRow::find_existing_recurring_invoice(
        conn,
        tenant_id,
        subscription_id,
        on,
    ))
    .await
    .map_err(Into::<Report<StoreError>>::into)?
    .is_some_and(|invoice| {
        !matches!(
            invoice.status,
            InvoiceStatusEnum::Void | InvoiceStatusEnum::Closed
        )
    });
    Ok(!live)
}
