use super::ledger_export::movement_label;
use super::mapping;
use meteroid_grpc::meteroid::api::wallets::v1 as server;
use meteroid_store::domain::credit::{CreditGrantSource, GrantClassification};
use meteroid_store::repositories::credit::activity::{
    CreditAddedGroup, GrantActivity, LabelledLedgerEntry, WalletActivity,
};

fn grant_type(classification: GrantClassification) -> server::CreditGrantType {
    match classification {
        GrantClassification::Paid => server::CreditGrantType::Paid,
        GrantClassification::Promotional => server::CreditGrantType::Promotional,
    }
}

fn added_group(group: &CreditAddedGroup) -> server::CreditAddedGroup {
    server::CreditAddedGroup {
        source: match (group.source, group.added_by_member) {
            (CreditGrantSource::Manual, true) => server::CreditGrantSource::Manual,
            (source, _) => mapping::grant_source(source, None),
        }
        .into(),
        r#type: grant_type(group.classification).into(),
        name: group.name.clone(),
        grants: group.grants,
        amount: group.amount_minor,
    }
}

pub(crate) fn grant_activity(grant: &GrantActivity) -> server::GrantActivity {
    server::GrantActivity {
        credit_grant_id: grant.grant_id.as_proto(),
        name: grant.name.clone(),
        invoiced: grant.invoiced_minor,
        settled_at_expiry: grant.settled_at_expiry_minor,
        unbilled: grant.unbilled_minor,
        removed: grant.removed_minor,
        rolled_over: grant.rolled_over_minor,
        invoice_number: grant.invoice_number.clone(),
        automatic_top_up: grant.automatic_top_up,
    }
}

pub fn wallet_activity(activity: &WalletActivity) -> server::WalletActivity {
    server::WalletActivity {
        wallet_id: activity.wallet_id.as_proto(),
        credit_added: activity.credit_added_minor,
        credit_added_groups: activity
            .credit_added_groups
            .iter()
            .map(added_group)
            .collect(),
        invoiced: activity.invoiced_minor,
        settled_at_expiry: activity.settled_at_expiry_minor,
        expired: activity.expired_minor,
        removed: activity.removed_minor,
        next_expiry: activity.next_expiry.map(mapping::timestamp),
        grants: activity
            .grants
            .iter()
            .filter(|grant| !grant.used_up)
            .map(grant_activity)
            .collect(),
    }
}

pub fn labelled_ledger_entry(entry: &LabelledLedgerEntry) -> server::CustomerLedgerEntry {
    server::CustomerLedgerEntry {
        entry: Some(mapping::ledger_entry(&entry.entry)),
        currency: entry.currency.clone(),
        credit_unit: entry.credit_unit.as_ref().map(mapping::credit_unit_label),
        grant_source: mapping::grant_source(entry.grant_source, entry.grant_created_by).into(),
        grant_type: grant_type(entry.grant_classification).into(),
        grant_name: entry.grant_name.clone(),
        invoice_number: entry.invoice_number.clone(),
        credit_note_number: entry.credit_note_number.clone(),
        automatic_top_up: entry.automatic_top_up,
        label: movement_label(entry),
    }
}
