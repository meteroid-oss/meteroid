use super::enums::{InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
use crate::domain::connectors::ConnectionMeta;
use crate::domain::coupons::CouponDiscount;
use crate::domain::invoice_lines::LineItem;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::{
    Address, AppliedCouponDetailed, CouponLineItem, Customer, PlanVersionOverview,
};
use crate::errors::{StoreError, StoreErrorReport};
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{
    BaseId, CustomerId, InvoiceId, InvoicingEntityId, PlanVersionId, StoredDocumentId,
    SubscriptionId, TenantId,
};
use common_utils::decimals::ToSubunit;
use diesel_models::invoices::DetailedInvoiceRow;
use diesel_models::invoices::InvoiceRow;
use diesel_models::invoices::InvoiceRowLinesPatch;
use diesel_models::invoices::InvoiceRowNew;
use diesel_models::invoices::InvoiceWithCustomerRow;
use diesel_models::payments::PaymentTransactionRow;
use error_stack::Report;
use itertools::Itertools;
use o2o::o2o;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use std::cmp::min;

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
    // TODO precision 4
    pub tax_rate: i32,
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
}

impl Invoice {
    pub fn can_edit(&self) -> bool {
        self.status == InvoiceStatusEnum::Draft
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
    pub tax_rate: i32,
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
    pub fn with_transactions(mut self, transactions: Vec<PaymentTransactionRow>) -> Self {
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

pub struct InvoiceTotalsParams<'a> {
    pub line_items: &'a Vec<LineItem>,
    pub subscription_applied_coupons: &'a Vec<AppliedCouponDetailed>,
    pub total: i64,
    pub amount_due: i64,
    pub tax_rate: i32,
    pub customer_balance_cents: i64,
    pub invoice_currency: &'a str,
}

pub struct InvoiceTotals {
    pub amount_due: i64,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub total: i64,
    pub tax_amount: i64,
    pub applied_credits: i64,
    pub applied_coupons: Vec<CouponLineItem>,
}

struct AppliedCouponsDiscount {
    pub discount_subunit: i64,
    pub applied_coupons: Vec<CouponLineItem>,
}

impl InvoiceTotals {
    pub fn from_params(params: InvoiceTotalsParams) -> Self {
        let subtotal = params.line_items.iter().fold(0, |acc, x| acc + x.subtotal);
        let coupons_discount = Self::calculate_coupons_discount(
            subtotal,
            params.invoice_currency,
            params.subscription_applied_coupons,
        );
        let subtotal_with_discounts = subtotal - coupons_discount.discount_subunit;
        let tax_amount = subtotal_with_discounts * params.tax_rate as i64 / 100;

        let total = subtotal_with_discounts + tax_amount;
        let applied_credits = min(total, params.customer_balance_cents);
        let already_paid = params.total - params.amount_due;
        let amount_due = total - already_paid - applied_credits;
        let subtotal_recurring = params
            .line_items
            .iter()
            .filter(|x| x.metric_id.is_none())
            .fold(0, |acc, x| acc + x.subtotal);

        Self {
            amount_due,
            subtotal,
            subtotal_recurring,
            total,
            tax_amount,
            applied_credits,
            applied_coupons: coupons_discount.applied_coupons,
        }
    }

    fn calculate_coupons_discount(
        subtotal: i64,
        invoice_currency: &str,
        coupons: &[AppliedCouponDetailed],
    ) -> AppliedCouponsDiscount {
        let applicable_coupons: Vec<&AppliedCouponDetailed> = coupons
            .iter()
            .filter(|x| x.is_invoice_applicable())
            .sorted_by_key(|x| x.applied_coupon.created_at)
            .collect::<Vec<_>>();

        let mut applied_coupons_items = vec![];

        let mut subtotal_subunits = Decimal::from(subtotal);

        for applicable_coupon in applicable_coupons {
            if subtotal_subunits <= Decimal::ONE {
                break;
            }
            let discount = match &applicable_coupon.coupon.discount {
                CouponDiscount::Percentage(percentage) => {
                    subtotal_subunits * percentage / Decimal::ONE_HUNDRED
                }
                CouponDiscount::Fixed { amount, currency } => {
                    // todo currency conversion
                    if currency != invoice_currency {
                        continue;
                    }
                    // todo domain should use Currency type instead of string
                    let cur = rusty_money::iso::find(currency).unwrap_or(rusty_money::iso::USD);

                    let consumed_amount = &applicable_coupon
                        .applied_coupon
                        .applied_amount
                        .unwrap_or(Decimal::ZERO);

                    let discount_subunits = (amount - consumed_amount)
                        .to_subunit_opt(cur.exponent as u8)
                        .unwrap_or(0);

                    Decimal::from(discount_subunits).min(subtotal_subunits)
                }
            };

            subtotal_subunits -= discount;

            let discount = discount.to_i64().unwrap_or(0);
            // (applicable_coupon.applied_coupon.id, discount)
            applied_coupons_items.push(CouponLineItem {
                coupon_id: applicable_coupon.coupon.id,
                applied_coupon_id: applicable_coupon.applied_coupon.id,
                name: format!("Coupon ({})", applicable_coupon.coupon.code), // TODO allow defining a name in coupon
                code: applicable_coupon.coupon.code.clone(),
                value: discount,
                discount: applicable_coupon.coupon.discount.clone(),
            });
        }

        AppliedCouponsDiscount {
            discount_subunit: applied_coupons_items.iter().map(|x| x.value).sum(),
            applied_coupons: applied_coupons_items,
        }
    }
}
