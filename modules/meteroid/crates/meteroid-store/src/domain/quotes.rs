use chrono::{NaiveDate, NaiveDateTime};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::domain::enums::{QuoteApprovalStatusEnum, QuoteSignatureLevelEnum, QuoteSignerRoleEnum};
use crate::domain::{
    Customer, InvoicingEntity, QuoteStatusEnum, SubscriptionActivationCondition, SubscriptionFee,
    SubscriptionFeeBillingPeriod, subscriptions::PaymentMethodsConfig,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::json_value_serde;
use common_domain::ids::BaseId;
use common_domain::ids::{
    AddOnId, CouponId, CustomerId, InvoiceId, PlanVersionId, PriceComponentId, PriceId, ProductId,
    QuoteAddOnId, QuoteAttachmentId, QuoteCouponId, QuoteId, QuotePriceComponentId,
    QuoteSignatureId, QuoteTemplateId, ResourceId, StoredDocumentId, SubscriptionId, TenantId,
};
use diesel_models::quote_add_ons::{QuoteAddOnRow, QuoteAddOnRowNew};
use diesel_models::quote_coupons::{QuoteCouponRow, QuoteCouponRowNew};
use diesel_models::quotes::{
    QuoteComponentRow, QuoteComponentRowNew, QuoteRow, QuoteRowNew, QuoteSignatureRow,
    QuoteSignatureRowNew, QuoteWithCustomerRow,
};
use o2o::o2o;

/// What a quote's trial does at its end: it converts, or an operator decides ("decide at the
/// end", R-167).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QuoteTrialEnd {
    #[default]
    Convert,
    Manual,
}

impl QuoteTrialEnd {
    pub fn as_str(&self) -> &'static str {
        match self {
            QuoteTrialEnd::Convert => "CONVERT",
            QuoteTrialEnd::Manual => "MANUAL",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "MANUAL" => QuoteTrialEnd::Manual,
            _ => QuoteTrialEnd::Convert,
        }
    }

    pub fn is_convert(&self) -> bool {
        *self == QuoteTrialEnd::Convert
    }
}

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
    /// With `trial_duration_days`: whether the trial is free, signed with it; `None` on a quote
    /// saved before, read as the plan's trial (R-155).
    pub trial_free: Option<bool>,
    /// With `trial_duration_days`: the plan whose features the trial gives.
    pub trial_plan_id: Option<common_domain::ids::PlanId>,
    /// With `trial_duration_days`: what the trial does at its end.
    #[from(QuoteTrialEnd::parse(&~))]
    pub trial_end_action: QuoteTrialEnd,
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
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize attachments".to_string(), e)
    }) ?)]
    pub attachments: Vec<QuoteAttachment>,
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
    /// User-defined property values, keyed by definition `key` (empty object when unset).
    pub custom_properties: serde_json::Value,
    pub revision: i32,
    /// SHA-256 of the canonical document the signers see (`QuoteDocument`).
    pub document_hash: Option<String>,
    pub published_at: Option<NaiveDateTime>,
    pub first_viewed_at: Option<NaiveDateTime>,
    pub owner_id: Option<common_domain::ids::UserId>,
    pub template_id: Option<QuoteTemplateId>,
    pub decline_reason: Option<String>,
    #[from(serde_json::from_value(~).map_err(|e| {
        StoreError::SerdeError("Failed to deserialize display_options".to_string(), e)
    })?)]
    pub display_options: QuoteDisplayOptions,
    #[from(serde_json::from_value(~).map_err(|e| {
        StoreError::SerdeError("Failed to deserialize sections".to_string(), e)
    })?)]
    pub sections: Vec<QuoteSection>,
    #[from(~.into())]
    pub signature_level: QuoteSignatureLevelEnum,
    pub consent_text: Option<String>,
    #[from(serde_json::from_value(~).map_err(|e| {
        StoreError::SerdeError("Failed to deserialize countersigners".to_string(), e)
    })?)]
    pub countersigners: Vec<QuoteCountersigner>,
    #[from(~.into())]
    pub approval_status: QuoteApprovalStatusEnum,
    pub hubspot_deal_id: Option<String>,
    pub max_discount_percent: Option<Decimal>,
    pub changes_requested_at: Option<NaiveDateTime>,
    pub allow_questions: bool,
    pub allow_change_requests: bool,
    pub allow_decline: bool,
    pub last_sent_at: Option<NaiveDateTime>,
    /// The plan option sold, by alias; none: the plan's default.
    pub plan_option: Option<String>,
    /// The stored resource its subscription will be for; kept by the conversion.
    pub resource_id: Option<ResourceId>,
    /// On a write, what its subscription would be accepted with: `plan_sold_for_mismatch`,
    /// `metric_not_attributed`.
    #[ghost({Vec::new()})]
    pub warnings: Vec<String>,
    /// With `metric_not_attributed`: the codes of the metrics its resource would never bill.
    #[ghost({Vec::new()})]
    pub metrics_not_attributed: Vec<crate::domain::usage_attribution::MetricCodes>,
}

