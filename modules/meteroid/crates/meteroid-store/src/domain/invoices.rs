use super::enums::{InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
use crate::domain::connectors::ConnectionMeta;
use crate::domain::invoice_lines::LineItem;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::{Address, CouponLineItem, Customer, PlanVersionOverview};
use crate::errors::{StoreError, StoreErrorReport};
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{
    BaseId, CustomerId, InvoiceId, InvoicingEntityId, PlanVersionId, StoredDocumentId,
    SubscriptionId, TenantId,
};
use diesel_models::invoices::DetailedInvoiceRow;
use diesel_models::invoices::InvoiceRow;
use diesel_models::invoices::InvoiceRowLinesPatch;
use diesel_models::invoices::InvoiceRowNew;
use diesel_models::invoices::InvoiceWithCustomerRow;
use diesel_models::payments::PaymentTransactionRow;
use error_stack::Report;
use meteroid_tax::TaxDetails;
use o2o::o2o;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, o2o, PartialEq, Eq)]
#[try_from_owned(InvoiceRow, StoreErrorReport)]
pub struct Invoice {
    pub id: InvoiceId,
    #[from(~.into())]
    pub status: InvoiceStatusEnum,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub invoice_number: String,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub plan_version_id: Option<PlanVersionId>,
    #[from(~.into())]
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub amount_due: i64,
    pub applied_credits: i64,
    pub net_terms: i32,
    pub reference: Option<String>,
    pub memo: Option<String>,
    pub due_at: Option<NaiveDateTime>,
    pub plan_name: Option<String>,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize customer_details".to_string(), e)
    }) ?)]
    pub customer_details: InlineCustomer,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize seller_details".to_string(), e)
    }) ?)]
    pub seller_details: InlineInvoicingEntity,
    pub pdf_document_id: Option<StoredDocumentId>,
    pub xml_document_id: Option<StoredDocumentId>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub conn_meta: Option<ConnectionMeta>,
    pub auto_advance: bool,
    pub issued_at: Option<NaiveDateTime>,
    #[from(~.into())]
    pub payment_status: InvoicePaymentStatus,
    pub paid_at: Option<NaiveDateTime>,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize coupons".to_string(), e)
    }) ?)]
    pub coupons: Vec<CouponLineItem>,
    pub discount: i64,
    pub purchase_order: Option<String>,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize tax_breakdown".to_string(), e)
    }) ?)]
    pub tax_breakdown: Vec<TaxBreakdownItem>,
}

