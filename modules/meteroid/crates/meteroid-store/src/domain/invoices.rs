use super::enums::{InvoiceExternalStatusEnum, InvoiceStatusEnum, InvoiceType};
use crate::domain::coupons::CouponDiscount;
use crate::domain::invoice_lines::LineItem;
use crate::domain::{Address, AppliedCouponDetailed, Customer, PlanVersionOverview};
use crate::errors::{StoreError, StoreErrorReport};
use crate::utils::decimals::ToSubunit;
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{BaseId, CustomerId, InvoiceId, SubscriptionId, TenantId};
use diesel_models::invoices::DetailedInvoiceRow;
use diesel_models::invoices::InvoiceRow;
use diesel_models::invoices::InvoiceRowLinesPatch;
use diesel_models::invoices::InvoiceRowNew;
use diesel_models::invoices::InvoiceWithCustomerRow;
use error_stack::Report;
use itertools::Itertools;
use o2o::o2o;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::cmp::min;
use uuid::Uuid;

#[derive(Debug, Clone, o2o, PartialEq, Eq)]
#[try_from_owned(InvoiceRow, StoreErrorReport)]
pub struct Invoice {
    pub id: InvoiceId,
    #[from(~.into())]
    pub status: InvoiceStatusEnum,
    #[from(~.map(| x | x.into()))]
    pub external_status: Option<InvoiceExternalStatusEnum>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub external_invoice_id: Option<String>,
    pub invoice_number: String,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    pub issued: bool,
    pub issue_attempts: i32,
    pub last_issue_attempt_at: Option<NaiveDateTime>,
    pub last_issue_error: Option<String>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub plan_version_id: Option<Uuid>,
    #[from(~.into())]
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub tax_rate: i32, // TODO decimal, I guess we need to support more than 2 dec
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
    pub pdf_document_id: Option<String>,
    pub xml_document_id: Option<String>,
}

#[derive(Debug, o2o)]
#[owned_try_into(InvoiceRowNew, StoreErrorReport)]
#[ghosts(id: {InvoiceId::new()})]
pub struct InvoiceNew {
    #[into(~.into())]
    pub status: InvoiceStatusEnum,
    #[into(~.map(| x | x.into()))]
    pub external_status: Option<InvoiceExternalStatusEnum>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub external_invoice_id: Option<String>,
    pub invoice_number: String,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    pub issued: bool,
    pub issue_attempts: i32,
    pub last_issue_attempt_at: Option<NaiveDateTime>,
    pub last_issue_error: Option<String>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub plan_version_id: Option<Uuid>,
    #[into(~.into())]
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub tax_rate: i32,
    pub tax_amount: i64,
    pub total: i64,
    pub amount_due: i64,
    pub net_terms: i32,
    pub reference: Option<String>,
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
    pub applied_coupons: Vec<(Uuid, i64)>,
}

impl InvoiceLinesPatch {
    pub fn new(
        detailed_invoice: &DetailedInvoice,
        line_items: Vec<LineItem>,
        applied_coupons: &[AppliedCouponDetailed],
    ) -> Self {
        let totals = InvoiceTotals::from_params(InvoiceTotalsParams {
            line_items: &line_items,
            total: detailed_invoice.invoice.total,
            amount_due: detailed_invoice.invoice.amount_due,
            tax_rate: detailed_invoice.invoice.tax_rate,
            customer_balance_cents: detailed_invoice.customer.balance_value_cents,
            subscription_applied_coupons: &applied_coupons.to_vec(),
            invoice_currency: detailed_invoice.invoice.currency.as_str(),
        });

        InvoiceLinesPatch {
            line_items,
            amount_due: totals.amount_due,
            subtotal: totals.subtotal,
            subtotal_recurring: totals.subtotal_recurring,
            total: totals.total,
            tax_amount: totals.tax_amount,
            applied_credits: totals.applied_credits,
            applied_coupons: totals.applied_coupons,
        }
    }
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
    pub id: Uuid,
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
}

impl TryFrom<DetailedInvoiceRow> for DetailedInvoice {
    type Error = Report<StoreError>;

    fn try_from(value: DetailedInvoiceRow) -> Result<Self, Self::Error> {
        Ok(DetailedInvoice {
            invoice: value.invoice.try_into()?,
            customer: value.customer.try_into()?,
            plan: value.plan.map(|x| x.into()),
        })
    }
}

pub struct InvoiceTotalsParams<'a> {
    pub line_items: &'a Vec<LineItem>,
    pub subscription_applied_coupons: &'a Vec<AppliedCouponDetailed>,
    pub total: i64,
    pub amount_due: i64,
    pub tax_rate: i32,
    pub customer_balance_cents: i32,
    pub invoice_currency: &'a str,
}

pub struct InvoiceTotals {
    pub amount_due: i64,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub total: i64,
    pub tax_amount: i64,
    pub applied_credits: i64,
    pub applied_coupons: Vec<(Uuid, i64)>,
}

struct AppliedCouponsDiscount {
    pub discount_subunit: i64,
    pub applied_coupons: Vec<(Uuid, i64)>,
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
        let applied_credits = min(total, params.customer_balance_cents as i64);
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

        let mut applied_coupons_amount = vec![];

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
            applied_coupons_amount.push((applicable_coupon.applied_coupon.id, discount));
        }

        AppliedCouponsDiscount {
            discount_subunit: applied_coupons_amount.iter().map(|x| x.1).sum(),
            applied_coupons: applied_coupons_amount,
        }
    }
}