/// How the priced lines read on the document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuoteDisplayOptions {
    /// Lines whose amount is zero are left out of the document.
    pub hide_zero_lines: bool,
    /// Discounted lines show their list price struck through.
    pub show_list_price: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QuoteSectionPlacement {
    #[default]
    BeforePricing,
    AfterPricing,
}

/// A free-text block of the document, in Markdown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuoteSection {
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub placement: QuoteSectionPlacement,
}

/// Someone on the seller's side who signs after the customer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuoteCountersigner {
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub title: Option<String>,
    /// Signed on the seller's behalf as soon as the customer has signed.
    #[serde(default)]
    pub automatic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipientDetails {
    pub name: String,
    pub email: String,
}

json_value_serde!(RecipientDetails);

/// An uploaded file that is part of the agreement. Immutable: the document hashes it as is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteAttachment {
    pub id: QuoteAttachmentId,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
}

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
    /// With `trial_duration_days`: whether the trial is free, signed with it; `None` on a quote
    /// saved before, read as the plan's trial (R-155).
    pub trial_free: Option<bool>,
    /// With `trial_duration_days`: the plan whose features the trial gives.
    pub trial_plan_id: Option<common_domain::ids::PlanId>,
    /// With `trial_duration_days`: what the trial does at its end.
    #[into(~.as_str().to_owned())]
    pub trial_end_action: QuoteTrialEnd,
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
    #[into(serde_json::to_value(&~).map_err(|e| {
        StoreError::SerdeError("Failed to serialize attachments".to_string(), e)
    })?)]
    pub attachments: Vec<QuoteAttachment>,
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
    /// User-supplied custom property values. Validated against the tenant's `QUOTE` definitions
    /// by `insert_quote_with_details`; `Null` means "not managed" (stored as an empty object).
    pub custom_properties: serde_json::Value,
    pub resource_id: Option<ResourceId>,
    /// Entitlement specs to create inline when the quote is created.
    /// Not persisted to `quote_rows` — inserted into the entitlements table instead.
    #[ghost]
    pub entitlements: Vec<crate::domain::entitlements::EntitlementSpec>,
    pub owner_id: Option<common_domain::ids::UserId>,
    pub template_id: Option<QuoteTemplateId>,
    #[into(serde_json::to_value(&~).map_err(|e| {
        StoreError::SerdeError("Failed to serialize display_options".to_string(), e)
    })?)]
    pub display_options: QuoteDisplayOptions,
    #[into(serde_json::to_value(&~).map_err(|e| {
        StoreError::SerdeError("Failed to serialize sections".to_string(), e)
    })?)]
    pub sections: Vec<QuoteSection>,
    #[map(~.into())]
    pub signature_level: QuoteSignatureLevelEnum,
    pub consent_text: Option<String>,
    #[into(serde_json::to_value(&~).map_err(|e| {
        StoreError::SerdeError("Failed to serialize countersigners".to_string(), e)
    })?)]
    pub countersigners: Vec<QuoteCountersigner>,
    pub hubspot_deal_id: Option<String>,
    pub max_discount_percent: Option<Decimal>,
    pub allow_questions: bool,
    pub allow_change_requests: bool,
    pub allow_decline: bool,
    /// The plan option sold, by alias; none: the plan's default.
    pub plan_option: Option<String>,
}

