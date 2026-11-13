//! Funding the credit an invoice's `Credit` lines sold: purchased credit when the invoice is paid,
//! included credit when it is issued. Each line funds its grant exactly once.
use crate::StoreResult;
use crate::domain::credit::{CreditCost, GrantClassification, GrantScope};
use crate::domain::price_components::CreditExpiry;
use crate::domain::{Invoice, LineItem, LineItemKind};
use crate::errors::StoreError;
use crate::hooks::CreditControlHooks;
use crate::repositories::credit::cost::purchased_cost;
use crate::repositories::credit::customer_credit::{
    self as credit, CreditFunding, DEFAULT_PRIORITY, source,
};
use crate::repositories::credit_notes::cumulative_grant_reversal;
use crate::services::Services;
use crate::services::credit::scope::{LineOwners, line_subscription};
use crate::services::credit::{invalid, precondition};
use crate::store::PgConn;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use common_domain::ids::{BaseId, CreditNoteId, InvoiceId, TenantId};
use diesel_models::credit_notes::CreditNoteRow;
use diesel_models::customer_credit::{
    CreditPurchaseRow, CustomerCreditGrantRow, CustomerCreditOperationRow,
};
use diesel_models::enums::{CreditNoteStatus, InvoicePaymentStatus};
use diesel_models::invoices::InvoiceRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscription_events::SubscriptionEventRow;
use error_stack::Report;
use std::collections::HashMap;
use uuid::Uuid;

impl Services {
    /// The choke point for marking an invoice Paid on a payment: the status and the credit its
    /// purchase lines sold commit together. Debt forgiveness (a credit note zeroing the balance)
    /// is not a payment and never comes here; it funds only the paid share itself.
    pub(crate) async fn apply_invoice_paid_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        paid_at: Option<NaiveDateTime>,
    ) -> StoreResult<Invoice> {
        self.apply_invoice_paid_at_tx(
            conn,
            tenant_id,
            invoice_id,
            paid_at,
            common_domain::clock::now(),
        )
        .await
    }

    /// `now` is the present the funded balance is judged at: the clock, or a replay's moment.
    pub(crate) async fn apply_invoice_paid_at_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        paid_at: Option<NaiveDateTime>,
        now: DateTime<Utc>,
    ) -> StoreResult<Invoice> {
        let row = InvoiceRow::apply_payment_status(
            conn,
            invoice_id,
            tenant_id,
            InvoicePaymentStatus::Paid,
            paid_at,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let invoice: Invoice = row.try_into()?;
        // Funded when the money landed: the stored payment time, which the status update set.
        let paid_at = invoice
            .paid_at
            .map(|at| at.and_utc())
            .ok_or_else(|| precondition("a paid invoice has no payment time"))?;
        mint_purchased_credit_tx(conn, &*self.store.hooks.credit, &invoice, paid_at, now).await?;
        self.credit_back_late_period_packs_tx(
            conn,
            &invoice,
            funding_time(&invoice, paid_at, now),
            None,
        )
        .await?;
        Ok(invoice)
    }
}

/// When credit an invoice sold is funded: when the money landed, not before the invoice was
/// issued, never later than `now`.
pub(crate) fn funding_time(
    invoice: &Invoice,
    paid_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    invoice
        .finalized_at
        .map_or(paid_at, |issued| paid_at.max(issued.and_utc()))
        .min(now)
}

fn purchased_lines(invoice: &Invoice) -> impl Iterator<Item = (&LineItem, i64)> {
    invoice.line_items.iter().filter_map(|line| {
        (line.kind == LineItemKind::Credit && line.amount_total >= 0)
            .then_some(line.credit_grant_amount)
            .flatten()
            .filter(|amount| *amount > 0)
            .map(|amount| (line, amount))
    })
}

/// The wallet a `Credit` line funds: its credit unit's, or the invoice currency's. The cost is
/// always in the invoice currency.
fn granted_denomination<'a>(invoice: &'a Invoice, line: &'a LineItem) -> &'a str {
    line.credit_grant_unit
        .as_ref()
        .map_or(invoice.currency.as_str(), |unit| unit.code.as_str())
}

