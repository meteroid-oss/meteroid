use chrono::{DateTime, Utc};
use common_domain::actor::{Actor, ActorType};
use common_domain::ids::{CustomerPaymentMethodId, ProductId};
use meteroid_grpc::meteroid::api::wallets::v1 as server;
use meteroid_store::domain::credit::{
    AutoTopUpMonthSpend, AutoTopUpStopReason, AutomaticTopUp, AutomaticTopUpConfig,
    AutomaticTopUpMethod, CreditApplication, CreditGrant, CreditGrantSource, CreditGrantState,
    CreditLedgerEntry, CreditWallet, CreditWalletSummary, GrantClassification, LedgerEntryKind,
    WalletHealth, WalletLiveBalance, WalletStatus,
};
use meteroid_store::domain::credit_exhausted::UsageAtZero;
use meteroid_store::domain::credit_groups::{
    self as groups, CreditGrantKind, CreditGroup, GrantPlans,
};
use meteroid_store::domain::credit_units::CreditUnit;
use meteroid_store::repositories::credit::grants_in_draw_order;
use std::collections::HashMap;
use tonic::Status;
use uuid::Uuid;

pub use meteroid_grpc_mapping::wallets::{group_kind, timestamp};

pub fn from_timestamp(at: prost_types::Timestamp) -> Result<DateTime<Utc>, Status> {
    DateTime::from_timestamp(at.seconds, u32::try_from(at.nanos).unwrap_or(u32::MAX))
        .ok_or_else(|| Status::invalid_argument("invalid timestamp"))
}

pub fn applies_to(application: CreditApplication) -> server::CreditAppliesTo {
    match application {
        CreditApplication::Invoice => server::CreditAppliesTo::All,
        CreditApplication::Usage => server::CreditAppliesTo::Usage,
    }
}

pub fn grant_status(state: CreditGrantState) -> server::CreditGrantStatus {
    match state {
        CreditGrantState::Pending => server::CreditGrantStatus::Pending,
        CreditGrantState::Active => server::CreditGrantStatus::Active,
        CreditGrantState::Depleted => server::CreditGrantStatus::Depleted,
        CreditGrantState::Expired => server::CreditGrantStatus::Expired,
    }
}

/// Manual credit with no recorded team member came through the API.
pub fn grant_source(
    source: CreditGrantSource,
    created_by: Option<Uuid>,
) -> server::CreditGrantSource {
    match source {
        CreditGrantSource::CustomerBalance => server::CreditGrantSource::CustomerBalance,
        CreditGrantSource::Purchase => server::CreditGrantSource::Purchase,
        CreditGrantSource::Included => server::CreditGrantSource::Included,
        CreditGrantSource::CreditNote => server::CreditGrantSource::CreditNote,
        CreditGrantSource::NegativeInvoice => server::CreditGrantSource::NegativeInvoice,
        CreditGrantSource::InvoiceVoid => server::CreditGrantSource::InvoiceVoid,
        CreditGrantSource::UsageSettlementReturn => server::CreditGrantSource::Unspecified,
        CreditGrantSource::Manual if created_by.is_some() => server::CreditGrantSource::Manual,
        CreditGrantSource::Manual => server::CreditGrantSource::Api,
        CreditGrantSource::Rollover => server::CreditGrantSource::Rollover,
        CreditGrantSource::Coupon => server::CreditGrantSource::Coupon,
        CreditGrantSource::Overpayment => server::CreditGrantSource::Overpayment,
    }
}

pub fn ledger_entry_type(kind: LedgerEntryKind) -> server::LedgerEntryType {
    match kind {
        LedgerEntryKind::Fund => server::LedgerEntryType::Granted,
        LedgerEntryKind::Draw => server::LedgerEntryType::Applied,
        LedgerEntryKind::DrawReversal => server::LedgerEntryType::Returned,
        LedgerEntryKind::Reversal => server::LedgerEntryType::Removed,
        LedgerEntryKind::Reinstatement => server::LedgerEntryType::Returned,
        LedgerEntryKind::Expiry => server::LedgerEntryType::Expired,
        LedgerEntryKind::Rollover => server::LedgerEntryType::RolledOver,
    }
}