impl Quote {
    /// The trial its subscription converts with: the one it states, none when it states none
    /// (R-155). A quote saved before its freeness was stored takes the plan's trial's.
    pub fn trial_request(&self) -> crate::domain::trials::TrialRequest {
        match self
            .trial_duration_days
            .and_then(|days| u32::try_from(days).ok())
        {
            None | Some(0) => crate::domain::trials::TrialRequest::None,
            Some(days) => crate::domain::trials::TrialRequest::Custom {
                length: crate::domain::trials::TrialLength::Days(days),
                free: self.trial_free,
                trial_plan: self.trial_free.map(|_| self.trial_plan_id),
                manual: self.trial_end_action == QuoteTrialEnd::Manual,
            },
        }
    }

    /// The quote as it would read once inserted, for computations over an editor's unsaved state.
    pub fn unsaved(new: QuoteNew) -> Self {
        Quote {
            id: new.id,
            status: new.status,
            created_at: common_clock::now().naive_utc(),
            updated_at: None,
            tenant_id: new.tenant_id,
            customer_id: new.customer_id,
            plan_version_id: new.plan_version_id,
            currency: new.currency,
            quote_number: new.quote_number,
            trial_duration_days: new.trial_duration_days,
            trial_free: new.trial_free,
            trial_plan_id: new.trial_plan_id,
            trial_end_action: new.trial_end_action,
            billing_start_date: new.billing_start_date,
            billing_end_date: new.billing_end_date,
            billing_day_anchor: new.billing_day_anchor,
            activation_condition: new.activation_condition,
            valid_until: new.valid_until,
            expires_at: new.expires_at,
            accepted_at: None,
            declined_at: None,
            internal_notes: new.internal_notes,
            cover_image: new.cover_image,
            overview: new.overview,
            terms_and_services: new.terms_and_services,
            net_terms: new.net_terms,
            attachments: new.attachments,
            pdf_document_id: new.pdf_document_id,
            sharing_key: new.sharing_key,
            converted_to_invoice_id: None,
            converted_to_subscription_id: None,
            converted_at: None,
            recipients: new.recipients,
            purchase_order: None,
            auto_advance_invoices: new.auto_advance_invoices,
            charge_automatically: new.charge_automatically,
            invoice_memo: new.invoice_memo,
            invoice_threshold: new.invoice_threshold,
            create_subscription_on_acceptance: new.create_subscription_on_acceptance,
            payment_methods_config: new.payment_methods_config,
            custom_properties: new.custom_properties,
            revision: 1,
            document_hash: None,
            published_at: None,
            first_viewed_at: None,
            owner_id: new.owner_id,
            template_id: new.template_id,
            decline_reason: None,
            display_options: new.display_options,
            sections: new.sections,
            signature_level: new.signature_level,
            consent_text: new.consent_text,
            countersigners: new.countersigners,
            approval_status: QuoteApprovalStatusEnum::NotRequired,
            hubspot_deal_id: new.hubspot_deal_id,
            max_discount_percent: new.max_discount_percent,
            changes_requested_at: None,
            allow_questions: new.allow_questions,
            allow_change_requests: new.allow_change_requests,
            allow_decline: new.allow_decline,
            last_sent_at: None,
            plan_option: new.plan_option,
            resource_id: new.resource_id,
            warnings: Vec::new(),
            metrics_not_attributed: Vec::new(),
        }
    }
}

#[derive(o2o, Debug, Clone)]
#[try_from_owned(QuoteWithCustomerRow, StoreErrorReport)]
pub struct QuoteWithCustomer {
    #[from(~.try_into()?)]
    pub quote: Quote,
    #[from(~.try_into()?)]
    pub customer: Customer,
}

/// What a quote adds up to, for display: amounts in the quote currency's major units.
#[derive(Debug, Clone, PartialEq)]
pub struct QuoteTotals {
    pub currency: String,
    /// By cadence, shortest first; one-time charges last.
    pub recurring: Vec<QuoteRecurringTotal>,
    /// Some line bills consumption, which `recurring` leaves out.
    pub has_usage: bool,
    pub first_invoice_subtotal: Option<Decimal>,
    pub contract: Option<QuoteContractValue>,
}