/// A relative term runs from payment; credit bound to the period ends with the period it was
/// bought for, however late it is paid.
fn purchased_expiry(
    expiry: CreditExpiry,
    funded_at: DateTime<Utc>,
    period: (NaiveDate, NaiveDate),
) -> Option<DateTime<Utc>> {
    match expiry {
        CreditExpiry::BillingPeriod => Some(midnight(period.1)),
        other => other.resolve(funded_at),
    }
}

/// When credit bound to its period becomes spendable: when the money lands or the period opens,
/// whichever is later. Paid after the period ended, it is dated back to the period's start: it
/// still pays that period's usage (settled at its expiry), never a later period's.
fn period_credit_effective_at(
    funded_at: DateTime<Utc>,
    period: (NaiveDate, NaiveDate),
) -> DateTime<Utc> {
    let (start, end) = (midnight(period.0), midnight(period.1));
    let from = funded_at.max(start);
    if from < end { from } else { start }
}

fn midnight(day: NaiveDate) -> DateTime<Utc> {
    day.and_time(chrono::NaiveTime::MIN).and_utc()
}

/// The period a pack line's credit is bound to: the line's, ended early by an immediate plan
/// change inside it, which removed that credit.
pub(crate) async fn credit_period_tx(
    conn: &mut PgConn,
    invoice: &Invoice,
    line: &LineItem,
) -> StoreResult<(NaiveDate, NaiveDate)> {
    let period = (line.start_date, line.end_date);
    if line.credit_grant_expiry != Some(CreditExpiry::BillingPeriod) {
        return Ok(period);
    }
    // A consolidated invoice carries no subscription: its line's component names it.
    let subscription_id = match (invoice.subscription_id, line.sub_component_id) {
        (Some(subscription_id), _) => Some(subscription_id),
        (None, Some(component)) => {
            SubscriptionComponentRow::subscription_of(conn, component).await?
        }
        (None, None) => None,
    };
    let Some(subscription_id) = subscription_id else {
        return Ok(period);
    };
    let Some(component) = line.sub_component_id else {
        return Ok(period);
    };
    let root = SubscriptionComponentRow::find_lineage_by_ids(conn, &[component])
        .await?
        .into_iter()
        .find_map(|(_, root)| root)
        .unwrap_or(component);
    // The pack's lineage (carried over unchanged or not) ended inside its period, on its first
    // day included, by an immediate plan change: its credit ends then.
    let Some(ended) = SubscriptionComponentRow::lineage_ended_on(conn, root)
        .await?
        .filter(|ended| period.0 <= *ended && *ended < period.1)
    else {
        return Ok(period);
    };
    if SubscriptionEventRow::plan_switched_on(conn, subscription_id, ended).await? {
        return Ok((period.0, ended));
    }
    Ok(period)
}

/// When a plan pack's grant ended before its expiry: the immediate plan change that removed its
/// pack, read from the line that funded it (`ends_at` records only the subscription's end).
pub(crate) async fn grant_ended_at_tx(
    conn: &mut PgConn,
    grant: &CustomerCreditGrantRow,
) -> StoreResult<Option<DateTime<Utc>>> {
    let (Some(invoice_id), Some(line_id)) = (grant.invoice_id, grant.line_local_id.as_deref())
    else {
        return Ok(None);
    };
    let invoice: Invoice = InvoiceRow::find_by_id(conn, grant.tenant_id, invoice_id)
        .await?
        .try_into()?;
    let Some(line) = invoice
        .line_items
        .iter()
        .find(|line| line.local_id == line_id && line.kind == LineItemKind::Credit)
    else {
        return Ok(None);
    };
    let (_, end) = credit_period_tx(conn, &invoice, line).await?;
    Ok((end < line.end_date).then(|| midnight(end)))
}