impl Invoice {
    pub fn can_edit(&self) -> bool {
        self.status == InvoiceStatusEnum::Draft
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaxExemptionType {
    ReverseCharge,
    TaxExempt,
    NotRegistered,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxBreakdownItem {
    pub taxable_amount: u64,
    #[serde(default)]
    pub tax_amount: u64,
    pub tax_rate: Decimal,
    pub name: String,
    pub exemption_type: Option<TaxExemptionType>,
}

impl From<meteroid_tax::TaxBreakdownItem> for TaxBreakdownItem {
    fn from(item: meteroid_tax::TaxBreakdownItem) -> Self {
        match item.details {
            TaxDetails::Tax {
                tax_rate,
                tax_name,
                tax_amount,
                ..
            } => TaxBreakdownItem {
                taxable_amount: item.taxable_amount,
                tax_amount,
                tax_rate,
                name: tax_name,
                exemption_type: None,
            },
            TaxDetails::Exempt(reason) => {
                use meteroid_tax::VatExemptionReason;
                let exemption_type = match reason {
                    VatExemptionReason::ReverseCharge => TaxExemptionType::ReverseCharge,
                    VatExemptionReason::TaxExempt => TaxExemptionType::TaxExempt,
                    VatExemptionReason::NotRegistered => TaxExemptionType::NotRegistered,
                    VatExemptionReason::Other(s) => TaxExemptionType::Other(s),
                };
                TaxBreakdownItem {
                    taxable_amount: item.taxable_amount,
                    tax_amount: 0,
                    tax_rate: Decimal::ZERO,
                    name: "Exempt".to_string(),
                    exemption_type: Some(exemption_type),
                }
            }
        }
    }
}

#[derive(Debug, o2o)]
#[owned_try_into(InvoiceRowNew, StoreErrorReport)]
#[ghosts(id: {InvoiceId::new()})]
pub struct InvoiceNew {
    #[into(~.into())]
    pub status: InvoiceStatusEnum,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub invoice_number: String,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize coupons".to_string(), e)
    }) ?)]
    pub coupons: Vec<CouponLineItem>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub plan_version_id: Option<PlanVersionId>,
    #[into(~.into())]
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub discount: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub amount_due: i64,
    pub net_terms: i32,
    pub reference: Option<String>,
    pub purchase_order: Option<String>,
    pub memo: Option<String>,
    pub due_at: Option<NaiveDateTime>, // TODO due_date
    pub plan_name: Option<String>,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize customer_details".to_string(), e)
    }) ?)]
    pub customer_details: InlineCustomer,
    #[into(serde_json::to_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize seller_details".to_string(), e)
    }) ?)]
    pub seller_details: InlineInvoicingEntity,
    pub auto_advance: bool,
    #[into(~.into())]
    pub payment_status: InvoicePaymentStatus,
    #[into(serde_json::to_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize tax_breakdown".to_string(), e)
    }) ?)]
    pub tax_breakdown: Vec<TaxBreakdownItem>,
}

#[derive(Debug, o2o)]
#[owned_try_into(InvoiceRowLinesPatch, StoreErrorReport)]
pub struct InvoiceLinesPatch {
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    pub amount_due: i64,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub total: i64,
    pub tax_amount: i64,
    pub applied_credits: i64,
    #[ghost({vec![]})]
    pub applied_coupons: Vec<CouponLineItem>,
    #[into(serde_json::to_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize tax_breakdown".to_string(), e)
    }) ?)]
    pub tax_breakdown: Vec<TaxBreakdownItem>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct InlineCustomer {
    pub id: CustomerId,
    pub name: String,
    pub email: Option<String>,
    pub alias: Option<String>,
    pub vat_number: Option<String>,
    pub billing_address: Option<Address>,
    pub snapshot_at: NaiveDateTime,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct InlineInvoicingEntity {
    pub id: InvoicingEntityId,
    pub legal_name: String,
    pub vat_number: Option<String>,
    pub address: Address,
    pub snapshot_at: NaiveDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceWithCustomer {
    pub invoice: Invoice,
    pub customer: Customer,
}

impl TryFrom<InvoiceWithCustomerRow> for InvoiceWithCustomer {
    type Error = Report<StoreError>;

    fn try_from(value: InvoiceWithCustomerRow) -> Result<Self, Self::Error> {
        Ok(InvoiceWithCustomer {
            invoice: value.invoice.try_into()?,
            customer: value.customer.try_into()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetailedInvoice {
    pub invoice: Invoice,
    pub customer: Customer,
    pub plan: Option<PlanVersionOverview>,
    pub transactions: Vec<PaymentTransaction>,
}

impl DetailedInvoice {
    pub fn with_transactions(mut self, transactions: Vec<PaymentTransaction>) -> Self {
        self.transactions = transactions;
        self
    }
    pub fn with_transaction_rows(mut self, transactions: Vec<PaymentTransactionRow>) -> Self {
        self.transactions = transactions.into_iter().map(|x| x.into()).collect();
        self
    }
}

impl TryFrom<DetailedInvoiceRow> for DetailedInvoice {
    type Error = Report<StoreError>;

    fn try_from(value: DetailedInvoiceRow) -> Result<Self, Self::Error> {
        Ok(DetailedInvoice {
            invoice: value.invoice.try_into()?,
            customer: value.customer.try_into()?,
            plan: value.plan.map(|x| x.into()),
            transactions: vec![],
        })
    }
}
