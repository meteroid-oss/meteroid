use crate::enums::{
    QuoteStatusEnum, SubscriptionActivationConditionEnum, SubscriptionFeeBillingPeriod,
};
use chrono::{NaiveDate, NaiveDateTime};

use crate::customers::CustomerRow;
use common_domain::ids::{
    CustomerId, InvoiceId, PlanVersionId, PriceComponentId, PriceId, ProductId, QuoteId,
    QuotePriceComponentId, QuoteSignatureId, StoredDocumentId, SubscriptionId, TenantId,
};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::quote)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteRow {
    pub id: QuoteId,
    pub status: QuoteStatusEnum,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: PlanVersionId,
    pub currency: String,
    pub quote_number: String,
    // Subscription-like fields
    pub trial_duration_days: Option<i32>,
    pub billing_start_date: Option<NaiveDate>,
    pub billing_end_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i32>,
    pub activation_condition: SubscriptionActivationConditionEnum,
    // Quote-specific fields
    pub valid_until: Option<NaiveDateTime>,
    pub expires_at: Option<NaiveDateTime>,
    pub accepted_at: Option<NaiveDateTime>,
    pub declined_at: Option<NaiveDateTime>,
    pub internal_notes: Option<String>,
    pub cover_image: Option<StoredDocumentId>,
    pub overview: Option<String>,
    pub terms_and_services: Option<String>,
    pub net_terms: i32,
    pub attachments: Vec<Option<StoredDocumentId>>,
    pub pdf_document_id: Option<StoredDocumentId>,
    pub sharing_key: Option<String>,
    pub converted_to_invoice_id: Option<InvoiceId>,
    pub converted_to_subscription_id: Option<SubscriptionId>,
    pub converted_at: Option<NaiveDateTime>,
    pub recipients: serde_json::Value,
    pub purchase_order: Option<String>,
    // Payment configuration fields
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub create_subscription_on_acceptance: bool,
    pub payment_methods_config: Option<serde_json::Value>,
    /// User-defined property values, keyed by definition `key`.
    pub custom_properties: serde_json::Value,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::quote)]
pub struct QuoteRowNew {
    pub id: QuoteId,
    pub status: QuoteStatusEnum,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: PlanVersionId,
    pub currency: String,
    pub quote_number: String,
    // Subscription-like fields
    pub trial_duration_days: Option<i32>,
    pub billing_start_date: Option<NaiveDate>,
    pub billing_end_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i32>,
    pub activation_condition: SubscriptionActivationConditionEnum,
    // Quote-specific fields
    pub valid_until: Option<NaiveDateTime>,
    pub expires_at: Option<NaiveDateTime>,
    pub internal_notes: Option<String>,
    pub cover_image: Option<StoredDocumentId>,
    pub overview: Option<String>,
    pub terms_and_services: Option<String>,
    pub net_terms: i32,
    pub attachments: Vec<Option<StoredDocumentId>>,
    pub pdf_document_id: Option<StoredDocumentId>,
    pub sharing_key: Option<String>,
    pub recipients: serde_json::Value,
    // Payment configuration fields
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub create_subscription_on_acceptance: bool,
    pub payment_methods_config: Option<serde_json::Value>,
    /// User-defined property values, keyed by definition `key`.
    pub custom_properties: serde_json::Value,
}

#[derive(Debug, AsChangeset, Default)]
#[diesel(table_name = crate::schema::quote)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteRowUpdate {
    pub status: Option<QuoteStatusEnum>,
    // Subscription-like fields
    pub trial_duration_days: Option<Option<i32>>,
    pub billing_start_date: Option<Option<NaiveDate>>,
    pub billing_end_date: Option<Option<NaiveDate>>,
    pub billing_day_anchor: Option<Option<i32>>,
    pub activation_condition: Option<SubscriptionActivationConditionEnum>,
    // Quote-specific fields
    pub valid_until: Option<Option<NaiveDateTime>>,
    pub expires_at: Option<Option<NaiveDateTime>>,
    pub accepted_at: Option<Option<NaiveDateTime>>,
    pub declined_at: Option<Option<NaiveDateTime>>,
    pub internal_notes: Option<Option<String>>,
    pub cover_image: Option<Option<StoredDocumentId>>,
    pub overview: Option<Option<String>>,
    pub terms_and_services: Option<Option<String>>,
    pub net_terms: Option<i32>,
    pub attachments: Option<Vec<Option<StoredDocumentId>>>,
    pub pdf_document_id: Option<Option<StoredDocumentId>>,
    pub sharing_key: Option<Option<String>>,
    pub converted_to_invoice_id: Option<Option<InvoiceId>>,
    pub converted_to_subscription_id: Option<Option<SubscriptionId>>,
    pub converted_at: Option<Option<NaiveDateTime>>,
    pub recipients: Option<serde_json::Value>,
    pub updated_at: Option<NaiveDateTime>,
    // Payment configuration fields
    pub auto_advance_invoices: Option<bool>,
    pub charge_automatically: Option<bool>,
    pub invoice_memo: Option<Option<String>>,
    pub invoice_threshold: Option<Option<rust_decimal::Decimal>>,
    pub create_subscription_on_acceptance: Option<bool>,
    pub payment_methods_config: Option<Option<serde_json::Value>>,
}

