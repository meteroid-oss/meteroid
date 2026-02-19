use crate::data::ids::*;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use chrono::NaiveDate;
use common_domain::country::CountryCode;
use common_domain::ids::{BaseId, CustomerId, PlanVersionId, PriceComponentId};
use diesel_models::enums::{FeeTypeEnum as DieselFeeTypeEnum, PlanStatusEnum, PlanTypeEnum};
use diesel_models::plan_versions::PlanVersionRowNew;
use diesel_models::plans::{PlanRowNew, PlanRowPatch};
use diesel_models::price_components::PriceComponentRowNew;
use diesel_models::products::ProductRowNew;
use meteroid::workers::pgmq::processors::{
    run_once_invoice_orchestration, run_once_outbox_dispatch,
};
use meteroid_mailer::service::MockMailerService;
use meteroid_store::clients::usage::MockUsageClient;
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::subscription_coupons::CreateSubscriptionCoupon;
use meteroid_store::domain::{
    Address, BillingPeriodEnum, CreateSubscription, CreateSubscriptionCoupons, CustomerCustomTax,
    FeeType, InvoicingEntityPatch, OrderByRequest, PaginationRequest,
    SubscriptionActivationCondition, SubscriptionNew, TermRate,
};
use meteroid_store::repositories::coupons::CouponInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::{CreditNoteInterface, InvoiceInterface};
use meteroid_store::store::PgConn;
use rust_decimal_macros::dec;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

