use crate::portal::pricing::cadence_to_proto;
use crate::wallets::mapping::{product_name, timestamp};
use chrono::NaiveTime;
use common_domain::ids::{CreditGrantId, InvoiceId, ProductId};
use meteroid_app::conversions::ProtoConv;
use meteroid_grpc::meteroid::portal::wallets::v1 as server;
use meteroid_store::domain::credit::{
    AutomaticTopUp, CreditApplication, CreditGrant, CreditGrantSource, CreditGrantState,
    CreditLedgerEntry, CreditWallet, GrantClassification, LedgerEntryKind, WalletHealth,
    WalletStatus,
};
use meteroid_store::domain::credit_groups::{
    self as groups, CreditGrantKind, CreditGroup, GrantPlans,
};
use meteroid_store::domain::credit_ledger_labels::LedgerMovement;
use meteroid_store::domain::credit_outlook::{CreditOutlook, OutlookVerdict};
use meteroid_store::domain::portal_credit::{
    PortalCreditGrant, PortalCreditSource, PortalWalletOverview,
};
use std::collections::{HashMap, HashSet};
use tonic::Status;

pub(crate) use meteroid_grpc_mapping::wallets::portal::{auto_top_up, credit_unit_label};

fn grant_kind(kind: CreditGrantKind) -> server::CreditGrantKind {
    match kind {
        CreditGrantKind::Included => server::CreditGrantKind::Included,
        CreditGrantKind::Prepaid => server::CreditGrantKind::Prepaid,
        CreditGrantKind::Bonus => server::CreditGrantKind::Bonus,
        CreditGrantKind::AccountBalance => server::CreditGrantKind::AccountBalance,
    }
}

fn grant_scope(
    grant: &CreditGrant,
    credit_unit: bool,
    product_names: &HashMap<ProductId, String>,
    plans: &GrantPlans,
) -> server::GrantScope {
    let subscription_id = grant.scope.subscription_id;
    let products = grant
        .scope
        .product_ids
        .iter()
        .map(|id| product_name(product_names, id))
        .collect::<Vec<_>>();
    server::GrantScope {
        kind: grant_kind(grant.kind(credit_unit)).into(),
        kind_label: groups::kind_label(grant.kind(credit_unit), subscription_id, plans),
        pays_for: groups::pays_for(
            subscription_id,
            grant.scope.application,
            credit_unit,
            &products,
            plans,
        ),
        subscription_id: subscription_id.map(|id| id.as_proto()),
        subscription_plan_name: subscription_id.and_then(|id| plans.get(&id).cloned()),
        usage_only: grant.scope.application == CreditApplication::Usage,
    }
}

fn credit_group(group: &CreditGroup) -> server::CreditGroup {
    server::CreditGroup {
        kind: grant_kind(group.kind).into(),
        label: group.label.clone(),
        pays_for: group.pays_for.clone(),
        subscription_id: group.subscription_id.map(|id| id.as_proto()),
        usage_only: group.application == CreditApplication::Usage,
        available: group.available_minor,
        upcoming: group.upcoming_minor,
        next_lapse_at: group.next_lapse_at.map(timestamp),
        next_lapse_amount: group.next_lapse_minor,
        rollover: group.rollover.map(|rollover| server::RolloverCap {
            max_amount: rollover.max_minor,
        }),
        rollover_term: group.rollover.as_ref().map(groups::rollover_term),
        product_ids: group.product_ids.iter().map(|id| id.as_proto()).collect(),
    }
}

fn grant(
    grant: &CreditGrant,
    credit_unit: bool,
    product_names: &HashMap<ProductId, String>,
    plans: &GrantPlans,
) -> Result<server::CreditGrant, Status> {
    Ok(server::CreditGrant {
        id: grant.id.as_proto(),
        amount: grant.amount_minor,
        remaining: grant.remaining_minor,
        status: match grant.state {
            CreditGrantState::Pending => server::CreditGrantStatus::Pending,
            CreditGrantState::Active => server::CreditGrantStatus::Active,
            CreditGrantState::Depleted => server::CreditGrantStatus::Depleted,
            CreditGrantState::Expired => server::CreditGrantStatus::Expired,
        }
        .into(),
        promotional: grant.classification == GrantClassification::Promotional,
        effective_at: Some(timestamp(grant.effective_at)),
        expires_at: grant.expires_at.map(timestamp),
        expired_amount: grant.expired_minor,
        applicable_products: grant
            .scope
            .product_ids
            .iter()
            .map(|id| server::GrantProduct {
                id: id.as_proto(),
                name: product_name(product_names, id),
            })
            .collect(),
        scope: Some(grant_scope(grant, credit_unit, product_names, plans)),
    })
}

