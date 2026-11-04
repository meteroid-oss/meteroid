//! How a wallet's credit reads: every grant falls in one group (included with a plan, purchased,
//! free, account balance), labelled here once so the admin, the portal and the API agree.
use crate::domain::credit::{
    CreditApplication, CreditGrant, CreditGrantSource, CreditGrantState, GrantClassification,
    GrantRollover,
};
use crate::domain::price_components::CreditExpiry;
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, CreditGrantId, ProductId, SubscriptionId};
use std::collections::HashMap;

/// The ledger reason of a plan's one-time (`once`) credit.
pub const WELCOME_CREDIT: &str = "Welcome credit";

/// The names of the kinds of credit, wherever a kind is named.
pub const PURCHASED_CREDIT: &str = "Purchased credit";
pub const FREE_CREDIT: &str = "Free credit";
pub const ACCOUNT_BALANCE: &str = "Account balance";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CreditGrantKind {
    /// Plan credit that belongs to a subscription and renews with it.
    Included,
    /// Bought: purchases, top-ups, paid packs, credit paid for outside Meteroid.
    Prepaid,
    /// Given: the bonus part of discounted credit, promo codes, welcome credit, goodwill grants.
    Bonus,
    /// Customer money that pays any invoice: credit notes, returned invoices, the carried balance.
    AccountBalance,
}

impl CreditGrant {
    /// `credit_unit`: the wallet holds a credit unit, which is never the account balance: only
    /// money pays any invoice.
    pub fn kind(&self, credit_unit: bool) -> CreditGrantKind {
        let any_charge = !credit_unit
            && self.scope.subscription_id.is_none()
            && self.scope.application == CreditApplication::Invoice
            && self.scope.product_ids.is_empty();
        match self.source {
            _ if self.is_bonus() => CreditGrantKind::Bonus,
            CreditGrantSource::Included if self.reason == WELCOME_CREDIT => CreditGrantKind::Bonus,
            CreditGrantSource::Included | CreditGrantSource::Rollover => CreditGrantKind::Included,
            CreditGrantSource::Coupon => CreditGrantKind::Bonus,
            CreditGrantSource::Purchase => CreditGrantKind::Prepaid,
            CreditGrantSource::Manual if any_charge => CreditGrantKind::AccountBalance,
            CreditGrantSource::Manual => match self.classification {
                GrantClassification::Paid => CreditGrantKind::Prepaid,
                GrantClassification::Promotional => CreditGrantKind::Bonus,
            },
            CreditGrantSource::CustomerBalance
            | CreditGrantSource::CreditNote
            | CreditGrantSource::Overpayment
            | CreditGrantSource::NegativeInvoice
            | CreditGrantSource::InvoiceVoid
            | CreditGrantSource::UsageSettlementReturn => CreditGrantKind::AccountBalance,
        }
    }
}

/// The plans of the subscriptions grants belong to, by subscription.
pub type GrantPlans = HashMap<SubscriptionId, String>;

pub fn kind_label(
    kind: CreditGrantKind,
    subscription_id: Option<SubscriptionId>,
    plans: &GrantPlans,
) -> String {
    match kind {
        CreditGrantKind::Included => match subscription_id.and_then(|id| plans.get(&id)) {
            Some(plan) => format!("Included with {plan}"),
            None => "Included with plan".to_owned(),
        },
        CreditGrantKind::Prepaid => PURCHASED_CREDIT.to_owned(),
        CreditGrantKind::Bonus => FREE_CREDIT.to_owned(),
        CreditGrantKind::AccountBalance => ACCOUNT_BALANCE.to_owned(),
    }
}

/// How many restricted products a label names before "+N".
const SHOWN_PRODUCTS: usize = 2;