/// Test credit notes with partial credits, discounts, taxes, and multiple line items
#[tokio::test]
async fn test_credit_note_partial_credits() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let mock_mailer = Arc::new(MockMailerService::new());
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let services = setup.services.clone();
    let store = setup.store.clone();
    let mut conn = setup.store.pool.get().await.unwrap();

    log::info!(">>> Testing partial credit notes with taxes and coupons");

    // 1. Set up manual tax resolver
    store
        .patch_invoicing_entity(
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(meteroid_store::domain::enums::TaxResolverEnum::Manual),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // 2. Create a plan with 4 price components
    let (plan_version_id, _component_ids) = create_plan_with_4_components(&mut conn).await;

    // 3. Create customer with custom tax rate (10%)
    let customer_id = create_customer_with_tax(&mut conn, 0).await; // No balance initially

    // 4. Create a 10% coupon
    let coupon_id = store
        .create_coupon(CouponNew {
            code: "CREDIT10".to_string(),
            description: "10% discount for credit note test".to_string(),
            tenant_id: TENANT_ID,
            discount: CouponDiscount::Percentage(dec!(10)),
            expires_at: None,
            redemption_limit: None,
            recurring_value: None,
            reusable: false,
            plan_ids: vec![],
        })
        .await
        .unwrap()
        .id;

    // 5. Create subscription with the plan and coupon
    let subscription = services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id,
                    plan_version_id,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
                    end_date: None,
                    billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                },
                price_components: None,
                add_ons: None,
                coupons: Some(CreateSubscriptionCoupons {
                    coupons: vec![CreateSubscriptionCoupon { coupon_id }],
                }),
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // 6. Process billing events to finalize invoice
    services.get_and_process_due_events().await.unwrap();

    // 7. Get the invoice
    let invoices = store
        .list_invoices(
            TENANT_ID,
            None,
            Some(subscription.id),
            None,
            None,
            OrderByRequest::DateAsc,
            PaginationRequest {
                page: 0,
                per_page: None,
            },
        )
        .await
        .unwrap()
        .items;

    assert_eq!(invoices.len(), 1, "Should have one invoice");
    let invoice = &invoices[0].invoice;
    assert_eq!(
        invoice.status,
        InvoiceStatusEnum::Finalized,
        "Invoice should be finalized"
    );

    // Verify invoice has 4 line items
    assert_eq!(
        invoice.line_items.len(),
        4,
        "Invoice should have 4 line items"
    );

    log::info!(
        "Invoice: subtotal={}, discount={}, tax={}, total={}",
        invoice.subtotal,
        invoice.discount,
        invoice.tax_amount,
        invoice.total
    );

    // Calculate expected values:
    // Subtotal: 1000 + 2000 + 3000 + 4000 = 10000 cents
    // Discount: 10% = 1000 cents
    // Taxable: 9000 cents
    // Tax: 10% of 9000 = 900 cents
    // Total: 9000 + 900 = 9900 cents
    assert_eq!(invoice.subtotal, 10000, "Subtotal should be 10000 cents");
    assert_eq!(
        invoice.discount, 1000,
        "Discount should be 1000 cents (10%)"
    );
    assert_eq!(
        invoice.tax_amount, 900,
        "Tax should be 900 cents (10% of 9000)"
    );
    assert_eq!(invoice.total, 9900, "Total should be 9900 cents");

    // Get line item local_ids for credit notes
    let line_ids: Vec<String> = invoice
        .line_items
        .iter()
        .map(|l| l.local_id.clone())
        .collect();

    log::info!("Line items:");
    for line in &invoice.line_items {
        log::info!(
            "  {}: subtotal={}, taxable={}, tax={}, total={}",
            line.local_id,
            line.amount_subtotal,
            line.taxable_amount,
            line.tax_amount,
            line.amount_total
        );
    }

    // 8. Create first partial credit note for lines 0 and 1
    let credit_note_1 = store
        .create_credit_note(
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![
                    CreditLineItem {
                        local_id: line_ids[0].clone(),
                        amount: None,
                    },
                    CreditLineItem {
                        local_id: line_ids[1].clone(),
                        amount: None,
                    },
                ],
                reason: Some("Partial refund - first batch".to_string()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
            },
        )
        .await
        .unwrap();

    log::info!(
        "Credit Note 1: subtotal={}, tax={}, total={}, credited={}",
        credit_note_1.subtotal,
        credit_note_1.tax_amount,
        credit_note_1.total,
        credit_note_1.credited_amount_cents
    );

    // Verify credit note 1 totals
    // Lines 0 and 1: subtotal = 1000 + 2000 = 3000
    // Their share of discount: 10% of 3000 = 300
    // Taxable: 2700
    // Tax: 10% of 2700 = 270
    // Total: 2700 + 270 = 2970
    assert_eq!(
        credit_note_1.subtotal, -3000,
        "Credit note 1 subtotal should be -3000"
    );
    assert_eq!(
        credit_note_1.tax_amount, -270,
        "Credit note 1 tax should be -270 (10% of 2700 taxable)"
    );
    assert_eq!(
        credit_note_1.total, -2970,
        "Credit note 1 total should be -2970"
    );
    assert_eq!(
        credit_note_1.credited_amount_cents, 2970,
        "Credit note 1 credited amount should be 2970"
    );
    assert_eq!(
        credit_note_1.refunded_amount_cents, 0,
        "Credit note 1 should not have refunded amount (CreditToBalance)"
    );

    // 9. Create second partial credit note for lines 2 and 3
    let credit_note_2 = store
        .create_credit_note(
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![
                    CreditLineItem {
                        local_id: line_ids[2].clone(),
                        amount: None,
                    },
                    CreditLineItem {
                        local_id: line_ids[3].clone(),
                        amount: None,
                    },
                ],
                reason: Some("Partial refund - second batch".to_string()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
            },
        )
        .await
        .unwrap();

    log::info!(
        "Credit Note 2: subtotal={}, tax={}, total={}, credited={}",
        credit_note_2.subtotal,
        credit_note_2.tax_amount,
        credit_note_2.total,
        credit_note_2.credited_amount_cents
    );

    // Verify credit note 2 totals
    // Lines 2 and 3: subtotal = 3000 + 4000 = 7000
    // Their share of discount: 10% of 7000 = 700
    // Taxable: 6300
    // Tax: 10% of 6300 = 630
    // Total: 6300 + 630 = 6930
    assert_eq!(
        credit_note_2.subtotal, -7000,
        "Credit note 2 subtotal should be -7000"
    );
    assert_eq!(
        credit_note_2.tax_amount, -630,
        "Credit note 2 tax should be -630 (10% of 6300 taxable)"
    );
    assert_eq!(
        credit_note_2.total, -6930,
        "Credit note 2 total should be -6930"
    );
    assert_eq!(
        credit_note_2.credited_amount_cents, 6930,
        "Credit note 2 credited amount should be 6930"
    );
    assert_eq!(
        credit_note_2.refunded_amount_cents, 0,
        "Credit note 2 should not have refunded amount (CreditToBalance)"
    );

    // 10. Verify sums match invoice (NO LOST CENTS)
    // Sum of subtotals
    let total_subtotal = (credit_note_1.subtotal + credit_note_2.subtotal).abs();
    assert_eq!(
        total_subtotal, invoice.subtotal,
        "Sum of credit note subtotals ({}) should equal invoice subtotal ({})",
        total_subtotal, invoice.subtotal
    );

    // Sum of tax amounts
    let total_tax = (credit_note_1.tax_amount + credit_note_2.tax_amount).abs();
    assert_eq!(
        total_tax, invoice.tax_amount,
        "Sum of credit note tax amounts ({}) should equal invoice tax ({})",
        total_tax, invoice.tax_amount
    );

    // Sum of totals (credit note totals are negative)
    let total_total = (credit_note_1.total + credit_note_2.total).abs();
    assert_eq!(
        total_total, invoice.total,
        "Sum of credit note totals ({}) should equal invoice total ({})",
        total_total, invoice.total
    );

    // Sum of credited amounts
    let total_credits = credit_note_1.credited_amount_cents + credit_note_2.credited_amount_cents;
    assert_eq!(
        total_credits, invoice.total,
        "Total credits ({}) should equal invoice total ({})",
        total_credits, invoice.total
    );

    // 11. Verify credit note line items have correct values
    // Credit note 1 should have 2 line items (negated versions of invoice lines 0 and 1)
    assert_eq!(
        credit_note_1.line_items.len(),
        2,
        "Credit note 1 should have 2 line items"
    );

    // Line 0: original subtotal=1000, discount=100, taxable=900, tax=90, total=990
    let cn1_line0 = &credit_note_1.line_items[0];
    assert_eq!(
        cn1_line0.amount_subtotal, -1000,
        "CN1 line 0 subtotal should be -1000"
    );
    assert_eq!(
        cn1_line0.taxable_amount, -900,
        "CN1 line 0 taxable should be -900"
    );
    assert_eq!(cn1_line0.tax_amount, -90, "CN1 line 0 tax should be -90");
    assert_eq!(
        cn1_line0.amount_total, -990,
        "CN1 line 0 total should be -990"
    );

    // Line 1: original subtotal=2000, discount=200, taxable=1800, tax=180, total=1980
    let cn1_line1 = &credit_note_1.line_items[1];
    assert_eq!(
        cn1_line1.amount_subtotal, -2000,
        "CN1 line 1 subtotal should be -2000"
    );
    assert_eq!(
        cn1_line1.taxable_amount, -1800,
        "CN1 line 1 taxable should be -1800"
    );
    assert_eq!(cn1_line1.tax_amount, -180, "CN1 line 1 tax should be -180");
    assert_eq!(
        cn1_line1.amount_total, -1980,
        "CN1 line 1 total should be -1980"
    );

    // Credit note 2 should have 2 line items
    assert_eq!(
        credit_note_2.line_items.len(),
        2,
        "Credit note 2 should have 2 line items"
    );

    // Line 2: original subtotal=3000, discount=300, taxable=2700, tax=270, total=2970
    let cn2_line0 = &credit_note_2.line_items[0];
    assert_eq!(
        cn2_line0.amount_subtotal, -3000,
        "CN2 line 0 subtotal should be -3000"
    );
    assert_eq!(
        cn2_line0.taxable_amount, -2700,
        "CN2 line 0 taxable should be -2700"
    );
    assert_eq!(cn2_line0.tax_amount, -270, "CN2 line 0 tax should be -270");
    assert_eq!(
        cn2_line0.amount_total, -2970,
        "CN2 line 0 total should be -2970"
    );

    // Line 3: original subtotal=4000, discount=400, taxable=3600, tax=360, total=3960
    let cn2_line1 = &credit_note_2.line_items[1];
    assert_eq!(
        cn2_line1.amount_subtotal, -4000,
        "CN2 line 1 subtotal should be -4000"
    );
    assert_eq!(
        cn2_line1.taxable_amount, -3600,
        "CN2 line 1 taxable should be -3600"
    );
    assert_eq!(cn2_line1.tax_amount, -360, "CN2 line 1 tax should be -360");
    assert_eq!(
        cn2_line1.amount_total, -3960,
        "CN2 line 1 total should be -3960"
    );

    // 12. Test that we can't credit the same line twice
    let duplicate_result = store
        .create_credit_note(
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![
                    CreditLineItem {
                        local_id: line_ids[0].clone(),
                        amount: None,
                    }, // Already credited
                ],
                reason: Some("Should fail - duplicate".to_string()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
            },
        )
        .await;

    assert!(
        duplicate_result.is_err(),
        "Should not be able to credit the same line twice"
    );

    // 12. List credit notes for the invoice
    let credit_notes = store
        .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
        .await
        .unwrap();

    assert_eq!(credit_notes.len(), 2, "Should have 2 credit notes");
}