/// A pack line's credit pays only its subscription's usage; there is no customer-wide fallback.
fn pack_scope(invoice: &Invoice, line: &LineItem, owners: &LineOwners) -> StoreResult<GrantScope> {
    line_subscription(line, invoice.subscription_id, owners)
        .map(GrantScope::subscription_usage)
        .ok_or_else(|| precondition("a credit pack line names no subscription"))
}

/// A plan's pack line names its subscription, component or add-on; credit sold or topped up
/// outside a plan names none and is the customer's to spend on any charge.
fn purchased_scope(
    invoice: &Invoice,
    line: &LineItem,
    owners: &LineOwners,
) -> StoreResult<GrantScope> {
    let sold_by_a_plan = line.subscription_id.is_some()
        || line.sub_component_id.is_some()
        || line.sub_add_on_id.is_some();
    if sold_by_a_plan {
        pack_scope(invoice, line, owners)
    } else {
        Ok(GrantScope::customer_wide())
    }
}

/// Subtotal finalized credit notes (but `except`) took off each line of the invoice, by line.
pub(crate) async fn credited_by_finalized_notes_tx(
    conn: &mut PgConn,
    invoice: &Invoice,
    except: Option<CreditNoteId>,
) -> StoreResult<HashMap<String, i64>> {
    let mut credited = HashMap::new();
    for note in CreditNoteRow::list_by_invoice_id(conn, invoice.tenant_id, invoice.id).await? {
        if note.status != CreditNoteStatus::Finalized || Some(note.id) == except {
            continue;
        }
        let lines: Vec<LineItem> = serde_json::from_value(note.line_items)
            .map_err(|error| StoreError::SerdeError("credit-note lines".into(), error))?;
        for line in lines {
            let slot = credited.entry(line.local_id).or_insert(0_i64);
            *slot = line
                .amount_subtotal
                .checked_abs()
                .and_then(|amount| slot.checked_add(amount))
                .ok_or_else(|| invalid("credited subtotal exceeds the supported range"))?;
        }
    }
    Ok(credited)
}

/// What a purchase line of `face` funds once `credited_subtotal` of it was credited by finalized
/// notes, rounded as their reversals are.
fn face_net_of_notes(line: &LineItem, face: i64, credited_subtotal: i64) -> StoreResult<i64> {
    let cancelled = match credited_subtotal {
        0 => 0,
        subtotal => cumulative_grant_reversal(face, subtotal, line.amount_subtotal)
            .ok_or_else(|| invalid("the credit notes of this purchase line cannot be prorated"))?,
    };
    face.checked_sub(cancelled)
        .ok_or_else(|| invalid("purchased credit overflow"))
}

/// What `mint_purchased_credit_tx` funds for the paid purchase lines of `invoice`, and their face:
/// a CreditToBalance note issued while the payment was in flight already returned its share.
pub(crate) async fn purchased_funding_tx(
    conn: &mut PgConn,
    invoice: &Invoice,
) -> StoreResult<(i64, i64)> {
    let credited = credited_by_finalized_notes_tx(conn, invoice, None).await?;
    let mut bought = 0_i64;
    let mut funded = 0_i64;
    for (line, face) in purchased_lines(invoice).filter(|(line, _)| !line.credit_grant_included) {
        let credited_subtotal = credited.get(&line.local_id).copied().unwrap_or(0);
        let net = face_net_of_notes(line, face, credited_subtotal)?.max(0);
        bought = bought
            .checked_add(face)
            .ok_or_else(|| invalid("purchased credit overflow"))?;
        funded = funded
            .checked_add(net)
            .ok_or_else(|| invalid("purchased credit overflow"))?;
    }
    Ok((bought, funded))
}