/// What a grant pays, in words: `products` names the products it is restricted to (none: any),
/// in name order.
/// Credit-unit credit pays only usage priced in its unit.
pub fn pays_for(
    subscription_id: Option<SubscriptionId>,
    application: CreditApplication,
    credit_unit: bool,
    products: &[String],
    plans: &GrantPlans,
) -> String {
    let plan = subscription_id.map(|id| {
        plans
            .get(&id)
            .map_or_else(|| "its subscription's".to_owned(), |plan| plan.clone())
    });
    let usage = application == CreditApplication::Usage || credit_unit;
    if !products.is_empty() {
        let mut sorted = products.to_vec();
        sorted.sort();
        let mut named = sorted
            .iter()
            .take(SHOWN_PRODUCTS)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        if products.len() > SHOWN_PRODUCTS {
            named = format!("{named} +{}", products.len() - SHOWN_PRODUCTS);
        }
        let paid = if usage { "usage" } else { "charges" };
        return match plan {
            Some(plan) => format!("Pays {named} {paid} on {plan}"),
            None => format!("Pays {named} {paid}"),
        };
    }
    match (plan, usage) {
        (Some(plan), true) => format!("Pays {plan} usage"),
        (Some(plan), false) => format!("Pays {plan} charges"),
        (None, true) => "Pays usage on any subscription".to_owned(),
        (None, false) => "Pays any invoice".to_owned(),
    }
}

/// How long credit that rolls over stays spendable, e.g. "1 month".
pub fn rollover_term(rollover: &GrantRollover) -> String {
    let plural = |n: u32, unit: &str| {
        if n == 1 {
            format!("1 {unit}")
        } else {
            format!("{n} {unit}s")
        }
    };
    match rollover.expiry {
        CreditExpiry::Day(days) => plural(days, "day"),
        CreditExpiry::Month(months) => plural(months, "month"),
        CreditExpiry::BillingPeriod => "1 billing period".to_owned(),
    }
}

/// The spendable and scheduled credit of one kind, belonging to one subscription (or none), that
/// pays the same charges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditGroup {
    pub kind: CreditGrantKind,
    pub subscription_id: Option<SubscriptionId>,
    pub application: CreditApplication,
    /// The products it is restricted to, sorted; none for credit that pays any product.
    pub product_ids: Vec<ProductId>,
    pub label: String,
    pub pays_for: String,
    pub available_minor: i64,
    /// Granted, taking effect later.
    pub upcoming_minor: i64,
    /// The soonest moment some of `available_minor` lapses, and how much lapses then.
    pub next_lapse_at: Option<DateTime<Utc>>,
    pub next_lapse_minor: i64,
    /// Set when what lapses next rolls over instead, up to its cap.
    pub rollover: Option<GrantRollover>,
    pub grant_ids: Vec<CreditGrantId>,
}