/// Test race condition: concurrent credit note creation should not cause issues
#[tokio::test]
async fn test_credit_note_race_condition() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let mock_mailer = Arc::new(MockMailerService::new());
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let store = setup.store.clone();
    let services = setup.services.clone();
    let mut conn = setup.store.pool.get().await.unwrap();

    // Create plan with 4 components
    let (plan_version_id, _) = create_plan_with_4_components(&mut conn).await;

    // Create customer
    let customer_id = create_customer_with_tax(&mut conn, 0).await;

    // Create subscription
    let subscription = services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id,
                    plan_version_id,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
                    end_date: None,
                    billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()),
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // Process billing to finalize invoice
    services.get_and_process_due_events().await.unwrap();

    // Get invoice
    let invoices = store
        .list_invoices(
            TENANT_ID,
            None,
            Some(subscription.id),
            None,
            None,
            OrderByRequest::DateAsc,
            PaginationRequest {
                page: 0,
                per_page: None,
            },
        )
        .await
        .unwrap()
        .items;

    let invoice = &invoices[0].invoice;
    let line_ids: Vec<String> = invoice
        .line_items
        .iter()
        .map(|l| l.local_id.clone())
        .collect();

    // Try to create two credit notes for the same line concurrently
    let store1 = store.clone();
    let store2 = store.clone();
    let invoice_id = invoice.id;
    let line_id = line_ids[0].clone();
    let line_id_clone = line_id.clone();

    let (result1, result2) = tokio::join!(
        async move {
            store1
                .create_credit_note(
                    TENANT_ID,
                    CreateCreditNoteParams {
                        invoice_id,
                        line_items: vec![CreditLineItem {
                            local_id: line_id,
                            amount: None,
                        }],
                        reason: Some("Concurrent 1".to_string()),
                        memo: None,
                        credit_type: CreditType::CreditToBalance,
                    },
                )
                .await
        },
        async move {
            store2
                .create_credit_note(
                    TENANT_ID,
                    CreateCreditNoteParams {
                        invoice_id,
                        line_items: vec![CreditLineItem {
                            local_id: line_id_clone,
                            amount: None,
                        }],
                        reason: Some("Concurrent 2".to_string()),
                        memo: None,
                        credit_type: CreditType::CreditToBalance,
                    },
                )
                .await
        }
    );

    // Only one should succeed
    let successes = [result1.is_ok(), result2.is_ok()]
        .iter()
        .filter(|&&x| x)
        .count();

    assert_eq!(
        successes, 1,
        "Exactly one concurrent credit note should succeed, got {} successes",
        successes
    );

    let error_message = if let Err(err) = if result1.is_err() { &result1 } else { &result2 } {
        format!("{:?}", err)
    } else {
        "No error".to_string()
    };

    assert!(
        error_message.contains("already been fully credited"),
        "The failed credit note should indicate the line was already credited, got error: {}",
        error_message
    );
}