/// Funds the paid `Credit` lines of a Paid invoice, net of what finalized credit notes cancelled
/// (rounded as their reversals are), once per line. A credit purchase (`sell_customer_credit`)
/// funds under its purchase row; a pack line funds under its invoice line. The credit is spendable,
/// and a relative term runs, from when the money landed (`paid_at`); money received before the
/// invoice was issued funds from the issue, and nothing funds later than `now`.
pub(crate) async fn mint_purchased_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    invoice: &Invoice,
    paid_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> StoreResult<()> {
    let funded_at = funding_time(invoice, paid_at, now);
    let lines = purchased_lines(invoice)
        .filter(|(line, _)| !line.credit_grant_included)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return Ok(());
    }
    let mut purchase =
        CreditPurchaseRow::find_by_invoice(conn, invoice.tenant_id, invoice.id).await?;
    if purchase.is_none()
        && let Some(parent) = invoice.parent_invoice_id
    {
        purchase = CreditPurchaseRow::find_by_invoice(conn, invoice.tenant_id, parent).await?;
    }
    if purchase.is_some() && lines.len() > 1 {
        return Err(precondition(
            "a credit purchase invoice carries a single credit line",
        ));
    }
    let credited = credited_by_finalized_notes_tx(conn, invoice, None).await?;
    let owners = LineOwners::load_tx(conn, invoice.tenant_id, &invoice.line_items).await?;
    // An automatic top-up pays from when it was queued: the usage that brought the balance down
    // is that credit's, even when the payment lands after the period or a retry.
    // Its operation bills this invoice; a top-up has a purchase row only when it carries an expiry.
    let triggered_at = CustomerCreditOperationRow::created_at_for_invoice(
        conn,
        invoice.tenant_id,
        purchase
            .as_ref()
            .map_or(invoice.id, |purchase| purchase.invoice_id),
        super::wallet_purchase::AUTOMATIC_TOP_UP_NAMESPACE,
    )
    .await?;
    for (line, face) in lines {
        let (source_id, created_by) = match &purchase {
            Some(purchase) => (purchase.id, purchase.created_by),
            None => (credit_line_source(invoice.id, &line.local_id), None),
        };
        if CustomerCreditGrantRow::find_by_source(
            conn,
            invoice.tenant_id,
            source::CREDIT_PURCHASE,
            source_id,
        )
        .await?
        .is_some()
        {
            continue;
        }
        let credited_subtotal = credited.get(&line.local_id).copied().unwrap_or(0);
        let amount_minor = face_net_of_notes(line, face, credited_subtotal)?;
        if amount_minor <= 0 {
            continue;
        }
        let period = credit_period_tx(conn, invoice, line).await?;
        let effective_at = if let Some(purchase) = purchase
            .as_ref()
            .filter(|purchase| purchase.expires_at.is_some())
        {
            purchase.created_at.and_utc()
        } else {
            match line.credit_grant_expiry {
                Some(CreditExpiry::BillingPeriod) if purchase.is_none() => {
                    period_credit_effective_at(funded_at, period)
                }
                _ => funded_at,
            }
        };
        let expires_at =
            if let Some(at) = purchase.as_ref().and_then(|purchase| purchase.expires_at) {
                // A purchase paid after its own chosen expiry would otherwise fund a grant
                // already dead on arrival.
                if at <= effective_at {
                    return Err(invalid(
                        "the credit expiry term leaves no usable credit at payment",
                    ));
                }
                Some(at)
            } else {
                match line.credit_grant_expiry {
                    Some(expiry) => Some(
                        purchased_expiry(expiry, effective_at, period)
                            .filter(|at| *at > effective_at)
                            .ok_or_else(|| {
                                invalid("the credit expiry term leaves no usable credit at payment")
                            })?,
                    ),
                    None => None,
                }
            };
        let cost = purchased_cost(
            line.taxable_amount,
            line.amount_subtotal,
            credited_subtotal,
            &invoice.currency,
        )?;
        // Credit bought for nothing (fully discounted) is given, not paid for: all bonus.
        let (classification, reason) = if line.taxable_amount > 0 {
            (
                GrantClassification::Paid,
                "Credit purchased on a paid invoice",
            )
        } else {
            (GrantClassification::Promotional, credit::BONUS_CREDIT)
        };
        let premium = cost.amount_minor > amount_minor;
        let scope = match purchase {
            Some(_) => GrantScope::customer_wide(),
            None => purchased_scope(invoice, line, &owners)?,
        };
        // A plan's pack is its period's credit, like included credit, however late it is paid.
        let pays_usage_from = match (&purchase, scope.subscription_id) {
            (None, Some(_)) => Some(midnight(line.start_date)),
            _ => triggered_at.map(|at| at.min(effective_at)),
        };
        let funding = CreditFunding {
            tenant_id: invoice.tenant_id,
            customer_id: invoice.customer_id,
            currency: granted_denomination(invoice, line).to_owned(),
            scope,
            amount_minor,
            classification,
            priority: DEFAULT_PRIORITY,
            effective_at,
            expires_at,
            occurred_at: funded_at,
            now,
            source_kind: source::CREDIT_PURCHASE,
            source_id,
            cost: Some(cost),
            rollover: None,
            invoice_id: Some(invoice.id),
            line_local_id: Some(line.local_id.clone()),
            credit_note_id: None,
            reason: reason.to_owned(),
            reference: None,
            created_by,
            pays_usage_from,
        };
        // A pack dearer than its credit (refused on new plans) is funded whole: its money landed.
        if premium {
            credit::fund_tx(conn, controls, funding).await?;
        } else {
            credit::fund_paid_tx(conn, controls, funding).await?;
        }
    }
    Ok(())
}