/// What a quote is worth as billing bills the subscription it converts to (R-165): its fixed
/// fees after coupons, trial and phases, before tax; usage aside.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteContractValue {
    /// From its start to its end date; None when it has none.
    pub term_months: Option<u32>,
    /// The months valued: the term, else at least the first year and the last phase's.
    pub months: u32,
    pub total: Decimal,
    /// `total` with the minimum commitments' shortfalls at no usage.
    pub committed_total: Decimal,
    /// A year's worth of `total`: all of it for a term shorter than a year.
    pub annual: Decimal,
    /// By contract year from the start; the last one may be partial.
    pub years: Vec<Decimal>,
    /// By phase of the price schedule, over its span; None for an open-ended last phase.
    pub phases: Vec<Option<Decimal>>,
    pub first_invoice: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuoteRecurringTotal {
    pub period: SubscriptionFeeBillingPeriod,
    pub amount: Decimal,
    pub list_amount: Decimal,
}

/// An editor's quote, prepared like a create but not stored.
#[derive(Debug, Clone)]
pub struct UnsavedQuote {
    pub quote: QuoteNew,
    pub components: Vec<QuotePriceComponentNew>,
    pub add_ons: Vec<QuoteAddOnNew>,
    pub coupons: Vec<QuoteCouponNew>,
    pub minimum: Option<(i64, crate::domain::plan_minimums::CreateMinimumScope)>,
}

#[derive(Debug, Clone)]
pub struct DetailedQuote {
    pub quote: Quote,
    pub customer: Customer,
    pub invoicing_entity: InvoicingEntity,
    pub components: Vec<QuotePriceComponent>,
    pub add_ons: Vec<QuoteAddOn>,
    pub coupons: Vec<QuoteCoupon>,
    /// The coupons `coupons` refers to, as currently defined.
    pub coupon_details: Vec<crate::domain::coupons::Coupon>,
    pub signatures: Vec<QuoteSignature>,
    pub entitlements: Vec<crate::domain::entitlements::Entitlement>,
    pub minimum: Option<crate::domain::QuoteMinimum>,
    /// The ramp the subscription it converts to starts with (R-155).
    pub phases: Vec<crate::domain::trial_offers::PlanPhaseTemplate>,
    /// The name of the plan whose features its trial gives.
    pub trial_plan_name: Option<String>,
}

impl DetailedQuote {
    /// The quote as its subscription starts: its components priced by the phase at offset 0, as
    /// conversion prices them (R-155). A phase refused there leaves the lines as they are.
    pub fn at_start(&self) -> Self {
        let mut quote = self.clone();
        let fees: Vec<_> = self
            .components
            .iter()
            .map(|component| (component.price_component_id, &component.fee))
            .collect();
        if let Ok(priced) = crate::services::plan_phases::priced_at_start(
            &self.phases,
            &fees,
            crate::domain::subscription_phases::precision_of(&self.quote.currency),
        ) {
            for (component, fee) in quote.components.iter_mut().zip(priced) {
                if let Some(fee) = fee {
                    component.fee = fee;
                }
            }
        }
        quote
    }

    /// The terms of the quote's coupons, in the order they were added.
    pub fn coupon_terms(&self) -> Vec<crate::domain::quote_document::QuoteDocumentCoupon> {
        self.coupons
            .iter()
            .filter_map(|c| self.coupon_details.iter().find(|d| d.id == c.coupon_id))
            .map(Into::into)
            .collect()
    }
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
    pub example_usage_quantity: Option<Decimal>,
    pub description: Option<String>,
    pub sort_order: i32,
    pub discount_percent: Option<Decimal>,
    /// The fee before `discount_percent`, when discounted.
    pub list_fee: Option<SubscriptionFee>,
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
            example_usage_quantity: row.example_usage_quantity,
            description: row.description,
            sort_order: row.sort_order,
            discount_percent: row.discount_percent,
            list_fee: row.list_fee.map(TryInto::try_into).transpose()?,
        })
    }
}