/// Full replacement of the editable columns of a draft quote; `None` clears the column.
#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::quote)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(treat_none_as_null = true)]
pub struct QuoteRowReplace {
    pub customer_id: CustomerId,
    pub plan_version_id: PlanVersionId,
    pub currency: String,
    pub quote_number: String,
    pub trial_duration_days: Option<i32>,
    pub billing_start_date: Option<NaiveDate>,
    pub billing_end_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i32>,
    pub activation_condition: SubscriptionActivationConditionEnum,
    pub valid_until: Option<NaiveDateTime>,
    pub expires_at: Option<NaiveDateTime>,
    pub internal_notes: Option<String>,
    pub cover_image: Option<StoredDocumentId>,
    pub overview: Option<String>,
    pub terms_and_services: Option<String>,
    pub net_terms: i32,
    pub attachments: Vec<Option<StoredDocumentId>>,
    pub pdf_document_id: Option<StoredDocumentId>,
    pub recipients: serde_json::Value,
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub create_subscription_on_acceptance: bool,
    pub payment_methods_config: Option<serde_json::Value>,
    pub custom_properties: serde_json::Value,
    pub updated_at: Option<NaiveDateTime>,
}

impl From<QuoteRowNew> for QuoteRowReplace {
    fn from(row: QuoteRowNew) -> Self {
        Self {
            customer_id: row.customer_id,
            plan_version_id: row.plan_version_id,
            currency: row.currency,
            quote_number: row.quote_number,
            trial_duration_days: row.trial_duration_days,
            billing_start_date: row.billing_start_date,
            billing_end_date: row.billing_end_date,
            billing_day_anchor: row.billing_day_anchor,
            activation_condition: row.activation_condition,
            valid_until: row.valid_until,
            expires_at: row.expires_at,
            internal_notes: row.internal_notes,
            cover_image: row.cover_image,
            overview: row.overview,
            terms_and_services: row.terms_and_services,
            net_terms: row.net_terms,
            attachments: row.attachments,
            pdf_document_id: None,
            recipients: row.recipients,
            auto_advance_invoices: row.auto_advance_invoices,
            charge_automatically: row.charge_automatically,
            invoice_memo: row.invoice_memo,
            invoice_threshold: row.invoice_threshold,
            create_subscription_on_acceptance: row.create_subscription_on_acceptance,
            payment_methods_config: row.payment_methods_config,
            custom_properties: row.custom_properties,
            updated_at: Some(common_domain::clock::now().naive_utc()),
        }
    }
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteWithCustomerRow {
    #[diesel(embed)]
    pub quote: QuoteRow,
    #[diesel(embed)]
    pub customer: CustomerRow,
}

#[derive(Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::quote_signature)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteSignatureRow {
    pub id: QuoteSignatureId,
    pub quote_id: QuoteId,
    pub signed_by_name: String,
    pub signed_by_email: String,
    pub signed_by_title: Option<String>,
    pub signature_data: Option<String>,
    pub signature_method: String,
    pub signed_at: NaiveDateTime,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub verification_token: Option<String>,
    pub verified_at: Option<NaiveDateTime>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::quote_signature)]
pub struct QuoteSignatureRowNew {
    pub id: QuoteSignatureId,
    pub quote_id: QuoteId,
    pub signed_by_name: String,
    pub signed_by_email: String,
    pub signed_by_title: Option<String>,
    pub signature_data: Option<String>,
    pub signature_method: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub verification_token: Option<String>,
}

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::quote_component)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QuoteComponentRow {
    pub id: QuotePriceComponentId,
    pub name: String,
    pub quote_id: QuoteId,
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub period: SubscriptionFeeBillingPeriod,
    pub legacy_fee: Option<serde_json::Value>,
    pub is_override: bool,
    pub price_id: Option<PriceId>,
    pub example_usage_quantity: Option<rust_decimal::Decimal>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::quote_component)]
pub struct QuoteComponentRowNew {
    pub id: QuotePriceComponentId,
    pub name: String,
    pub quote_id: QuoteId,
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub period: SubscriptionFeeBillingPeriod,
    pub legacy_fee: Option<serde_json::Value>,
    pub is_override: bool,
    pub price_id: Option<PriceId>,
    pub example_usage_quantity: Option<rust_decimal::Decimal>,
}