/// Funds the credit a plan includes at no charge, as promotional credit for the invoiced period,
/// recorded when the invoice was issued. Keyed by component and period, so a re-issued invoice
/// for the same period grants nothing more.
pub(crate) async fn mint_included_credit_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    invoice: &Invoice,
    issued: credit::MovementTime,
) -> StoreResult<()> {
    let owners = LineOwners::load_tx(conn, invoice.tenant_id, &invoice.line_items).await?;
    let included = || {
        invoice
            .line_items
            .iter()
            .filter(|l| l.credit_grant_included)
    };
    let welcome = SubscriptionComponentRow::once_credit_of(
        conn,
        invoice.tenant_id,
        &included()
            .filter_map(|l| l.sub_component_id)
            .collect::<Vec<_>>(),
        &included()
            .filter_map(|l| l.sub_add_on_id)
            .collect::<Vec<_>>(),
    )
    .await?;
    for (line, amount_minor) in purchased_lines(invoice).filter(|(l, _)| l.credit_grant_included) {
        let once = line
            .sub_component_id
            .map(|id| id.as_uuid())
            .or_else(|| line.sub_add_on_id.map(|id| id.as_uuid()))
            .is_some_and(|id| welcome.contains(&id));
        let component = line
            .sub_component_id
            .map(|id| id.as_uuid())
            .or_else(|| line.sub_add_on_id.map(|id| id.as_uuid()))
            .unwrap_or_else(|| credit_line_source(invoice.id, &line.local_id));
        let source_id = Uuid::new_v5(
            &component,
            format!("included-credit:{}", line.start_date).as_bytes(),
        );
        if CustomerCreditGrantRow::find_by_source(
            conn,
            invoice.tenant_id,
            source::INCLUDED_CREDIT,
            source_id,
        )
        .await?
        .is_some()
        {
            continue;
        }
        let effective_at = line.start_date.and_time(chrono::NaiveTime::MIN).and_utc();
        let period = credit_period_tx(conn, invoice, line).await?;
        // Cut to its first day by a plan change: the pack was removed before it took effect.
        if period.1 < line.end_date && period.1 <= line.start_date {
            continue;
        }
        let expires_at = match line.credit_grant_expiry {
            Some(expiry) => Some(
                expiry
                    .resolve_for_period(effective_at, period.1)
                    .filter(|at| *at > effective_at)
                    .ok_or_else(|| {
                        invalid("the included credit expiry term leaves no usable credit")
                    })?,
            ),
            None => None,
        };
        if line.credit_grant_rollover.is_some() && expires_at.is_none() {
            return Err(invalid(
                "included credit that never expires cannot roll over",
            ));
        }
        credit::fund_tx(
            conn,
            controls,
            CreditFunding {
                tenant_id: invoice.tenant_id,
                customer_id: invoice.customer_id,
                currency: granted_denomination(invoice, line).to_owned(),
                scope: pack_scope(invoice, line, &owners)?,
                amount_minor,
                classification: GrantClassification::Promotional,
                priority: DEFAULT_PRIORITY,
                effective_at,
                expires_at,
                occurred_at: issued.at,
                now: issued.now,
                source_kind: source::INCLUDED_CREDIT,
                source_id,
                cost: Some(CreditCost::given(&invoice.currency)),
                rollover: line.credit_grant_rollover,
                invoice_id: Some(invoice.id),
                line_local_id: Some(line.local_id.clone()),
                credit_note_id: None,
                reason: if once {
                    crate::domain::credit_groups::WELCOME_CREDIT.to_owned()
                } else {
                    "Credit included for the subscription period".to_owned()
                },
                reference: None,
                created_by: None,
                pays_usage_from: None,
            },
        )
        .await?;
    }
    Ok(())
}