impl QuotePriceComponent {
    pub fn unsaved(new: QuotePriceComponentNew) -> Self {
        QuotePriceComponent {
            id: QuotePriceComponentId::new(),
            name: new.name,
            quote_id: new.quote_id,
            price_component_id: new.price_component_id,
            product_id: new.product_id,
            period: new.period,
            fee: new.fee,
            is_override: new.is_override,
            price_id: new.price_id,
            example_usage_quantity: new.example_usage_quantity,
            description: new.description,
            sort_order: new.sort_order,
            discount_percent: new.discount_percent,
            list_fee: new.list_fee,
        }
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
    pub example_usage_quantity: Option<Decimal>,
    pub description: Option<String>,
    pub sort_order: i32,
    pub discount_percent: Option<Decimal>,
    /// The fee before `discount_percent`, when discounted.
    pub list_fee: Option<SubscriptionFee>,
}

impl TryInto<QuoteComponentRowNew> for QuotePriceComponentNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<QuoteComponentRowNew, Self::Error> {
        let legacy_fee = Some(self.fee.try_into()?);
        let list_fee = self.list_fee.map(TryInto::try_into).transpose()?;

        Ok(QuoteComponentRowNew {
            id: QuotePriceComponentId::new(),
            name: self.name,
            quote_id: self.quote_id,
            price_component_id: self.price_component_id,
            product_id: self.product_id,
            period: self.period.into(),
            legacy_fee,
            is_override: self.is_override,
            price_id: self.price_id,
            example_usage_quantity: self.example_usage_quantity,
            description: self.description,
            sort_order: self.sort_order,
            discount_percent: self.discount_percent,
            list_fee,
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
    #[from(~.into())]
    pub signer_role: QuoteSignerRoleEnum,
    pub revision: Option<i32>,
    pub document_hash: Option<String>,
    pub consent_text: Option<String>,
    pub otp_verified_at: Option<NaiveDateTime>,
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
    #[into(~.into())]
    pub signer_role: QuoteSignerRoleEnum,
    pub revision: Option<i32>,
    pub document_hash: Option<String>,
    pub consent_text: Option<String>,
    pub otp_verified_at: Option<NaiveDateTime>,
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
    pub quantity: i32,
    pub description: Option<String>,
    pub sort_order: i32,
    pub discount_percent: Option<Decimal>,
    pub list_fee: Option<SubscriptionFee>,
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
            quantity: row.quantity,
            description: row.description,
            sort_order: row.sort_order,
            discount_percent: row.discount_percent,
            list_fee: row.list_fee.map(TryInto::try_into).transpose()?,
        })
    }
}

impl QuoteAddOn {
    pub fn unsaved(new: QuoteAddOnNew) -> Self {
        QuoteAddOn {
            id: QuoteAddOnId::new(),
            name: new.name,
            quote_id: new.quote_id,
            add_on_id: new.add_on_id,
            period: new.period,
            fee: new.fee,
            product_id: new.product_id,
            price_id: new.price_id,
            quantity: new.quantity,
            description: new.description,
            sort_order: new.sort_order,
            discount_percent: new.discount_percent,
            list_fee: new.list_fee,
        }
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
    pub quantity: i32,
    pub description: Option<String>,
    pub sort_order: i32,
    pub discount_percent: Option<Decimal>,
    pub list_fee: Option<SubscriptionFee>,
}

impl TryInto<QuoteAddOnRowNew> for QuoteAddOnNew {
    type Error = StoreErrorReport;

    fn try_into(self) -> Result<QuoteAddOnRowNew, Self::Error> {
        let legacy_fee = Some(self.fee.try_into()?);
        let list_fee = self.list_fee.map(TryInto::try_into).transpose()?;

        Ok(QuoteAddOnRowNew {
            id: QuoteAddOnId::new(),
            name: self.name,
            quote_id: self.quote_id,
            add_on_id: self.add_on_id,
            period: self.period.into(),
            legacy_fee,
            product_id: self.product_id,
            price_id: self.price_id,
            quantity: self.quantity,
            description: self.description,
            sort_order: self.sort_order,
            discount_percent: self.discount_percent,
            list_fee,
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

impl QuoteCoupon {
    pub fn unsaved(new: QuoteCouponNew) -> Self {
        QuoteCoupon {
            id: QuoteCouponId::new(),
            quote_id: new.quote_id,
            coupon_id: new.coupon_id,
        }
    }
}

#[derive(Debug, Clone, o2o)]
#[owned_into(QuoteCouponRowNew)]
#[ghosts(id: {QuoteCouponId::new()})]
pub struct QuoteCouponNew {
    pub quote_id: QuoteId,
    pub coupon_id: CouponId,
}