pub fn auto_top_up_method(method: AutomaticTopUpMethod) -> server::AutoTopUpMethod {
    match method {
        AutomaticTopUpMethod::Fixed => server::AutoTopUpMethod::Fixed,
        AutomaticTopUpMethod::ToTarget => server::AutoTopUpMethod::ToTarget,
    }
}

pub fn auto_top_up_method_from_server(
    method: server::AutoTopUpMethod,
) -> Result<AutomaticTopUpMethod, Status> {
    match method {
        server::AutoTopUpMethod::Fixed => Ok(AutomaticTopUpMethod::Fixed),
        server::AutoTopUpMethod::ToTarget => Ok(AutomaticTopUpMethod::ToTarget),
        server::AutoTopUpMethod::Unspecified => Err(Status::invalid_argument(
            "an automatic top-up method is required",
        )),
    }
}

/// Product names come from `grant_product_names`, archived products included, and a grant's
/// products cannot be deleted (`ON DELETE RESTRICT`); a name still missing reads as its id.
pub fn product_name(product_names: &HashMap<ProductId, String>, id: &ProductId) -> String {
    product_names.get(id).cloned().unwrap_or_else(|| {
        log::warn!("credit grant product {id} has no name loaded");
        format!("Product {id}")
    })
}

pub fn grant_kind(kind: CreditGrantKind) -> server::CreditGrantKind {
    match kind {
        CreditGrantKind::Included => server::CreditGrantKind::Included,
        CreditGrantKind::Prepaid => server::CreditGrantKind::Prepaid,
        CreditGrantKind::Bonus => server::CreditGrantKind::Bonus,
        CreditGrantKind::AccountBalance => server::CreditGrantKind::AccountBalance,
    }
}

pub fn credit_group(group: &CreditGroup) -> server::CreditGroup {
    server::CreditGroup {
        kind: grant_kind(group.kind).into(),
        subscription_id: group.subscription_id.map(|id| id.as_proto()),
        applies_to: applies_to(group.application).into(),
        label: group.label.clone(),
        pays_for: group.pays_for.clone(),
        available: group.available_minor,
        upcoming: group.upcoming_minor,
        next_lapse_at: group.next_lapse_at.map(timestamp),
        next_lapse_amount: group.next_lapse_minor,
        rollover: group
            .rollover
            .as_ref()
            .map(|rollover| server::CreditRollover {
                max_amount: rollover.max_minor,
                term: groups::rollover_term(rollover),
            }),
        credit_grant_ids: group.grant_ids.iter().map(|id| id.as_proto()).collect(),
        product_ids: group.product_ids.iter().map(|id| id.as_proto()).collect(),
    }
}