/// Deterministic per invoice line, so a replayed payment funds the same grant.
pub(crate) fn credit_line_source(invoice_id: InvoiceId, local_id: &str) -> Uuid {
    Uuid::new_v5(
        &invoice_id.as_uuid(),
        format!("credit-line:{local_id}").as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn a_period_pack_ends_with_its_period_however_late_it_is_paid() {
        let january = (day("2026-01-01"), day("2026-02-01"));
        for paid in [
            "2025-12-20T00:00:00Z",
            "2026-01-04T00:00:00Z",
            "2026-03-15T00:00:00Z",
        ] {
            assert_eq!(
                purchased_expiry(CreditExpiry::BillingPeriod, at(paid), january),
                Some(at("2026-02-01T00:00:00Z")),
                "paid {paid}"
            );
        }
    }

    #[test]
    fn a_period_pack_is_spendable_from_payment_or_from_its_period_when_paid_after_it() {
        let january = (day("2026-01-01"), day("2026-02-01"));
        assert_eq!(
            period_credit_effective_at(at("2025-12-20T00:00:00Z"), january),
            at("2026-01-01T00:00:00Z")
        );
        assert_eq!(
            period_credit_effective_at(at("2026-01-04T10:00:00Z"), january),
            at("2026-01-04T10:00:00Z")
        );
        assert_eq!(
            period_credit_effective_at(at("2026-02-03T00:00:00Z"), january),
            at("2026-01-01T00:00:00Z"),
            "paid after its period: it still pays that period's usage"
        );
    }

    #[test]
    fn a_relative_term_runs_from_payment() {
        let paid = Utc.with_ymd_and_hms(2026, 1, 31, 0, 0, 0).unwrap();
        assert_eq!(
            purchased_expiry(
                CreditExpiry::Month(1),
                paid,
                (day("2026-01-01"), day("2026-02-01"))
            ),
            Some(Utc.with_ymd_and_hms(2026, 2, 28, 0, 0, 0).unwrap())
        );
        assert_eq!(
            purchased_expiry(
                CreditExpiry::Day(30),
                paid,
                (day("2026-01-01"), day("2026-02-01"))
            ),
            Some(Utc.with_ymd_and_hms(2026, 3, 2, 0, 0, 0).unwrap())
        );
    }

    #[test]
    fn a_line_source_is_stable_and_distinct() {
        let invoice = InvoiceId::new();
        assert_eq!(
            credit_line_source(invoice, "a"),
            credit_line_source(invoice, "a")
        );
        assert_ne!(
            credit_line_source(invoice, "a"),
            credit_line_source(invoice, "b")
        );
        assert_ne!(
            credit_line_source(invoice, "a"),
            credit_line_source(InvoiceId::new(), "a")
        );
    }
}
