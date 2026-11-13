//! The customer's credit statement as a CSV download: a short-lived signed link, streamed page by
//! page with the columns and words of the dashboard's statement.
use bytes::Bytes;
use chrono::SecondsFormat;
use common_domain::ids::{CustomerId, LedgerEntryId, SubscriptionId, TenantId, WalletId};
use error_stack::Report;
use futures::Stream;
use meteroid_store::constants::Currencies;
use meteroid_store::domain::credit::{
    CreditGrantSource, GrantClassification, LedgerEntryKind, is_removal_reason,
};
use meteroid_store::domain::credit_ledger_labels::LedgerMovement;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::credit::activity::LabelledLedgerEntry;
use meteroid_store::repositories::credit::{
    CustomerCreditActivityInterface, CustomerCreditWalletsInterface,
};
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::subscriptions::SubscriptionInterface;
use meteroid_store::{Store, StoreResult};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const SUBJECT: &str = "credit-ledger";
const LINK_MINUTES: i64 = 5;
const PAGE_SIZE: i64 = 1_000;

pub const HEADER: [&str; 10] = [
    "Date (UTC)",
    "Wallet",
    "Movement",
    "Description",
    "Reference",
    "Amount",
    "Credit on account after",
    "Currency",
    "Invoice",
    "Credit note",
];

/// The statement exported: the customer's wallets (or one), optionally one subscription's credit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerScope {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub wallet_id: Option<WalletId>,
    pub subscription_id: Option<SubscriptionId>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LedgerExportClaims {
    sub: String,
    exp: usize,
    scope: LedgerScope,
}

pub fn sign(
    scope: LedgerScope,
    secret: &SecretString,
) -> Result<String, jsonwebtoken::errors::Error> {
    let claims = LedgerExportClaims {
        sub: SUBJECT.to_owned(),
        exp: (common_domain::clock::wall() + chrono::Duration::minutes(LINK_MINUTES)).timestamp()
            as usize,
        scope,
    };
    meteroid_store::jwt::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.expose_secret().as_bytes()),
    )
}

/// The scope a valid, unexpired export link names.
pub fn verify(token: &str, secret: &SecretString) -> Option<LedgerScope> {
    meteroid_store::jwt::decode::<LedgerExportClaims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(secret.expose_secret().as_bytes()),
        &jsonwebtoken::Validation::default(),
    )
    .ok()
    .filter(|data| data.claims.sub == SUBJECT)
    .map(|data| data.claims.scope)
}

fn not_found(what: &str) -> Report<StoreError> {
    Report::new(StoreError::ValueNotFound(what.to_owned()))
}

/// Refuses a customer of another tenant, and a wallet or a subscription of another customer.
pub async fn check_scope(store: &Store, scope: &LedgerScope) -> StoreResult<()> {
    store
        .find_customer_by_id(scope.customer_id, scope.tenant_id)
        .await?;
    if let Some(wallet_id) = scope.wallet_id
        && !store
            .is_customer_wallet(scope.tenant_id, scope.customer_id, wallet_id)
            .await?
    {
        return Err(not_found("wallet"));
    }
    if let Some(subscription_id) = scope.subscription_id
        && store
            .get_subscription(scope.tenant_id, subscription_id)
            .await?
            .customer_id
            != scope.customer_id
    {
        return Err(not_found("subscription"));
    }
    Ok(())
}

/// Text a spreadsheet would run as a formula is kept as text.
fn text_cell(value: &str) -> String {
    if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}

/// What a removal or a return took back, unless its label already says it.
fn removal_detail(row: &LabelledLedgerEntry, label: &str) -> Option<String> {
    let reason = &row.entry.reason;
    if !reason.is_empty() && reason == label {
        return Some(row.grant_name.clone()).filter(|name| !name.is_empty());
    }
    Some(reason.clone()).filter(|reason| !reason.is_empty())
}

/// "Monthly credit (free)", the part after the movement label.
fn description(row: &LabelledLedgerEntry, label: &str) -> Option<String> {
    if is_removal_reason(&row.entry.reason) {
        return removal_detail(row, label);
    }
    match row.entry.kind {
        LedgerEntryKind::Fund => {
            // The labels of these sources already say whether the credit is free.
            let self_described = matches!(
                row.grant_source,
                CreditGrantSource::Included
                    | CreditGrantSource::Rollover
                    | CreditGrantSource::Purchase
                    | CreditGrantSource::Coupon
                    | CreditGrantSource::Manual
            );
            Some(
                if row.grant_classification == GrantClassification::Promotional && !self_described {
                    format!("{} (free)", row.grant_name)
                } else {
                    row.grant_name.clone()
                },
            )
        }
        LedgerEntryKind::Expiry => Some(row.grant_name.clone()),
        LedgerEntryKind::Reversal
        | LedgerEntryKind::DrawReversal
        | LedgerEntryKind::Reinstatement => removal_detail(row, label),
        // The dashboard reads "Applied to invoice INV-0023".
        LedgerEntryKind::Draw => Some(invoice(row))
            .filter(|invoice| !invoice.is_empty())
            .map(|invoice| format!("Applied to invoice {invoice}")),
        LedgerEntryKind::Rollover => None,
    }
}

