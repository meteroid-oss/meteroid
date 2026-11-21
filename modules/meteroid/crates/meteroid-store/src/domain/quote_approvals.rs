use chrono::NaiveDateTime;
use common_domain::ids::{QuoteApprovalId, QuoteApprovalRuleId, QuoteId, UserId};
use diesel_models::quote_approvals::{QuoteApprovalRow, QuoteApprovalRuleRow};
use rust_decimal::Decimal;

use crate::domain::enums::QuoteApprovalStatusEnum;
use crate::domain::quotes::Quote;

/// A condition under which a quote needs someone's approval before it is published.
#[derive(Debug, Clone)]
pub struct QuoteApprovalRule {
    pub id: QuoteApprovalRuleId,
    pub name: String,
    /// A line discount or percentage coupon at least this large.
    pub min_discount_percent: Option<Decimal>,
    /// Payment terms longer than this many days.
    pub max_net_terms_days: Option<i32>,
    /// A trial longer than this many days.
    pub max_trial_days: Option<i32>,
    /// Every quote.
    pub always: bool,
    /// Who may approve; empty means any member allowed to manage quotes.
    pub approver_user_ids: Vec<UserId>,
    pub created_at: NaiveDateTime,
}

impl From<QuoteApprovalRuleRow> for QuoteApprovalRule {
    fn from(r: QuoteApprovalRuleRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            min_discount_percent: r.min_discount_percent,
            max_net_terms_days: r.max_net_terms_days,
            max_trial_days: r.max_trial_days,
            always: r.always,
            approver_user_ids: r.approver_user_ids.into_iter().flatten().collect(),
            created_at: r.created_at,
        }
    }
}

impl QuoteApprovalRule {
    pub fn matches(&self, quote: &Quote) -> bool {
        self.always
            || self
                .min_discount_percent
                .zip(quote.max_discount_percent)
                .is_some_and(|(min, max)| max >= min)
            || self
                .max_net_terms_days
                .is_some_and(|days| quote.net_terms > days)
            || self
                .max_trial_days
                .zip(quote.trial_duration_days)
                .is_some_and(|(days, trial)| trial > days)
    }

    pub fn may_approve(&self, user_id: UserId) -> bool {
        self.approver_user_ids.is_empty() || self.approver_user_ids.contains(&user_id)
    }
}

#[derive(Debug, Clone)]
pub struct QuoteApproval {
    pub id: QuoteApprovalId,
    pub quote_id: QuoteId,
    pub revision: i32,
    pub document_hash: String,
    pub rule_ids: Vec<QuoteApprovalRuleId>,
    pub status: QuoteApprovalStatusEnum,
    pub requested_by: Option<UserId>,
    pub requested_at: NaiveDateTime,
    pub request_note: Option<String>,
    pub decided_by: Option<UserId>,
    pub decided_at: Option<NaiveDateTime>,
    pub decision_note: Option<String>,
}

impl From<QuoteApprovalRow> for QuoteApproval {
    fn from(r: QuoteApprovalRow) -> Self {
        Self {
            id: r.id,
            quote_id: r.quote_id,
            revision: r.revision,
            document_hash: r.document_hash,
            rule_ids: r.rule_ids.into_iter().flatten().collect(),
            status: r.status.into(),
            requested_by: r.requested_by,
            requested_at: r.requested_at,
            request_note: r.request_note,
            decided_by: r.decided_by,
            decided_at: r.decided_at,
            decision_note: r.decision_note,
        }
    }
}

/// What publishing the quote as it stands requires.
#[derive(Debug, Clone)]
pub struct QuoteApprovalRequirement {
    pub matched_rules: Vec<QuoteApprovalRule>,
    pub latest: Option<QuoteApproval>,
}

impl QuoteApprovalRequirement {
    /// Approved for exactly this content, under every rule matching it now: an edit since, or a
    /// rule added since, needs a new approval.
    pub fn is_satisfied(&self, document_hash: Option<&str>) -> bool {
        self.matched_rules.is_empty()
            || self.latest.as_ref().is_some_and(|a| {
                a.status == QuoteApprovalStatusEnum::Approved
                    && Some(a.document_hash.as_str()) == document_hash
                    && self
                        .matched_rules
                        .iter()
                        .all(|r| a.rule_ids.contains(&r.id))
            })
    }

    pub fn status(&self, document_hash: Option<&str>) -> QuoteApprovalStatusEnum {
        if self.matched_rules.is_empty() {
            return QuoteApprovalStatusEnum::NotRequired;
        }
        match &self.latest {
            Some(a)
                if Some(a.document_hash.as_str()) == document_hash
                    && (a.status != QuoteApprovalStatusEnum::Approved
                        || self.is_satisfied(document_hash)) =>
            {
                a.status
            }
            _ => QuoteApprovalStatusEnum::NotRequired,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;

    fn rule() -> QuoteApprovalRule {
        QuoteApprovalRule {
            id: QuoteApprovalRuleId::new(),
            name: "Discounts".to_string(),
            min_discount_percent: None,
            max_net_terms_days: None,
            max_trial_days: None,
            always: true,
            approver_user_ids: vec![],
            created_at: NaiveDateTime::default(),
        }
    }

    fn approved(rule_ids: Vec<QuoteApprovalRuleId>) -> QuoteApproval {
        QuoteApproval {
            id: QuoteApprovalId::new(),
            quote_id: QuoteId::new(),
            revision: 1,
            document_hash: "h".to_string(),
            rule_ids,
            status: QuoteApprovalStatusEnum::Approved,
            requested_by: None,
            requested_at: NaiveDateTime::default(),
            request_note: None,
            decided_by: None,
            decided_at: None,
            decision_note: None,
        }
    }

    #[test]
    fn an_approval_covers_only_the_rules_it_was_given_for() {
        let (first, added) = (rule(), rule());
        let requirement = |rules: Vec<QuoteApprovalRule>, ids| QuoteApprovalRequirement {
            matched_rules: rules,
            latest: Some(approved(ids)),
        };
        assert!(requirement(vec![first.clone()], vec![first.id]).is_satisfied(Some("h")));
        assert!(!requirement(vec![first.clone()], vec![first.id]).is_satisfied(Some("edited")));
        assert!(
            !requirement(vec![first.clone(), added], vec![first.id]).is_satisfied(Some("h")),
            "a rule added since the approval needs its own"
        );
    }
}
