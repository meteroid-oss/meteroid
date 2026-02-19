use chrono::{NaiveDate, NaiveDateTime};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::domain::{
    Customer, InvoicingEntity, QuoteStatusEnum, SubscriptionActivationCondition, SubscriptionFee,
    SubscriptionFeeBillingPeriod, subscriptions::PaymentMethodsConfig,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use common_domain::ids::BaseId;
use common_domain::ids::{
    AddOnId, CouponId, CustomerId, InvoiceId, PlanVersionId, PriceComponentId, PriceId, ProductId,
    QuoteActivityId, QuoteAddOnId, QuoteCouponId, QuoteId, QuotePriceComponentId, QuoteSignatureId,
    StoredDocumentId, SubscriptionId, TenantId,
};
use diesel_models::quote_add_ons::{QuoteAddOnRow, QuoteAddOnRowNew};
use diesel_models::quote_coupons::{QuoteCouponRow, QuoteCouponRowNew};
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
    pub billing_start_date: Option<NaiveDate>,
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
    pub purchase_order: Option<String>,
    // Payment configuration fields
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub create_subscription_on_acceptance: bool,
    #[from(~.map(serde_json::from_value).transpose().map_err(|e| {
        StoreError::SerdeError("Failed to deserialize payment_methods_config".to_string(), e)
    })?)]
    pub payment_methods_config: Option<PaymentMethodsConfig>,
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
    pub billing_start_date: Option<NaiveDate>,
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
    // Payment configuration fields
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<Decimal>,
    pub create_subscription_on_acceptance: bool,
    #[into(~.map(|c| serde_json::to_value(c)).transpose().map_err(|e| {
    StoreError::SerdeError("Failed to serialize payment_methods_config".to_string(), e)
    })?)]
    pub payment_methods_config: Option<PaymentMethodsConfig>,
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
    pub add_ons: Vec<QuoteAddOn>,
    pub coupons: Vec<QuoteCoupon>,
    pub signatures: Vec<QuoteSignature>,
    pub activities: Vec<QuoteActivity>,
}

#[derive(Debug, Clone)]
pub struct QuotePriceComponent {
    pub id: QuotePriceComponentId,
    pub name: String,
    pub quote_id: QuoteId,
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub is_override: bool,
    pub price_id: Option<PriceId>,
}

impl TryFrom<QuoteComponentRow> for QuotePriceComponent {
    type Error = StoreErrorReport;

    fn try_from(row: QuoteComponentRow) -> Result<Self, Self::Error> {
        let fee: SubscriptionFee = row
            .legacy_fee
            .ok_or_else(|| {
                StoreError::InvalidArgument("quote_component has no legacy_fee".to_string())
            })?
            .try_into()?;

        Ok(QuotePriceComponent {
            id: row.id,
            name: row.name,
            quote_id: row.quote_id,
            price_component_id: row.price_component_id,
            product_id: row.product_id,
            period: row.period.into(),
            fee,
            is_override: row.is_override,
            price_id: row.price_id,
        })
    }
}

#[derive(Debug, Clone)]
pub struct QuotePriceComponentNew {
    pub name: String,
    pub quote_id: QuoteId,
    pub price_component_id: Option<PriceComponentId>,
    pub product_id: Option<ProductId>,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub is_override: bool,
    pub price_id: Option<PriceId>,
}

impl TryInto<QuoteComponentRowNew> for QuotePriceComponentNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<QuoteComponentRowNew, Self::Error> {
        let legacy_fee: serde_json::Value = self.fee.try_into()?;

        Ok(QuoteComponentRowNew {
            id: QuotePriceComponentId::new(),
            name: self.name,
            quote_id: self.quote_id,
            price_component_id: self.price_component_id,
            product_id: self.product_id,
            period: self.period.into(),
            legacy_fee: Some(legacy_fee),
            is_override: self.is_override,
            price_id: self.price_id,
        })
    }
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

// Quote Add-On structs
#[derive(Debug, Clone)]
pub struct QuoteAddOn {
    pub id: QuoteAddOnId,
    pub name: String,
    pub quote_id: QuoteId,
    pub add_on_id: AddOnId,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
}

impl TryFrom<QuoteAddOnRow> for QuoteAddOn {
    type Error = StoreErrorReport;

    fn try_from(row: QuoteAddOnRow) -> Result<Self, Self::Error> {
        let fee: SubscriptionFee = row
            .legacy_fee
            .ok_or_else(|| {
                StoreError::InvalidArgument("quote_add_on has no legacy_fee".to_string())
            })?
            .try_into()?;

        Ok(QuoteAddOn {
            id: row.id,
            name: row.name,
            quote_id: row.quote_id,
            add_on_id: row.add_on_id,
            period: row.period.into(),
            fee,
            product_id: row.product_id,
            price_id: row.price_id,
        })
    }
}

#[derive(Debug, Clone)]
pub struct QuoteAddOnNew {
    pub name: String,
    pub quote_id: QuoteId,
    pub add_on_id: AddOnId,
    pub period: SubscriptionFeeBillingPeriod,
    pub fee: SubscriptionFee,
    pub product_id: Option<ProductId>,
    pub price_id: Option<PriceId>,
}

impl TryInto<QuoteAddOnRowNew> for QuoteAddOnNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<QuoteAddOnRowNew, Self::Error> {
        let legacy_fee: serde_json::Value = self.fee.try_into()?;

        Ok(QuoteAddOnRowNew {
            id: QuoteAddOnId::new(),
            name: self.name,
            quote_id: self.quote_id,
            add_on_id: self.add_on_id,
            period: self.period.into(),
            legacy_fee: Some(legacy_fee),
            product_id: self.product_id,
            price_id: self.price_id,
        })
    }
}

// Quote Coupon structs
#[derive(Debug, Clone, o2o)]
#[from_owned(QuoteCouponRow)]
pub struct QuoteCoupon {
    pub id: QuoteCouponId,
    pub quote_id: QuoteId,
    pub coupon_id: CouponId,
}

#[derive(Debug, Clone, o2o)]
#[owned_into(QuoteCouponRowNew)]
#[ghosts(id: {QuoteCouponId::new()})]
pub struct QuoteCouponNew {
    pub quote_id: QuoteId,
    pub coupon_id: CouponId,
}