pub(crate) fn grant(
    grant: &CreditGrant,
    used_next: bool,
    credit_unit: bool,
    product_names: &HashMap<ProductId, String>,
    plans: &GrantPlans,
) -> Result<server::CreditGrant, Status> {
    let subscription_id = grant.scope.subscription_id;
    Ok(server::CreditGrant {
        id: grant.id.as_proto(),
        r#type: match grant.classification {
            GrantClassification::Paid => server::CreditGrantType::Paid,
            GrantClassification::Promotional => server::CreditGrantType::Promotional,
        }
        .into(),
        status: grant_status(grant.state).into(),
        source: grant_source(grant.source, grant.created_by).into(),
        amount: grant.amount_minor,
        remaining: grant.remaining_minor,
        expired_amount: grant.expired_minor,
        priority: grant.priority,
        effective_at: Some(timestamp(grant.effective_at)),
        expires_at: grant.expires_at.map(timestamp),
        invoice_id: grant.invoice_id.map(|id| id.as_proto()),
        credit_note_id: grant.credit_note_id.map(|id| id.as_proto()),
        reason: grant.reason.clone(),
        reference: grant.reference.clone(),
        created_at: Some(timestamp(grant.created_at)),
        removable: grant.removable(),
        used_next,
        cost: grant.cost.as_ref().map(|cost| cost.amount_minor),
        cost_currency: grant.cost.as_ref().map(|cost| cost.currency.clone()),
        applicable_products: grant
            .scope
            .product_ids
            .iter()
            .map(|id| server::GrantProduct {
                id: id.as_proto(),
                name: product_name(product_names, id),
            })
            .collect(),
        subscription_id: subscription_id.map(|id| id.as_proto()),
        subscription_plan_name: subscription_id.and_then(|id| plans.get(&id).cloned()),
        applies_to: applies_to(grant.scope.application).into(),
        ended_at: grant.ends_at.map(timestamp),
        kind: grant_kind(grant.kind(credit_unit)).into(),
        kind_label: groups::kind_label(grant.kind(credit_unit), subscription_id, plans),
        pays_for: groups::pays_for(
            subscription_id,
            grant.scope.application,
            credit_unit,
            &grant
                .scope
                .product_ids
                .iter()
                .map(|id| product_name(product_names, id))
                .collect::<Vec<_>>(),
            plans,
        ),
    })
}

/// Display names of the operators who attested a customer's agreement, by actor.
pub type AttesterNames = HashMap<(ActorType, Uuid), String>;

fn auto_top_up(
    stored: &AutomaticTopUp,
    attesters: &AttesterNames,
    month: Option<&AutoTopUpMonthSpend>,
) -> server::AutoTopUp {
    let config = &stored.config;
    server::AutoTopUp {
        charged_this_month: month.map_or(0, |month| month.charged_minor),
        charge_currency: month
            .map(|month| month.currency.clone())
            .unwrap_or_default(),
        config: Some(server::AutoTopUpConfig {
            method: auto_top_up_method(config.method).into(),
            threshold: config.threshold_minor,
            amount: config.amount_minor,
            target: config.target_minor,
            minimum: config.minimum_minor,
            maximum: config.maximum_minor,
            daily_cap: config.daily_cap_minor,
            monthly_cap: config.monthly_cap_minor,
            payment_method_id: config.payment_method_id.map(|id| id.as_proto()),
        }),
        enabled: !stored.parked,
        consecutive_failures: config.consecutive_failures,
        last_succeeded_at: config.last_succeeded_at.map(timestamp),
        suspended_at: config.suspended_at.map(timestamp),
        last_error: config.last_error.clone(),
        authorized_by: stored
            .authorization
            .as_ref()
            .map(|authorization| authorization.actor.clone()),
        authorized_at: stored
            .authorization
            .as_ref()
            .map(|authorization| timestamp(authorization.recorded_at)),
        stop_reason: config
            .stop_reason()
            .map_or(
                server::AutoTopUpStopReason::Unspecified,
                auto_top_up_stop_reason,
            )
            .into(),
        agreed_price: config
            .agreed_price
            .as_ref()
            .map(|price| server::AgreedUnitPrice {
                currency: price.currency.clone(),
                unit_price: price.unit_price.normalize().to_string(),
            }),
        daily_cap_is_default: config.daily_cap_is_default,
        maximum_is_default: config.maximum_is_default,
        consent_attestation: stored.consent_attestation.as_ref().map(|attestation| {
            let actor = &attestation.attested_by;
            server::ConsentAttestation {
                attested_by: match actor {
                    Actor::System => Some("System".to_owned()),
                    _ => actor
                        .as_uuid()
                        .and_then(|id| attesters.get(&(actor.actor_type(), id)).cloned()),
                },
                attested_at: Some(timestamp(attestation.attested_at)),
            }
        }),
    }
}