/// Test CreditType::Refund with applied credits
/// When invoice was partially paid with customer balance, refund should:
/// - Restore the balance portion
/// - Only refund the actually paid amount
#[tokio::test]
async fn test_credit_note_refund_with_applied_credits() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let mock_mailer = Arc::new(MockMailerService::new());
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let store = setup.store.clone();
    let services = setup.services.clone();
    let mut conn = setup.store.pool.get().await.unwrap();

    // Create plan with 4 components
    let (plan_version_id, _) = create_plan_with_4_components(&mut conn).await;

    // Create customer with 2000 cents balance
    let customer_id = create_customer_with_tax(&mut conn, 2000).await;

    // Create subscription
    let subscription = services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id,
                    plan_version_id,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
                    end_date: None,
                    billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap()),
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // Get invoice
    let invoices = store
        .list_invoices(
            TENANT_ID,
            None,
            Some(subscription.id),
            None,
            None,
            OrderByRequest::DateAsc,
            PaginationRequest {
                page: 0,
                per_page: None,
            },
        )
        .await
        .unwrap()
        .items;

    assert_eq!(invoices.len(), 1, "Should have a single invoice");

    // Process billing
    // services.get_and_process_cycle_transitions().await.unwrap(); // no need to create the next invoice
    services.get_and_process_due_events().await.unwrap();

    let invoices = store
        .list_invoices(
            TENANT_ID,
            None,
            Some(subscription.id),
            None,
            None,
            OrderByRequest::DateAsc,
            PaginationRequest {
                page: 0,
                per_page: None,
            },
        )
        .await
        .unwrap()
        .items;

    assert_eq!(invoices.len(), 1, "Should have a single invoice");

    let customer_balance = store
        .find_customer_by_id(customer_id, TENANT_ID)
        .await
        .unwrap()
        .balance_value_cents;

    assert_eq!(
        customer_balance, 0,
        "Customer should have updated balance of zero"
    );

    // Get invoice
    let invoices = store
        .list_invoices(
            TENANT_ID,
            None,
            Some(subscription.id),
            None,
            None,
            OrderByRequest::DateAsc,
            PaginationRequest {
                page: 0,
                per_page: None,
            },
        )
        .await
        .unwrap()
        .items;

    let invoice = &invoices[0].invoice;

    let _res = services
        .mark_invoice_as_paid(
            TENANT_ID,
            invoice.id,
            dec!(80.00), // amount due after applying 20€ credits from 100€ plan
            invoice.created_at,
            None,
        )
        .await
        .unwrap();

    run_once_outbox_dispatch(Arc::new(store.clone())).await;

    run_once_invoice_orchestration(Arc::new(store.clone()), Arc::new(services.clone())).await;

    // Invoice should have used applied_credits
    log::info!(
        "Invoice: total={}, applied_credits={}, amount_due={}",
        invoice.total,
        invoice.applied_credits,
        invoice.amount_due
    );

    // Total should be 10000 (no discount or tax in this test since no coupon and no tax resolver)
    // applied_credits should be 2000 (customer balance)
    // amount_due should be 8000
    assert_eq!(invoice.total, 10000, "Invoice total should be 10000");
    assert_eq!(
        invoice.applied_credits, 2000,
        "Applied credits should be 2000"
    );
    assert_eq!(invoice.amount_due, 8000, "Amount due should be 8000");

    // Create credit note with CreditType::Refund
    let credit_note = store
        .create_credit_note(
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![], // All lines with full amounts
                reason: Some("Full refund with applied credits".to_string()),
                memo: None,
                credit_type: CreditType::Refund,
            },
        )
        .await
        .unwrap();

    log::info!(
        "Credit note: credited={}, refunded={}, status={:?}",
        credit_note.credited_amount_cents,
        credit_note.refunded_amount_cents,
        credit_note.status
    );

    // For a refund:
    // - credited_amount_cents should be the amount that was paid from balance (to restore it)
    // - refunded_amount_cents should be the amount actually paid (amount_due)
    //
    // Total credit = 10000, but:
    // - 2000 was from customer balance -> should be credited back
    // - 8000 was actually paid -> should be refunded
    assert_eq!(
        credit_note.credited_amount_cents, 2000,
        "Credited amount should restore the applied credits (2000)"
    );
    assert_eq!(
        credit_note.refunded_amount_cents, 8000,
        "Refunded amount should be the amount actually paid (8000)"
    );

    assert_eq!(
        credit_note.status,
        meteroid_store::domain::enums::CreditNoteStatus::Draft,
        "Credit note should be draft until processed"
    );

    let customer_balance = store
        .find_customer_by_id(customer_id, TENANT_ID)
        .await
        .unwrap()
        .balance_value_cents;

    assert_eq!(
        customer_balance, 0,
        "Customer should have balance of 0 cents before finalize"
    );

    let finalized = store
        .finalize_credit_note(TENANT_ID, credit_note.id)
        .await
        .unwrap();

    assert_eq!(
        finalized.credited_amount_cents, 2000,
        "Credited amount should restore the applied credits (2000)"
    );
    assert_eq!(
        finalized.refunded_amount_cents, 8000,
        "Refunded amount should be the amount actually paid (8000)"
    );

    assert_eq!(
        finalized.status,
        meteroid_store::domain::enums::CreditNoteStatus::Finalized,
        "Credit note should be finalized"
    );

    let customer_balance = store
        .find_customer_by_id(customer_id, TENANT_ID)
        .await
        .unwrap()
        .balance_value_cents;

    assert_eq!(
        customer_balance, 2000,
        "Customer should have final balance of 2000 cents"
    );
}