fn invoice(row: &LabelledLedgerEntry) -> String {
    row.invoice_number
        .clone()
        .or_else(|| row.entry.invoice_id.map(|id| id.as_proto()))
        .unwrap_or_default()
}

fn amount(minor: i64, row: &LabelledLedgerEntry) -> String {
    let precision = row.credit_unit.as_ref().map_or_else(
        || Currencies::resolve_currency_precision(&row.currency).unwrap_or(2),
        |unit| unit.precision,
    );
    Decimal::new(minor, u32::from(precision)).to_string()
}

pub fn csv_row(row: &LabelledLedgerEntry) -> [String; 10] {
    let entry = &row.entry;
    let label = movement_label(row);
    [
        entry
            .effective_at
            .to_rfc3339_opts(SecondsFormat::Millis, true),
        row.credit_unit
            .as_ref()
            .map_or_else(|| row.currency.clone(), |unit| unit.plural_name.clone()),
        label.clone(),
        text_cell(&description(row, &label).unwrap_or_default()),
        text_cell(entry.reference.as_deref().unwrap_or_default()),
        amount(entry.amount_minor, row),
        amount(entry.balance_after_minor, row),
        row.currency.clone(),
        invoice(row),
        row.credit_note_number
            .clone()
            .or_else(|| entry.credit_note_id.map(|id| id.as_proto()))
            .unwrap_or_default(),
    ]
}

fn csv_chunk(header: bool, rows: &[[String; 10]]) -> Result<Bytes, std::io::Error> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    if header {
        writer.write_record(HEADER)?;
    }
    for row in rows {
        writer.write_record(row)?;
    }
    writer
        .into_inner()
        .map(Bytes::from)
        .map_err(|error| std::io::Error::other(error.to_string()))
}

struct Cursor {
    store: Store,
    scope: LedgerScope,
    offset: i64,
    seen: HashSet<LedgerEntryId>,
    done: bool,
}

/// The statement, newest first, a page per chunk. Movements recorded meanwhile shift older ones
/// to later pages: those repeats are dropped, and the ledger being append-only, none is missed.
pub fn csv_stream(
    store: Store,
    scope: LedgerScope,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    let cursor = Cursor {
        store,
        scope,
        offset: 0,
        seen: HashSet::new(),
        done: false,
    };
    futures::stream::unfold(cursor, |mut cursor| async move {
        if cursor.done {
            return None;
        }
        let scope = cursor.scope;
        let page = cursor
            .store
            .customer_credit_labelled_ledger(
                scope.tenant_id,
                scope.customer_id,
                scope.wallet_id,
                scope.subscription_id,
                PAGE_SIZE,
                cursor.offset,
            )
            .await;
        let entries = match page {
            Ok((entries, _)) => entries,
            Err(error) => {
                log::error!("Error exporting a credit ledger: {error:?}");
                cursor.done = true;
                return Some((
                    Err(std::io::Error::other("the credit ledger could not be read")),
                    cursor,
                ));
            }
        };
        let header = cursor.offset == 0;
        cursor.done = (entries.len() as i64) < PAGE_SIZE;
        cursor.offset += PAGE_SIZE;
        let rows = entries
            .iter()
            .filter(|entry| cursor.seen.insert(entry.entry.id))
            .map(csv_row)
            .collect::<Vec<_>>();
        Some((csv_chunk(header, &rows), cursor))
    })
}