/// One group per kind, subscription and charges paid (products included), ordered: included,
/// prepaid, bonus, account balance; within a kind, a subscription's credit before customer-wide
/// credit. `product_name` names a restricted product.
pub fn credit_groups(
    grants: &[CreditGrant],
    credit_unit: bool,
    plans: &GrantPlans,
    product_name: impl Fn(&ProductId) -> String,
) -> Vec<CreditGroup> {
    let mut groups: Vec<CreditGroup> = Vec::new();
    for grant in grants {
        let available = grant.state == CreditGrantState::Active && grant.remaining_minor > 0;
        let upcoming = grant.state == CreditGrantState::Pending;
        if !available && !upcoming {
            continue;
        }
        let kind = grant.kind(credit_unit);
        let mut product_ids = grant.scope.product_ids.clone();
        product_ids.sort_by_key(|id| id.as_uuid());
        product_ids.dedup();
        let index = match groups.iter().position(|group| {
            (group.kind, group.subscription_id, group.application)
                == (kind, grant.scope.subscription_id, grant.scope.application)
                && group.product_ids == product_ids
        }) {
            Some(index) => index,
            None => {
                let products = product_ids.iter().map(&product_name).collect::<Vec<_>>();
                groups.push(CreditGroup {
                    kind,
                    subscription_id: grant.scope.subscription_id,
                    application: grant.scope.application,
                    pays_for: pays_for(
                        grant.scope.subscription_id,
                        grant.scope.application,
                        credit_unit,
                        &products,
                        plans,
                    ),
                    product_ids,
                    label: kind_label(kind, grant.scope.subscription_id, plans),
                    available_minor: 0,
                    upcoming_minor: 0,
                    next_lapse_at: None,
                    next_lapse_minor: 0,
                    rollover: None,
                    grant_ids: Vec::new(),
                });
                groups.len() - 1
            }
        };
        let group = &mut groups[index];
        group.grant_ids.push(grant.id);
        if upcoming {
            group.upcoming_minor = group.upcoming_minor.saturating_add(grant.remaining_minor);
            continue;
        }
        group.available_minor = group.available_minor.saturating_add(grant.remaining_minor);
        if let Some(at) = grant.lapses_at() {
            match group.next_lapse_at {
                Some(next) if next < at => {}
                Some(next) if next == at => {
                    group.next_lapse_minor =
                        group.next_lapse_minor.saturating_add(grant.remaining_minor);
                    group.rollover = group.rollover.or(rolls_over(grant));
                }
                _ => {
                    group.next_lapse_at = Some(at);
                    group.next_lapse_minor = grant.remaining_minor;
                    group.rollover = rolls_over(grant);
                }
            }
        }
    }
    groups.sort_by(|a, b| {
        (
            a.kind,
            a.subscription_id.is_none(),
            &a.label,
            a.application.as_db_str(),
            !a.product_ids.is_empty(),
            &a.pays_for,
        )
            .cmp(&(
                b.kind,
                b.subscription_id.is_none(),
                &b.label,
                b.application.as_db_str(),
                !b.product_ids.is_empty(),
                &b.pays_for,
            ))
    });
    groups
}