/// Test credit notes with partial amounts (crediting less than full line item amount)
#[tokio::test]
async fn test_credit_note_partial_amounts() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;

    let mock_mailer = Arc::new(MockMailerService::new());
    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        Arc::new(MockUsageClient::noop()),
        mock_mailer.clone(),
    )
    .await;

    let services = setup.services.clone();
    let store = setup.store.clone();
    let mut conn = setup.store.pool.get().await.unwrap();

    log::info!(">>> Testing credit notes with partial amounts");

    // 1. Set up manual tax resolver with 10% tax
    store
        .patch_invoicing_entity(
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(meteroid_store::domain::enums::TaxResolverEnum::Manual),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // 2. Create a plan with 4 price components
    let (plan_version_id, _) = create_plan_with_4_components(&mut conn).await;

    // 3. Create customer with custom tax rate (10%)
    let customer_id = create_customer_with_tax(&mut conn, 0).await;

    // 4. Create subscription
    let subscription = services
        .insert_subscription(
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id,
                    plan_version_id,
                    created_by: USER_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: NaiveDate::from_ymd_opt(2024, 4, 1).unwrap(),
                    end_date: None,
                    billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 4, 1).unwrap()),
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
            },
            TENANT_ID,
        )
        .await
        .unwrap();

    // 5. Process billing events to finalize invoice
    services.get_and_process_due_events().await.unwrap();

    // 6. Get the invoice
    let invoices = store
        .list_invoices(
            TENANT_ID,
            None,
            Some(subscription.id),
            None,
            None,
            OrderByRequest::DateAsc,
            PaginationRequest {
                page: 0,
                per_page: None,
            },
        )
        .await
        .unwrap()
        .items;

    assert_eq!(invoices.len(), 1, "Should have one invoice");
    let invoice = &invoices[0].invoice;
    assert_eq!(
        invoice.status,
        InvoiceStatusEnum::Finalized,
        "Invoice should be finalized"
    );

    // Invoice has 4 line items:
    // Line 0: subtotal=1000, tax=100, total=1100
    // Line 1: subtotal=2000, tax=200, total=2200
    // Line 2: subtotal=3000, tax=300, total=3300
    // Line 3: subtotal=4000, tax=400, total=4400
    // Total: 10000 + 1000 tax = 11000 (no discount in this test)
    let line_ids: Vec<String> = invoice
        .line_items
        .iter()
        .map(|l| l.local_id.clone())
        .collect();

    log::info!("Line items:");
    for line in &invoice.line_items {
        log::info!(
            "  {}: subtotal={}, tax={}, total={}",
            line.local_id,
            line.amount_subtotal,
            line.tax_amount,
            line.amount_total
        );
    }

    // 7. Create a credit note with partial amounts (EXCLUDING TAX):
    // - Line 0: credit only 500 (half of subtotal 1000)
    // - Line 1: credit full amount
    let credit_note = store
        .create_credit_note(
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![
                    CreditLineItem {
                        local_id: line_ids[0].clone(),
                        amount: Some(500),
                    }, // Half of subtotal
                    CreditLineItem {
                        local_id: line_ids[1].clone(),
                        amount: None,
                    }, // Full
                ],
                reason: Some("Partial amount credit test".to_string()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
            },
        )
        .await
        .unwrap();

    log::info!(
        "Credit Note: subtotal={}, tax={}, total={}, credited={}",
        credit_note.subtotal,
        credit_note.tax_amount,
        credit_note.total,
        credit_note.credited_amount_cents
    );

    // Verify credit note totals
    // Line 0 partial (500 = 50% of subtotal 1000):
    //   - subtotal: 500 -> -500
    //   - tax: 100 * 0.5 = 50 -> -50
    //   - total: 500 + 50 = 550 -> -550
    // Line 1 full:
    //   - subtotal: -2000
    //   - tax: -200
    //   - total: -2200
    // Combined:
    //   - subtotal: -500 + -2000 = -2500
    //   - tax: -50 + -200 = -250
    //   - total: -550 + -2200 = -2750

    assert_eq!(
        credit_note.total, -2750,
        "Credit note total should be -2750"
    );
    assert_eq!(
        credit_note.credited_amount_cents, 2750,
        "Credit note credited amount should be 2750"
    );

    // Verify line items
    assert_eq!(credit_note.line_items.len(), 2, "Should have 2 line items");

    // Line 0: partial credit (500 subtotal excl. tax)
    let cn_line0 = &credit_note.line_items[0];
    assert_eq!(
        cn_line0.amount_subtotal, -500,
        "CN line 0 subtotal should be -500"
    );
    // Tax is prorated: 100 * (500/1000) = 50
    assert_eq!(cn_line0.tax_amount, -50, "CN line 0 tax should be -50");
    // Total = subtotal + tax
    assert_eq!(
        cn_line0.amount_total, -550,
        "CN line 0 total should be -550 (partial)"
    );
    // For partial credits, quantity=1 and unit_price=credited_subtotal
    assert_eq!(
        cn_line0.quantity,
        Some(dec!(1)),
        "CN line 0 quantity should be 1"
    );
    assert_eq!(
        cn_line0.unit_price,
        Some(dec!(-5.00)),
        "CN line 0 unit_price should be -5.00 (-500 cents)"
    );

    // Line 1: full credit (keeps original quantity/unit_price)
    let cn_line1 = &credit_note.line_items[1];
    assert_eq!(
        cn_line1.amount_subtotal, -2000,
        "CN line 1 subtotal should be -2000"
    );
    assert_eq!(cn_line1.tax_amount, -200, "CN line 1 tax should be -200");
    assert_eq!(
        cn_line1.amount_total, -2200,
        "CN line 1 total should be -2200 (full)"
    );

    // 8. Test validation: amount exceeds original subtotal
    let exceed_result = store
        .create_credit_note(
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![
                    CreditLineItem {
                        local_id: line_ids[2].clone(),
                        amount: Some(9999),
                    }, // Exceeds subtotal 3000
                ],
                reason: Some("Should fail - exceeds subtotal".to_string()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
            },
        )
        .await;

    assert!(
        exceed_result.is_err(),
        "Should not be able to credit more than line item subtotal"
    );

    let error_message = format!("{:?}", exceed_result.unwrap_err());
    assert!(
        error_message.contains("exceeds"),
        "Error should mention exceeding amount, got: {}",
        error_message
    );

    // 9. Test validation: negative amount
    let negative_result = store
        .create_credit_note(
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![CreditLineItem {
                    local_id: line_ids[2].clone(),
                    amount: Some(-100),
                }],
                reason: Some("Should fail - negative amount".to_string()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
            },
        )
        .await;

    assert!(
        negative_result.is_err(),
        "Should not be able to credit a negative amount"
    );

    let error_message = format!("{:?}", negative_result.unwrap_err());
    assert!(
        error_message.contains("positive"),
        "Error should mention positive amount, got: {}",
        error_message
    );

    log::info!(">>> Partial amount credit note test passed!");
}

