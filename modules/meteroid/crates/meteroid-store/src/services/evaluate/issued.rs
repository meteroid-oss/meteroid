//! What a subscription's issued invoices billed, line by line, as reconcile reads it (R-075,
//! R-080). A line attached to no fee is listed apart: reconcile never guesses its charge.

use std::collections::HashMap;

use billing_core::charge::{Denomination, Fee};
use billing_core::issued::{IssuedKind, IssuedLine};
use chrono::NaiveDate;
use common_domain::ids::{BaseId, InvoiceId, SubscriptionId, TenantId};
use diesel_models::invoices::InvoiceRow;
use error_stack::Report;

use crate::StoreResult;
use crate::domain::{Invoice, LineItem, LineItemKind};
use crate::errors::StoreError;
use crate::services::credit::scope::{LineOwners, line_subscription};
use crate::services::invoice_lines::charges::{Lineages, evaluated_charge_key};
use crate::services::invoice_lines::minimum::is_up_front;
use crate::services::subscriptions::amendment::load_credit_matching_context;
use crate::store::PgConn;

/// An issued line and the invoice it is on: two invoices may bill one charge's id (a correction
/// reissuing a credited invoice).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedOn {
    pub invoice: InvoiceId,
    pub line: IssuedLine,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Issued {
    pub lines: Vec<IssuedOn>,
    /// Lines naming no component or add-on: past the watermark they bill nothing.
    pub unattached: Vec<IssuedOn>,
}

/// The lines of the issued invoices of `subscription_id` settling periods from `from` on; on a
/// consolidated parent, only this subscription's.
pub async fn load_issued(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    from: NaiveDate,
) -> StoreResult<Issued> {
    let rows = InvoiceRow::list_issued_of_subscription(conn, tenant_id, subscription_id, from)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let mut issued = Issued::default();
    for row in rows {
        let mut invoice: Invoice = row.try_into()?;
        if invoice.subscription_id.is_none() {
            let owners = LineOwners::load_tx(conn, tenant_id, &invoice.line_items).await?;
            invoice
                .line_items
                .retain(|line| line_subscription(line, None, &owners) == Some(subscription_id));
        }
        let (line_lineage, already_credited, corrected, _) =
            load_credit_matching_context(conn, tenant_id, &invoice).await?;
        let lineages = Lineages::of_lines(&invoice.line_items, &line_lineage);
        for line in &invoice.line_items {
            let issued_line = issued_line(
                line,
                &invoice,
                subscription_id,
                &lineages,
                &line_lineage,
                &already_credited,
                &corrected,
            );
            let on = IssuedOn {
                invoice: invoice.id,
                line: issued_line,
            };
            if on.line.key.fee == Fee::Unattached {
                issued.unattached.push(on);
            } else {
                issued.lines.push(on);
            }
        }
    }
    Ok(issued)
}

/// A line as reconcile reads it. One priced in a credit unit bills the credits it consumed, in
/// that unit; the notes on it return money of its overage, never credits, so none count (R-039).
fn issued_line(
    line: &LineItem,
    invoice: &Invoice,
    subscription_id: SubscriptionId,
    lineages: &Lineages,
    line_lineage: &HashMap<String, String>,
    already_credited: &HashMap<String, i64>,
    corrected: &HashMap<String, i64>,
) -> IssuedLine {
    let mut key = evaluated_charge_key(line, lineages);
    key.subscription = key.subscription.or(Some(subscription_id.as_uuid()));
    let in_credits = key.denomination != Denomination::Money;
    let money = |of: &HashMap<String, i64>| {
        if in_credits {
            0
        } else {
            of.get(&line.local_id).copied().unwrap_or(0)
        }
    };
    IssuedLine {
        id: line.local_id.clone(),
        kind: match (&line.kind, line.metric_id) {
            (LineItemKind::Regular, None) => IssuedKind::Fee {
                price_component: line.price_component_id.map(|id| id.as_uuid()),
                add_on: line.sub_add_on_id.is_some(),
            },
            (LineItemKind::MinimumShortfall, _) => IssuedKind::Shortfall {
                up_front: is_up_front(line, invoice.billing_period_start),
            },
            _ => IssuedKind::Other,
        },
        lineage: line_lineage.get(&line.local_id).cloned(),
        quantity: line.quantity,
        unit_priced: match &line.credit_unit {
            Some(unit) => unit.unit_price.is_some(),
            None => line.unit_price.is_some(),
        },
        from: line.start_date,
        until: line.end_date,
        subtotal: match &line.credit_unit {
            Some(unit) => unit.consumed,
            None => line.amount_subtotal,
        },
        quoted: line
            .tax_inclusive_quote
            .as_ref()
            .filter(|_| !in_credits)
            .map(|quote| quote.amount_subtotal),
        already_credited: money(already_credited),
        corrected: money(corrected),
        settled: None,
        key,
    }
}