/// What the movement was, as the dashboard and the portal word it.
pub fn movement_label(entry: &LabelledLedgerEntry) -> String {
    LedgerMovement {
        entry: &entry.entry,
        source: Some(entry.grant_source),
        classification: Some(entry.grant_classification),
        bonus: entry.grant_bonus,
        automatic_top_up: entry.automatic_top_up,
    }
    .label()
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use common_domain::ids::{BaseId, CreditGrantId, InvoiceId};
    use meteroid_store::domain::credit::{
        CANCELLATION_REMOVAL, CreditApplication, CreditLedgerEntry,
    };

    fn row(kind: LedgerEntryKind, amount_minor: i64, reason: &str) -> LabelledLedgerEntry {
        LabelledLedgerEntry {
            entry: CreditLedgerEntry {
                id: LedgerEntryId::new(),
                wallet_id: WalletId::new(),
                grant_id: CreditGrantId::new(),
                sequence: 1,
                kind,
                amount_minor,
                balance_after_minor: 12_345,
                invoice_id: None,
                credit_note_id: None,
                reason: reason.to_owned(),
                reference: None,
                created_by: None,
                effective_at: Utc.with_ymd_and_hms(2026, 3, 1, 10, 30, 0).unwrap(),
                recorded_at: Utc.with_ymd_and_hms(2026, 3, 1, 10, 30, 0).unwrap(),
                cost: None,
                application: CreditApplication::Invoice,
                subscription_id: None,
            },
            currency: "EUR".to_owned(),
            credit_unit: None,
            grant_source: CreditGrantSource::Manual,
            grant_classification: GrantClassification::Paid,
            grant_created_by: None,
            grant_name: "Goodwill".to_owned(),
            invoice_number: None,
            credit_note_number: None,
            automatic_top_up: false,
            grant_bonus: false,
        }
    }

    #[test]
    fn a_row_reads_as_the_dashboard_statement_in_the_wallet_precision() {
        let mut funded = row(LedgerEntryKind::Fund, 1_050, "Goodwill");
        funded.entry.reference = Some("TICKET-42".to_owned());
        assert_eq!(
            csv_row(&funded),
            [
                "2026-03-01T10:30:00.000Z",
                "EUR",
                "Credit added",
                "Goodwill",
                "TICKET-42",
                "10.50",
                "123.45",
                "EUR",
                "",
                "",
            ]
            .map(str::to_owned)
        );

        let mut applied = row(LedgerEntryKind::Draw, -300, "");
        assert_eq!(csv_row(&applied)[3], "");
        let invoice = InvoiceId::new();
        applied.entry.invoice_id = Some(invoice);
        assert_eq!(
            csv_row(&applied)[3],
            format!("Applied to invoice {}", invoice.as_proto())
        );
        assert_eq!(csv_row(&applied)[5], "-3.00");
        assert_eq!(csv_row(&applied)[8], invoice.as_proto());
        applied.invoice_number = Some("INV-0007".to_owned());
        assert_eq!(csv_row(&applied)[3], "Applied to invoice INV-0007");
        assert_eq!(csv_row(&applied)[8], "INV-0007");
    }

    #[test]
    fn free_credit_is_named_so_unless_its_label_says_it() {
        let mut returned = row(LedgerEntryKind::Fund, 500, "");
        returned.grant_source = CreditGrantSource::CreditNote;
        returned.grant_classification = GrantClassification::Promotional;
        assert_eq!(csv_row(&returned)[3], "Goodwill (free)");
        returned.grant_source = CreditGrantSource::Coupon;
        assert_eq!(csv_row(&returned)[3], "Goodwill");
    }

    #[test]
    fn a_removal_named_by_its_label_shows_its_credit_instead() {
        let removed = row(LedgerEntryKind::Reversal, -500, CANCELLATION_REMOVAL);
        let cells = csv_row(&removed);
        assert_eq!(cells[2], CANCELLATION_REMOVAL);
        assert_eq!(cells[3], "Goodwill");
        let clawed = row(LedgerEntryKind::Reversal, -500, "Clawback, ticket 9");
        assert_eq!(csv_row(&clawed)[3], "Clawback, ticket 9");
    }

    #[test]
    fn text_a_spreadsheet_would_run_is_kept_as_text() {
        let mut row = row(LedgerEntryKind::Fund, 100, "");
        row.grant_name = "=HYPERLINK(\"x\")".to_owned();
        row.entry.reference = Some("+33 6".to_owned());
        let cells = csv_row(&row);
        assert_eq!(cells[3], "'=HYPERLINK(\"x\")");
        assert_eq!(cells[4], "'+33 6");
        let chunk = csv_chunk(true, &[cells]).unwrap();
        let text = std::str::from_utf8(&chunk).unwrap();
        assert!(text.starts_with("Date (UTC),Wallet,Movement,"), "{text}");
        assert!(text.contains("\"'=HYPERLINK(\"\"x\"\")\""), "{text}");
    }

    #[test]
    fn a_link_opens_only_the_scope_it_was_signed_for() {
        let secret = SecretString::from("export-secret".to_owned());
        let scope = LedgerScope {
            tenant_id: TenantId::new(),
            customer_id: CustomerId::new(),
            wallet_id: Some(WalletId::new()),
            subscription_id: None,
        };
        let token = sign(scope, &secret).unwrap();
        assert_eq!(verify(&token, &secret), Some(scope));
        assert_eq!(
            verify(&token, &SecretString::from("other-secret".to_owned())),
            None
        );
    }
}