fn wallet_status(status: WalletStatus) -> server::WalletStatus {
    match status {
        WalletStatus::Healthy => server::WalletStatus::Healthy,
        WalletStatus::Low => server::WalletStatus::Low,
        WalletStatus::Depleted => server::WalletStatus::Depleted,
        WalletStatus::UsageBlocked => server::WalletStatus::UsageBlocked,
    }
}

pub fn wallet(
    wallet: &CreditWallet,
    stored: Option<&AutomaticTopUp>,
    health: &WalletHealth,
    top_up_enabled: bool,
    unit_top_ups: bool,
    sale: Option<&meteroid_store::services::UnitSale>,
    product_names: &HashMap<ProductId, String>,
    plans: &GrantPlans,
) -> Result<server::Wallet, Status> {
    let credit_unit = wallet.credit_unit.is_some();
    Ok(server::Wallet {
        id: wallet.id.as_proto(),
        currency: wallet.currency.clone(),
        available: wallet.available_minor,
        pending: wallet.pending_minor,
        grants: wallet
            .grants
            .iter()
            .filter(|stored| !stored.used_up())
            .map(|stored| grant(stored, credit_unit, product_names, plans))
            .collect::<Result<_, _>>()?,
        groups: groups::credit_groups(&wallet.grants, credit_unit, plans, |id| {
            product_name(product_names, id)
        })
        .iter()
        .map(credit_group)
        .collect(),
        auto_top_up: stored.map(|stored| auto_top_up(&stored.config, !stored.parked)),
        // A credit unit is bought only once it has a price in the customer's currency.
        can_top_up: top_up_enabled
            && (wallet.credit_unit.is_none() || (unit_top_ups && sale.is_some())),
        sale_price: sale.map(|sale| server::CreditUnitSalePrice {
            currency: sale.currency.clone(),
            unit_price: sale.unit_price.normalize().to_string(),
        }),
        credit_unit: wallet.credit_unit.as_ref().map(credit_unit_label),
        pauses_at_zero: wallet.pauses_at_zero(),
        available_after_usage: health.live.as_ref().map(|live| live.live_balance_minor),
        pending_usage: health.live.as_ref().map(|live| live.pending_usage_minor),
        as_of: health.live.as_ref().map(|live| timestamp(live.as_of)),
        status: wallet_status(health.status).into(),
        uncovered_usage: health
            .live
            .as_ref()
            .map_or(0, |live| live.uncovered_usage_minor),
        live_stale: health.live.is_some() && health.live_stale,
    })
}

