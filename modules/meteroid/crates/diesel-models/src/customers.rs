use chrono::NaiveDateTime;

use crate::enums::{CustomerTaxStatusEnum, CustomerTypeEnum, CustomerVatValidationStatusEnum};
use common_domain::ids::{
    ConnectedAccountId, CustomerId, CustomerPaymentMethodId, InvoicingEntityId,
    PaymentReminderSequenceId, TenantId, UserId,
};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, QueryableByName, Selectable};

#[derive(Clone, Debug, Identifiable, Queryable, QueryableByName, Selectable)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerRow {
    pub id: CustomerId,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub phone: Option<String>,
    /// Retired and frozen at its carried value; the customer credit ledger holds the live balance.
    pub balance_value_cents: i64,
    pub currency: String,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: InvoicingEntityId,
    pub current_payment_method_id: Option<CustomerPaymentMethodId>,
    pub vat_number: Option<String>,
    pub invoicing_emails: Vec<Option<String>>,
    pub conn_meta: Option<serde_json::Value>,
    //  logo_url -> Nullable<Text>,
    //  website_url -> Nullable<Text>,
    pub vat_number_format_valid: bool,
    pub custom_taxes: serde_json::Value,
    pub connected_account_id: Option<ConnectedAccountId>,
    pub vat_number_validation_status: Option<CustomerVatValidationStatusEnum>,
    pub vat_number_checked_at: Option<NaiveDateTime>,
    pub vat_number_vies_check: Option<serde_json::Value>,
    /// Trigger-maintained analytics ingestion marker; never written by app code.
    pub store_committed_at: NaiveDateTime,
    pub custom_properties: serde_json::Value,
    pub customer_type: CustomerTypeEnum,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub tax_status: CustomerTaxStatusEnum,
    pub exemption_reason: Option<String>,
    /// BT-47 — the buyer's national register identifier (SIREN/SIRET, HRB).
    pub legal_number: Option<String>,
    pub preferred_locales: Vec<Option<String>>,
    /// Internal user who owns this account (ADR-0002). Nullable; set/unset via the
    /// dedicated assign path, which validates org membership.
    pub owner_user_id: Option<UserId>,
    pub payment_reminders_opt_out: bool,
    /// Override sequence for this customer's invoices; NULL = resolve by audience.
    pub payment_reminder_sequence_id: Option<PaymentReminderSequenceId>,
    /// BT-10 — the reference the buyer routes invoices by (Leitweg-ID).
    pub buyer_reference: Option<String>,
}

/// A `CustomerRow` plus the derived list metrics (MRR + lifecycle_state) computed
/// by `CustomerRow::list_with_metrics`. `total_count` is a windowed count for pagination.
#[derive(Debug, QueryableByName)]
pub struct CustomerWithMetricsRow {
    #[diesel(embed)]
    pub customer: CustomerRow,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub mrr_cents: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub lifecycle_state: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub total_count: i64,
}

/// The lifecycle presence flags for a single customer, mirroring the correlated
/// EXISTS columns of `CustomerProjectionRow` so the badge stays consistent with
/// the analytics projection.
#[derive(QueryableByName, Debug)]
pub struct CustomerLifecycleSignalRow {
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub has_active_paid_sub: bool,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub is_trialing: bool,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub has_active_free_sub: bool,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub has_active_sub: bool,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub has_suspended_sub: bool,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub has_paused_sub: bool,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub had_paid_sub_ended: bool,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub ever_paid: bool,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub trial_expired: bool,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub has_closed_lost_quote: bool,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub has_open_quote: bool,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub reactivated: bool,
}

#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerBriefRow {
    pub id: CustomerId,
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerRowNew {
    pub id: CustomerId,
    pub name: String,
    pub tenant_id: TenantId,
    pub alias: Option<String>,
    pub phone: Option<String>,
    pub currency: String,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: InvoicingEntityId,
    // for seed, else default to None
    pub created_at: Option<NaiveDateTime>,
    pub billing_email: Option<String>,
    pub current_payment_method_id: Option<CustomerPaymentMethodId>,
    pub vat_number: Option<String>,
    pub invoicing_emails: Vec<Option<String>>,
    pub tax_status: CustomerTaxStatusEnum,
    pub exemption_reason: Option<String>,
    pub custom_taxes: serde_json::Value,
    pub vat_number_format_valid: bool,
    pub connected_account_id: Option<ConnectedAccountId>,
    pub vat_number_validation_status: Option<CustomerVatValidationStatusEnum>,
    pub custom_properties: serde_json::Value,
    /// BT-47 — the buyer's national register identifier (SIREN/SIRET, HRB).
    pub legal_number: Option<String>,
    pub preferred_locales: Vec<Option<String>>,
    pub customer_type: CustomerTypeEnum,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub buyer_reference: Option<String>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerRowPatch {
    pub id: CustomerId,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Option<Vec<Option<String>>>,
    pub phone: Option<String>,
    pub currency: Option<String>,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    pub vat_number: Option<Option<String>>,
    pub current_payment_method_id: Option<Option<CustomerPaymentMethodId>>,
    pub tax_status: Option<CustomerTaxStatusEnum>,
    pub exemption_reason: Option<Option<String>>,
    pub custom_taxes: Option<serde_json::Value>,
    pub vat_number_format_valid: Option<bool>,
    pub connected_account_id: Option<Option<ConnectedAccountId>>,
    // outer None = leave untouched; Some(_) = reset on vat number change
    pub vat_number_validation_status: Option<Option<CustomerVatValidationStatusEnum>>,
    pub vat_number_checked_at: Option<Option<NaiveDateTime>>,
    pub vat_number_vies_check: Option<Option<serde_json::Value>>,
    pub custom_properties: Option<serde_json::Value>,
    pub legal_number: Option<Option<String>>,
    // None = leave untouched; an empty list = reset to "inherit"
    pub preferred_locales: Option<Vec<Option<String>>>,
    pub customer_type: Option<CustomerTypeEnum>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub buyer_reference: Option<Option<String>>,
    // owner_user_id is intentionally absent: as an AsChangeset it is never in the SET
    // clause, so a patch preserves the owner. Owner mutation goes through assign_owner.
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(treat_none_as_null = true)]
pub struct CustomerRowUpdate {
    pub id: CustomerId,
    pub name: String,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Vec<Option<String>>,
    pub phone: Option<String>,
    pub currency: String,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: InvoicingEntityId,
    pub vat_number: Option<String>,
    pub tax_status: CustomerTaxStatusEnum,
    pub exemption_reason: Option<String>,
    pub custom_taxes: serde_json::Value,
    pub vat_number_format_valid: bool,
    pub vat_number_validation_status: Option<CustomerVatValidationStatusEnum>,
    pub vat_number_checked_at: Option<NaiveDateTime>,
    pub vat_number_vies_check: Option<serde_json::Value>,
    pub custom_properties: serde_json::Value,
    /// BT-47 — the buyer's national register identifier (SIREN/SIRET, HRB).
    pub legal_number: Option<String>,
    pub preferred_locales: Vec<Option<String>>,
    // NOT NULL in DB; always set on full-replace update (never NULL under treat_none_as_null).
    pub customer_type: CustomerTypeEnum,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub buyer_reference: Option<String>,
    // owner_user_id intentionally absent: a replace-update never touches the owner column,
    // so ownership is preserved. Owner mutation goes through assign_owner only.
}