fn auto_top_up_stop_reason(reason: AutoTopUpStopReason) -> server::AutoTopUpStopReason {
    match reason {
        AutoTopUpStopReason::PriceChanged => server::AutoTopUpStopReason::PriceChanged,
        AutoTopUpStopReason::Unsellable => server::AutoTopUpStopReason::Unsellable,
        AutoTopUpStopReason::PaymentReversed => server::AutoTopUpStopReason::PaymentReversed,
        AutoTopUpStopReason::PaymentFailed => server::AutoTopUpStopReason::PaymentFailed,
        AutoTopUpStopReason::AwaitingCustomer => server::AutoTopUpStopReason::AwaitingCustomer,
    }
}

pub fn wallet_status(status: WalletStatus) -> server::WalletStatus {
    match status {
        WalletStatus::Healthy => server::WalletStatus::Healthy,
        WalletStatus::Low => server::WalletStatus::Low,
        WalletStatus::Depleted => server::WalletStatus::Depleted,
        WalletStatus::UsageBlocked => server::WalletStatus::UsageBlocked,
    }
}

pub fn wallet_status_from_server(status: server::WalletStatus) -> Option<WalletStatus> {
    match status {
        server::WalletStatus::Unspecified => None,
        server::WalletStatus::Healthy => Some(WalletStatus::Healthy),
        server::WalletStatus::Low => Some(WalletStatus::Low),
        server::WalletStatus::Depleted => Some(WalletStatus::Depleted),
        server::WalletStatus::UsageBlocked => Some(WalletStatus::UsageBlocked),
    }
}

fn live_balance(live: &WalletLiveBalance, stale: bool) -> server::LiveBalance {
    server::LiveBalance {
        available_after_usage: live.live_balance_minor,
        pending_usage: live.pending_usage_minor,
        uncovered_usage: live.uncovered_usage_minor,
        as_of: Some(timestamp(live.as_of)),
        stale,
    }
}

/// The types of the grants, paid first.
fn grant_types(grants: &[CreditGrant]) -> Vec<i32> {
    [
        (GrantClassification::Paid, server::CreditGrantType::Paid),
        (
            GrantClassification::Promotional,
            server::CreditGrantType::Promotional,
        ),
    ]
    .into_iter()
    .filter(|(classification, _)| {
        grants
            .iter()
            .any(|grant| grant.classification == *classification)
    })
    .map(|(_, grant_type)| grant_type.into())
    .collect()
}