// =============================================================================
// Helper functions
// =============================================================================

/// Create a plan with 4 price components for testing (totalling 100€)
async fn create_plan_with_4_components(
    conn: &mut PgConn,
) -> (PlanVersionId, Vec<PriceComponentId>) {
    use diesel_async::AsyncConnection;
    use diesel_async::scoped_futures::ScopedFutureExt;
    use diesel_models::errors::DatabaseErrorContainer;

    let plan_id = common_domain::ids::PlanId::new();
    let plan_version_id = common_domain::ids::PlanVersionId::new();

    let component_ids: Vec<PriceComponentId> = (0..4)
        .map(|_| common_domain::ids::PriceComponentId::new())
        .collect();

    conn.transaction(|tx| {
        let component_ids = component_ids.clone();
        async move {
            PlanRowNew {
                id: plan_id,
                name: "CreditNoteTestPlan".to_string(),
                description: Some("Plan for credit note testing".to_string()),
                created_by: USER_ID,
                tenant_id: TENANT_ID,
                product_family_id: PRODUCT_FAMILY_ID,
                plan_type: PlanTypeEnum::Standard,
                status: PlanStatusEnum::Active,
            }
            .insert(tx)
            .await?;

            PlanVersionRowNew {
                id: plan_version_id,
                is_draft_version: false,
                plan_id,
                version: 1,
                trial_duration_days: None,
                tenant_id: TENANT_ID,
                period_start_day: None,
                net_terms: 0,
                currency: "EUR".to_string(),
                billing_cycles: None,
                created_by: USER_ID,
                trialing_plan_id: None,
                trial_is_free: false,
                uses_product_pricing: true,
            }
            .insert(tx)
            .await?;

            PlanRowPatch {
                id: plan_id,
                tenant_id: TENANT_ID,
                name: None,
                description: None,
                active_version_id: Some(Some(plan_version_id)),
                draft_version_id: None,
            }
            .update(tx)
            .await?;

            // Create 4 components with prices: 1000, 2000, 3000, 4000 cents
            for (i, component_id) in component_ids.iter().enumerate() {
                let price = rust_decimal::Decimal::new(((i + 1) * 1000) as i64, 2);
                let product_id = common_domain::ids::ProductId::new();
                ProductRowNew {
                    id: product_id,
                    name: format!("Component {} Product", i + 1),
                    description: None,
                    created_by: USER_ID,
                    tenant_id: TENANT_ID,
                    product_family_id: PRODUCT_FAMILY_ID,
                    fee_type: DieselFeeTypeEnum::Rate,
                    fee_structure: serde_json::to_value(&meteroid_store::domain::prices::FeeStructure::Rate {}).unwrap(),
                }
                .insert(tx)
                .await?;

                PriceComponentRowNew {
                    id: *component_id,
                    name: format!("Component {}", i + 1),
                    legacy_fee: Some(FeeType::Rate {
                        rates: vec![TermRate {
                            price,
                            term: BillingPeriodEnum::Monthly,
                        }],
                    }
                    .try_into()
                    .unwrap()),
                    plan_version_id,
                    product_id: Some(product_id),
                    billable_metric_id: None,
                }
                .insert(tx)
                .await?;
            }

            Ok::<(), DatabaseErrorContainer>(())
        }
        .scope_boxed()
    })
    .await
    .unwrap();

    (plan_version_id, component_ids)
}

