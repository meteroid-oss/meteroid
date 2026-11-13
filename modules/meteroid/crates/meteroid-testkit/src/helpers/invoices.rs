//! Hand-authored one-off invoices of the seeded customer (Uber), finalized and paid as the
//! credit tests state them.
use std::sync::Arc;

use chrono::Utc;
use common_domain::actor::Actor;
use common_domain::ids::{CreditGrantId, InvoiceId};
use meteroid::workers::pgmq::processors::run_once_invoice_orchestration;
use meteroid_store::domain::enums::{InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
use meteroid_store::domain::{
    DetailedInvoice, Invoice, InvoiceNew, LineItem, LineItemKind, TaxBreakdownItem, TaxDetail,
};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use rust_decimal::Decimal;

use crate::data::ids::{CUST_UBER_ID, INVOICING_ENTITY_ID, TENANT_ID};
use crate::helpers::credit::{Grant, fund_promotional_grant, pack_subscription};
use crate::helpers::invoice_lines::line;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn test_error(error: impl std::fmt::Debug) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

/// A fresh database seeded with products, and meteroid on it.
pub async fn setup() -> MeteroidSetup {
    crate::helpers::init::logging();
    let database = crate::meteroid_it::container::create_test_database().await;
    crate::meteroid_it::container::start_meteroid(database, SeedLevel::PRODUCT).await
}

/// A draft one-off invoice with hand-authored lines, which finalization keeps as they are.
/// Lines record the subscription they bill, as the plan's line builders do: a draft granting
/// included credit bills the customer's pack subscription, and so does every later draft.
pub async fn draft(setup: &MeteroidSetup, lines: Vec<LineItem>) -> TestResult<Invoice> {
    let customer = setup
        .store
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .map_err(test_error)?;
    let sells_pack = lines.iter().any(|line| {
        line.kind == LineItemKind::Credit
            && line.credit_grant_included
            && line.subscription_id.is_none()
    });
    let mut lines = lines;
    if let Some(subscription_id) = pack_subscription(
        &setup.store,
        TENANT_ID,
        CUST_UBER_ID,
        &customer.currency,
        sells_pack,
    )
    .await
    {
        for line in lines
            .iter_mut()
            .filter(|line| line.subscription_id.is_none() && line.sub_component_id.is_none())
        {
            line.subscription_id = Some(subscription_id);
        }
    }
    let entity = setup
        .store
        .get_invoicing_entity(TENANT_ID, Some(INVOICING_ENTITY_ID))
        .await
        .map_err(test_error)?;
    let total: i64 = lines.iter().map(|line| line.amount_total).sum();
    let subtotal: i64 = lines.iter().map(|line| line.amount_subtotal).sum();
    let tax: i64 = lines.iter().map(|line| line.tax_amount).sum();
    let today = Utc::now().date_naive();
    setup
        .store
        .insert_invoice(InvoiceNew {
            status: InvoiceStatusEnum::Draft,
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            subscription_id: None,
            cycle_index: None,
            currency: customer.currency.clone(),
            invoice_number: "draft".to_owned(),
            line_items: lines,
            coupons: Vec::new(),
            data_updated_at: None,
            invoice_date: today,
            billing_period_start: today,
            tax_inclusive: false,
            plan_version_id: None,
            invoice_type: InvoiceType::OneOff,
            finalized_at: None,
            subtotal,
            subtotal_recurring: 0,
            discount: subtotal + tax - total,
            tax_amount: tax,
            total,
            amount_due: total,
            applied_credits: 0,
            promotional_credits: 0,
            net_terms: 0,
            reference: None,
            purchase_order: None,
            memo: None,
            due_at: None,
            plan_name: None,
            customer_details: customer.into(),
            seller_details: entity.into(),
            auto_advance: false,
            payment_status: InvoicePaymentStatus::Unpaid,
            tax_breakdown: Vec::new(),
            manual: true,
            invoicing_entity_id: INVOICING_ENTITY_ID,
            parent_invoice_id: None,
            consolidated_into_invoice_id: None,
            annexes: Vec::new(),
            custom_properties: serde_json::Value::Null,
        })
        .await
        .map_err(test_error)
}

pub async fn finalize(setup: &MeteroidSetup, invoice: InvoiceId) -> TestResult<DetailedInvoice> {
    setup
        .services
        .finalize_invoice(Actor::System, invoice, TENANT_ID)
        .await
        .map_err(test_error)
}

/// Records a manual payment of `cents` and runs its settlement.
pub async fn pay_amount(setup: &MeteroidSetup, invoice: InvoiceId, cents: i64) -> TestResult {
    setup
        .services
        .mark_invoice_as_paid(
            Actor::System,
            TENANT_ID,
            invoice,
            Decimal::new(cents, 2),
            Utc::now().fixed_offset(),
            Some(format!("payment-{invoice}")),
        )
        .await
        .map_err(test_error)?;
    run_once_invoice_orchestration(
        Arc::new(setup.store.clone()),
        Arc::new(setup.services.clone()),
    )
    .await;
    Ok(())
}

fn vat() -> Decimal {
    Decimal::new(20, 2)
}

/// A line of `net` taxed at 20% on top.
pub fn taxed(name: &str, net: i64) -> LineItem {
    let tax = net / 5;
    LineItem {
        tax_rate: vat(),
        tax_amount: tax,
        amount_total: net + tax,
        tax_details: vec![TaxDetail {
            tax_rate: vat(),
            tax_name: "VAT".to_owned(),
            tax_amount: tax,
        }],
        ..line(name, net, false)
    }
}

/// A draft one-off invoice of taxed lines, which finalization keeps as they are.
/// One breakdown row per tax rate of the lines, as the tax engine writes them.
fn breakdown_of(lines: &[LineItem]) -> Vec<TaxBreakdownItem> {
    let mut rows: Vec<TaxBreakdownItem> = Vec::new();
    for line in lines.iter().filter(|line| line.tax_amount != 0) {
        match rows.iter_mut().find(|row| row.tax_rate == line.tax_rate) {
            Some(row) => {
                row.taxable_amount += line.taxable_amount;
                row.tax_amount += line.tax_amount;
            }
            None => rows.push(TaxBreakdownItem {
                taxable_amount: line.taxable_amount,
                tax_amount: line.tax_amount,
                tax_rate: line.tax_rate,
                name: format!("VAT {}", line.tax_rate),
                exemption_type: None,
                exemption_reason: None,
                tax_reference: None,
                overridden: false,
                supply_basis: None,
                tax_kind: None,
                prepaid_credit: false,
            }),
        }
    }
    rows
}

pub async fn taxed_draft(
    setup: &MeteroidSetup,
    lines: Vec<LineItem>,
    tax_inclusive: bool,
) -> TestResult<Invoice> {
    let customer = setup
        .store
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .map_err(test_error)?;
    let entity = setup
        .store
        .get_invoicing_entity(TENANT_ID, Some(INVOICING_ENTITY_ID))
        .await
        .map_err(test_error)?;
    let subtotal: i64 = lines.iter().map(|line| line.amount_subtotal).sum();
    let tax_amount: i64 = lines.iter().map(|line| line.tax_amount).sum();
    let total: i64 = lines.iter().map(|line| line.amount_total).sum();
    // What the lines were discounted by, none of these drafts carrying promotional credit yet.
    let discount: i64 = lines
        .iter()
        .map(|line| line.amount_subtotal - line.taxable_amount)
        .sum();
    let tax_breakdown = breakdown_of(&lines);
    let today = Utc::now().date_naive();
    setup
        .store
        .insert_invoice(InvoiceNew {
            status: InvoiceStatusEnum::Draft,
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            subscription_id: None,
            cycle_index: None,
            currency: customer.currency.clone(),
            invoice_number: "draft".to_owned(),
            line_items: lines,
            coupons: Vec::new(),
            data_updated_at: None,
            invoice_date: today,
            billing_period_start: today,
            tax_inclusive,
            plan_version_id: None,
            invoice_type: InvoiceType::OneOff,
            finalized_at: None,
            subtotal,
            subtotal_recurring: subtotal,
            discount,
            tax_amount,
            total,
            amount_due: total,
            applied_credits: 0,
            promotional_credits: 0,
            net_terms: 0,
            reference: None,
            purchase_order: None,
            memo: None,
            due_at: None,
            plan_name: None,
            customer_details: customer.into(),
            seller_details: entity.into(),
            auto_advance: false,
            payment_status: InvoicePaymentStatus::Unpaid,
            tax_breakdown,
            manual: true,
            invoicing_entity_id: INVOICING_ENTITY_ID,
            parent_invoice_id: None,
            consolidated_into_invoice_id: None,
            annexes: Vec::new(),
            custom_properties: serde_json::Value::Null,
        })
        .await
        .map_err(test_error)
}

pub async fn promotional(setup: &MeteroidSetup, grant: Grant) -> CreditGrantId {
    fund_promotional_grant(&setup.store, TENANT_ID, CUST_UBER_ID, None, grant).await
}