/// A grant that ends with its subscription before its expiry never rolls over.
fn rolls_over(grant: &CreditGrant) -> Option<GrantRollover> {
    let ended_first = match (grant.ends_at, grant.expires_at) {
        (Some(ends_at), Some(expires_at)) => ends_at < expires_at,
        (Some(_), None) => true,
        (None, _) => false,
    };
    grant.rollover.filter(|_| !ended_first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::credit::GrantScope;
    use common_domain::ids::WalletId;

    fn grant(
        source: CreditGrantSource,
        classification: GrantClassification,
        scope: GrantScope,
        remaining: i64,
    ) -> CreditGrant {
        let at = Utc::now();
        CreditGrant {
            id: CreditGrantId::new(),
            wallet_id: WalletId::new(),
            amount_minor: remaining,
            remaining_minor: remaining,
            expired_minor: None,
            classification,
            priority: 100,
            source,
            source_id: uuid::Uuid::nil(),
            invoice_id: None,
            credit_note_id: None,
            effective_at: at,
            expires_at: None,
            state: CreditGrantState::Active,
            reason: "credit".to_owned(),
            reference: None,
            created_by: None,
            created_at: at,
            cost: None,
            scope,
            ends_at: None,
            end_reason: None,
            rollover: None,
        }
    }

    #[test]
    fn every_source_reads_as_one_kind() {
        let pro = SubscriptionId::new();
        let included = grant(
            CreditGrantSource::Included,
            GrantClassification::Promotional,
            GrantScope::subscription_usage(pro),
            1_000,
        );
        assert_eq!(included.kind(false), CreditGrantKind::Included);
        let welcome = CreditGrant {
            reason: WELCOME_CREDIT.to_owned(),
            ..included.clone()
        };
        assert_eq!(welcome.kind(false), CreditGrantKind::Bonus);
        let pack = grant(
            CreditGrantSource::Purchase,
            GrantClassification::Paid,
            GrantScope::subscription_usage(pro),
            1_000,
        );
        assert_eq!(pack.kind(false), CreditGrantKind::Prepaid);
        let bonus = CreditGrant {
            classification: GrantClassification::Promotional,
            reason: crate::repositories::credit::customer_credit::BONUS_CREDIT.to_owned(),
            ..pack.clone()
        };
        assert_eq!(bonus.kind(false), CreditGrantKind::Bonus);
        let goodwill = grant(
            CreditGrantSource::Manual,
            GrantClassification::Promotional,
            GrantScope::customer_wide(),
            1_000,
        );
        assert_eq!(goodwill.kind(false), CreditGrantKind::AccountBalance);
        assert_eq!(goodwill.kind(true), CreditGrantKind::Bonus);
        let usage_goodwill = CreditGrant {
            scope: GrantScope {
                application: CreditApplication::Usage,
                ..GrantScope::customer_wide()
            },
            ..goodwill.clone()
        };
        assert_eq!(usage_goodwill.kind(false), CreditGrantKind::Bonus);
        let note = grant(
            CreditGrantSource::CreditNote,
            GrantClassification::Paid,
            GrantScope::customer_wide(),
            1_000,
        );
        assert_eq!(note.kind(false), CreditGrantKind::AccountBalance);
    }

    #[test]
    fn groups_split_a_wallet_by_kind_and_subscription_with_their_own_figures() {
        let pro = SubscriptionId::new();
        let basic = SubscriptionId::new();
        let plans: GrantPlans = [(pro, "Pro".to_owned()), (basic, "Basic".to_owned())].into();
        let now = Utc::now();
        let soon = now + chrono::Duration::days(3);
        let later = now + chrono::Duration::days(30);
        let pro_pack = CreditGrant {
            expires_at: Some(soon),
            rollover: Some(GrantRollover {
                max_minor: Some(300),
                expiry: CreditExpiry::Month(1),
            }),
            ..grant(
                CreditGrantSource::Included,
                GrantClassification::Promotional,
                GrantScope::subscription_usage(pro),
                1_000,
            )
        };
        let basic_pack = grant(
            CreditGrantSource::Included,
            GrantClassification::Promotional,
            GrantScope::subscription_usage(basic),
            400,
        );
        let top_up = CreditGrant {
            expires_at: Some(later),
            ..grant(
                CreditGrantSource::Purchase,
                GrantClassification::Paid,
                GrantScope::customer_wide(),
                5_000,
            )
        };
        let scheduled = CreditGrant {
            state: CreditGrantState::Pending,
            ..grant(
                CreditGrantSource::Purchase,
                GrantClassification::Paid,
                GrantScope::customer_wide(),
                2_000,
            )
        };
        let spent = CreditGrant {
            state: CreditGrantState::Depleted,
            remaining_minor: 0,
            ..grant(
                CreditGrantSource::CreditNote,
                GrantClassification::Paid,
                GrantScope::customer_wide(),
                0,
            )
        };
        let groups = credit_groups(
            &[top_up, spent, basic_pack, pro_pack, scheduled],
            false,
            &plans,
            |id| id.to_string(),
        );
        let shown: Vec<_> = groups
            .iter()
            .map(|g| {
                (
                    g.label.as_str(),
                    g.pays_for.as_str(),
                    g.available_minor,
                    g.upcoming_minor,
                )
            })
            .collect();
        assert_eq!(
            shown,
            vec![
                ("Included with Basic", "Pays Basic usage", 400, 0),
                ("Included with Pro", "Pays Pro usage", 1_000, 0),
                ("Purchased credit", "Pays any invoice", 5_000, 2_000),
            ]
        );
        assert_eq!(groups[1].next_lapse_at, Some(soon));
        assert_eq!(groups[1].rollover.map(|r| r.max_minor), Some(Some(300)));
        assert_eq!(groups[2].next_lapse_minor, 5_000);
    }

    /// A €4 promotional grant restricted to the LLM API next to €10 of goodwill for anything: two
    /// groups, the restricted one naming its product, never "Pays any invoice".
    #[test]
    fn credit_restricted_to_products_names_them() {
        let (llm, search, storage) = (ProductId::new(), ProductId::new(), ProductId::new());
        let names: HashMap<ProductId, String> = [
            (llm, "LLM API".to_owned()),
            (search, "Search".to_owned()),
            (storage, "Storage".to_owned()),
        ]
        .into();
        let restricted = |products: Vec<ProductId>, remaining| CreditGrant {
            reason: crate::repositories::credit::customer_credit::BONUS_CREDIT.to_owned(),
            ..grant(
                CreditGrantSource::Purchase,
                GrantClassification::Promotional,
                GrantScope {
                    product_ids: products,
                    ..GrantScope::customer_wide()
                },
                remaining,
            )
        };
        let usage_only = grant(
            CreditGrantSource::Manual,
            GrantClassification::Promotional,
            GrantScope {
                application: CreditApplication::Usage,
                product_ids: vec![llm, search, storage],
                ..GrantScope::customer_wide()
            },
            300,
        );
        let goodwill = CreditGrant {
            reason: crate::repositories::credit::customer_credit::BONUS_CREDIT.to_owned(),
            ..grant(
                CreditGrantSource::Purchase,
                GrantClassification::Promotional,
                GrantScope::customer_wide(),
                1_000,
            )
        };
        let groups = credit_groups(
            &[restricted(vec![llm], 400), goodwill, usage_only],
            false,
            &GrantPlans::new(),
            |id| names[id].clone(),
        );
        let shown: Vec<_> = groups
            .iter()
            .map(|g| (g.label.as_str(), g.pays_for.as_str(), g.available_minor))
            .collect();
        assert_eq!(
            shown,
            vec![
                ("Free credit", "Pays any invoice", 1_000),
                ("Free credit", "Pays LLM API charges", 400),
                ("Free credit", "Pays LLM API, Search +1 usage", 300),
            ]
        );
        let pro = SubscriptionId::new();
        assert_eq!(
            pays_for(
                Some(pro),
                CreditApplication::Usage,
                false,
                &["LLM API".to_owned()],
                &[(pro, "Pro".to_owned())].into(),
            ),
            "Pays LLM API usage on Pro"
        );
        // Named in name order; two sets reading alike stay two groups, told apart by their ids.
        assert_eq!(
            pays_for(
                None,
                CreditApplication::Invoice,
                false,
                &[
                    "Storage".to_owned(),
                    "LLM API".to_owned(),
                    "Search".to_owned()
                ],
                &GrantPlans::new(),
            ),
            "Pays LLM API, Search +1 charges"
        );
        let other = ProductId::new();
        let names: HashMap<ProductId, String> = [
            (llm, "LLM API".to_owned()),
            (search, "Search".to_owned()),
            (storage, "Storage".to_owned()),
            (other, "Other".to_owned()),
        ]
        .into();
        let groups = credit_groups(
            &[
                restricted(vec![llm, search, storage], 100),
                restricted(vec![llm, search, other], 200),
            ],
            false,
            &GrantPlans::new(),
            |id| names[id].clone(),
        );
        assert_eq!(groups.len(), 2);
        assert_ne!(groups[0].product_ids, groups[1].product_ids);
    }

    #[test]
    fn credit_ending_with_its_subscription_never_rolls_over() {
        let now = Utc::now();
        let ended = CreditGrant {
            expires_at: Some(now + chrono::Duration::days(10)),
            ends_at: Some(now + chrono::Duration::days(1)),
            rollover: Some(GrantRollover {
                max_minor: None,
                expiry: CreditExpiry::Month(1),
            }),
            ..grant(
                CreditGrantSource::Included,
                GrantClassification::Promotional,
                GrantScope::subscription_usage(SubscriptionId::new()),
                1_000,
            )
        };
        let groups = credit_groups(&[ended], false, &GrantPlans::new(), |id| id.to_string());
        assert_eq!(groups[0].label, "Included with plan");
        assert!(groups[0].rollover.is_none());
    }
}