/// Create a customer with custom tax rate and optional balance
async fn create_customer_with_tax(conn: &mut PgConn, balance_cents: i64) -> CustomerId {
    use diesel_models::customers::CustomerRowNew;

    let customer_id = CustomerId::from_proto(Uuid::new_v4().to_string()).unwrap();

    let address = Address {
        line1: Some("123 Test Street".to_string()),
        line2: None,
        city: Some("Test City".to_string()),
        country: Some(CountryCode::from_str("FR").expect("failed to parse country code")),
        state: Some("Test State".to_string()),
        zip_code: Some("12345".to_string()),
    };

    let customer_row = CustomerRowNew {
        id: customer_id,
        name: format!("Credit Note Test Customer {}", Uuid::new_v4()),
        created_at: Some(chrono::Utc::now().naive_utc()),
        created_by: USER_ID,
        tenant_id: TENANT_ID,
        alias: None,
        balance_value_cents: balance_cents,
        currency: "EUR".to_string(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(serde_json::to_value(&address).unwrap()),
        shipping_address: None,
        billing_email: None,
        current_payment_method_id: None,
        vat_number: Some("FR12345678901".to_string()),
        custom_taxes: serde_json::to_value(vec![CustomerCustomTax {
            tax_code: "vat".to_string(),
            name: "VAT".to_string(),
            rate: dec!(0.10), // 10% tax
        }])
        .unwrap(),
        invoicing_emails: vec![],
        phone: None,
        is_tax_exempt: false,
        vat_number_format_valid: true,
    };

    customer_row.insert(conn).await.unwrap();

    customer_id
}
