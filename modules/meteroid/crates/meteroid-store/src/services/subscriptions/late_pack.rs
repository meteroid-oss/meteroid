//! What a period pack funded after its period was invoiced would have paid of that period on time:
//! at its expiry, the period's usage as invoiced, drawn from it first (with any other credit
//! lapsing then) as the expiry settlement draws: promotional before tax, paid after it.
use crate::StoreResult;
use crate::domain::credit::LedgerEntryKind;
use crate::domain::credit_units::DenominationCode;
use crate::domain::{LineItem, LineItemKind};
use crate::repositories::credit::allocation::{GrantCandidate, GrantPool};
use crate::repositories::credit::customer_credit::{
    UsageOwed, candidate, draw_owed, normalized_currency,
};
use crate::repositories::credit_notes::Returned;
use crate::services::credit::promotional::taxed_base;
use crate::services::credit::scope::{LineOwners, line_subscription};
use crate::services::subscriptions::amendment::gather_period_credit_context;
use crate::store::PgConn;
use chrono::{Duration, NaiveDate};
use common_domain::ids::{CreditGrantId, SubscriptionId, TenantId};
use diesel_models::customer_credit::{CustomerCreditEntryRow, CustomerCreditGrantRow};
use std::collections::HashMap;

/// A period pack paid after its period was invoiced: its paid grant and its bonus, if any.
pub(super) struct LatePack<'a> {
    pub subscription_id: SubscriptionId,
    pub period: (NaiveDate, NaiveDate),
    pub denomination: &'a str,
    pub paid: &'a CustomerCreditGrantRow,
    pub bonus: Option<&'a CustomerCreditGrantRow>,
}

/// The face of the pack, bonus included, its period's usage would have drawn at its expiry: the
/// usage lines dated in the period on the subscription's finalized invoices (net of their notes,
/// gross of the credit they drew), the pack's grants at face.
pub(super) async fn pack_face_drawn_on_time_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    pack: &LatePack<'_>,
) -> StoreResult<i64> {
    let (start, end) = pack.period;
    let ctx = gather_period_credit_context(
        conn,
        tenant_id,
        pack.subscription_id,
        start,
        end.succ_opt().unwrap_or(end),
    )
    .await?;
    let denomination = normalized_currency(pack.denomination);
    let unit = DenominationCode::parse(&denomination).is_ok_and(|code| code.is_unit());
    let owners = LineOwners::load_tx(conn, tenant_id, &ctx.all_lines).await?;
    let mut owed = Vec::new();
    for invoice in &ctx.invoices {
        for line in invoice.line_items.iter().filter(|line| {
            line.metric_id.is_some()
                && line.kind == LineItemKind::Regular
                && start <= line.start_date
                && line.start_date < end
                && line_subscription(line, invoice.subscription_id, &owners)
                    == Some(pack.subscription_id)
        }) {
            owed.extend(usage_owed(
                line,
                invoice.tax_inclusive,
                unit.then_some(denomination.as_str()),
                ctx.returned.get(&line.local_id),
            ));
        }
    }
    if owed.is_empty() {
        return Ok(0);
    }

    let mut budgets: HashMap<CreditGrantId, i64> = std::iter::once(pack.paid)
        .chain(pack.bonus)
        .map(|grant| (grant.id, grant.amount.max(0)))
        .collect();
    let own = |id: CreditGrantId| id == pack.paid.id || pack.bonus.is_some_and(|b| b.id == id);
    for invoice in &ctx.invoices {
        for entry in CustomerCreditEntryRow::list_by_invoice_in_currency(
            conn,
            tenant_id,
            invoice.id,
            &denomination,
        )
        .await?
        {
            let kind = LedgerEntryKind::from_db_str(&entry.kind)?;
            if matches!(kind, LedgerEntryKind::Draw | LedgerEntryKind::DrawReversal)
                && !own(entry.grant_id)
            {
                *budgets.entry(entry.grant_id).or_insert(0) -= entry.amount;
            }
        }
    }
    let bound = pack
        .paid
        .lapses_at
        .unwrap_or_else(|| end.and_time(chrono::NaiveTime::MIN).and_utc());
    let ids = budgets.keys().copied().collect::<Vec<_>>();
    let mut lapsing = Vec::new();
    // Other credit lapsing with the pack pays beside it, as much as it did pay.
    for row in CustomerCreditGrantRow::list_by_ids_scoped(conn, tenant_id, &ids).await? {
        let grant = candidate(&row)?;
        if own(grant.id) || grant.lapses_at == Some(bound) {
            lapsing.push(GrantCandidate {
                remaining_minor: budgets.get(&grant.id).copied().unwrap_or(0),
                lapses_at: Some(bound),
                ..grant
            });
        }
    }
    let mut left = owed
        .iter()
        .map(|owed| owed.base_minor.max(0))
        .collect::<Vec<_>>();
    let draws = draw_owed(
        &owed,
        &mut left,
        (pack.subscription_id, bound),
        &[&lapsing],
        GrantPool::sequence(&denomination),
        bound - Duration::microseconds(1),
        &mut HashMap::new(),
    )?;
    Ok(draws
        .into_iter()
        .filter(|(grant_id, _)| own(*grant_id))
        .map(|(_, amount)| amount)
        .sum())
}

/// What a usage line owes the pack's denomination: its credits of `unit`, else its money before
/// tax and before the promotional credit it drew, both net of what its notes `returned`.
fn usage_owed(
    line: &LineItem,
    tax_inclusive: bool,
    unit: Option<&str>,
    returned: Option<&Returned>,
) -> Option<UsageOwed> {
    let base_minor = match unit {
        Some(code) => {
            line.credit_unit
                .as_ref()
                .filter(|block| normalized_currency(&block.code) == code)?
                .consumed
        }
        None => {
            let returned = returned.map_or(0, |returned| {
                returned.taxable() + if tax_inclusive { returned.tax() } else { 0 }
            });
            taxed_base(line, tax_inclusive) - returned
        }
    };
    Some(UsageOwed {
        product_id: line.credit_product(),
        base_minor: base_minor.max(0),
        tax_inclusive: tax_inclusive && unit.is_none(),
        taxes: if unit.is_none() {
            line.tax_details.clone()
        } else {
            Vec::new()
        },
    })
}