/// `period_start`: the earliest current period start of the customer's live subscriptions. The
/// used-up grants that took effect since stay listed, for the credit granted this period.
#[allow(clippy::too_many_arguments)]
pub fn wallet(
    wallet: &CreditWallet,
    stored_auto_top_up: Option<&AutomaticTopUp>,
    auto_top_up_month: Option<&AutoTopUpMonthSpend>,
    health: &WalletHealth,
    usage_paused_at: Option<DateTime<Utc>>,
    product_names: &HashMap<ProductId, String>,
    plans: &GrantPlans,
    attesters: &AttesterNames,
    period_start: Option<DateTime<Utc>>,
) -> Result<server::Wallet, Status> {
    let credit_unit = wallet.credit_unit.is_some();
    let now = common_domain::clock::now();
    Ok(server::Wallet {
        id: wallet.id.as_proto(),
        customer_id: wallet.customer_id.as_proto(),
        currency: wallet.currency.clone(),
        available: wallet.available_minor,
        pending: wallet.pending_minor,
        low_balance_thresholds: wallet.low_balance_thresholds_minor.clone(),
        usage_at_zero: usage_at_zero_to_proto(wallet.usage_at_zero).into(),
        plan_rules: wallet
            .plan_rules
            .iter()
            .map(|plan| server::WalletPlanRule {
                subscription_id: plan.subscription_id.as_proto(),
                plan_name: plan.plan_name.clone(),
                rule: Some(
                    crate::plans::mapping::plans::credit_exhausted_rule_to_proto(&plan.rule),
                ),
            })
            .collect(),
        pauses_at_zero: wallet.pauses_at_zero(),
        grants: grants_in_draw_order(&wallet.grants)
            .into_iter()
            .filter(|stored| {
                !stored.used_up() || period_start.is_some_and(|start| stored.effective_at >= start)
            })
            .enumerate()
            .map(|(index, stored)| {
                grant(
                    stored,
                    index == 0
                        && stored.state == CreditGrantState::Active
                        && stored.remaining_minor > 0,
                    credit_unit,
                    product_names,
                    plans,
                )
            })
            .collect::<Result<Vec<_>, _>>()?,
        first_credit_at: wallet
            .grants
            .iter()
            .map(|stored| stored.effective_at)
            .filter(|at| *at <= now)
            .min()
            .map(timestamp),
        grant_types: grant_types(&wallet.grants),
        auto_top_up: stored_auto_top_up
            .map(|stored| auto_top_up(stored, attesters, auto_top_up_month)),
        created_at: Some(timestamp(wallet.created_at)),
        live: health
            .live
            .as_ref()
            .map(|live| live_balance(live, health.live_stale)),
        status: wallet_status(health.status).into(),
        credit_unit: wallet.credit_unit.as_ref().map(credit_unit_label),
        usage_paused_at: usage_paused_at.map(timestamp),
        groups: groups::credit_groups(&wallet.grants, credit_unit, plans, |id| {
            product_name(product_names, id)
        })
        .iter()
        .map(credit_group)
        .collect(),
        unit_price: wallet
            .unit_price
            .as_ref()
            .map(|price| server::AgreedUnitPrice {
                currency: price.currency.clone(),
                unit_price: price.unit_price.normalize().to_string(),
            }),
    })
}

pub fn credit_unit_label(unit: &CreditUnit) -> server::CreditUnitLabel {
    server::CreditUnitLabel {
        id: unit.id.as_proto(),
        code: unit.code.as_str().to_owned(),
        name: unit.name.clone(),
        plural_name: unit.plural_name.clone(),
        symbol: unit.symbol.clone(),
        precision: u32::from(unit.precision),
    }
}

pub fn ledger_entry(entry: &CreditLedgerEntry) -> server::LedgerEntry {
    server::LedgerEntry {
        id: entry.id.as_proto(),
        wallet_id: entry.wallet_id.as_proto(),
        credit_grant_id: entry.grant_id.as_proto(),
        sequence: entry.sequence,
        // Credit a plan change or cancellation removes reads as removed, refunded or not.
        r#type: if meteroid_store::domain::credit::is_removal_reason(&entry.reason) {
            server::LedgerEntryType::Removed
        } else {
            ledger_entry_type(entry.kind)
        }
        .into(),
        amount: entry.amount_minor,
        balance_after: entry.balance_after_minor,
        invoice_id: entry.invoice_id.map(|id| id.as_proto()),
        credit_note_id: entry.credit_note_id.map(|id| id.as_proto()),
        reason: entry.reason.clone(),
        reference: entry.reference.clone(),
        effective_at: Some(timestamp(entry.effective_at)),
        recorded_at: Some(timestamp(entry.recorded_at)),
        cost: entry.cost.as_ref().map(|cost| cost.amount_minor),
        cost_currency: entry.cost.as_ref().map(|cost| cost.currency.clone()),
    }
}