/// The shared label, with what the customer may read beside it: the invoice or credit note
/// number, a promo code, or a reason the system wrote (an operator's stays internal).
fn description(
    movement: &LedgerMovement,
    grant: Option<&CreditGrant>,
    number: Option<&str>,
) -> String {
    let label = movement.label();
    if movement.entry.kind == LedgerEntryKind::Draw {
        return match number {
            Some(number) => format!("{label} {number}"),
            None => label.into_owned(),
        };
    }
    let code = grant
        .filter(|grant| grant.source == CreditGrantSource::Coupon)
        .and_then(|grant| grant.reference.as_deref())
        .filter(|_| movement.entry.kind == LedgerEntryKind::Fund);
    [
        Some(label.as_ref()),
        code,
        movement.customer_reason(),
        number,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

pub fn ledger_entry(
    entry: &CreditLedgerEntry,
    wallet: &CreditWallet,
    automatic_top_up_grants: &HashSet<CreditGrantId>,
    invoice_numbers: &HashMap<InvoiceId, String>,
) -> server::LedgerEntry {
    let grant = wallet
        .grants
        .iter()
        .find(|grant| grant.id == entry.grant_id);
    let movement = LedgerMovement {
        entry,
        source: grant.map(|grant| grant.source),
        classification: grant.map(|grant| grant.classification),
        bonus: grant.is_some_and(CreditGrant::is_bonus),
        automatic_top_up: automatic_top_up_grants.contains(&entry.grant_id),
    };
    let invoice_number = entry
        .invoice_id
        .and_then(|id| invoice_numbers.get(&id))
        .map(String::as_str);
    server::LedgerEntry {
        id: entry.id.as_proto(),
        r#type: match entry.kind {
            LedgerEntryKind::Fund => server::LedgerEntryType::Granted,
            LedgerEntryKind::Draw => server::LedgerEntryType::Applied,
            LedgerEntryKind::DrawReversal => server::LedgerEntryType::Returned,
            LedgerEntryKind::Reversal => server::LedgerEntryType::Removed,
            LedgerEntryKind::Reinstatement => server::LedgerEntryType::Returned,
            LedgerEntryKind::Expiry
                if meteroid_store::domain::credit::is_removal_reason(&entry.reason) =>
            {
                server::LedgerEntryType::Removed
            }
            LedgerEntryKind::Expiry => server::LedgerEntryType::Expired,
            LedgerEntryKind::Rollover => server::LedgerEntryType::RolledOver,
        }
        .into(),
        amount: entry.amount_minor,
        balance_after: entry.balance_after_minor,
        description: description(&movement, grant, invoice_number),
        invoice_id: entry.invoice_id.map(|id| id.as_proto()),
        credit_note_id: entry.credit_note_id.map(|id| id.as_proto()),
        effective_at: Some(timestamp(entry.effective_at)),
        invoice_number: invoice_number.map(str::to_owned),
    }
}

fn credit_source(source: PortalCreditSource) -> server::CreditSource {
    match source {
        PortalCreditSource::Purchased => server::CreditSource::Purchased,
        PortalCreditSource::Included => server::CreditSource::Included,
        PortalCreditSource::Added => server::CreditSource::Added,
        PortalCreditSource::RolledOver => server::CreditSource::RolledOver,
        PortalCreditSource::AutomaticTopUp => server::CreditSource::AutomaticTopUp,
        PortalCreditSource::CreditNote => server::CreditSource::CreditNote,
        PortalCreditSource::Returned => server::CreditSource::Returned,
        PortalCreditSource::PreviousBalance => server::CreditSource::PreviousBalance,
        PortalCreditSource::Bonus => server::CreditSource::Bonus,
        PortalCreditSource::Overpayment => server::CreditSource::Overpayment,
    }
}

fn grant_view(
    grant: &PortalCreditGrant,
    credit_unit: bool,
    product_names: &HashMap<ProductId, String>,
    plans: &GrantPlans,
) -> server::CreditGrantView {
    server::CreditGrantView {
        id: grant.grant.id.as_proto(),
        source: credit_source(grant.source).into(),
        amount: grant.grant.amount_minor,
        remaining: grant.grant.remaining_minor,
        remaining_after_usage: grant.remaining_after_usage_minor,
        effective_at: Some(timestamp(grant.grant.effective_at)),
        expires_at: grant.grant.expires_at.map(timestamp),
        scheduled: grant.grant.state == CreditGrantState::Pending,
        rollover: grant
            .rollover_max_minor
            .map(|max_amount| server::RolloverCap { max_amount }),
        invoice_number: grant.invoice_number.clone(),
        applicable_products: grant
            .grant
            .scope
            .product_ids
            .iter()
            .map(|id| server::GrantProduct {
                id: id.as_proto(),
                name: product_name(product_names, id),
            })
            .collect(),
        scope: Some(grant_scope(&grant.grant, credit_unit, product_names, plans)),
    }
}

pub fn wallet_overview(
    overview: &PortalWalletOverview,
    top_up_enabled: bool,
    unit_top_ups: bool,
    sale: Option<&meteroid_store::services::UnitSale>,
    product_names: &HashMap<ProductId, String>,
    plans: &GrantPlans,
    outlooks: &[CreditOutlook],
) -> Result<server::WalletOverview, Status> {
    let credit_unit = overview.wallet.credit_unit.is_some();
    Ok(server::WalletOverview {
        wallet: Some(wallet(
            &overview.wallet,
            overview.auto_top_up.as_ref(),
            &overview.health,
            top_up_enabled,
            unit_top_ups,
            sale,
            product_names,
            plans,
        )?),
        grants: overview
            .grants
            .iter()
            .map(|grant| grant_view(grant, credit_unit, product_names, plans))
            .collect(),
        period: overview.period.map(|(start, end)| server::CreditPeriod {
            start: Some(timestamp(start.and_time(NaiveTime::MIN).and_utc())),
            end: Some(timestamp(end.and_time(NaiveTime::MIN).and_utc())),
        }),
        projected_usage: overview.projected_usage_minor,
        usage_quantity: overview
            .usage_quantity
            .as_ref()
            .map(|quantity| server::UsageQuantity {
                quantity: quantity.quantity.normalize().to_string(),
                metric_name: quantity.metric_name.clone(),
                unit_label: crate::billablemetrics::mapping::metric::unit_label_to_server(
                    quantity.unit_singular.clone(),
                    quantity.unit_plural.clone(),
                ),
            }),
        totals: Some(server::CreditTotals {
            since: overview.totals.since.map(timestamp),
            added: overview.totals.added_minor,
            used: overview.totals.used_minor,
            expired: overview.totals.expired_minor,
        }),
        next_credit: overview
            .next_credit
            .as_ref()
            .map(|next| server::NextCredit {
                amount: next.amount_minor,
                date: next.date.format("%Y-%m-%d").to_string(),
                price: next.price_minor,
                price_currency: next.price_currency.clone(),
                cadence: cadence_to_proto(next.cadence),
                price_excludes_tax: next.price_excludes_tax,
            }),
        next_expiry: overview.next_expiry.map(|next| server::ExpiringCredit {
            expires_at: Some(timestamp(next.expires_at)),
            remaining: next.remaining_minor,
            rollover_amount: next.rollover_minor,
        }),
        usage_paused_at: overview.usage_paused_at.map(timestamp),
        period_usage: overview.period_usage_minor,
        outlooks: outlooks
            .iter()
            .filter(|outlook| outlook.wallet_id == overview.wallet.id)
            .map(credit_outlook)
            .collect(),
    })
}

fn credit_outlook(outlook: &CreditOutlook) -> server::CreditOutlook {
    use server::outlook_verdict::{
        AutomaticTopUp, BeyondCredit, Covered, ProjectedBeyond, RunsOut, TopUpFailing, Updating,
        UsagePaused, Verdict,
    };
    let verdict = match &outlook.verdict {
        OutlookVerdict::Stale => Verdict::Updating(Updating {}),
        OutlookVerdict::UsagePaused { since } => Verdict::UsagePaused(UsagePaused {
            since: Some(timestamp(*since)),
        }),
        OutlookVerdict::TopUpFailing { .. } => Verdict::TopUpFailing(TopUpFailing {}),
        OutlookVerdict::BeyondCredit { uncovered_minor } => Verdict::BeyondCredit(BeyondCredit {
            uncovered: *uncovered_minor,
        }),
        OutlookVerdict::RunsOut { on, pauses } => Verdict::RunsOut(RunsOut {
            on: on.as_proto(),
            pauses: *pauses,
        }),
        OutlookVerdict::ProjectedBeyond { amount_minor, by } => {
            Verdict::ProjectedBeyond(ProjectedBeyond {
                amount: *amount_minor,
                by: by.as_proto(),
            })
        }
        OutlookVerdict::AutomaticTopUp { .. } => Verdict::AutomaticTopUp(AutomaticTopUp {}),
        OutlookVerdict::Covered {
            through,
            projected_minor,
        } => Verdict::Covered(Covered {
            through: through.as_proto(),
            projected: *projected_minor,
        }),
    };
    server::CreditOutlook {
        subscription_id: outlook.subscription_id.as_proto(),
        plan_name: outlook.plan_name.clone(),
        horizon: outlook.horizon.as_proto(),
        as_of: Some(timestamp(outlook.as_of)),
        balance: outlook.balance_minor,
        included: outlook.included_minor,
        shared: outlook.shared_minor,
        projected_usage: outlook.projected_usage_minor,
        verdict: Some(server::OutlookVerdict {
            verdict: Some(verdict),
        }),
        partial: outlook.partial,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::{BaseId, InvoiceId, LedgerEntryId, WalletId};

    fn entry(kind: LedgerEntryKind, reason: &str) -> CreditLedgerEntry {
        CreditLedgerEntry {
            id: LedgerEntryId::new(),
            wallet_id: WalletId::new(),
            grant_id: CreditGrantId::new(),
            sequence: 1,
            kind,
            amount_minor: 5_000,
            balance_after_minor: 5_000,
            invoice_id: None,
            credit_note_id: None,
            reason: reason.to_owned(),
            reference: None,
            created_by: None,
            effective_at: chrono::Utc::now(),
            recorded_at: chrono::Utc::now(),
            cost: None,
            application: meteroid_store::domain::credit::CreditApplication::Invoice,
            subscription_id: None,
        }
    }

    fn movement(entry: &CreditLedgerEntry, source: CreditGrantSource) -> LedgerMovement<'_> {
        LedgerMovement {
            entry,
            source: Some(source),
            classification: Some(GrantClassification::Paid),
            bonus: false,
            automatic_top_up: false,
        }
    }

    #[test]
    fn the_statement_adds_numbers_and_system_reasons_never_operator_ones() {
        let fund = entry(LedgerEntryKind::Fund, "");
        assert_eq!(
            description(
                &movement(&fund, CreditGrantSource::Purchase),
                None,
                Some("INV-7")
            ),
            "Purchased credit · INV-7"
        );
        let mut draw = entry(LedgerEntryKind::Draw, "");
        draw.invoice_id = Some(InvoiceId::new());
        assert_eq!(
            description(
                &movement(&draw, CreditGrantSource::Purchase),
                None,
                Some("INV-8")
            ),
            "Applied to invoice INV-8"
        );
        let disputed = entry(
            LedgerEntryKind::Reversal,
            meteroid_store::domain::credit_ledger_labels::PAYMENT_DISPUTE_LOST,
        );
        assert_eq!(
            description(
                &movement(&disputed, CreditGrantSource::Purchase),
                None,
                None
            ),
            "Credit removed · Payment lost to a dispute"
        );
        let operator = entry(LedgerEntryKind::Reversal, "Clawback, ticket 42");
        assert_eq!(
            description(&movement(&operator, CreditGrantSource::Manual), None, None),
            "Credit removed"
        );
    }
}
