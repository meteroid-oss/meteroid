use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

use crate::domain::enums::{QuoteStatusEnum, SubscriptionActivationCondition};
use crate::domain::{Customer, InvoicingEntity, SubscriptionFee};
use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use common_domain::ids::BaseId;
use common_domain::ids::{
    CustomerId, InvoiceId, PlanVersionId, PriceComponentId, ProductId, QuoteActivityId, QuoteId,
    QuotePriceComponentId, QuoteSignatureId, StoredDocumentId, SubscriptionId, TenantId,
};
use diesel_models::enums::SubscriptionFeeBillingPeriod;
use diesel_models::quotes::{
    QuoteActivityRow, QuoteActivityRowNew, QuoteComponentRow, QuoteComponentRowNew, QuoteRow,
    QuoteRowNew, QuoteSignatureRow, QuoteSignatureRowNew, QuoteWithCustomerRow,
};
use o2o::o2o;

#[derive(o2o, Debug, Clone)]
#[try_from_owned(QuoteRow, StoreErrorReport)]
pub struct Quote {
    pub id: QuoteId,
    #[from(~.into())]
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
    pub billing_start_date: NaiveDate,
    pub billing_end_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i32>,
    #[from(~.into())]
    pub activation_condition: SubscriptionActivationCondition,
    // Quote-specific fields
    pub valid_until: Option<NaiveDateTime>,
    pub expires_at: Option<NaiveDateTime>,
    pub accepted_at: Option<NaiveDateTime>,
    pub declined_at: Option<NaiveDateTime>,
    pub internal_notes: Option<String>,
    pub cover_image: Option<StoredDocumentId>,
    // the markdown text before pricing
    pub overview: Option<String>,
    // the markdown text after pricing
    pub terms_and_services: Option<String>,
    pub net_terms: i32,
    pub attachments: Vec<Option<StoredDocumentId>>,
    pub pdf_document_id: Option<StoredDocumentId>,
    pub sharing_key: Option<String>,
    pub converted_to_invoice_id: Option<InvoiceId>,
    pub converted_to_subscription_id: Option<SubscriptionId>,
    pub converted_at: Option<NaiveDateTime>,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize recipients".to_string(), e)
    }) ?)]
    pub recipients: Vec<RecipientDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipientDetails {
    pub name: String,
    pub email: String,
}

json_value_serde!(RecipientDetails);

#[derive(o2o, Debug, Clone)]
#[owned_try_into(QuoteRowNew, StoreErrorReport)]
pub struct QuoteNew {
    pub id: QuoteId,
    #[map(~.into())]
    pub status: QuoteStatusEnum,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: PlanVersionId,
    pub currency: String,
    pub quote_number: String,
    // Subscription-like fields
    pub trial_duration_days: Option<i32>,
    pub billing_start_date: NaiveDate,
    pub billing_end_date: Option<NaiveDate>,
    pub billing_day_anchor: Option<i32>,
    #[map(~.into())]
    pub activation_condition: SubscriptionActivationCondition,
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
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize recipients".to_string(), e)
    }) ?)]
    pub recipients: Vec<RecipientDetails>,
}

#[derive(o2o, Debug, Clone)]
#[try_from_owned(QuoteWithCustomerRow, StoreErrorReport)]
pub struct QuoteWithCustomer {
    #[from(~.try_into()?)]
    pub quote: Quote,
    #[from(~.try_into()?)]
    pub customer: Customer,
}

#[derive(Debug, Clone)]
pub struct DetailedQuote {
    pub quote: Quote,
    pub customer: Customer,
    pub invoicing_entity: InvoicingEntity,
    pub components: Vec<QuotePriceComponent>,
    pub signatures: Vec<QuoteSignature>,
    pub activities: Vec<QuoteActivity>,
}

#[derive(Debug, Clone, o2o)]
#[try_from_owned(QuoteComponentRow, StoreErrorReport)]
pub struct QuotePriceComponent {
    pub id: QuotePriceComponentId,
    pub name: String,
    pub quote_id: QuoteId,
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub period: SubscriptionFeeBillingPeriod,
    #[from(~.try_into()?)]
    pub fee: SubscriptionFee,
    pub is_override: bool,
}

#[derive(Debug, Clone, o2o)]
#[owned_try_into(QuoteComponentRowNew, StoreErrorReport)]
#[ghosts(id: {QuotePriceComponentId::new()})]
pub struct QuotePriceComponentNew {
    pub name: String,
    pub quote_id: QuoteId,
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub period: SubscriptionFeeBillingPeriod,
    #[into(~.try_into()?)]
    pub fee: SubscriptionFee,
    pub is_override: bool,
}

#[derive(o2o, Debug, Clone)]
#[from_owned(QuoteSignatureRow)]
pub struct QuoteSignature {
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

#[derive(o2o, Debug, Clone)]
#[from_owned(QuoteActivityRow)]
pub struct QuoteActivity {
    pub id: QuoteActivityId,
    pub quote_id: QuoteId,
    pub activity_type: String,
    pub description: String,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub created_at: NaiveDateTime,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, o2o)]
#[owned_into(QuoteActivityRowNew)]
#[ghosts(id: {QuoteActivityId::new()})]
pub struct QuoteActivityNew {
    pub quote_id: QuoteId,
    pub activity_type: String,
    pub description: String,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, o2o)]
#[owned_into(QuoteSignatureRowNew)]
#[ghosts(id: {QuoteSignatureId::new()})]
pub struct QuoteSignatureNew {
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