pub fn wallet_summary(summary: &CreditWalletSummary) -> server::WalletSummary {
    server::WalletSummary {
        id: summary.id.as_proto(),
        customer_id: summary.customer_id.as_proto(),
        customer_name: summary.customer_name.clone(),
        customer_alias: summary.customer_alias.clone(),
        currency: summary.currency.clone(),
        available: summary.available_minor,
        pending: summary.pending_minor,
        low_balance_thresholds: summary.low_balance_thresholds_minor.clone(),
        auto_top_up_enabled: summary.auto_top_up_enabled,
        auto_top_up_suspended: summary.auto_top_up_suspended,
        auto_top_up_threshold: summary.auto_top_up_threshold_minor,
        auto_top_up_failing: summary.auto_top_up_failing,
        usage_at_zero: usage_at_zero_to_proto(summary.usage_at_zero).into(),
        pauses_at_zero: summary.pauses_at_zero,
        // The tenant-wide roster has no per-wallet last-ledger-change signal yet.
        live: summary
            .live
            .as_ref()
            .map(|live| live_balance(live, summary.live_stale)),
        status: wallet_status(summary.status).into(),
        credit_unit: summary.credit_unit.as_ref().map(credit_unit_label),
    }
}

pub fn auto_top_up_config(config: server::AutoTopUpConfig) -> Result<AutomaticTopUpConfig, Status> {
    let method = auto_top_up_method_from_server(config.method())?;
    Ok(AutomaticTopUpConfig {
        minimum_minor: config.minimum,
        maximum_minor: config.maximum,
        daily_cap_minor: config.daily_cap,
        monthly_cap_minor: config.monthly_cap,
        ..AutomaticTopUpConfig::new(
            method,
            config.threshold,
            config.amount,
            config.target,
            CustomerPaymentMethodId::from_proto_opt(config.payment_method_id)?,
        )
    })
}

pub(crate) fn usage_at_zero_to_proto(value: UsageAtZero) -> server::UsageAtZero {
    match value {
        UsageAtZero::FollowPlan => server::UsageAtZero::FollowPlan,
        UsageAtZero::AlwaysPause => server::UsageAtZero::AlwaysPause,
        UsageAtZero::NeverPause => server::UsageAtZero::NeverPause,
    }
}

pub(crate) fn usage_at_zero_from_proto(value: server::UsageAtZero) -> UsageAtZero {
    match value {
        server::UsageAtZero::FollowPlan => UsageAtZero::FollowPlan,
        server::UsageAtZero::AlwaysPause => UsageAtZero::AlwaysPause,
        server::UsageAtZero::NeverPause => UsageAtZero::NeverPause,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::{BaseId, WalletId};

    #[test]
    fn an_automatic_top_up_shows_what_its_monthly_cap_counted_this_month() {
        let stored = AutomaticTopUp {
            wallet_id: WalletId::new(),
            config: AutomaticTopUpConfig {
                monthly_cap_minor: Some(50_000),
                ..AutomaticTopUpConfig::new(
                    AutomaticTopUpMethod::Fixed,
                    1_000,
                    Some(5_000),
                    None,
                    None,
                )
            },
            parked: false,
            authorization: None,
            consent_attestation: None,
        };
        let month = AutoTopUpMonthSpend {
            charged_minor: 12_000,
            currency: "EUR".to_owned(),
        };

        let shown = auto_top_up(&stored, &HashMap::new(), Some(&month));
        assert_eq!(shown.charged_this_month, 12_000);
        assert_eq!(shown.charge_currency, "EUR");
        assert_eq!(
            shown.config.and_then(|config| config.monthly_cap),
            Some(50_000)
        );

        let unknown = auto_top_up(&stored, &HashMap::new(), None);
        assert_eq!(
            (unknown.charged_this_month, unknown.charge_currency.as_str()),
            (0, "")
        );
    }

    #[test]
    fn manual_credit_without_a_team_member_came_through_the_api() {
        assert_eq!(
            grant_source(CreditGrantSource::Manual, None),
            server::CreditGrantSource::Api
        );
        assert_eq!(
            grant_source(CreditGrantSource::Manual, Some(Uuid::now_v7())),
            server::CreditGrantSource::Manual
        );
        assert_eq!(
            grant_source(CreditGrantSource::Purchase, None),
            server::CreditGrantSource::Purchase
        );
    }
}
